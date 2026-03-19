//! CLI Integration Tests
//!
//! Tests for command-line interface arguments and behavior

use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::process::Command;
use tempfile::TempDir;

/// Test that `--help` shows all available commands
#[test]
fn test_help_shows_commands() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Commands:"))
        .stdout(predicate::str::contains("fetch"))
        .stdout(predicate::str::contains("batch"))
        .stdout(predicate::str::contains("stdin"))
        .stdout(predicate::str::contains("selector"))
        .stdout(predicate::str::contains("check"));
}

/// Test fetch command help
#[test]
fn test_fetch_help() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("fetch")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("URL to fetch"))
        .stdout(predicate::str::contains("-o, --output"))
        .stdout(predicate::str::contains("-v, --verbose"))
        .stdout(predicate::str::contains("-w, --wait-render"));
}

/// Test batch command help
#[test]
fn test_batch_help() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("batch")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("File containing URLs"))
        .stdout(predicate::str::contains("-d, --dir"))
        .stdout(predicate::str::contains("-c, --concurrency"));
}

/// Test selector command help
#[test]
fn test_selector_help() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("selector")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("CSS selector"));
}

/// Test stdin command help
#[test]
fn test_stdin_help() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("stdin")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Read URL from stdin"));
}

/// Test check command help
#[test]
fn test_check_help() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("check")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Check API connectivity"));
}

/// Test missing URL argument shows error
#[test]
fn test_fetch_missing_url() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("fetch")
        .assert()
        .failure()
        .stderr(predicate::str::contains("the following required arguments were not provided"));
}

/// Test invalid URL shows validation error
#[test]
fn test_fetch_invalid_url() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("fetch")
        .arg("not-a-url")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid URL"));
}

/// Test URL without scheme shows helpful error
#[test]
fn test_fetch_url_without_scheme() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("fetch")
        .arg("example.com")
        .assert()
        .failure()
        .stderr(predicate::str::contains("http:// or https://"));
}

/// Test batch with missing file shows error
#[test]
fn test_batch_missing_file() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("batch")
        .arg("nonexistent.txt")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Failed to read URL file"));
}

/// Test batch with empty file shows error
#[test]
fn test_batch_empty_file() {
    let temp_dir = TempDir::new().unwrap();
    let empty_file = temp_dir.path().join("empty.txt");
    std::fs::write(&empty_file, "").unwrap();

    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("batch")
        .arg(empty_file)
        .assert()
        .failure()
        .stderr(predicate::str::contains("No URLs found"));
}

/// Test batch with only comments shows error
#[test]
fn test_batch_only_comments() {
    let temp_dir = TempDir::new().unwrap();
    let file = temp_dir.path().join("comments.txt");
    std::fs::write(&file, "# This is a comment\n# Another comment").unwrap();

    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("batch")
        .arg(file)
        .assert()
        .failure()
        .stderr(predicate::str::contains("No URLs found"));
}

/// Test stdin with no input shows error
#[test]
fn test_stdin_empty_input() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("stdin")
        .stdin(std::process::Stdio::null())
        .assert()
        .failure()
        .stderr(predicate::str::contains("No URL provided"));
}

/// Test version flag works
#[test]
fn test_version_flag() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("jf"))
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

/// Test visible aliases work for fetch command
#[test]
fn test_fetch_alias() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("get")
        .arg("--help")
        .assert()
        .success();
}

/// Test visible aliases work for selector command
#[test]
fn test_selector_alias() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("extract")
        .arg("--help")
        .assert()
        .success();
}

/// Test timeout flag parsing
#[test]
fn test_fetch_timeout_flag() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("fetch")
        .arg("https://example.com")
        .arg("-t")
        .arg("60")
        .arg("--help")
        .assert()
        .success();
}

/// Test retries flag parsing
#[test]
fn test_fetch_retries_flag() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("fetch")
        .arg("https://example.com")
        .arg("-r")
        .arg("5")
        .arg("--help")
        .assert()
        .success();
}

/// Test no-retry flag exists
#[test]
fn test_fetch_no_retry_flag() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("fetch")
        .arg("https://example.com")
        .arg("--no-retry")
        .arg("--help")
        .assert()
        .success();
}

/// Test verbose flag
#[test]
fn test_fetch_verbose_flag() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("fetch")
        .arg("https://example.com")
        .arg("-v")
        .arg("--help")
        .assert()
        .success();
}

/// Test wait-render flag
#[test]
fn test_fetch_wait_render_flag() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("fetch")
        .arg("https://example.com")
        .arg("-w")
        .arg("--help")
        .assert()
        .success();
}

/// Test output flag
#[test]
fn test_fetch_output_flag() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("fetch")
        .arg("https://example.com")
        .arg("-o")
        .arg("output.md")
        .arg("--help")
        .assert()
        .success();
}

/// Test concurrency flag for batch
#[test]
fn test_batch_concurrency_flag() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("batch")
        .arg("urls.txt")
        .arg("-c")
        .arg("10")
        .arg("--help")
        .assert()
        .success();
}

/// Test continue-on-error flag for batch
#[test]
fn test_batch_continue_flag() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("batch")
        .arg("urls.txt")
        .arg("--continue-on-error")
        .arg("--help")
        .assert()
        .success();
}

/// Test batch valid URL file parsing (comments and blank lines)
#[test]
fn test_batch_file_with_comments() {
    let temp_dir = TempDir::new().unwrap();
    let file = temp_dir.path().join("urls.txt");
    std::fs::write(
        &file,
        "# Comment\nhttps://example.com\n\nhttps://test.com\n# Another",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("batch")
        .arg(file)
        .arg("--help")
        .assert()
        .success();
}

/// Test selector flag
#[test]
fn test_selector_selector_flag() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("selector")
        .arg("https://example.com")
        .arg("-s")
        .arg("article")
        .arg("--help")
        .assert()
        .success();
}

/// Test check timeout flag
#[test]
fn test_check_timeout_flag() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("check")
        .arg("-t")
        .arg("5")
        .arg("--help")
        .assert()
        .success();
}

/// Test completions command for bash
#[test]
fn test_completions_bash() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("completions")
        .arg("bash")
        .assert()
        .success()
        .stdout(predicate::str::contains("_jf"));
}

/// Test completions command for zsh
#[test]
fn test_completions_zsh() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("completions")
        .arg("zsh")
        .assert()
        .success()
        .stdout(predicate::str::contains("#compdef"));
}

/// Test completions command help
#[test]
fn test_completions_help() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("completions")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Shell to generate completions for"));
}

/// Test unknown subcommand shows error
#[test]
fn test_unknown_command() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("unknown-command")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));
}

/// Test unknown flag shows error
#[test]
fn test_unknown_flag() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("fetch")
        .arg("https://example.com")
        .arg("--unknown-flag")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument"));
}

/// Test stdin with --no-retry flag
#[test]
fn test_stdin_no_retry_flag() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("stdin")
        .arg("--no-retry")
        .arg("--help")
        .assert()
        .success();
}

/// Test stdin timeout and retries flags
#[test]
fn test_stdin_timeout_retries() {
    let mut cmd = Command::cargo_bin("jf").unwrap();
    cmd.arg("stdin")
        .arg("-t")
        .arg("60")
        .arg("-r")
        .arg("5")
        .arg("--help")
        .assert()
        .success();
}