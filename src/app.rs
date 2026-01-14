use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::ai::Summarizer;
use crate::config::Config;
use crate::db::Repository;
use crate::error::Result;
use crate::feed::{export_opml_file, parse_opml_file, FeedFetcher};
use crate::models::{Article, Feed, Summary, SummaryStatus};
use crate::services::{ContentFetcher, RaindropClient};
use crate::tui::AppAction;

// Message for completed summary
pub struct SummaryResult {
    pub article_id: i64,
    pub result: std::result::Result<(String, String), String>, // (content, model) or error
}

// Message for completed refresh
pub struct RefreshResult {
    pub results: Vec<(i64, Vec<crate::models::NewArticle>)>,
}

// Message for completed feed discovery
pub struct FeedDiscoveryResult {
    pub result: std::result::Result<crate::models::NewFeed, String>,
}

pub struct App {
    // Data
    pub feeds: Vec<Feed>,
    pub articles: Vec<Article>,
    pub current_summary: Option<Summary>,

    // UI State
    pub selected_index: usize,
    pub show_help: bool,
    pub tag_input_active: bool,
    pub tag_input: String,
    pub feed_input_active: bool,
    pub feed_input: String,
    pub feed_input_status: Option<String>,
    pub opml_input_active: bool,
    pub opml_input: String,
    pub opml_input_status: Option<String>,
    pub opml_export_active: bool,
    pub opml_export_input: String,
    pub opml_export_status: Option<String>,
    pub is_saved_to_raindrop: bool,
    pub last_deleted: Option<(i64, String)>, // (feed_id, guid) for undo
    pub spinner_frame: usize,

    // Async state
    pub is_refreshing: bool,
    pub summary_status: SummaryStatus,
    pub pending_summary_article_id: Option<i64>,
    summary_rx: mpsc::Receiver<SummaryResult>,
    summary_tx: mpsc::Sender<SummaryResult>,
    refresh_rx: mpsc::Receiver<RefreshResult>,
    refresh_tx: mpsc::Sender<RefreshResult>,
    discovery_rx: mpsc::Receiver<FeedDiscoveryResult>,
    discovery_tx: mpsc::Sender<FeedDiscoveryResult>,

    // Services
    pub repository: Repository,
    fetcher: FeedFetcher,
    summarizer: Option<Arc<Summarizer>>,
    raindrop: Option<RaindropClient>,
    content_fetcher: ContentFetcher,
}

impl App {
    pub async fn new(config: &Config) -> Result<Self> {
        let repository = Repository::new(&config.db_path).await?;
        let fetcher = FeedFetcher::new();

        let summarizer = config
            .claude_api_key
            .as_ref()
            .map(|key| Arc::new(Summarizer::new(key.clone())));

        let raindrop = config
            .raindrop_token
            .as_ref()
            .map(|token| RaindropClient::new(token.clone()));

        let content_fetcher = ContentFetcher::new();

        // Clean up articles older than 7 days
        let deleted = repository.delete_old_articles(7).await?;
        if deleted > 0 {
            tracing::info!("Deleted {} articles older than 7 days", deleted);
        }

        let feeds = repository.get_all_feeds().await?;
        let articles = repository.get_all_articles_sorted().await?;

        let (summary_tx, summary_rx) = mpsc::channel(1);
        let (refresh_tx, refresh_rx) = mpsc::channel(1);
        let (discovery_tx, discovery_rx) = mpsc::channel(1);

        Ok(Self {
            feeds,
            articles,
            current_summary: None,
            selected_index: 0,
            show_help: false,
            tag_input_active: false,
            tag_input: String::new(),
            feed_input_active: false,
            feed_input: String::new(),
            feed_input_status: None,
            opml_input_active: false,
            opml_input: String::new(),
            opml_input_status: None,
            opml_export_active: false,
            opml_export_input: String::new(),
            opml_export_status: None,
            is_saved_to_raindrop: false,
            last_deleted: None,
            spinner_frame: 0,
            is_refreshing: false,
            summary_status: SummaryStatus::NotGenerated,
            pending_summary_article_id: None,
            summary_rx,
            summary_tx,
            refresh_rx,
            refresh_tx,
            discovery_rx,
            discovery_tx,
            repository,
            fetcher,
            summarizer,
            raindrop,
            content_fetcher,
        })
    }

    pub fn filtered_articles(&self) -> Vec<&Article> {
        self.articles.iter().collect()
    }

    pub fn selected_article(&self) -> Option<&Article> {
        let articles = self.filtered_articles();
        articles.get(self.selected_index).copied()
    }

    pub async fn handle_action(&mut self, action: AppAction) -> Result<bool> {
        match action {
            AppAction::Quit => {
                // Compact database on exit (remove old articles and vacuum)
                let _ = self.repository.compact_database(7).await;
                return Ok(true);
            }

            AppAction::MoveUp => {
                let len = self.filtered_articles().len();
                if len > 0 && self.selected_index > 0 {
                    self.selected_index -= 1;
                    self.on_selection_changed().await?;
                }
            }

            AppAction::MoveDown => {
                let len = self.filtered_articles().len();
                if len > 0 && self.selected_index < len - 1 {
                    self.selected_index += 1;
                    self.on_selection_changed().await?;
                }
            }

            AppAction::MoveToTop => {
                if !self.filtered_articles().is_empty() && self.selected_index != 0 {
                    self.selected_index = 0;
                    self.on_selection_changed().await?;
                }
            }

            AppAction::MoveToBottom => {
                let len = self.filtered_articles().len();
                if len > 0 && self.selected_index != len - 1 {
                    self.selected_index = len - 1;
                    self.on_selection_changed().await?;
                }
            }

            AppAction::SelectArticle => {
                self.generate_summary().await?;
            }

            AppAction::RefreshFeeds => {
                self.refresh_feeds();
            }

            AppAction::OpenInBrowser => {
                if let Some(article) = self.selected_article() {
                    let url = article.url.clone();
                    let _ = open::that(&url);
                }
            }

            AppAction::EmailArticle => {
                if let Some(article) = self.selected_article() {
                    self.email_article(article);
                }
            }

            AppAction::SaveToRaindrop => {
                if self.raindrop.is_some() && self.selected_article().is_some() {
                    self.tag_input_active = true;
                    self.tag_input.clear();
                }
            }

            AppAction::RegenerateSummary => {
                self.summary_status = SummaryStatus::NotGenerated;
                self.current_summary = None;
                self.generate_summary().await?;
            }

            AppAction::DeleteArticle => {
                if let Some(article) = self.selected_article() {
                    let id = article.id;
                    let feed_id = article.feed_id;
                    let guid = article.guid.clone();
                    self.repository.delete_article(id).await?;
                    // Store for undo
                    self.last_deleted = Some((feed_id, guid));
                    // Remove from local list
                    self.articles.retain(|a| a.id != id);
                    // Adjust selection if needed
                    let len = self.filtered_articles().len();
                    if len > 0 && self.selected_index >= len {
                        self.selected_index = len - 1;
                    }
                    // Reset summary state
                    self.summary_status = SummaryStatus::NotGenerated;
                    self.current_summary = None;
                }
            }

            AppAction::DeleteFeed => {
                if let Some(article) = self.selected_article() {
                    let feed_id = article.feed_id;
                    // Delete the feed (cascades to articles via foreign key)
                    self.repository.delete_feed(feed_id).await?;
                    // Remove all articles from this feed from local list
                    self.articles.retain(|a| a.feed_id != feed_id);
                    // Reload feeds list
                    self.feeds = self.repository.get_all_feeds().await?;
                    // Adjust selection if needed
                    let len = self.filtered_articles().len();
                    if len > 0 && self.selected_index >= len {
                        self.selected_index = len - 1;
                    }
                    // Reset summary state
                    self.summary_status = SummaryStatus::NotGenerated;
                    self.current_summary = None;
                }
            }

            AppAction::UndeleteArticle => {
                if let Some((feed_id, guid)) = self.last_deleted.take() {
                    self.repository.undelete_article(feed_id, &guid).await?;
                    // Article will return on next refresh (press 'r')
                }
            }

            AppAction::ShowHelp => {
                self.show_help = true;
            }

            AppAction::HideHelp => {
                self.show_help = false;
            }

            AppAction::TagInputChar(c) => {
                self.tag_input.push(c);
            }

            AppAction::TagInputBackspace => {
                self.tag_input.pop();
            }

            AppAction::TagInputConfirm => {
                self.save_to_raindrop().await?;
                self.tag_input_active = false;
                self.tag_input.clear();
            }

            AppAction::TagInputCancel => {
                self.tag_input_active = false;
                self.tag_input.clear();
            }

            AppAction::AddFeed => {
                self.feed_input_active = true;
                self.feed_input.clear();
                self.feed_input_status = None;
            }

            AppAction::FeedInputChar(c) => {
                self.feed_input.push(c);
            }

            AppAction::FeedInputBackspace => {
                self.feed_input.pop();
            }

            AppAction::FeedInputConfirm => {
                self.start_feed_discovery();
            }

            AppAction::FeedInputCancel => {
                self.feed_input_active = false;
                self.feed_input.clear();
                self.feed_input_status = None;
            }

            AppAction::ImportOpmlStart => {
                self.opml_input_active = true;
                self.opml_input.clear();
                self.opml_input_status = None;
            }

            AppAction::OpmlInputChar(c) => {
                self.opml_input.push(c);
            }

            AppAction::OpmlInputBackspace => {
                self.opml_input.pop();
            }

            AppAction::OpmlInputConfirm => {
                self.import_opml_from_input().await?;
            }

            AppAction::OpmlInputCancel => {
                self.opml_input_active = false;
                self.opml_input.clear();
                self.opml_input_status = None;
            }

            AppAction::ExportOpmlStart => {
                self.opml_export_active = true;
                self.opml_export_input = "~/feeds.opml".to_string();
                self.opml_export_status = None;
            }

            AppAction::OpmlExportChar(c) => {
                self.opml_export_input.push(c);
            }

            AppAction::OpmlExportBackspace => {
                self.opml_export_input.pop();
            }

            AppAction::OpmlExportConfirm => {
                self.export_opml_to_file()?;
            }

            AppAction::OpmlExportCancel => {
                self.opml_export_active = false;
                self.opml_export_input.clear();
                self.opml_export_status = None;
            }
        }

        Ok(false)
    }

    async fn on_selection_changed(&mut self) -> Result<()> {
        // Reset state when selection changes
        self.summary_status = SummaryStatus::NotGenerated;
        self.current_summary = None;
        self.is_saved_to_raindrop = false;

        // Check if current article is saved to raindrop
        let article_id = self.selected_article().map(|a| a.id);
        if let Some(id) = article_id {
            self.is_saved_to_raindrop = self
                .repository
                .is_saved_to_raindrop(id)
                .await?;

            // Check for cached summary
            if let Some(summary) = self.repository.get_summary(id).await? {
                self.current_summary = Some(summary);
                self.summary_status = SummaryStatus::Generated;
            }
        }

        Ok(())
    }

    async fn generate_summary(&mut self) -> Result<()> {
        let Some(summarizer) = &self.summarizer else {
            self.summary_status = SummaryStatus::NoApiKey;
            return Ok(());
        };

        let Some(article) = self.selected_article() else {
            return Ok(());
        };

        // Check cache first
        if let Some(summary) = self.repository.get_summary(article.id).await? {
            self.current_summary = Some(summary);
            self.summary_status = SummaryStatus::Generated;
            return Ok(());
        }

        let article_id = article.id;
        let title = article.title.clone();
        let article_url = article.url.clone();

        // Get RSS content as fallback
        let rss_content = article
            .content_text
            .clone()
            .or_else(|| article.content.clone())
            .unwrap_or_default();

        self.summary_status = SummaryStatus::Generating;
        self.pending_summary_article_id = Some(article_id);

        // Try to fetch full content using browser cookies
        let content = match self.content_fetcher.fetch_full_content(&article_url).await {
            Ok(Some(full_content)) => {
                tracing::info!("Fetched full content for: {}", article_url);
                full_content
            }
            Ok(None) => {
                tracing::debug!("No full content available, using RSS content");
                rss_content
            }
            Err(e) => {
                tracing::debug!("Failed to fetch full content: {}, using RSS", e);
                rss_content
            }
        };

        // Spawn background task for summary generation
        let summarizer = Arc::clone(summarizer);
        let tx = self.summary_tx.clone();

        tokio::spawn(async move {
            let result = match summarizer.generate_summary(&title, &content).await {
                Ok(summary_text) => {
                    let model = summarizer.model_version().to_string();
                    Ok((summary_text, model))
                }
                Err(e) => Err(e.to_string()),
            };

            let _ = tx.send(SummaryResult { article_id, result }).await;
        });

        // Don't update local is_read state - keep article visible in filtered list
        // Database is already updated, so it will show as read next session

        Ok(())
    }

    /// Advance the spinner animation frame
    pub fn tick_spinner(&mut self) {
        self.spinner_frame = (self.spinner_frame + 1) % 10;
    }

    /// Get the current spinner character
    pub fn spinner_char(&self) -> char {
        const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        SPINNER[self.spinner_frame]
    }

    /// Poll for completed summary results (non-blocking)
    pub async fn poll_summary_result(&mut self) -> Result<()> {
        if let Ok(result) = self.summary_rx.try_recv() {
            // Only process if this is the summary we're waiting for
            if self.pending_summary_article_id == Some(result.article_id) {
                // Check if the article still exists (might have been deleted)
                let article_exists = self.articles.iter().any(|a| a.id == result.article_id);

                match result.result {
                    Ok((summary_text, model)) => {
                        if article_exists {
                            // Save to database only if article still exists
                            if let Err(e) = self
                                .repository
                                .save_summary(result.article_id, summary_text.clone(), model.clone())
                                .await
                            {
                                tracing::warn!("Failed to save summary (article may have been deleted): {}", e);
                            }

                            self.current_summary = Some(Summary {
                                id: 0,
                                article_id: result.article_id,
                                content: summary_text,
                                model_version: model,
                                generated_at: chrono::Utc::now(),
                            });
                            self.summary_status = SummaryStatus::Generated;
                        } else {
                            tracing::debug!("Discarding summary for deleted article {}", result.article_id);
                            self.summary_status = SummaryStatus::NotGenerated;
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to generate summary: {}", e);
                        self.summary_status = SummaryStatus::Failed;
                    }
                }
                self.pending_summary_article_id = None;
            }
        }
        Ok(())
    }

    /// Start discovering a feed from a URL (non-blocking)
    fn start_feed_discovery(&mut self) {
        let url = self.feed_input.trim().to_string();
        if url.is_empty() {
            self.feed_input_active = false;
            return;
        }

        // Normalize URL - add https:// if no protocol specified
        let url = if !url.starts_with("http://") && !url.starts_with("https://") {
            format!("https://{}", url)
        } else {
            url
        };

        self.feed_input_status = Some("Discovering feed...".to_string());

        let fetcher = self.fetcher.clone();
        let tx = self.discovery_tx.clone();

        tokio::spawn(async move {
            let result = match fetcher.discover_feed(&url).await {
                Ok(feed) => Ok(feed),
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(FeedDiscoveryResult { result }).await;
        });
    }

    /// Poll for completed feed discovery results (non-blocking)
    pub async fn poll_discovery_result(&mut self) -> Result<()> {
        if let Ok(result) = self.discovery_rx.try_recv() {
            match result.result {
                Ok(new_feed) => {
                    // Check if feed already exists
                    if self.feeds.iter().any(|f| f.url == new_feed.url) {
                        self.feed_input_status = Some(format!("Feed already exists: {}", new_feed.title));
                        return Ok(());
                    }

                    let feed_title = new_feed.title.clone();
                    match self.repository.insert_feed(new_feed).await {
                        Ok(feed_id) => {
                            self.feed_input_status = Some(format!("Added: {}", feed_title));
                            tracing::info!("Added new feed: {} (id={})", feed_title, feed_id);

                            // Reload feeds list
                            self.feeds = self.repository.get_all_feeds().await?;

                            // Clear input after short delay to show success message
                            self.feed_input_active = false;
                            self.feed_input.clear();

                            // Refresh the new feed
                            self.refresh_feeds();
                        }
                        Err(e) => {
                            self.feed_input_status = Some(format!("Error: {}", e));
                            tracing::error!("Failed to insert feed: {}", e);
                        }
                    }
                }
                Err(_) => {
                    self.feed_input_status = Some("No feed here.".to_string());
                }
            }
        }
        Ok(())
    }

    pub fn refresh_feeds(&mut self) {
        if self.is_refreshing {
            return; // Already refreshing
        }
        self.is_refreshing = true;

        let feeds = self.feeds.clone();
        let fetcher = self.fetcher.clone();
        let tx = self.refresh_tx.clone();

        tokio::spawn(async move {
            let results = fetcher.refresh_all(feeds).await;
            let _ = tx.send(RefreshResult { results }).await;
        });
    }

    /// Poll for completed refresh results (non-blocking)
    pub async fn poll_refresh_result(&mut self) -> Result<()> {
        if let Ok(result) = self.refresh_rx.try_recv() {
            // Process the refresh results
            for (feed_id, articles) in result.results {
                for article in articles {
                    if let Err(e) = self.repository.upsert_article(article).await {
                        tracing::warn!("Failed to upsert article: {}", e);
                    }
                }
                if let Err(e) = self.repository.update_feed_last_fetched(feed_id).await {
                    tracing::warn!("Failed to update feed last_fetched: {}", e);
                }
            }

            // Clean up articles older than 7 days after refresh
            let deleted = self.repository.delete_old_articles(7).await?;
            if deleted > 0 {
                tracing::info!("Deleted {} articles older than 7 days", deleted);
            }

            self.reload_articles().await?;
            self.is_refreshing = false;
        }
        Ok(())
    }

    /// Refresh feeds and wait for completion (blocking, for CLI/headless use)
    pub async fn refresh_feeds_blocking(&mut self) -> Result<()> {
        self.refresh_feeds();

        // Wait for the refresh to complete
        while self.is_refreshing {
            self.poll_refresh_result().await?;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        Ok(())
    }

    async fn reload_articles(&mut self) -> Result<()> {
        self.articles = self.repository.get_all_articles_sorted().await?;
        Ok(())
    }

    async fn save_to_raindrop(&mut self) -> Result<()> {
        let Some(raindrop) = &self.raindrop else {
            return Ok(());
        };

        let Some(article) = self.selected_article() else {
            return Ok(());
        };

        let tags: Vec<String> = self
            .tag_input
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let article_id = article.id;
        let url = article.url.clone();
        let title = article.title.clone();

        // Get excerpt: first sentence of summary (cleaned), or first sentence of article content
        let excerpt = self
            .current_summary
            .as_ref()
            .map(|s| Self::clean_summary_for_excerpt(&s.content))
            .filter(|s| !s.is_empty())
            .or_else(|| {
                article
                    .content_text
                    .as_ref()
                    .or(article.content.as_ref())
                    .map(|c| Self::get_first_sentence(c))
            });

        // Get AI summary for note field (if available)
        let note = self.current_summary.as_ref().map(|s| s.content.clone());

        match raindrop
            .save_bookmark(&url, Some(&title), excerpt.as_deref(), note.as_deref(), tags.clone())
            .await
        {
            Ok(raindrop_id) => {
                self.repository
                    .mark_saved_to_raindrop(article_id, raindrop_id, tags)
                    .await?;
                self.is_saved_to_raindrop = true;
                tracing::info!("Saved to Raindrop: {}", url);
            }
            Err(e) => {
                tracing::error!("Failed to save to Raindrop: {}", e);
            }
        }

        // Don't reload - keep article visible in filtered list this session

        Ok(())
    }

    /// Extract the first sentence from text (up to ~200 chars for Raindrop excerpt)
    fn get_first_sentence(text: &str) -> String {
        let text = text.trim();
        // Find the end of the first sentence
        let sentence_end = text
            .find(". ")
            .or_else(|| text.find(".\n"))
            .or_else(|| text.find('.'))
            .map(|i| i + 1)
            .unwrap_or(text.len());

        let first_sentence = &text[..sentence_end.min(text.len())];

        // Truncate to ~200 chars for Raindrop
        if first_sentence.len() > 200 {
            let truncated = &first_sentence[..200];
            // Find last space to avoid cutting mid-word
            if let Some(last_space) = truncated.rfind(' ') {
                format!("{}...", &truncated[..last_space])
            } else {
                format!("{}...", truncated)
            }
        } else {
            first_sentence.to_string()
        }
    }

    /// Clean AI summary prefixes and extract first sentence for Raindrop excerpt
    fn clean_summary_for_excerpt(summary: &str) -> String {
        // Common AI summary prefixes to strip
        let prefixes = [
            "summary:",
            "here's the summary of the article:",
            "here's a summary of the article:",
            "here is the summary of the article:",
            "here is a summary of the article:",
            "here's the summary:",
            "here's a summary:",
            "here is the summary:",
            "here is a summary:",
        ];

        // Skip blank lines and find first non-empty line
        let mut text = summary
            .lines()
            .map(|line| line.trim())
            .skip_while(|line| line.is_empty())
            .collect::<Vec<_>>()
            .join(" ");

        // Strip common prefixes (case-insensitive)
        let text_lower = text.to_lowercase();
        for prefix in &prefixes {
            if text_lower.starts_with(prefix) {
                text = text[prefix.len()..].trim_start().to_string();
                break;
            }
        }

        // Extract first sentence
        Self::get_first_sentence(&text)
    }

    pub async fn import_opml(&mut self, path: &Path) -> Result<()> {
        let feeds = parse_opml_file(path)?;

        for feed in feeds {
            match self.repository.insert_feed(feed).await {
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("Failed to insert feed: {}", e);
                }
            }
        }

        self.feeds = self.repository.get_all_feeds().await?;

        // Refresh the newly imported feeds
        self.refresh_feeds();

        Ok(())
    }

    async fn import_opml_from_input(&mut self) -> Result<()> {
        let input = self.opml_input.trim().to_string();
        if input.is_empty() {
            self.opml_input_status = Some("Enter a file path".to_string());
            return Ok(());
        }

        // Expand ~ to home directory
        let expanded = if let Some(rest) = input.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                home.join(rest)
            } else {
                PathBuf::from(&input)
            }
        } else {
            PathBuf::from(&input)
        };

        self.opml_input_status = Some("Importing...".to_string());

        if !expanded.exists() {
            self.opml_input_status = Some("Not found: file does not exist".to_string());
            return Ok(());
        }

        match self.import_opml(&expanded).await {
            Ok(()) => {
                let count = self.feeds.len();
                self.opml_input_status = Some(format!("Imported! {} feeds total", count));
                self.opml_input_active = false;
                self.opml_input.clear();
            }
            Err(e) => {
                self.opml_input_status = Some(format!("Error: {}", e));
            }
        }

        Ok(())
    }

    fn export_opml_to_file(&mut self) -> Result<()> {
        let input = self.opml_export_input.trim().to_string();
        if input.is_empty() {
            self.opml_export_status = Some("Enter a file path".to_string());
            return Ok(());
        }

        // Expand ~ to home directory
        let expanded = if let Some(rest) = input.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                home.join(rest)
            } else {
                PathBuf::from(&input)
            }
        } else {
            PathBuf::from(&input)
        };

        self.opml_export_status = Some("Exporting...".to_string());

        match export_opml_file(&expanded, &self.feeds) {
            Ok(()) => {
                let count = self.feeds.len();
                self.opml_export_status = Some(format!("Exported {} feeds!", count));
                self.opml_export_active = false;
                self.opml_export_input.clear();
            }
            Err(e) => {
                self.opml_export_status = Some(format!("Error: {}", e));
            }
        }

        Ok(())
    }

    fn email_article(&self, article: &Article) {
        let subject = urlencoding::encode(&article.title);

        // Build email body with title, URL, summary (if available), and content
        let mut body_parts = Vec::new();

        // Add title
        body_parts.push(format!("Title: {}", article.title));
        body_parts.push(String::new()); // blank line

        // Add URL
        body_parts.push(format!("URL: {}", article.url));
        body_parts.push(String::new()); // blank line

        // Add AI summary if available
        if let Some(summary) = &self.current_summary {
            body_parts.push("AI Summary:".to_string());
            body_parts.push(summary.content.clone());
            body_parts.push(String::new()); // blank line
        }

        // Add article content if available
        if let Some(content) = article.content_text.as_ref().or(article.content.as_ref()) {
            body_parts.push("Article Content:".to_string());
            body_parts.push(content.clone());
        }

        let body_text = body_parts.join("\n");
        let body = urlencoding::encode(&body_text);

        let mailto_url = format!("mailto:?subject={}&body={}", subject, body);

        // Open the mailto link in the default email client
        let _ = open::that(&mailto_url);
    }
}
