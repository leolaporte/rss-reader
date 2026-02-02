use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::{AppError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_db_path")]
    pub db_path: String,

    pub claude_api_key: Option<String>,
    pub raindrop_token: Option<String>,

    #[serde(default = "default_refresh_interval")]
    pub refresh_interval_minutes: u32,

    #[serde(default)]
    pub default_tags: Vec<String>,
}

fn default_db_path() -> String {
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("beatcheck");
    std::fs::create_dir_all(&data_dir).ok();
    data_dir.join("feeds.db").to_string_lossy().to_string()
}

fn default_refresh_interval() -> u32 {
    30
}

impl Default for Config {
    fn default() -> Self {
        Self {
            db_path: default_db_path(),
            claude_api_key: None,
            raindrop_token: None,
            refresh_interval_minutes: default_refresh_interval(),
            default_tags: vec!["rss".to_string()],
        }
    }
}

impl Config {
    /// Parse config from a TOML string
    pub fn from_str(content: &str) -> Result<Self> {
        let config: Config = toml::from_str(content)?;
        Ok(config)
    }

    /// Serialize config to a TOML string
    pub fn to_string(&self) -> Result<String> {
        toml::to_string_pretty(self).map_err(|e| AppError::Config(e.to_string()).into())
    }

    pub fn load() -> Result<Self> {
        let config_path = Self::config_path();

        let mut config = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            toml::from_str(&content)?
        } else {
            let config = Config::default();
            config.save()?;
            config
        };

        // Environment variables override config file values
        if let Ok(key) = std::env::var("CLAUDE_API_KEY") {
            config.claude_api_key = Some(key);
        }
        if let Ok(token) = std::env::var("RAINDROP_TOKEN") {
            config.raindrop_token = Some(token);
        }

        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let config_path = Self::config_path();
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)
            .map_err(|e| AppError::Config(e.to_string()))?;
        std::fs::write(config_path, content)?;
        Ok(())
    }

    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("beatcheck")
            .join("config.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== Default values ====================

    #[test]
    fn test_default_config() {
        let config = Config::default();

        assert!(config.db_path.contains("beatcheck"));
        assert!(config.db_path.ends_with("feeds.db"));
        assert_eq!(config.claude_api_key, None);
        assert_eq!(config.raindrop_token, None);
        assert_eq!(config.refresh_interval_minutes, 30);
        assert_eq!(config.default_tags, vec!["rss".to_string()]);
    }

    #[test]
    fn test_default_refresh_interval() {
        assert_eq!(default_refresh_interval(), 30);
    }

    // ==================== TOML parsing ====================

    #[test]
    fn test_parse_full_config() {
        let toml = r#"
db_path = "/custom/path/feeds.db"
claude_api_key = "sk-test-key"
raindrop_token = "rd-token-123"
refresh_interval_minutes = 60
default_tags = ["news", "tech"]
"#;

        let config = Config::from_str(toml).unwrap();

        assert_eq!(config.db_path, "/custom/path/feeds.db");
        assert_eq!(config.claude_api_key, Some("sk-test-key".to_string()));
        assert_eq!(config.raindrop_token, Some("rd-token-123".to_string()));
        assert_eq!(config.refresh_interval_minutes, 60);
        assert_eq!(config.default_tags, vec!["news", "tech"]);
    }

    #[test]
    fn test_parse_minimal_config_uses_defaults() {
        // Empty config should use all defaults
        let toml = "";

        let config = Config::from_str(toml).unwrap();

        // db_path gets default
        assert!(config.db_path.contains("beatcheck"));
        assert_eq!(config.claude_api_key, None);
        assert_eq!(config.raindrop_token, None);
        assert_eq!(config.refresh_interval_minutes, 30);
        assert!(config.default_tags.is_empty()); // serde default for Vec is empty
    }

    #[test]
    fn test_parse_partial_config() {
        let toml = r#"
refresh_interval_minutes = 15
default_tags = ["podcast"]
"#;

        let config = Config::from_str(toml).unwrap();

        // Specified values
        assert_eq!(config.refresh_interval_minutes, 15);
        assert_eq!(config.default_tags, vec!["podcast"]);

        // Defaults for unspecified
        assert!(config.db_path.contains("beatcheck"));
        assert_eq!(config.claude_api_key, None);
        assert_eq!(config.raindrop_token, None);
    }

    #[test]
    fn test_parse_config_with_only_api_keys() {
        let toml = r#"
claude_api_key = "my-claude-key"
raindrop_token = "my-raindrop-token"
"#;

        let config = Config::from_str(toml).unwrap();

        assert_eq!(config.claude_api_key, Some("my-claude-key".to_string()));
        assert_eq!(config.raindrop_token, Some("my-raindrop-token".to_string()));
        // Defaults applied
        assert_eq!(config.refresh_interval_minutes, 30);
    }

    #[test]
    fn test_parse_invalid_toml() {
        let bad_toml = "this is not valid toml [[[";

        let result = Config::from_str(bad_toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_wrong_type() {
        let toml = r#"
refresh_interval_minutes = "not a number"
"#;

        let result = Config::from_str(toml);
        assert!(result.is_err());
    }

    // ==================== Serialization ====================

    #[test]
    fn test_serialize_config() {
        let config = Config {
            db_path: "/test/feeds.db".to_string(),
            claude_api_key: Some("test-key".to_string()),
            raindrop_token: None,
            refresh_interval_minutes: 45,
            default_tags: vec!["a".to_string(), "b".to_string()],
        };

        let toml = config.to_string().unwrap();

        assert!(toml.contains("db_path = \"/test/feeds.db\""));
        assert!(toml.contains("claude_api_key = \"test-key\""));
        assert!(toml.contains("refresh_interval_minutes = 45"));
        assert!(toml.contains("default_tags = ["));
    }

    #[test]
    fn test_roundtrip_serialization() {
        let original = Config {
            db_path: "/my/custom/path.db".to_string(),
            claude_api_key: Some("key123".to_string()),
            raindrop_token: Some("token456".to_string()),
            refresh_interval_minutes: 120,
            default_tags: vec!["tag1".to_string(), "tag2".to_string(), "tag3".to_string()],
        };

        let toml = original.to_string().unwrap();
        let parsed = Config::from_str(&toml).unwrap();

        assert_eq!(parsed.db_path, original.db_path);
        assert_eq!(parsed.claude_api_key, original.claude_api_key);
        assert_eq!(parsed.raindrop_token, original.raindrop_token);
        assert_eq!(parsed.refresh_interval_minutes, original.refresh_interval_minutes);
        assert_eq!(parsed.default_tags, original.default_tags);
    }

    // ==================== Edge cases ====================

    #[test]
    fn test_empty_tags_array() {
        let toml = r#"
default_tags = []
"#;

        let config = Config::from_str(toml).unwrap();
        assert!(config.default_tags.is_empty());
    }

    #[test]
    fn test_zero_refresh_interval() {
        let toml = r#"
refresh_interval_minutes = 0
"#;

        let config = Config::from_str(toml).unwrap();
        assert_eq!(config.refresh_interval_minutes, 0);
    }

    #[test]
    fn test_large_refresh_interval() {
        let toml = r#"
refresh_interval_minutes = 10080
"#;

        let config = Config::from_str(toml).unwrap();
        assert_eq!(config.refresh_interval_minutes, 10080); // 1 week in minutes
    }

    #[test]
    fn test_special_characters_in_api_key() {
        let toml = r#"
claude_api_key = "sk-ant-api03-abc123!@#$%^&*()_+-=[]{}|;':\",./<>?"
"#;

        let config = Config::from_str(toml).unwrap();
        assert!(config.claude_api_key.is_some());
        assert!(config.claude_api_key.unwrap().starts_with("sk-ant-api03"));
    }

    #[test]
    fn test_unicode_in_tags() {
        let toml = r#"
default_tags = ["日本語", "émoji", "🎉"]
"#;

        let config = Config::from_str(toml).unwrap();
        assert_eq!(config.default_tags.len(), 3);
        assert_eq!(config.default_tags[0], "日本語");
        assert_eq!(config.default_tags[2], "🎉");
    }

    #[test]
    fn test_config_path_contains_beatcheck() {
        let path = Config::config_path();
        assert!(path.to_string_lossy().contains("beatcheck"));
        assert!(path.to_string_lossy().ends_with("config.toml"));
    }
}
