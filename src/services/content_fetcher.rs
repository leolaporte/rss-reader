use std::time::Duration;
use reqwest::header::{HeaderMap, HeaderValue, COOKIE, USER_AGENT};
use reqwest::Client;
use rusqlite::params;
use url::Url;

use crate::error::Result;

const USER_AGENT_STRING: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";

pub struct ContentFetcher {
    client: Client,
}

impl ContentFetcher {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");
        Self { client }
    }

    /// Fetch full article content using browser cookies
    pub async fn fetch_full_content(&self, article_url: &str) -> Result<Option<String>> {
        let url = match Url::parse(article_url) {
            Ok(u) => u,
            Err(_) => return Ok(None),
        };

        let domain = match url.host_str() {
            Some(d) => d,
            None => return Ok(None),
        };

        // Get cookies for this domain from Chrome
        let cookies = self.get_chrome_cookies(domain)?;

        // Build request with cookies
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_STRING));

        if !cookies.is_empty() {
            if let Ok(cookie_header) = HeaderValue::from_str(&cookies) {
                headers.insert(COOKIE, cookie_header);
            }
        }

        // Fetch the page
        let response = self
            .client
            .get(article_url)
            .headers(headers)
            .send()
            .await?;

        if !response.status().is_success() {
            tracing::debug!("Failed to fetch {}: {}", article_url, response.status());
            return Ok(None);
        }

        let html = response.text().await?;

        // Extract readable content
        let content = self.extract_content(&html, article_url);

        Ok(content)
    }

    /// Read cookies from Chrome or Firefox for a given domain
    fn get_chrome_cookies(&self, domain: &str) -> Result<String> {
        // Try Chrome first
        if let Ok(cookies) = self.get_chrome_cookies_internal(domain) {
            if !cookies.is_empty() {
                return Ok(cookies);
            }
        }

        // Fall back to Firefox
        self.get_firefox_cookies_internal(domain)
    }

    fn get_chrome_cookies_internal(&self, domain: &str) -> Result<String> {
        // Try Chrome, then Chromium
        let chrome_paths = vec![
            dirs::home_dir().map(|h| h.join(".config/google-chrome/Default/Cookies")),
            dirs::home_dir().map(|h| h.join(".config/chromium/Default/Cookies")),
        ];

        let cookies_db = chrome_paths
            .into_iter()
            .flatten()
            .find(|p| p.exists());

        let cookies_db = match cookies_db {
            Some(db) => db,
            None => {
                tracing::debug!("No Chrome/Chromium cookies found");
                return Ok(String::new());
            }
        };

        // Chrome locks the database, so we need to copy it first
        let temp_db = std::env::temp_dir().join("beatcheck-chrome-cookies.sqlite");
        if let Err(e) = std::fs::copy(&cookies_db, &temp_db) {
            tracing::debug!("Failed to copy Chrome cookies database: {}", e);
            return Ok(String::new());
        }

        let conn = match rusqlite::Connection::open(&temp_db) {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!("Failed to open Chrome cookies database: {}", e);
                return Ok(String::new());
            }
        };

        // Current time in Chrome's timestamp format (microseconds since 1601-01-01)
        // Chrome uses Windows FILETIME epoch, which is 11,644,473,600 seconds before Unix epoch
        let now = (chrono::Utc::now().timestamp() + 11_644_473_600) * 1_000_000;

        // Query cookies for this domain (including subdomains)
        let mut stmt = conn.prepare(
            "SELECT name, value FROM cookies
             WHERE (host_key = ?1 OR host_key LIKE ?2)
             AND expires_utc > ?3
             AND name != '' AND value != ''",
        )?;

        let domain_pattern = format!(".{}", domain);

        let cookies: Vec<String> = stmt
            .query_map(params![domain, domain_pattern, now], |row| {
                let name: String = row.get(0)?;
                let value: String = row.get(1)?;
                Ok(format!("{}={}", name, value))
            })?
            .filter_map(|r| r.ok())
            .collect();

        // Clean up temp file
        let _ = std::fs::remove_file(&temp_db);

        Ok(cookies.join("; "))
    }

    fn get_firefox_cookies_internal(&self, domain: &str) -> Result<String> {
        let firefox_path = match Self::find_firefox_cookies() {
            Some(path) => path,
            None => {
                tracing::debug!("No Firefox cookies found");
                return Ok(String::new());
            }
        };

        // Firefox locks the database, so we need to copy it first
        let temp_db = std::env::temp_dir().join("beatcheck-firefox-cookies.sqlite");
        if let Err(e) = std::fs::copy(&firefox_path, &temp_db) {
            tracing::debug!("Failed to copy Firefox cookies database: {}", e);
            return Ok(String::new());
        }

        let conn = match rusqlite::Connection::open(&temp_db) {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!("Failed to open Firefox cookies database: {}", e);
                return Ok(String::new());
            }
        };

        // Current time in Unix timestamp (seconds) - Firefox uses standard Unix epoch
        let now = chrono::Utc::now().timestamp();

        // Query cookies for this domain (including subdomains)
        let mut stmt = conn.prepare(
            "SELECT name, value FROM moz_cookies
             WHERE (host = ?1 OR host LIKE ?2)
             AND expiry > ?3
             AND name != '' AND value != ''",
        )?;

        let domain_pattern = format!(".{}", domain);

        let cookies: Vec<String> = stmt
            .query_map(params![domain, domain_pattern, now], |row| {
                let name: String = row.get(0)?;
                let value: String = row.get(1)?;
                Ok(format!("{}={}", name, value))
            })?
            .filter_map(|r| r.ok())
            .collect();

        // Clean up temp file
        let _ = std::fs::remove_file(&temp_db);

        Ok(cookies.join("; "))
    }

    fn find_firefox_cookies() -> Option<std::path::PathBuf> {
        let home = dirs::home_dir()?;
        let firefox_dir = home.join(".mozilla/firefox");

        if !firefox_dir.exists() {
            return None;
        }

        // Look for profiles.ini to find the default profile
        let profiles_ini = firefox_dir.join("profiles.ini");
        if profiles_ini.exists() {
            if let Ok(content) = std::fs::read_to_string(&profiles_ini) {
                let mut current_path: Option<String> = None;
                let mut is_default = false;

                for line in content.lines() {
                    if line.starts_with("Path=") {
                        current_path = Some(line.trim_start_matches("Path=").to_string());
                    }
                    if line == "Default=1" {
                        is_default = true;
                    }
                    if line.starts_with('[') && line != "[General]" {
                        if is_default {
                            if let Some(path) = current_path {
                                let profile_dir = firefox_dir.join(&path);
                                let cookies_path = profile_dir.join("cookies.sqlite");
                                if cookies_path.exists() {
                                    return Some(cookies_path);
                                }
                            }
                        }
                        current_path = None;
                        is_default = false;
                    }
                }

                // Check last section
                if is_default {
                    if let Some(path) = current_path {
                        let profile_dir = firefox_dir.join(&path);
                        let cookies_path = profile_dir.join("cookies.sqlite");
                        if cookies_path.exists() {
                            return Some(cookies_path);
                        }
                    }
                }
            }
        }

        // Fallback: find any profile with cookies.sqlite
        if let Ok(entries) = std::fs::read_dir(&firefox_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let cookies_path = path.join("cookies.sqlite");
                    if cookies_path.exists() {
                        return Some(cookies_path);
                    }
                }
            }
        }

        None
    }


    /// Extract readable content from HTML using html2text
    fn extract_content(&self, html: &str, _url: &str) -> Option<String> {
        // Use html2text to convert HTML to plain text
        // This avoids the html5ever namespace warnings from readability
        let text = match html2text::from_read(html.as_bytes(), 80) {
            Ok(t) => t,
            Err(e) => {
                tracing::debug!("Failed to convert HTML to text: {}", e);
                return None;
            }
        };

        // Clean up the text - remove excessive whitespace
        let cleaned: String = text
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        if cleaned.len() > 200 {
            Some(cleaned)
        } else {
            tracing::debug!("Extracted content too short ({} chars)", cleaned.len());
            None
        }
    }
}

impl Default for ContentFetcher {
    fn default() -> Self {
        Self::new()
    }
}
