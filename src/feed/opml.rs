use opml::{Outline, OPML};
use std::path::Path;

use crate::error::{AppError, Result};
use crate::models::{Feed, NewFeed};

pub fn parse_opml_file(path: &Path) -> Result<Vec<NewFeed>> {
    let content = std::fs::read_to_string(path)?;
    parse_opml_string(&content)
}

/// Parse OPML content from a string
pub fn parse_opml_string(content: &str) -> Result<Vec<NewFeed>> {
    let opml = OPML::from_str(content).map_err(|e| AppError::OpmlParse(e.to_string()))?;

    let mut feeds = Vec::new();
    collect_feeds(&opml.body.outlines, &mut feeds);

    Ok(feeds)
}

fn collect_feeds(outlines: &[Outline], feeds: &mut Vec<NewFeed>) {
    for outline in outlines {
        // Check if this outline is a feed (has xmlUrl)
        if let Some(xml_url) = &outline.xml_url {
            feeds.push(NewFeed {
                title: outline.text.clone(),
                url: xml_url.clone(),
                site_url: outline.html_url.clone(),
                description: outline.description.clone(),
            });
        }

        // Recursively process nested outlines (categories/folders)
        if !outline.outlines.is_empty() {
            collect_feeds(&outline.outlines, feeds);
        }
    }
}

pub fn export_opml_file(path: &Path, feeds: &[Feed]) -> Result<()> {
    let mut opml = OPML::default();
    opml.head = Some(opml::Head {
        title: Some("BeatCheck Feeds".to_string()),
        ..Default::default()
    });

    for feed in feeds {
        let outline = Outline {
            text: feed.title.clone(),
            r#type: Some("rss".to_string()),
            xml_url: Some(feed.url.clone()),
            html_url: feed.site_url.clone(),
            description: feed.description.clone(),
            ..Default::default()
        };
        opml.body.outlines.push(outline);
    }

    let content = opml.to_string().map_err(|e| AppError::OpmlParse(e.to_string()))?;
    std::fs::write(path, content)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn make_feed(id: i64, title: &str, url: &str) -> Feed {
        Feed {
            id,
            title: title.to_string(),
            url: url.to_string(),
            site_url: Some(format!("https://{}.com", title.to_lowercase())),
            description: Some(format!("{} feed", title)),
            last_fetched: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_parse_flat_opml() {
        let opml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0">
  <head><title>My Feeds</title></head>
  <body>
    <outline text="Ars Technica" type="rss" xmlUrl="https://feeds.arstechnica.com/arstechnica/index" htmlUrl="https://arstechnica.com"/>
    <outline text="Hacker News" type="rss" xmlUrl="https://news.ycombinator.com/rss" htmlUrl="https://news.ycombinator.com"/>
  </body>
</opml>"#;

        let feeds = parse_opml_string(opml_content).unwrap();

        assert_eq!(feeds.len(), 2);
        assert_eq!(feeds[0].title, "Ars Technica");
        assert_eq!(feeds[0].url, "https://feeds.arstechnica.com/arstechnica/index");
        assert_eq!(feeds[0].site_url, Some("https://arstechnica.com".to_string()));
        assert_eq!(feeds[1].title, "Hacker News");
    }

    #[test]
    fn test_parse_nested_opml() {
        let opml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0">
  <head><title>My Feeds</title></head>
  <body>
    <outline text="Tech">
      <outline text="Ars Technica" type="rss" xmlUrl="https://feeds.arstechnica.com/arstechnica/index"/>
      <outline text="The Verge" type="rss" xmlUrl="https://www.theverge.com/rss/index.xml"/>
    </outline>
    <outline text="News">
      <outline text="BBC" type="rss" xmlUrl="https://feeds.bbci.co.uk/news/rss.xml"/>
    </outline>
  </body>
</opml>"#;

        let feeds = parse_opml_string(opml_content).unwrap();

        assert_eq!(feeds.len(), 3);
        assert_eq!(feeds[0].title, "Ars Technica");
        assert_eq!(feeds[1].title, "The Verge");
        assert_eq!(feeds[2].title, "BBC");
    }

    #[test]
    fn test_parse_deeply_nested_opml() {
        let opml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0">
  <body>
    <outline text="Level1">
      <outline text="Level2">
        <outline text="Level3">
          <outline text="Deep Feed" type="rss" xmlUrl="https://deep.example.com/feed"/>
        </outline>
      </outline>
    </outline>
  </body>
</opml>"#;

        let feeds = parse_opml_string(opml_content).unwrap();

        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].title, "Deep Feed");
        assert_eq!(feeds[0].url, "https://deep.example.com/feed");
    }

    #[test]
    fn test_parse_empty_opml() {
        // OPML with no feeds (just empty folders) returns empty vec
        let opml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0">
  <head><title>Empty</title></head>
  <body>
    <outline text="Empty Folder"/>
  </body>
</opml>"#;

        let feeds = parse_opml_string(opml_content).unwrap();
        assert!(feeds.is_empty());
    }

    #[test]
    fn test_parse_truly_empty_opml_errors() {
        // OPML library rejects body with zero outlines
        let opml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0">
  <head><title>Empty</title></head>
  <body></body>
</opml>"#;

        let result = parse_opml_string(opml_content);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_opml_with_description() {
        let opml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0">
  <body>
    <outline text="My Blog" type="rss" xmlUrl="https://blog.example.com/feed" description="A great blog about stuff"/>
  </body>
</opml>"#;

        let feeds = parse_opml_string(opml_content).unwrap();

        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].description, Some("A great blog about stuff".to_string()));
    }

    #[test]
    fn test_parse_opml_skips_folders() {
        // Folders (outlines without xmlUrl) should not become feeds
        let opml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0">
  <body>
    <outline text="This is a folder"/>
    <outline text="Real Feed" type="rss" xmlUrl="https://example.com/feed"/>
  </body>
</opml>"#;

        let feeds = parse_opml_string(opml_content).unwrap();

        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].title, "Real Feed");
    }

    #[test]
    fn test_parse_malformed_opml() {
        let bad_content = "this is not xml at all";
        let result = parse_opml_string(bad_content);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_opml_file_not_found() {
        let result = parse_opml_file(Path::new("/nonexistent/path/feeds.opml"));
        assert!(result.is_err());
    }

    #[test]
    fn test_export_opml() {
        let feeds = vec![
            make_feed(1, "Feed One", "https://one.example.com/feed"),
            make_feed(2, "Feed Two", "https://two.example.com/feed"),
        ];

        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_path_buf();

        export_opml_file(&path, &feeds).unwrap();

        // Read back and verify
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Feed One"));
        assert!(content.contains("Feed Two"));
        assert!(content.contains("https://one.example.com/feed"));
        assert!(content.contains("BeatCheck Feeds"));
    }

    #[test]
    fn test_export_empty_feeds() {
        let feeds: Vec<Feed> = vec![];

        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_path_buf();

        export_opml_file(&path, &feeds).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("<body"));
        assert!(content.contains("BeatCheck Feeds"));
    }

    #[test]
    fn test_roundtrip_export_import() {
        let original_feeds = vec![
            make_feed(1, "Ars Technica", "https://feeds.arstechnica.com/arstechnica/index"),
            make_feed(2, "Hacker News", "https://news.ycombinator.com/rss"),
        ];

        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_path_buf();

        // Export
        export_opml_file(&path, &original_feeds).unwrap();

        // Import
        let imported = parse_opml_file(&path).unwrap();

        // Verify round-trip preserves data
        assert_eq!(imported.len(), original_feeds.len());
        for (imported, original) in imported.iter().zip(original_feeds.iter()) {
            assert_eq!(imported.title, original.title);
            assert_eq!(imported.url, original.url);
            assert_eq!(imported.site_url, original.site_url);
            assert_eq!(imported.description, original.description);
        }
    }

    #[test]
    fn test_parse_opml_file_from_disk() {
        let opml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0">
  <body>
    <outline text="Test Feed" type="rss" xmlUrl="https://test.example.com/feed"/>
  </body>
</opml>"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(opml_content.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let feeds = parse_opml_file(temp_file.path()).unwrap();

        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].title, "Test Feed");
    }
}
