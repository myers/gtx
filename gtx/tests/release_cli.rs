mod common;

use std::sync::{Arc, OnceLock};

use common::FakeGitea;

/// Gitea's `browser_download_url` is a web route (`{root}/attachments/{uuid}`),
/// not an API path, and it 303s to the login page unless the token is sent.
#[test]
fn release_download_fetches_web_url_with_token() {
    let base: Arc<OnceLock<String>> = Arc::new(OnceLock::new());
    let server = FakeGitea::start_with({
        let base = Arc::clone(&base);
        move |_, target| match target {
            "/api/v1/repos/o/r/releases/4" => (
                200,
                format!(
                    r#"{{"id":4,"tag_name":"v1","assets":[{{"id":9,"name":"dbg.txt","browser_download_url":"{}/attachments/abc-123"}}]}}"#,
                    base.get().unwrap()
                ),
            ),
            "/attachments/abc-123" => (200, "asset bytes\n".into()),
            _ => (404, "{}".into()),
        }
    });
    base.set(server.url.clone()).unwrap();

    let dir = std::env::temp_dir().join(format!("gtx-release-dl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    server
        .gtx()
        .current_dir(&dir)
        .args(["release", "download", "-R", "o/r", "4"])
        .assert()
        .success();

    assert_eq!(std::fs::read_to_string(dir.join("dbg.txt")).unwrap(), "asset bytes\n");
    let seen = server.seen();
    assert_eq!(seen[1], "GET /attachments/abc-123", "{seen:?}");
    assert_eq!(server.auth()[1].as_deref(), Some("token t"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The token must not follow a download URL off to another host.
#[test]
fn release_download_withholds_token_from_foreign_host() {
    let other = FakeGitea::start_with(|_, _| (200, "elsewhere\n".into()));
    let other_url = other.url.clone();
    let server = FakeGitea::start_with(move |_, target| match target {
        "/api/v1/repos/o/r/releases/4" => (
            200,
            format!(
                r#"{{"id":4,"tag_name":"v1","assets":[{{"id":9,"name":"dbg.txt","browser_download_url":"{other_url}/attachments/abc-123"}}]}}"#
            ),
        ),
        _ => (404, "{}".into()),
    });

    let dir = std::env::temp_dir().join(format!("gtx-release-dl-foreign-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    server
        .gtx()
        .current_dir(&dir)
        .args(["release", "download", "-R", "o/r", "4"])
        .assert()
        .success();

    assert_eq!(other.seen(), ["GET /attachments/abc-123"]);
    assert_eq!(other.auth(), [None]);
    let _ = std::fs::remove_dir_all(&dir);
}
