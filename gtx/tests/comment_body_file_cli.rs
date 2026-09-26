//! `issue comment` and `pr comment` take gh's `-F/--body-file` (`-` for stdin),
//! mutually exclusive with `-b/--body`, and upload local refs like `issue create -F`.

mod common;

use common::FakeGitea;
use predicates::str::contains;

fn server() -> FakeGitea {
    FakeGitea::start_with(|method, target| {
        let path = target.split('?').next().unwrap_or(target);
        let body = match (method, path) {
            ("POST", "/api/v1/repos/o/r/issues/7/comments") => r#"{"id":1}"#,
            ("POST", "/api/v1/repos/o/r/issues/7/assets") => {
                r#"{"id":2,"browser_download_url":"http://h/attachments/abc"}"#
            }
            _ => "{}",
        };
        (200, body.to_string())
    })
}

/// (request, body) pairs the server saw, minus GETs.
fn writes(s: &FakeGitea) -> Vec<(String, String)> {
    s.seen()
        .into_iter()
        .zip(s.bodies())
        .filter(|(r, _)| !r.starts_with("GET "))
        .collect()
}

fn tmp_dir(test: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("gtx-comment-{}-{test}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The one comment POST's `body` field.
fn posted_comment(s: &FakeGitea) -> String {
    let w = writes(s);
    let posts: Vec<_> = w
        .iter()
        .filter(|(r, _)| r == "POST /api/v1/repos/o/r/issues/7/comments")
        .collect();
    assert_eq!(posts.len(), 1, "{w:?}");
    let v: serde_json::Value = serde_json::from_str(&posts[0].1).unwrap();
    v["body"].as_str().unwrap().to_string()
}

#[test]
fn issue_comment_body_file() {
    let s = server();
    let f = tmp_dir("issue-file").join("c.md");
    std::fs::write(&f, "from a file\n").unwrap();
    s.gtx()
        .args(["issue", "comment", "-R", "o/r", "7", "-F"])
        .arg(&f)
        .assert()
        .success();
    assert_eq!(posted_comment(&s), "from a file\n");
}

#[test]
fn issue_comment_body_file_stdin() {
    let s = server();
    s.gtx()
        .args(["issue", "comment", "-R", "o/r", "7", "--body-file", "-"])
        .write_stdin("from stdin")
        .assert()
        .success();
    assert_eq!(posted_comment(&s), "from stdin");
}

#[test]
fn pr_comment_body_file() {
    let s = server();
    let f = tmp_dir("pr-file").join("c.md");
    std::fs::write(&f, "pr comment").unwrap();
    s.gtx()
        .args(["pr", "comment", "-R", "o/r", "7", "-F"])
        .arg(&f)
        .assert()
        .success();
    assert_eq!(posted_comment(&s), "pr comment");
}

#[test]
fn comment_body_file_uploads_local_refs() {
    let s = server();
    let dir = tmp_dir("refs");
    std::fs::write(dir.join("shot.png"), b"png").unwrap();
    let f = dir.join("c.md");
    std::fs::write(&f, "see ![shot](shot.png)").unwrap();
    s.gtx()
        .args(["issue", "comment", "-R", "o/r", "7", "-F"])
        .arg(&f)
        .assert()
        .success();
    let w = writes(&s);
    assert!(
        w[0].0.starts_with("POST /api/v1/repos/o/r/issues/7/assets"),
        "{w:?}"
    );
    assert_eq!(posted_comment(&s), "see ![shot](http://h/attachments/abc)");
}

#[test]
fn body_and_body_file_conflict() {
    for cmd in ["issue", "pr"] {
        let s = server();
        s.gtx()
            .args([cmd, "comment", "-R", "o/r", "7", "-b", "x", "-F", "-"])
            .assert()
            .failure()
            .stderr(contains("cannot be used with"));
        assert!(s.seen().is_empty());
    }
}

#[test]
fn body_required() {
    for cmd in ["issue", "pr"] {
        let s = server();
        s.gtx()
            .args([cmd, "comment", "-R", "o/r", "7"])
            .assert()
            .failure()
            .stderr(contains("--body"));
        assert!(s.seen().is_empty());
    }
}

#[test]
fn inline_body_still_works() {
    let s = server();
    s.gtx()
        .args(["issue", "comment", "-R", "o/r", "7", "-b", "inline"])
        .assert()
        .success();
    assert_eq!(posted_comment(&s), "inline");
}
