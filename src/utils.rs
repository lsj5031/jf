// ============================================================================
// Utility Functions
// ============================================================================

use anyhow::Context;
use colored::Colorize;
use std::io::{self, Write};
use std::path::PathBuf;

/// Validate and normalize a URL
pub fn validate_url(url: &str) -> Result<String, crate::error::FetchError> {
    let url = url.trim();

    // Basic scheme check
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(crate::error::FetchError::Validation {
            message: format!(
                "Invalid URL: must start with http:// or https://\n\
                 Provided: '{}'\n\
                 Tip: jf expects full URLs including the scheme",
                url
            ),
        });
    }

    // Check for obviously malformed URLs
    if url.contains(' ') || url.contains('"') || url.contains('\'') {
        return Err(crate::error::FetchError::Validation {
            message: "Invalid URL: contains invalid characters".to_string(),
        });
    }

    // Try to parse to validate structure
    let parsed = url::Url::parse(url).map_err(|e| crate::error::FetchError::Validation {
        message: format!("Invalid URL format: {}", e),
    })?;

    // Ensure there's a host
    if parsed.host_str().is_none() {
        return Err(crate::error::FetchError::Validation {
            message: "Invalid URL: missing host".to_string(),
        });
    }

    Ok(url.to_string())
}

/// Convert a URL to a safe filename
pub fn url_to_filename(url: &str) -> String {
    let url = url.trim_end_matches('/');

    let path_segment = url
        .split("://")
        .nth(1)
        .and_then(|s| s.split('/').next_back())
        .unwrap_or(url);

    let mut filename = path_segment
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();

    if filename.len() > 100 {
        filename = filename[..100].to_string();
    }
    if !filename.ends_with(".md") {
        filename.push_str(".md");
    }

    filename
}

/// Write output to file or stdout
pub fn write_output(
    content: &str,
    output: Option<&PathBuf>,
    verbose: bool,
) -> Result<(), anyhow::Error> {
    match output {
        Some(path) => {
            std::fs::write(path, content)
                .with_context(|| format!("Failed to write to: {}", path.display()))?;
            if verbose {
                eprintln!(
                    "{} {}",
                    "✓ Written to".green(),
                    path.display().to_string().cyan()
                );
            }
        }
        None => {
            io::stdout().write_all(content.as_bytes())?;
            if !content.is_empty() && !content.ends_with('\n') {
                io::stdout().write_all(b"\n")?;
            }
        }
    }
    Ok(())
}

/// Parse URLs from a file (one per line, supports comments)
pub fn parse_url_file(contents: &str) -> Vec<String> {
    contents
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim().starts_with('#'))
        .map(|s| s.trim().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_url_https() {
        let result = validate_url("https://example.com");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "https://example.com");
    }

    #[test]
    fn test_validate_url_http() {
        let result = validate_url("http://example.com/path");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_url_no_scheme() {
        let result = validate_url("example.com");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_url_with_space() {
        let result = validate_url("https://example .com");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_url_strips_whitespace() {
        let result = validate_url("  https://example.com  ");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "https://example.com");
    }

    #[test]
    fn test_url_to_filename_basic() {
        let filename = url_to_filename("https://example.com/article");
        assert!(filename.ends_with(".md"));
        assert!(filename.contains("article"));
    }

    #[test]
    fn test_url_to_filename_strips_slash() {
        let filename = url_to_filename("https://example.com/");
        assert!(filename.ends_with(".md"));
        assert!(filename.contains("example_com") || filename.contains("example.com"));
    }

    #[test]
    fn test_url_to_filename_truncates_long() {
        let long_url = format!("https://example.com/{}", "a".repeat(200));
        let filename = url_to_filename(&long_url);
        assert!(filename.len() <= 105); // 100 + .md
    }

    #[test]
    fn test_parse_url_file_basic() {
        let contents = "https://example.com\nhttps://test.com\n";
        let urls = parse_url_file(contents);
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0], "https://example.com");
    }

    #[test]
    fn test_parse_url_file_ignores_comments() {
        let contents = "# This is a comment\nhttps://example.com\n# Another comment\n";
        let urls = parse_url_file(contents);
        assert_eq!(urls.len(), 1);
    }

    #[test]
    fn test_parse_url_file_ignores_empty_lines() {
        let contents = "\n\nhttps://example.com\n\n\n";
        let urls = parse_url_file(contents);
        assert_eq!(urls.len(), 1);
    }

    #[test]
    fn test_parse_url_file_strips_whitespace() {
        let contents = "  https://example.com  \n  https://test.com  ";
        let urls = parse_url_file(contents);
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0], "https://example.com");
    }
}
