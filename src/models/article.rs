use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Article {
    pub id: i64,
    pub feed_id: i64,
    pub guid: String,
    pub title: String,
    pub url: String,
    pub author: Option<String>,
    pub content: Option<String>,
    pub content_text: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub fetched_at: DateTime<Utc>,
    pub feed_title: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewArticle {
    pub feed_id: i64,
    pub guid: String,
    pub title: String,
    pub url: String,
    pub author: Option<String>,
    pub content: Option<String>,
    pub content_text: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
}
