//! `pr review` with gh's event flags and `--comments-file` line comments, and
//! `pr view --comments` showing reviews and their line comments.

mod common;

use common::FakeGitea;
use predicates::str::contains;

const PR: &str = r#"{"number":5,"title":"T","state":"open","head":{"ref":"feat","label":"feat"},"base":{"ref":"main","label":"main"},"html_url":"http://h/o/r/pulls/5","comments":2}"#;

fn server() -> FakeGitea {
    FakeGitea::start_with(|method, target| {
        let path = target.split('?').next().unwrap_or(target);
        let body = match (method, path) {
            ("POST", "/api/v1/repos/o/r/pulls/5/reviews") => r#"{"id":1}"#,
            ("GET", "/api/v1/repos/o/r/pulls") => {
                if target.contains("page=1") {
                    r#"[{"number":4,"head":{"ref":"other"}},{"number":5,"head":{"ref":"feat"}}]"#
                } else {
                    "[]"
                }
            }
            ("GET", "/api/v1/repos/o/r/pulls/5") => PR,
            ("GET", "/api/v1/repos/o/r/issues/5/comments") => {
                r#"[{"id":1,"user":{"login":"carol"},"body":"first comment","created_at":"2026-01-01T10:00:00Z"},
                    {"id":2,"user":{"login":"dave"},"body":"last comment","created_at":"2026-01-01T10:10:00Z"}]"#
            }
            ("GET", "/api/v1/repos/o/r/pulls/5/reviews") => {
                if target.contains("page=1") {
                    r#"[{"id":8,"user":{"id":3,"login":"erin"},"state":"REQUEST_REVIEW","body":"","comments_count":0,"submitted_at":"2026-01-01T09:00:00Z"},
                        {"id":9,"user":{"id":2,"login":"bob"},"state":"COMMENT","body":"review body","comments_count":2,"submitted_at":"2026-01-01T10:05:00Z"},
                        {"id":10,"user":{"id":4,"login":"alice"},"state":"APPROVED","body":"lgtm","comments_count":0,"submitted_at":"2026-01-01T10:20:00Z"}]"#
                } else {
                    "[]"
                }
            }
            ("GET", "/api/v1/repos/o/r/pulls/5/reviews/9/comments") => {
                r#"[{"id":31,"path":"src/a.rs","body":"new side note","position":12,"original_position":0},
                    {"id":32,"path":"src/b.rs","body":"old side note","position":0,"original_position":7}]"#
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

fn json(body: &str) -> serde_json::Value {
    serde_json::from_str(body).unwrap_or_else(|e| panic!("{e}: {body}"))
}

fn tmp_file(name: &str, content: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("gtx-pr-review-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, content).unwrap();
    p
}

const TWO: &str = r#"[{"path":"src/a.rs","body":"new side","new_position":12},
                      {"path":"src/b.rs","body":"old side","old_position":7}]"#;

fn review_post(s: &FakeGitea) -> serde_json::Value {
    let w = writes(s);
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(w[0].0, "POST /api/v1/repos/o/r/pulls/5/reviews");
    json(&w[0].1)
}

#[test]
fn comment_review_with_comments_file() {
    let s = server();
    let f = tmp_file("two.json", TWO);
    s.gtx()
        .args(["pr", "review", "-R", "o/r", "5", "-c", "-b", "B"])
        .arg("--comments-file")
        .arg(&f)
        .assert()
        .success();
    let v = review_post(&s);
    assert_eq!(v["event"], "COMMENT");
    assert_eq!(v["body"], "B");
    assert_eq!(
        v["comments"],
        serde_json::json!([
            {"path":"src/a.rs","body":"new side","new_position":12},
            {"path":"src/b.rs","body":"old side","old_position":7},
        ])
    );
}

#[test]
fn comments_file_from_stdin() {
    let s = server();
    s.gtx()
        .args([
            "pr",
            "review",
            "-R",
            "o/r",
            "5",
            "-r",
            "-b",
            "fix",
            "--comments-file",
            "-",
        ])
        .write_stdin(TWO)
        .assert()
        .success();
    let v = review_post(&s);
    assert_eq!(v["event"], "REQUEST_CHANGES");
    assert_eq!(v["comments"].as_array().unwrap().len(), 2);
}

#[test]
fn comment_review_body_may_be_blank_with_line_comments() {
    let s = server();
    s.gtx()
        .args([
            "pr",
            "review",
            "-R",
            "o/r",
            "5",
            "-c",
            "--comments-file",
            "-",
        ])
        .write_stdin(TWO)
        .assert()
        .success();
    assert_eq!(review_post(&s)["event"], "COMMENT");
}

#[test]
fn approve_with_body_file() {
    let s = server();
    let f = tmp_file("body.md", "looks good\n");
    s.gtx()
        .args(["pr", "review", "-R", "o/r", "5", "--approve", "-F"])
        .arg(&f)
        .assert()
        .success();
    let v = review_post(&s);
    assert_eq!(v["event"], "APPROVED");
    assert_eq!(v["body"], "looks good\n");
    assert!(v["comments"].as_array().is_none_or(|a| a.is_empty()), "{v}");
}

#[test]
fn body_file_from_stdin() {
    let s = server();
    s.gtx()
        .args(["pr", "review", "-R", "o/r", "5", "-c", "-F", "-"])
        .write_stdin("from stdin")
        .assert()
        .success();
    assert_eq!(review_post(&s)["body"], "from stdin");
}

#[test]
fn event_flag_required() {
    let s = server();
    s.gtx()
        .args(["pr", "review", "-R", "o/r", "5", "-b", "x"])
        .assert()
        .failure();
    s.gtx()
        .args(["pr", "review", "-R", "o/r", "5", "-a", "-c", "-b", "x"])
        .assert()
        .failure();
    assert!(writes(&s).is_empty());
}

#[test]
fn blank_body_rejected_for_request_changes() {
    let s = server();
    s.gtx()
        .args(["pr", "review", "-R", "o/r", "5", "-r"])
        .assert()
        .failure()
        .stderr(contains("body cannot be blank for request-changes review"));
    s.gtx()
        .args(["pr", "review", "-R", "o/r", "5", "-c"])
        .assert()
        .failure()
        .stderr(contains("body cannot be blank for comment review"));
    assert!(writes(&s).is_empty());
}

fn bad_file(content: &str, want: &str) {
    let s = server();
    s.gtx()
        .args(["pr", "review", "-R", "o/r", "5", "-c", "-b", "x"])
        .args(["--comments-file", "-"])
        .write_stdin(content)
        .assert()
        .failure()
        .stderr(contains(want));
    assert!(writes(&s).is_empty());
}

#[test]
fn comments_file_validation() {
    bad_file(r#"{"path":"a"}"#, "expected a JSON array");
    bad_file("not json", "comments file");
    bad_file(
        r#"[{"path":"a","body":"b","new_position":1},{"body":"b","new_position":2}]"#,
        r#"comments[1]: missing "path""#,
    );
    bad_file(
        r#"[{"path":"a","new_position":1}]"#,
        r#"comments[0]: missing "body""#,
    );
    bad_file(
        r#"[{"path":"a","body":"b"}]"#,
        r#"comments[0]: needs "new_position" or "old_position""#,
    );
    bad_file(
        r#"[{"path":"a","body":"b","new_position":1,"old_position":2}]"#,
        r#"comments[0]: give only one of "new_position" and "old_position""#,
    );
    bad_file(
        r#"[{"path":"a","body":"b","new_position":0}]"#,
        r#"comments[0]: "new_position" must be a line number"#,
    );
    bad_file(
        r#"[{"path":"a","body":"b","line":3}]"#,
        r#"comments[0]: unknown key "line""#,
    );
    bad_file(r#"["x"]"#, "comments[0]: expected an object");
}

#[test]
fn review_defaults_to_current_branch_pr() {
    let s = server();
    let dir = std::env::temp_dir().join(format!("gtx-pr-review-git-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let git = |args: &[&str]| {
        assert!(
            std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap()
                .status
                .success()
        )
    };
    git(&["init", "-q", "-b", "feat"]);
    s.gtx()
        .current_dir(&dir)
        .args(["pr", "review", "-R", "o/r", "-a"])
        .assert()
        .success();
    assert_eq!(review_post(&s)["event"], "APPROVED");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn view_comments_interleaves_reviews_and_line_comments() {
    let s = server();
    let out = s
        .gtx()
        .args(["pr", "view", "-R", "o/r", "5", "--comments"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let out = String::from_utf8(out).unwrap();
    let pos = |needle: &str| {
        out.find(needle)
            .unwrap_or_else(|| panic!("{needle:?} not in:\n{out}"))
    };
    let order = [
        pos("first comment"),
        pos("bob commented"),
        pos("review body"),
        pos("src/a.rs:12"),
        pos("new side note"),
        pos("src/b.rs:7 (old)"),
        pos("old side note"),
        pos("last comment"),
        pos("alice approved"),
        pos("lgtm"),
    ];
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{out}");
    assert!(!out.contains("erin"), "review request shown:\n{out}");
    assert!(
        s.seen().iter().all(|r| !r.contains("/reviews/10/comments")),
        "fetched comments of a review without any"
    );
}
