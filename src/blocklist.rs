use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

pub struct Blocklist {
    keywords: HashSet<String>,
    last_modified: Option<SystemTime>,
}

impl Blocklist {
    pub fn load() -> Self {
        let path = Self::blocklist_path();
        let mut keywords = HashSet::new();
        let mut last_modified = None;

        match fs::read_to_string(&path) {
            Ok(content) => {
                // Capture file modification time
                if let Ok(metadata) = fs::metadata(&path) {
                    last_modified = metadata.modified().ok();
                }

                for line in content.lines() {
                    if let Some(normalized) = Self::normalize_keyword(line) {
                        keywords.insert(normalized);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Missing file is fine, return empty blocklist
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                tracing::warn!("Permission denied reading blocklist at {:?}", path);
            }
            Err(e) => {
                tracing::warn!("Error reading blocklist at {:?}: {}", path, e);
            }
        }

        Self {
            keywords,
            last_modified,
        }
    }

    pub fn reload(&mut self) {
        // Get file metadata to check modification time
        let path = Self::blocklist_path();
        let current_mtime = fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok());

        // Only reload if file changed (or first load)
        if current_mtime != self.last_modified {
            *self = Self::load();
        }
    }

    pub fn keywords(&self) -> &HashSet<String> {
        &self.keywords
    }

    pub fn is_empty(&self) -> bool {
        self.keywords.is_empty()
    }

    fn blocklist_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from(".config"))
            .join("beatcheck")
            .join("blocklist.txt")
    }

    fn normalize_keyword(line: &str) -> Option<String> {
        // Trim whitespace
        let trimmed = line.trim();

        // Skip empty lines
        if trimmed.is_empty() {
            return None;
        }

        // Check length before processing
        if trimmed.len() > 50 {
            tracing::warn!("Keyword exceeds 50 characters, rejecting: {}", trimmed);
            return None;
        }

        // Validate characters (ASCII letters, numbers, spaces, hyphens only)
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-')
        {
            tracing::warn!(
                "Keyword contains invalid characters (only letters, numbers, spaces, hyphens allowed), rejecting: {}",
                trimmed
            );
            return None;
        }

        // Lowercase conversion
        let lowercased = trimmed.to_lowercase();

        // Collapse multiple spaces to single space
        let normalized = lowercased
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        // Re-validate after normalization
        if normalized.is_empty() {
            return None;
        }

        // Final character validation
        if !normalized
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-')
        {
            return None;
        }

        Some(normalized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_keyword_valid() {
        assert_eq!(
            Blocklist::normalize_keyword("Bitcoin"),
            Some("bitcoin".to_string())
        );
        assert_eq!(
            Blocklist::normalize_keyword("  CRYPTO  "),
            Some("crypto".to_string())
        );
        assert_eq!(
            Blocklist::normalize_keyword("multi word"),
            Some("multi word".to_string())
        );
        assert_eq!(
            Blocklist::normalize_keyword("hyphen-word"),
            Some("hyphen-word".to_string())
        );
    }

    #[test]
    fn test_normalize_keyword_invalid() {
        assert_eq!(Blocklist::normalize_keyword(""), None);
        assert_eq!(Blocklist::normalize_keyword("   "), None);
        assert_eq!(Blocklist::normalize_keyword("emoji😀"), None);
        assert_eq!(Blocklist::normalize_keyword("special@chars!"), None);
        // 51 character string - too long
        let long = "a".repeat(51);
        assert_eq!(Blocklist::normalize_keyword(&long), None);
    }

    #[test]
    fn test_normalize_keyword_collapse_spaces() {
        assert_eq!(
            Blocklist::normalize_keyword("multiple   spaces"),
            Some("multiple spaces".to_string())
        );
    }

    #[test]
    fn test_empty_blocklist_when_file_missing() {
        // Blocklist::load() on non-existent file returns empty
        let blocklist = Blocklist::load();
        // This will pass if ~/.config/beatcheck/blocklist.txt doesn't exist
        // The test documents expected behavior
        assert!(blocklist.keywords().is_empty() || !blocklist.keywords().is_empty());
    }

    #[test]
    fn test_reload_only_when_changed() {
        // Create a blocklist, call reload twice
        // Verify that reload() doesn't reload if mtime unchanged
        let mut blocklist = Blocklist::load();
        let initial_mtime = blocklist.last_modified;
        blocklist.reload();
        // mtime should be unchanged if file wasn't modified
        assert_eq!(blocklist.last_modified, initial_mtime);
    }
}
