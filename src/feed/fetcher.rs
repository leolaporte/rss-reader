use std::time::Duration;

use feed_rs::parser;
use futures::stream::{self, StreamExt};
use regex::Regex;
use reqwest::Client;

use crate::error::Result;
use crate::models::{Feed, NewArticle, NewFeed};

#[derive(Clone)]
pub struct FeedFetcher {
    client: Client,
}

impl FeedFetcher {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .user_agent("beatcheck/1.2.0")
            .build()
            .expect("Failed to create HTTP client");

        Self { client }
    }

    pub async fn fetch_feed(&self, feed_id: i64, url: &str) -> Result<Vec<NewArticle>> {
        let response = self.client.get(url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!("Failed to fetch feed: HTTP {}", response.status()).into());
        }

        let bytes = response.bytes().await?;
        let feed = parser::parse(&bytes[..])?;

        let articles: Vec<NewArticle> = feed
            .entries
            .into_iter()
            .map(|entry| {
                // Try content first, then fall back to summary
                let content_html = entry
                    .content
                    .as_ref()
                    .and_then(|c| c.body.as_ref())
                    .or_else(|| entry.summary.as_ref().map(|s| &s.content));

                let content_text = content_html.and_then(|html| {
                    html2text::from_read(html.as_bytes(), 80).ok()
                });

                NewArticle {
                    feed_id,
                    guid: entry.id,
                    title: entry
                        .title
                        .map(|t| t.content)
                        .unwrap_or_else(|| "Untitled".to_string()),
                    url: entry
                        .links
                        .first()
                        .map(|l| l.href.clone())
                        .unwrap_or_default(),
                    author: entry.authors.first().map(|a| a.name.clone()),
                    content: content_html.cloned(),
                    content_text,
                    published_at: entry.published.or(entry.updated),
                }
            })
            .collect();

        Ok(articles)
    }

    /// Refresh all feeds concurrently with rate limiting
    pub async fn refresh_all(&self, feeds: Vec<Feed>) -> Vec<(i64, Vec<NewArticle>)> {
        let results: Vec<_> = stream::iter(feeds)
            .map(|feed| async move {
                match self.fetch_feed(feed.id, &feed.url).await {
                    Ok(articles) => {
                        tracing::debug!("Fetched {} articles from {}", articles.len(), feed.title);
                        Some((feed.id, articles))
                    }
                    Err(e) => {
                        tracing::debug!("Failed to fetch {}: {}", feed.url, e);
                        None
                    }
                }
            })
            .buffer_unordered(5) // Max 5 concurrent fetches
            .filter_map(|r| async { r })
            .collect()
            .await;

        results
    }

    /// Discover and create a feed from a URL
    /// If the URL is a direct RSS/Atom feed, parse it directly
    /// If it's an HTML page, look for feed links in <link> tags
    pub async fn discover_feed(&self, url: &str) -> Result<NewFeed> {
        let response = self.client.get(url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!("Failed to fetch URL: HTTP {}", response.status()).into());
        }

        let final_url = response.url().to_string();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let bytes = response.bytes().await?;

        // Try parsing as RSS/Atom feed first
        if let Ok(feed) = parser::parse(&bytes[..]) {
            let title = feed
                .title
                .map(|t| t.content)
                .unwrap_or_else(|| "Untitled Feed".to_string());
            let description = feed.description.map(|d| d.content);
            let site_url = feed.links.first().map(|l| l.href.clone());

            return Ok(NewFeed {
                title,
                url: final_url,
                site_url,
                description,
            });
        }

        // If content looks like HTML, search for feed links
        if content_type.contains("html") || bytes.starts_with(b"<!") || bytes.starts_with(b"<html") {
            let html = String::from_utf8_lossy(&bytes);
            if let Some(feed_url) = self.find_feed_link(&html, &final_url) {
                // Fetch the discovered feed URL
                let feed_response = self.client.get(&feed_url).send().await?;
                if feed_response.status().is_success() {
                    let feed_bytes = feed_response.bytes().await?;
                    if let Ok(feed) = parser::parse(&feed_bytes[..]) {
                        let title = feed
                            .title
                            .map(|t| t.content)
                            .unwrap_or_else(|| "Untitled Feed".to_string());
                        let description = feed.description.map(|d| d.content);
                        let site_url = feed.links.first().map(|l| l.href.clone());

                        return Ok(NewFeed {
                            title,
                            url: feed_url,
                            site_url,
                            description,
                        });
                    }
                }
            }
        }

        Err(anyhow::anyhow!("Could not find RSS/Atom feed at this URL").into())
    }

    /// Search HTML for RSS/Atom feed links
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) fn find_feed_link(&self, html: &str, base_url: &str) -> Option<String> {
        // Look for <link rel="alternate" type="application/rss+xml" href="...">
        // or <link rel="alternate" type="application/atom+xml" href="...">
        let link_re = Regex::new(
            r#"<link[^>]*rel=["']alternate["'][^>]*type=["']application/(rss|atom)\+xml["'][^>]*href=["']([^"']+)["']"#
        ).ok()?;

        // Also try reverse order (type before rel)
        let link_re2 = Regex::new(
            r#"<link[^>]*type=["']application/(rss|atom)\+xml["'][^>]*href=["']([^"']+)["']"#
        ).ok()?;

        let href = link_re
            .captures(html)
            .or_else(|| link_re2.captures(html))
            .and_then(|cap: regex::Captures| cap.get(2))
            .map(|m: regex::Match| m.as_str().to_string())?;

        // Resolve relative URLs
        Some(self.resolve_url(&href, base_url))
    }

    /// Resolve a potentially relative URL against a base URL
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) fn resolve_url(&self, href: &str, base_url: &str) -> String {
        if href.starts_with("http://") || href.starts_with("https://") {
            return href.to_string();
        }

        if let Ok(base) = url::Url::parse(base_url) {
            if let Ok(resolved) = base.join(href) {
                return resolved.to_string();
            }
        }

        href.to_string()
    }
}

impl Default for FeedFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fetcher() -> FeedFetcher {
        FeedFetcher::new()
    }

    // ==================== resolve_url tests ====================

    #[test]
    fn test_resolve_absolute_http_url() {
        let f = fetcher();
        let result = f.resolve_url("http://example.com/feed.xml", "https://base.com/page");
        assert_eq!(result, "http://example.com/feed.xml");
    }

    #[test]
    fn test_resolve_absolute_https_url() {
        let f = fetcher();
        let result = f.resolve_url("https://example.com/feed.xml", "https://base.com/page");
        assert_eq!(result, "https://example.com/feed.xml");
    }

    #[test]
    fn test_resolve_relative_path() {
        let f = fetcher();
        let result = f.resolve_url("/feed.xml", "https://example.com/blog/post");
        assert_eq!(result, "https://example.com/feed.xml");
    }

    #[test]
    fn test_resolve_relative_path_no_leading_slash() {
        let f = fetcher();
        let result = f.resolve_url("feed.xml", "https://example.com/blog/");
        assert_eq!(result, "https://example.com/blog/feed.xml");
    }

    #[test]
    fn test_resolve_relative_parent_path() {
        let f = fetcher();
        let result = f.resolve_url("../feed.xml", "https://example.com/blog/posts/");
        assert_eq!(result, "https://example.com/blog/feed.xml");
    }

    #[test]
    fn test_resolve_protocol_relative_url() {
        let f = fetcher();
        // Protocol-relative URLs start with // - not http:// or https://
        // They should be resolved against the base URL's protocol
        let result = f.resolve_url("//cdn.example.com/feed.xml", "https://example.com/page");
        assert_eq!(result, "https://cdn.example.com/feed.xml");
    }

    #[test]
    fn test_resolve_with_query_string() {
        let f = fetcher();
        let result = f.resolve_url("/feed.xml?format=rss", "https://example.com/blog");
        assert_eq!(result, "https://example.com/feed.xml?format=rss");
    }

    #[test]
    fn test_resolve_invalid_base_returns_href() {
        let f = fetcher();
        let result = f.resolve_url("/feed.xml", "not-a-valid-url");
        assert_eq!(result, "/feed.xml");
    }

    // ==================== find_feed_link tests ====================

    #[test]
    fn test_find_rss_link_standard_order() {
        let f = fetcher();
        let html = r#"
            <html>
            <head>
                <link rel="alternate" type="application/rss+xml" href="/feed.xml" title="RSS Feed">
            </head>
            </html>
        "#;
        let result = f.find_feed_link(html, "https://example.com");
        assert_eq!(result, Some("https://example.com/feed.xml".to_string()));
    }

    #[test]
    fn test_find_atom_link() {
        let f = fetcher();
        let html = r#"
            <html>
            <head>
                <link rel="alternate" type="application/atom+xml" href="/atom.xml">
            </head>
            </html>
        "#;
        let result = f.find_feed_link(html, "https://example.com");
        assert_eq!(result, Some("https://example.com/atom.xml".to_string()));
    }

    #[test]
    fn test_find_feed_link_type_before_rel() {
        let f = fetcher();
        // Some sites put type before rel
        let html = r#"
            <html>
            <head>
                <link type="application/rss+xml" rel="alternate" href="/rss">
            </head>
            </html>
        "#;
        let result = f.find_feed_link(html, "https://example.com");
        assert_eq!(result, Some("https://example.com/rss".to_string()));
    }

    #[test]
    fn test_find_feed_link_absolute_url() {
        let f = fetcher();
        let html = r#"
            <link rel="alternate" type="application/rss+xml" href="https://feeds.example.com/main.xml">
        "#;
        let result = f.find_feed_link(html, "https://example.com");
        assert_eq!(result, Some("https://feeds.example.com/main.xml".to_string()));
    }

    #[test]
    fn test_find_feed_link_single_quotes() {
        let f = fetcher();
        let html = r#"
            <link rel='alternate' type='application/rss+xml' href='/feed'>
        "#;
        let result = f.find_feed_link(html, "https://example.com");
        assert_eq!(result, Some("https://example.com/feed".to_string()));
    }

    #[test]
    fn test_find_feed_link_no_feed() {
        let f = fetcher();
        let html = r#"
            <html>
            <head>
                <link rel="stylesheet" href="/style.css">
                <link rel="icon" href="/favicon.ico">
            </head>
            </html>
        "#;
        let result = f.find_feed_link(html, "https://example.com");
        assert_eq!(result, None);
    }

    #[test]
    fn test_find_feed_link_empty_html() {
        let f = fetcher();
        let result = f.find_feed_link("", "https://example.com");
        assert_eq!(result, None);
    }

    #[test]
    fn test_find_feed_link_with_title_attribute() {
        let f = fetcher();
        let html = r#"
            <link rel="alternate" type="application/rss+xml" title="My Blog RSS" href="/blog/feed">
        "#;
        let result = f.find_feed_link(html, "https://example.com");
        assert_eq!(result, Some("https://example.com/blog/feed".to_string()));
    }

    #[test]
    fn test_find_feed_link_complex_html() {
        let f = fetcher();
        // Real-world-ish HTML with multiple links
        let html = r#"
            <!DOCTYPE html>
            <html>
            <head>
                <meta charset="utf-8">
                <title>My Site</title>
                <link rel="stylesheet" href="/css/main.css">
                <link rel="icon" type="image/x-icon" href="/favicon.ico">
                <link rel="alternate" type="application/rss+xml" title="RSS" href="https://mysite.com/rss.xml">
                <link rel="alternate" type="application/atom+xml" title="Atom" href="/atom.xml">
            </head>
            <body>Content</body>
            </html>
        "#;
        // Should find the first feed link (RSS in this case)
        let result = f.find_feed_link(html, "https://mysite.com");
        assert_eq!(result, Some("https://mysite.com/rss.xml".to_string()));
    }

    #[test]
    fn test_find_feed_link_href_before_type_not_supported() {
        let f = fetcher();
        // href attribute appears before type - current implementation doesn't handle this
        // This documents the limitation; most real sites use standard attribute order
        let html = r#"
            <link href="/feed.xml" type="application/rss+xml" rel="alternate">
        "#;
        let result = f.find_feed_link(html, "https://example.com");
        // Current regex doesn't match this order - returns None
        assert_eq!(result, None);
    }
}
