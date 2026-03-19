// ============================================================================
// Configuration Module
// ============================================================================

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Default configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Default timeout in seconds
    pub timeout: u64,
    /// Default number of retries
    pub retries: u32,
    /// Wait for JavaScript rendering by default
    pub wait_render: bool,
    /// Default output directory for batch mode
    pub output_dir: Option<PathBuf>,
    /// Concurrent requests for batch mode (0 = unlimited)
    pub concurrency: Option<usize>,
    /// Default user agent
    pub user_agent: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            timeout: 30,
            retries: 3,
            wait_render: false,
            output_dir: None,
            concurrency: None,
            user_agent: None,
        }
    }
}

impl Config {
    /// Load config from file, falling back to defaults
    #[allow(dead_code)]
    pub fn load_from_file(path: &PathBuf) -> Option<Self> {
        match fs::read_to_string(path) {
            Ok(contents) => toml::from_str(&contents).ok(),
            Err(_) => None,
        }
    }

    /// Find config file in standard locations
    #[allow(dead_code)]
    pub fn find_config_file() -> Option<PathBuf> {
        // Check current directory first
        let local_paths = vec![".jf.toml", "jf.toml", ".jf/config.toml"];
        for name in local_paths {
            let path = PathBuf::from(name);
            if path.exists() {
                return Some(path);
            }
        }

        // Check home directory
        if let Some(home) = dirs::home_dir() {
            let home_paths = vec![home.join(".jf/config.toml"), home.join(".jf.toml")];
            for path in home_paths {
                if path.exists() {
                    return Some(path);
                }
            }
        }

        None
    }

    /// Load config from standard locations or return defaults
    #[allow(dead_code)]
    pub fn load() -> Self {
        Self::find_config_file()
            .and_then(|path| Self::load_from_file(&path))
            .unwrap_or_default()
    }

    /// Create a merged config with CLI overrides
    #[allow(dead_code)]
    pub fn with_cli_overrides(
        &self,
        timeout: Option<u64>,
        retries: Option<u32>,
        wait_render: Option<bool>,
        output_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            timeout: timeout.unwrap_or(self.timeout),
            retries: retries.unwrap_or(self.retries),
            wait_render: wait_render.unwrap_or(self.wait_render),
            output_dir: output_dir.or(self.output_dir.clone()),
            concurrency: self.concurrency,
            user_agent: self.user_agent.clone(),
        }
    }
}

/// Retry configuration with exponential backoff
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_retries: u32,
    /// Initial backoff duration
    pub initial_backoff: std::time::Duration,
    /// Maximum backoff duration
    pub max_backoff: std::time::Duration,
    /// Multiplier for exponential backoff
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff: std::time::Duration::from_millis(500),
            max_backoff: std::time::Duration::from_secs(30),
            backoff_multiplier: 2.0,
        }
    }
}

impl RetryConfig {
    /// Calculate backoff duration for a given attempt (0-indexed)
    pub fn backoff_for_attempt(&self, attempt: u32) -> std::time::Duration {
        let backoff_secs =
            self.initial_backoff.as_secs_f64() * self.backoff_multiplier.powi(attempt as i32);
        let capped = backoff_secs.min(self.max_backoff.as_secs_f64());
        std::time::Duration::from_secs_f64(capped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.timeout, 30);
        assert_eq!(config.retries, 3);
        assert!(!config.wait_render);
        assert!(config.output_dir.is_none());
    }

    #[test]
    fn test_config_with_cli_overrides() {
        let config = Config::default();
        let merged = config.with_cli_overrides(Some(60), Some(5), Some(true), None);
        assert_eq!(merged.timeout, 60);
        assert_eq!(merged.retries, 5);
        assert!(merged.wait_render);
    }

    #[test]
    fn test_config_from_file() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");
        fs::write(
            &config_path,
            r#"
timeout = 60
retries = 5
wait_render = true
"#,
        )
        .unwrap();

        let config = Config::load_from_file(&config_path).unwrap();
        assert_eq!(config.timeout, 60);
        assert_eq!(config.retries, 5);
        assert!(config.wait_render);
    }

    #[test]
    fn test_config_from_invalid_file() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("invalid.toml");
        fs::write(&config_path, "invalid toml {").unwrap();

        let config = Config::load_from_file(&config_path);
        assert!(config.is_none());
    }

    #[test]
    fn test_retry_backoff_calculation() {
        let config = RetryConfig::default();

        // First attempt: 500ms * 2^0 = 500ms
        assert_eq!(config.backoff_for_attempt(0).as_millis(), 500);

        // Second attempt: 500ms * 2^1 = 1000ms
        assert_eq!(config.backoff_for_attempt(1).as_millis(), 1000);

        // Third attempt: 500ms * 2^2 = 2000ms
        assert_eq!(config.backoff_for_attempt(2).as_millis(), 2000);
    }

    #[test]
    fn test_retry_backoff_cap() {
        let config = RetryConfig {
            max_backoff: std::time::Duration::from_millis(1000),
            ..Default::default()
        };

        // Should be capped at 1000ms
        assert_eq!(config.backoff_for_attempt(10).as_millis(), 1000);
    }
}
