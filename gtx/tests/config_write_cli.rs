//! Every command that writes the config file must read-modify-write it,
//! leaving unrelated sections (and comments) intact. Each test points
//! `GTX_CONFIG` at a temp file so the real config is never touched.

use assert_cmd::Command;
use std::path::Path;

const EXISTING: &str = r#"# my gtx config
[default]
url = "https://old.example.com"
token = "old-token"

[servers.work]
url = "https://gitea.work.com" # work instance
token = "work-token"

[aliases]
co = "pr checkout"
"#;

fn gtx(config: &Path) -> Command {
    let mut cmd = Command::cargo_bin("gtx").unwrap();
    cmd.env("GTX_CONFIG", config)
        .env_remove("GITEA_URL")
        .env_remove("GITEA_TOKEN")
        .env_remove("GITEA_SERVER");
    cmd
}

fn setup() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, EXISTING).unwrap();
    (dir, path)
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

fn assert_preserved(content: &str) {
    assert!(content.contains("# my gtx config"), "{content}");
    assert!(content.contains("[servers.work]"), "{content}");
    assert!(
        content.contains(r#"url = "https://gitea.work.com" # work instance"#),
        "{content}"
    );
    assert!(content.contains(r#"token = "work-token""#), "{content}");
    assert!(content.contains("[aliases]"), "{content}");
    assert!(content.contains(r#"co = "pr checkout""#), "{content}");
}

#[test]
fn auth_login_preserves_other_sections() {
    let (_dir, path) = setup();
    gtx(&path)
        .args(["auth", "login", "--url", "https://new.example.com", "--token", "new-token"])
        .assert()
        .success();

    let content = read(&path);
    assert_preserved(&content);
    let parsed: toml::Table = content.parse().unwrap();
    assert_eq!(parsed["default"]["url"].as_str(), Some("https://new.example.com"));
    assert_eq!(parsed["default"]["token"].as_str(), Some("new-token"));
    assert!(!content.contains("old-token"), "{content}");
}

#[test]
fn auth_login_creates_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sub").join("config.toml");
    gtx(&path)
        .args(["auth", "login", "--url", "https://new.example.com", "--token", "a\"b"])
        .assert()
        .success();

    let parsed: toml::Table = read(&path).parse().unwrap();
    assert_eq!(parsed["default"]["url"].as_str(), Some("https://new.example.com"));
    assert_eq!(parsed["default"]["token"].as_str(), Some("a\"b"));
}

#[test]
fn auth_logout_removes_only_default_credentials() {
    let (_dir, path) = setup();
    gtx(&path).args(["auth", "logout"]).assert().success();

    let content = read(&path);
    assert_preserved(&content);
    assert!(!content.contains("old-token"), "{content}");
    assert!(!content.contains("https://old.example.com"), "{content}");
}

#[test]
fn config_set_preserves_other_sections() {
    let (_dir, path) = setup();
    gtx(&path)
        .args(["config", "set", "default.url", "https://set.example.com"])
        .assert()
        .success();

    let content = read(&path);
    assert_preserved(&content);
    assert!(content.contains(r#"token = "old-token""#), "{content}");
    let parsed: toml::Table = content.parse().unwrap();
    assert_eq!(parsed["default"]["url"].as_str(), Some("https://set.example.com"));
}

#[test]
fn alias_set_and_delete_preserve_other_sections() {
    let (_dir, path) = setup();
    gtx(&path)
        .args(["alias", "set", "iv", "issue view"])
        .assert()
        .success();
    let content = read(&path);
    assert_preserved(&content);
    assert!(content.contains(r#"iv = "issue view""#), "{content}");

    gtx(&path).args(["alias", "delete", "iv"]).assert().success();
    let content = read(&path);
    assert_preserved(&content);
    assert!(!content.contains("iv ="), "{content}");
    assert!(content.contains(r#"token = "old-token""#), "{content}");
}

#[test]
fn writers_refuse_to_clobber_invalid_config() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "not valid toml {{{{\n").unwrap();

    gtx(&path)
        .args(["alias", "set", "iv", "issue view"])
        .assert()
        .failure();
    gtx(&path)
        .args(["auth", "login", "--url", "https://x.example.com", "--token", "t"])
        .assert()
        .failure();
    assert_eq!(read(&path), "not valid toml {{{{\n");
}

#[test]
fn config_set_nested_key_creates_server_profile() {
    let (_dir, path) = setup();
    gtx(&path)
        .args(["config", "set", "servers.home.url", "https://home.example.com"])
        .assert()
        .success();
    gtx(&path)
        .args(["config", "set", "servers.work.url", "https://new.work.com"])
        .assert()
        .success();

    let content = read(&path);
    assert!(content.contains("[servers.home]"), "{content}");
    assert!(!content.contains("[servers]\n"), "{content}");
    assert!(content.contains("# my gtx config"), "{content}");
    let parsed: toml::Table = content.parse().unwrap();
    assert_eq!(parsed["servers"]["home"]["url"].as_str(), Some("https://home.example.com"));
    assert_eq!(parsed["servers"]["work"]["url"].as_str(), Some("https://new.work.com"));
    assert_eq!(parsed["servers"]["work"]["token"].as_str(), Some("work-token"));

    gtx(&path)
        .args(["config", "get", "servers.home.url"])
        .assert()
        .success()
        .stdout("https://home.example.com\n");
}

#[test]
fn config_set_rejects_bad_keys() {
    let (_dir, path) = setup();
    for key in ["default.url.x", "a..b", ".a", "a."] {
        gtx(&path).args(["config", "set", key, "v"]).assert().failure();
    }
    assert_eq!(read(&path), EXISTING);
}

#[test]
fn config_list_recurses_and_masks_tokens() {
    let (_dir, path) = setup();
    let out = gtx(&path).args(["config", "list"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("default.url = https://old.example.com\n"), "{stdout}");
    assert!(stdout.contains("servers.work.url = https://gitea.work.com\n"), "{stdout}");
    assert!(stdout.contains("servers.work.token = "), "{stdout}");
    assert!(stdout.contains("default.token = "), "{stdout}");
    assert!(stdout.contains("aliases.co = pr checkout\n"), "{stdout}");
    assert!(!stdout.contains("old-token"), "{stdout}");
    assert!(!stdout.contains("work-token"), "{stdout}");
    // gh-style: no prefix underscore, so every character is starred.
    assert!(stdout.contains("default.token = *********\n"), "{stdout}");
    assert!(stdout.contains("servers.work.token = **********\n"), "{stdout}");
}

#[test]
fn auth_status_masks_non_ascii_token_without_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[default]\nurl = \"https://gitea.example.com\"\ntoken = \"abcdefg\u{e9}xyz\"\n",
    )
    .unwrap();
    gtx(&path)
        .args(["auth", "status", "--no-check"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Token: ***********\n"));
}

#[test]
fn auth_token_prints_resolved_token() {
    let (_dir, path) = setup();
    gtx(&path).args(["auth", "token"]).assert().success().stdout("old-token\n");
    gtx(&path)
        .env("GITEA_SERVER", "work")
        .args(["auth", "token"])
        .assert()
        .success()
        .stdout("work-token\n");
}
