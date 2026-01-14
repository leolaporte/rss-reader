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
    pub is_read: bool,
    pub is_starred: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArticleFilter {
    #[default]
    All,
    Unread,
}

impl ArticleFilter {
    pub fn cycle(&self) -> Self {
        match self {
            ArticleFilter::All => ArticleFilter::Unread,
            ArticleFilter::Unread => ArticleFilter::All,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ArticleFilter::All => "All",
            ArticleFilter::Unread => "Unread",
        }
    }
}
