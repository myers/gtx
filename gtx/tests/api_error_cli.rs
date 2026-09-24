//! API failures must carry the server's reason, not just the status line (#7).
//! Gitea's error bodies are `{"message": ..., "url": ...}`; the format follows
//! `gh`: `HTTP <code>: <message> (<request url>)`.

mod common;

use std::sync::{Arc, OnceLock};

use predicates::prelude::*;

use common::FakeGitea;

const EXISTS: &str =
    r#"{"message":"label already exists","url":"https://gitea.example/api/swagger"}"#;

/// A status the spec documents for the operation (progenitor's `ErrorResponse`).
#[test]
fn documented_error_status_shows_server_message() {
    let server = FakeGitea::start_with(|_, _| {
        (
            404,
            r#"{"message":"issue does not exist [id: 0, repo_id: 1, index: 99]","url":"x"}"#.into(),
        )
    });
    let url = format!("{}/api/v1/repos/o/r/issues/99", server.url);
    server
        .gtx()
        .args(["issue", "view", "-R", "o/r", "99"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(format!(
            "HTTP 404: issue does not exist [id: 0, repo_id: 1, index: 99] ({url})"
        )));
}

/// A status the spec does not list (progenitor's `UnexpectedResponse`).
#[test]
fn undocumented_error_status_shows_server_message() {
    let server = FakeGitea::start_with(|_, _| (409, EXISTS.into()));
    server
        .gtx()
        .args([
            "label", "create", "-R", "o/r", "--name", "bug", "--color", "ff0000",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("HTTP 409: label already exists ("));
}

/// A non-JSON body is shown as-is (first line, truncated).
#[test]
fn plain_text_error_body_is_shown() {
    let long = "x".repeat(500);
    let server = FakeGitea::start_with({
        let long = long.clone();
        move |_, _| (500, format!("database is locked {long}\nmore"))
    });
    let assert = server
        .gtx()
        .args(["issue", "view", "-R", "o/r", "1"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("HTTP 500: database is locked xxx"));
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(!stderr.contains("more"), "only the first line: {stderr}");
    assert!(!stderr.contains(&long), "truncated: {stderr}");
}

/// An empty body falls back to the canonical reason.
#[test]
fn empty_error_body_falls_back_to_reason() {
    let server = FakeGitea::start_with(|_, _| (403, String::new()));
    server
        .gtx()
        .args(["issue", "view", "-R", "o/r", "1"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("HTTP 403: Forbidden ("));
}

/// Hand-rolled request path (`raw_request`).
#[test]
fn raw_request_error_shows_server_message() {
    let server =
        FakeGitea::start_with(|_, _| (403, r#"{"message":"user should be an owner"}"#.into()));
    server
        .gtx()
        .args(["runner", "registration-token", "-R", "o/r"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "HTTP 403: user should be an owner",
        ));
}

/// `Gitea::download` path (release asset).
#[test]
fn release_download_error_shows_server_message() {
    let base: Arc<OnceLock<String>> = Arc::new(OnceLock::new());
    let server = FakeGitea::start_with({
        let base = Arc::clone(&base);
        move |_, target| match target {
            "/api/v1/repos/o/r/releases/4" => (
                200,
                format!(
                    r#"{{"id":4,"tag_name":"v1","assets":[{{"id":9,"name":"dbg.txt","browser_download_url":"{}/attachments/gone"}}]}}"#,
                    base.get().unwrap()
                ),
            ),
            _ => (404, r#"{"message":"attachment does not exist"}"#.into()),
        }
    });
    base.set(server.url.clone()).unwrap();
    let dir = std::env::temp_dir().join(format!("gtx-release-err-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    server
        .gtx()
        .current_dir(&dir)
        .args(["release", "download", "-R", "o/r", "4"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "HTTP 404: attachment does not exist",
        ));
    let _ = std::fs::remove_dir_all(&dir);
}
