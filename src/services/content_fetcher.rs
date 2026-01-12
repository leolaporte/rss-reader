use std::path::PathBuf;
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

        // Get cookies for this domain from Firefox
        let cookies = self.get_firefox_cookies(domain)?;

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

    /// Read cookies from Firefox for a given domain
    fn get_firefox_cookies(&self, domain: &str) -> Result<String> {
        let firefox_dir = match Self::find_firefox_profile() {
            Some(dir) => dir,
            None => {
                tracing::debug!("No Firefox profile found");
                return Ok(String::new());
            }
        };

        let cookies_db = firefox_dir.join("cookies.sqlite");
        if !cookies_db.exists() {
            tracing::debug!("Firefox cookies.sqlite not found");
            return Ok(String::new());
        }

        // Firefox locks the database, so we need to copy it first
        let temp_db = std::env::temp_dir().join("rss-reader-cookies.sqlite");
        if let Err(e) = std::fs::copy(&cookies_db, &temp_db) {
            tracing::debug!("Failed to copy cookies database: {}", e);
            return Ok(String::new());
        }

        let conn = match rusqlite::Connection::open(&temp_db) {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!("Failed to open cookies database: {}", e);
                return Ok(String::new());
            }
        };

        // Query cookies for this domain (including subdomains)
        let mut stmt = conn.prepare(
            "SELECT name, value FROM moz_cookies WHERE host LIKE ?1 OR host LIKE ?2",
        )?;

        let domain_pattern = format!("%{}", domain);
        let exact_domain = domain.to_string();

        let cookies: Vec<String> = stmt
            .query_map(params![domain_pattern, exact_domain], |row| {
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

    /// Find the default Firefox profile directory
    fn find_firefox_profile() -> Option<PathBuf> {
        let home = dirs::home_dir()?;

        // Check common Firefox profile locations
        let firefox_dir = home.join(".mozilla/firefox");
        if !firefox_dir.exists() {
            return None;
        }

        // Look for profiles.ini to find the default profile
        let profiles_ini = firefox_dir.join("profiles.ini");
        if profiles_ini.exists() {
            if let Ok(content) = std::fs::read_to_string(&profiles_ini) {
                // Find the default profile path
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
                                let profile_dir = firefox_dir.join(path);
                                if profile_dir.exists() {
                                    return Some(profile_dir);
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
                        let profile_dir = firefox_dir.join(path);
                        if profile_dir.exists() {
                            return Some(profile_dir);
                        }
                    }
                }
            }
        }

        // Fallback: find any profile directory with cookies.sqlite
        if let Ok(entries) = std::fs::read_dir(&firefox_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && path.join("cookies.sqlite").exists() {
                    return Some(path);
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
