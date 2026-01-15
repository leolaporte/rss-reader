use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};
use tokio_rusqlite::Connection;

use crate::error::Result;
use crate::models::{Article, Feed, NewArticle, NewFeed, Summary};

use super::schema::SCHEMA;

pub struct Repository {
    conn: Connection,
}

impl Repository {
    pub async fn new(db_path: &str) -> Result<Self> {
        let conn = Connection::open(db_path).await?;

        conn.call(|conn| {
            // Set busy timeout to 5 seconds to handle concurrent access
            conn.busy_timeout(std::time::Duration::from_secs(5))?;
            conn.execute_batch(SCHEMA)?;
            Ok(())
        })
        .await?;

        Ok(Self { conn })
    }

    // Feed operations

    pub async fn insert_feed(&self, feed: NewFeed) -> Result<i64> {
        let id = self
            .conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO feeds (title, url, site_url, description) VALUES (?1, ?2, ?3, ?4)",
                    params![feed.title, feed.url, feed.site_url, feed.description],
                )?;
                Ok(conn.last_insert_rowid())
            })
            .await?;
        Ok(id)
    }

    pub async fn get_all_feeds(&self) -> Result<Vec<Feed>> {
        let feeds = self
            .conn
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, title, url, site_url, description, last_fetched, created_at, updated_at FROM feeds ORDER BY title",
                )?;
                let feeds = stmt
                    .query_map([], |row| Ok(feed_from_row(row)))?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(feeds)
            })
            .await?;
        Ok(feeds)
    }

    pub async fn update_feed_last_fetched(&self, id: i64) -> Result<()> {
        self.conn
            .call(move |conn| {
                conn.execute(
                    "UPDATE feeds SET last_fetched = datetime('now'), updated_at = datetime('now') WHERE id = ?1",
                    params![id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn delete_feed(&self, id: i64) -> Result<()> {
        self.conn
            .call(move |conn| {
                conn.execute("DELETE FROM feeds WHERE id = ?1", params![id])?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    // Article operations

    pub async fn upsert_article(&self, article: NewArticle) -> Result<i64> {
        let id = self
            .conn
            .call(move |conn| {
                // Check if this article was previously deleted
                let was_deleted: bool = conn.query_row(
                    "SELECT 1 FROM deleted_articles WHERE feed_id = ?1 AND guid = ?2",
                    params![article.feed_id, article.guid],
                    |_| Ok(true),
                ).unwrap_or(false);

                if was_deleted {
                    return Ok(0); // Skip deleted articles
                }

                conn.execute(
                    r#"INSERT INTO articles (feed_id, guid, title, url, author, content, content_text, published_at)
                       VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                       ON CONFLICT(feed_id, guid) DO UPDATE SET
                           title = excluded.title,
                           url = excluded.url,
                           author = excluded.author,
                           content = excluded.content,
                           content_text = excluded.content_text,
                           published_at = excluded.published_at"#,
                    params![
                        article.feed_id,
                        article.guid,
                        article.title,
                        article.url,
                        article.author,
                        article.content,
                        article.content_text,
                        article.published_at.map(|dt| dt.to_rfc3339()),
                    ],
                )?;
                Ok(conn.last_insert_rowid())
            })
            .await?;
        Ok(id)
    }

    pub async fn get_all_articles_sorted(&self) -> Result<Vec<Article>> {
        let articles = self
            .conn
            .call(|conn| {
                let mut stmt = conn.prepare(
                    r#"SELECT a.id, a.feed_id, a.guid, a.title, a.url, a.author, a.content,
                              a.content_text, a.published_at, a.fetched_at,
                              f.title as feed_title
                       FROM articles a
                       JOIN feeds f ON a.feed_id = f.id
                       ORDER BY a.published_at DESC NULLS LAST, a.fetched_at DESC"#,
                )?;
                let articles = stmt
                    .query_map([], |row| Ok(article_from_row(row)))?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(articles)
            })
            .await?;
        Ok(articles)
    }

    pub async fn delete_article(&self, id: i64) -> Result<()> {
        self.conn
            .call(move |conn| {
                // Record the article's feed_id and guid before deleting (to prevent re-adding)
                conn.execute(
                    r#"INSERT OR IGNORE INTO deleted_articles (feed_id, guid)
                       SELECT feed_id, guid FROM articles WHERE id = ?1"#,
                    params![id],
                )?;
                // Delete related data first
                conn.execute("DELETE FROM summaries WHERE article_id = ?1", params![id])?;
                conn.execute(
                    "DELETE FROM saved_to_raindrop WHERE article_id = ?1",
                    params![id],
                )?;
                // Delete the article
                conn.execute("DELETE FROM articles WHERE id = ?1", params![id])?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn undelete_article(&self, feed_id: i64, guid: &str) -> Result<()> {
        let guid = guid.to_string();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM deleted_articles WHERE feed_id = ?1 AND guid = ?2",
                    params![feed_id, guid],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn delete_old_articles(&self, days: i64) -> Result<usize> {
        let deleted = self
            .conn
            .call(move |conn| {
                // Delete summaries and raindrop entries for old articles first
                conn.execute(
                    r#"DELETE FROM summaries WHERE article_id IN (
                        SELECT id FROM articles
                        WHERE published_at < datetime('now', '-' || ?1 || ' days')
                           OR (published_at IS NULL AND fetched_at < datetime('now', '-' || ?1 || ' days'))
                    )"#,
                    params![days],
                )?;
                conn.execute(
                    r#"DELETE FROM saved_to_raindrop WHERE article_id IN (
                        SELECT id FROM articles
                        WHERE published_at < datetime('now', '-' || ?1 || ' days')
                           OR (published_at IS NULL AND fetched_at < datetime('now', '-' || ?1 || ' days'))
                    )"#,
                    params![days],
                )?;
                // Delete old articles (using published_at, fallback to fetched_at if null)
                let deleted = conn.execute(
                    r#"DELETE FROM articles
                       WHERE published_at < datetime('now', '-' || ?1 || ' days')
                          OR (published_at IS NULL AND fetched_at < datetime('now', '-' || ?1 || ' days'))"#,
                    params![days],
                )?;
                Ok(deleted)
            })
            .await?;
        Ok(deleted)
    }

    pub async fn compact_database(&self, days: i64) -> Result<usize> {
        let result = self
            .conn
            .call(move |conn| {
                // Delete old articles first
                conn.execute(
                    r#"DELETE FROM summaries WHERE article_id IN (
                        SELECT id FROM articles
                        WHERE published_at < datetime('now', '-' || ?1 || ' days')
                           OR (published_at IS NULL AND fetched_at < datetime('now', '-' || ?1 || ' days'))
                    )"#,
                    params![days],
                )?;
                conn.execute(
                    r#"DELETE FROM saved_to_raindrop WHERE article_id IN (
                        SELECT id FROM articles
                        WHERE published_at < datetime('now', '-' || ?1 || ' days')
                           OR (published_at IS NULL AND fetched_at < datetime('now', '-' || ?1 || ' days'))
                    )"#,
                    params![days],
                )?;
                let old_deleted = conn.execute(
                    r#"DELETE FROM articles
                       WHERE published_at < datetime('now', '-' || ?1 || ' days')
                          OR (published_at IS NULL AND fetched_at < datetime('now', '-' || ?1 || ' days'))"#,
                    params![days],
                )?;

                // Clean up old deleted_articles tracking entries
                conn.execute(
                    "DELETE FROM deleted_articles WHERE deleted_at < datetime('now', '-' || ?1 || ' days')",
                    params![days],
                )?;

                // Vacuum to reclaim space
                conn.execute("VACUUM", [])?;

                Ok(old_deleted)
            })
            .await?;
        Ok(result)
    }

    // Summary operations

    pub async fn get_summary(&self, article_id: i64) -> Result<Option<Summary>> {
        let summary = self
            .conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, article_id, content, model_version, generated_at FROM summaries WHERE article_id = ?1",
                )?;
                let summary = stmt
                    .query_row(params![article_id], |row| Ok(summary_from_row(row)))
                    .optional()?;
                Ok(summary)
            })
            .await?;
        Ok(summary)
    }

    pub async fn save_summary(&self, article_id: i64, content: String, model: String) -> Result<()> {
        self.conn
            .call(move |conn| {
                conn.execute(
                    r#"INSERT INTO summaries (article_id, content, model_version)
                       VALUES (?1, ?2, ?3)
                       ON CONFLICT(article_id) DO UPDATE SET
                           content = excluded.content,
                           model_version = excluded.model_version,
                           generated_at = datetime('now')"#,
                    params![article_id, content, model],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    // Raindrop tracking

    pub async fn mark_saved_to_raindrop(
        &self,
        article_id: i64,
        raindrop_id: i64,
        tags: Vec<String>,
    ) -> Result<()> {
        let tags_json = serde_json::to_string(&tags)?;
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO saved_to_raindrop (article_id, raindrop_id, tags) VALUES (?1, ?2, ?3)",
                    params![article_id, raindrop_id, tags_json],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn is_saved_to_raindrop(&self, article_id: i64) -> Result<bool> {
        let exists = self
            .conn
            .call(move |conn| {
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM saved_to_raindrop WHERE article_id = ?1",
                    params![article_id],
                    |row| row.get(0),
                )?;
                Ok(count > 0)
            })
            .await?;
        Ok(exists)
    }
}

fn parse_datetime(s: &str) -> Option<DateTime<Utc>> {
    // Try RFC3339 first (e.g., "2026-01-11T12:34:56+00:00")
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    // Try SQLite datetime format (e.g., "2026-01-11 12:34:56")
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(naive.and_utc());
    }
    None
}

fn feed_from_row(row: &Row) -> Feed {
    Feed {
        id: row.get(0).unwrap(),
        title: row.get(1).unwrap(),
        url: row.get(2).unwrap(),
        site_url: row.get(3).unwrap(),
        description: row.get(4).unwrap(),
        last_fetched: row
            .get::<_, Option<String>>(5)
            .unwrap()
            .and_then(|s| parse_datetime(&s)),
        created_at: row
            .get::<_, String>(6)
            .ok()
            .and_then(|s| parse_datetime(&s))
            .unwrap_or_else(Utc::now),
        updated_at: row
            .get::<_, String>(7)
            .ok()
            .and_then(|s| parse_datetime(&s))
            .unwrap_or_else(Utc::now),
    }
}

fn article_from_row(row: &Row) -> Article {
    Article {
        id: row.get(0).unwrap(),
        feed_id: row.get(1).unwrap(),
        guid: row.get(2).unwrap(),
        title: row.get(3).unwrap(),
        url: row.get(4).unwrap(),
        author: row.get(5).unwrap(),
        content: row.get(6).unwrap(),
        content_text: row.get(7).unwrap(),
        published_at: row
            .get::<_, Option<String>>(8)
            .unwrap()
            .and_then(|s| parse_datetime(&s)),
        fetched_at: row
            .get::<_, String>(9)
            .ok()
            .and_then(|s| parse_datetime(&s))
            .unwrap_or_else(Utc::now),
        feed_title: row.get(10).unwrap(),
    }
}

fn summary_from_row(row: &Row) -> Summary {
    Summary {
        id: row.get(0).unwrap(),
        article_id: row.get(1).unwrap(),
        content: row.get(2).unwrap(),
        model_version: row.get(3).unwrap(),
        generated_at: row
            .get::<_, String>(4)
            .ok()
            .and_then(|s| parse_datetime(&s))
            .unwrap_or_else(Utc::now),
    }
}
