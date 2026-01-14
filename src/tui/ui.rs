use chrono::Datelike;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::app::App;
use crate::models::SummaryStatus;

pub fn draw(frame: &mut Frame, app: &App) {
    // Main horizontal split: 1/3 left, 2/3 right
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Ratio(1, 3), // Left pane: article list
            Constraint::Ratio(2, 3), // Right pane: summary
        ])
        .split(frame.area());

    // Left pane: header + article list + status
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title bar
            Constraint::Min(0),    // Article list
            Constraint::Length(1), // Status line
        ])
        .split(main_chunks[0]);

    // Right pane: title + feed content + AI summary + status
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),   // Article title (feed + title)
            Constraint::Percentage(30), // Feed content (30%)
            Constraint::Percentage(70), // AI summary (70%)
            Constraint::Length(1),   // Status (generating/cached)
        ])
        .split(main_chunks[1]);

    // Render left pane
    render_header(frame, app, left_chunks[0]);
    render_article_list(frame, app, left_chunks[1]);
    render_left_status(frame, app, left_chunks[2]);

    // Render right pane
    render_article_title(frame, app, right_chunks[0]);
    render_feed_content(frame, app, right_chunks[1]);
    render_summary(frame, app, right_chunks[2]);
    render_right_status(frame, app, right_chunks[3]);

    // Render tag input popup if active
    if app.tag_input_active {
        render_tag_input(frame, app);
    }

    // Render feed input popup if active
    if app.feed_input_active {
        render_feed_input(frame, app);
    }

    // Render OPML input popup if active
    if app.opml_input_active {
        render_opml_input(frame, app);
    }

    // Render OPML export popup if active
    if app.opml_export_active {
        render_opml_export(frame, app);
    }

    // Render help popup if active
    if app.show_help {
        render_help(frame);
    }
}

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let filter_label = app.filter.label();
    let total_articles = app.articles.len();
    let unread_count = app.articles.iter().filter(|a| !a.is_read).count();

    let title = format!(" SpeedyReader [{filter_label}] ");
    let stats = format!(" {} Stories | {} Unread", total_articles, unread_count);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let paragraph = Paragraph::new(stats).style(Style::default().fg(Color::White));
    frame.render_widget(paragraph, inner);
}

fn render_article_list(frame: &mut Frame, app: &App, area: Rect) {
    let articles = app.filtered_articles();

    let items: Vec<ListItem> = articles
        .iter()
        .map(|article| {
            let style = if article.is_read {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(Color::White)
            };

            let day = article
                .published_at
                .map(|dt| {
                    match dt.weekday() {
                        chrono::Weekday::Mon => "M  ",
                        chrono::Weekday::Tue => "T  ",
                        chrono::Weekday::Wed => "W  ",
                        chrono::Weekday::Thu => "Th ",
                        chrono::Weekday::Fri => "F  ",
                        chrono::Weekday::Sat => "Sa ",
                        chrono::Weekday::Sun => "Su ",
                    }
                })
                .unwrap_or("?  ");
            let title = &article.title;

            let line = Line::from(vec![
                Span::styled(day, Style::default().fg(Color::DarkGray)),
                Span::styled(title, style),
            ]);

            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        );

    let mut state = ListState::default();
    state.select(Some(app.selected_index));

    frame.render_stateful_widget(list, area, &mut state);
}

fn render_left_status(frame: &mut Frame, app: &App, area: Rect) {
    let (status, color) = if app.is_refreshing {
        (format!("{} Refreshing feeds...", app.spinner_char()), Color::Cyan)
    } else {
        ("j/k:nav  r:refresh  o:open  e:email  ?:help  q:quit".to_string(), Color::DarkGray)
    };

    let paragraph = Paragraph::new(status).style(Style::default().fg(color));
    frame.render_widget(paragraph, area);
}

fn render_article_title(frame: &mut Frame, app: &App, area: Rect) {
    let (feed, title) = app
        .selected_article()
        .map(|a| {
            let feed = a.feed_title.as_deref().unwrap_or("Unknown");
            (feed, a.title.as_str())
        })
        .unwrap_or(("", "No article selected"));

    let block = Block::default()
        .title(" Article ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let text = vec![
        Line::from(Span::styled(feed, Style::default().fg(Color::Blue))),
        Line::from(Span::styled(title, Style::default().fg(Color::White))),
    ];

    let paragraph = Paragraph::new(text).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, inner);
}

fn render_feed_content(frame: &mut Frame, app: &App, area: Rect) {
    let content = app
        .selected_article()
        .and_then(|a| a.content_text.as_ref().or(a.content.as_ref()))
        .map(|c| c.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_else(|| "No content available".to_string());

    let block = Block::default()
        .title(" Feed Content ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));

    let paragraph = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: true });

    frame.render_widget(paragraph, area);
}

fn render_summary(frame: &mut Frame, app: &App, area: Rect) {
    let content = match app.summary_status {
        SummaryStatus::NotGenerated => "Press Enter to generate summary...".to_string(),
        SummaryStatus::Generating => format!("{} Generating summary...", app.spinner_char()),
        SummaryStatus::Failed => "Failed to generate summary. Press 'g' to retry.".to_string(),
        SummaryStatus::NoApiKey => "Claude API key not configured.\n\nPlease add your API key to:\n~/.config/speedy-reader/config.toml\n\nExample:\nclaude_api_key = \"sk-ant-...\"".to_string(),
        SummaryStatus::Generated => app
            .current_summary
            .as_ref()
            .map(|s| s.content.clone())
            .unwrap_or_else(|| "No summary available".to_string()),
    };

    let block = Block::default()
        .title(" AI Summary ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let paragraph = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: true });

    frame.render_widget(paragraph, area);
}

fn render_right_status(frame: &mut Frame, app: &App, area: Rect) {
    let status = match app.summary_status {
        SummaryStatus::NotGenerated => String::new(),
        SummaryStatus::Generating => format!("{} Generating...", app.spinner_char()),
        SummaryStatus::Failed => "❌ Failed".to_string(),
        SummaryStatus::NoApiKey => "⚠️  No API key".to_string(),
        SummaryStatus::Generated => "✓ Cached".to_string(),
    };

    let raindrop_status = if app.selected_article().map(|a| a.id).is_some() {
        if app.is_saved_to_raindrop {
            " | 💧 Saved"
        } else {
            " | b:bookmark"
        }
    } else {
        ""
    };

    let text = format!("{status}{raindrop_status}");
    let paragraph = Paragraph::new(text).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(paragraph, area);
}

fn render_tag_input(frame: &mut Frame, app: &App) {
    let area = centered_rect(60, 20, frame.area());

    let block = Block::default()
        .title(" Save to Raindrop.io - Enter tags (comma separated) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let inner = block.inner(area);

    // Clear the area first
    frame.render_widget(ratatui::widgets::Clear, area);
    frame.render_widget(block, area);

    let input_text = format!("> {}_", app.tag_input);
    let paragraph = Paragraph::new(input_text).style(Style::default().fg(Color::White));
    frame.render_widget(paragraph, inner);
}

fn render_feed_input(frame: &mut Frame, app: &App) {
    let area = centered_rect(70, 25, frame.area());

    let block = Block::default()
        .title(" Add Feed - Enter URL or website address ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);

    // Clear the area first
    frame.render_widget(ratatui::widgets::Clear, area);
    frame.render_widget(block, area);

    // Split inner area for input and status
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    let input_text = format!("> {}_", app.feed_input);
    let paragraph = Paragraph::new(input_text).style(Style::default().fg(Color::White));
    frame.render_widget(paragraph, chunks[0]);

    // Show status message if any
    if let Some(status) = &app.feed_input_status {
        let (display_status, color) = if status.starts_with("Added:") {
            (status.clone(), Color::Green)
        } else if status.starts_with("Error:") || status.starts_with("Not found:") {
            (status.clone(), Color::Red)
        } else if status.starts_with("Feed already") {
            (status.clone(), Color::Yellow)
        } else if status.starts_with("Discovering") {
            (format!("{} {}", app.spinner_char(), status), Color::Cyan)
        } else {
            (status.clone(), Color::DarkGray)
        };
        let status_paragraph = Paragraph::new(display_status).style(Style::default().fg(color));
        frame.render_widget(status_paragraph, chunks[1]);
    }
}

fn render_opml_input(frame: &mut Frame, app: &App) {
    let area = centered_rect(70, 25, frame.area());

    let block = Block::default()
        .title(" Import OPML - Enter file path ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let inner = block.inner(area);

    // Clear the area first
    frame.render_widget(ratatui::widgets::Clear, area);
    frame.render_widget(block, area);

    // Split inner area for input and status
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    let input_text = format!("> {}_", app.opml_input);
    let paragraph = Paragraph::new(input_text).style(Style::default().fg(Color::White));
    frame.render_widget(paragraph, chunks[0]);

    // Show status message if any
    if let Some(status) = &app.opml_input_status {
        let (display_status, color) = if status.starts_with("Imported") {
            (status.clone(), Color::Green)
        } else if status.starts_with("Error:") || status.starts_with("Not found:") {
            (status.clone(), Color::Red)
        } else if status == "Importing..." {
            (format!("{} Importing...", app.spinner_char()), Color::Yellow)
        } else {
            (status.clone(), Color::DarkGray)
        };
        let status_paragraph = Paragraph::new(display_status).style(Style::default().fg(color));
        frame.render_widget(status_paragraph, chunks[1]);
    }
}

fn render_opml_export(frame: &mut Frame, app: &App) {
    let area = centered_rect(70, 25, frame.area());

    let block = Block::default()
        .title(" Export OPML - Enter file path ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let inner = block.inner(area);

    // Clear the area first
    frame.render_widget(ratatui::widgets::Clear, area);
    frame.render_widget(block, area);

    // Split inner area for input and status
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    let input_text = format!("> {}_", app.opml_export_input);
    let paragraph = Paragraph::new(input_text).style(Style::default().fg(Color::White));
    frame.render_widget(paragraph, chunks[0]);

    // Show status message if any
    if let Some(status) = &app.opml_export_status {
        let (display_status, color) = if status.starts_with("Exported") {
            (status.clone(), Color::Green)
        } else if status.starts_with("Error:") {
            (status.clone(), Color::Red)
        } else if status == "Exporting..." {
            (format!("{} Exporting...", app.spinner_char()), Color::Green)
        } else {
            (status.clone(), Color::DarkGray)
        };
        let status_paragraph = Paragraph::new(display_status).style(Style::default().fg(color));
        frame.render_widget(status_paragraph, chunks[1]);
    }
}

fn render_help(frame: &mut Frame) {
    let area = centered_rect(50, 60, frame.area());

    let help_text = vec![
        "",
        " Navigation:",
        "   j / ↓    Move down",
        "   k / ↑    Move up",
        "   <        Go to top",
        "   >        Go to bottom",
        "   Enter    Select / Generate summary",
        "",
        " Actions:",
        "   r        Refresh all feeds",
        "   a        Add new feed",
        "   i        Import OPML file",
        "   w        Export OPML file",
        "   m        Toggle read/unread",
        "   o        Open in browser",
        "   e        Email article",
        "   b        Save to Raindrop.io",
        "   g        Regenerate summary",
        "   f        Cycle filter",
        "   d        Delete article",
        "   u        Undelete last",
        "",
        " General:",
        "   ?        Toggle this help",
        "   q        Quit",
        "",
        " Press any key to close",
    ];

    let block = Block::default()
        .title(" Help ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(help_text.join("\n"))
        .block(block)
        .style(Style::default().fg(Color::White));

    frame.render_widget(ratatui::widgets::Clear, area);
    frame.render_widget(paragraph, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
