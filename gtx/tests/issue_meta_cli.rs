//! gh-shaped label / milestone / assignee flags on `issue create/edit/list`
//! and `pr edit`, checked against the requests they send.

mod common;

use common::FakeGitea;
use predicates::str::contains;

/// A fake repo `o/r` with labels bug(7) and docs(8), milestone v1(3), issue
/// #5 assigned to alice and bob, and current user `me`.
fn server() -> FakeGitea {
    FakeGitea::start_with(|method, target| {
        let path = target.split('?').next().unwrap_or(target);
        let first_page =
            !target.contains("page=") || target.contains("page=1&") || target.ends_with("page=1");
        let body = match (method, path) {
            ("GET", "/api/v1/repos/o/r/labels") if first_page => {
                r#"[{"id":7,"name":"bug"},{"id":8,"name":"docs"}]"#
            }
            ("GET", "/api/v1/repos/o/r/milestones") if first_page => r#"[{"id":3,"title":"v1"}]"#,
            ("GET", "/api/v1/user") => r#"{"login":"me"}"#,
            ("GET", "/api/v1/repos/o/r/issues") => "[]",
            ("GET", _) if path.starts_with("/api/v1/repos/o/r/issues/") => {
                r#"{"number":5,"assignees":[{"login":"alice"},{"login":"bob"}],"html_url":"http://h/o/r/issues/5"}"#
            }
            ("PATCH", _) | ("POST", "/api/v1/repos/o/r/issues") => {
                r#"{"number":5,"html_url":"http://h/o/r/issues/5"}"#
            }
            ("POST", _) => "[]",
            ("GET", _) => "[]",
            _ => "{}",
        };
        (200, body.to_string())
    })
}

/// (request, body) pairs the server saw, minus plain GETs.
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

#[test]
fn issue_create_milestone_by_title() {
    let s = server();
    s.gtx()
        .args([
            "issue", "create", "-R", "o/r", "-t", "T", "-b", "B", "-m", "v1",
        ])
        .assert()
        .success();
    let seen = s.seen();
    assert!(
        seen.iter()
            .any(|r| r.starts_with("GET /api/v1/repos/o/r/milestones?") && r.contains("state=all")),
        "{seen:?}"
    );
    let w = writes(&s);
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(w[0].0, "POST /api/v1/repos/o/r/issues");
    assert_eq!(json(&w[0].1)["milestone"], 3);
}

#[test]
fn issue_create_unknown_milestone_errors() {
    let s = server();
    s.gtx()
        .args(["issue", "create", "-R", "o/r", "-t", "T", "-m", "nope"])
        .assert()
        .failure()
        .stderr(contains("'nope' not found"));
    assert!(writes(&s).is_empty());
}

#[test]
fn issue_create_labels_and_assignees_comma_separated() {
    let s = server();
    s.gtx()
        .args([
            "issue", "create", "-R", "o/r", "-t", "T", "-l", "bug,docs", "-a", "@me,x",
        ])
        .assert()
        .success();
    let w = writes(&s);
    let b = json(&w[0].1);
    assert_eq!(b["labels"], serde_json::json!([7, 8]));
    assert_eq!(b["assignees"], serde_json::json!(["me", "x"]));
}

#[test]
fn issue_edit_add_label_posts_ids() {
    let s = server();
    s.gtx()
        .args(["issue", "edit", "-R", "o/r", "5", "--add-label", "bug,docs"])
        .assert()
        .success()
        .stdout(contains("http://h/o/r/issues/5"));
    let w = writes(&s);
    assert_eq!(w[0].0, "POST /api/v1/repos/o/r/issues/5/labels", "{w:?}");
    assert_eq!(json(&w[0].1)["labels"], serde_json::json!([7, 8]));
}

#[test]
fn issue_edit_remove_label_deletes_by_id() {
    let s = server();
    s.gtx()
        .args(["issue", "edit", "-R", "o/r", "5", "--remove-label", "bug"])
        .assert()
        .success();
    let w = writes(&s);
    assert!(
        w.iter()
            .any(|(r, _)| r == "DELETE /api/v1/repos/o/r/issues/5/labels/7"),
        "{w:?}"
    );
}

#[test]
fn issue_edit_unknown_label_errors() {
    let s = server();
    s.gtx()
        .args(["issue", "edit", "-R", "o/r", "5", "--add-label", "nope"])
        .assert()
        .failure()
        .stderr(contains("'nope' not found"));
    assert!(writes(&s).is_empty());
}

#[test]
fn issue_edit_milestone_and_remove_milestone() {
    let s = server();
    s.gtx()
        .args(["issue", "edit", "-R", "o/r", "5", "-m", "v1"])
        .assert()
        .success();
    let w = writes(&s);
    assert_eq!(w.last().unwrap().0, "PATCH /api/v1/repos/o/r/issues/5");
    assert_eq!(json(&w.last().unwrap().1)["milestone"], 3);

    let s = server();
    s.gtx()
        .args(["issue", "edit", "-R", "o/r", "5", "--remove-milestone"])
        .assert()
        .success();
    let w = writes(&s);
    assert_eq!(json(&w.last().unwrap().1)["milestone"], 0);

    server()
        .gtx()
        .args([
            "issue",
            "edit",
            "-R",
            "o/r",
            "5",
            "-m",
            "v1",
            "--remove-milestone",
        ])
        .assert()
        .failure();
}

#[test]
fn issue_edit_assignees_add_and_remove() {
    let s = server();
    s.gtx()
        .args([
            "issue",
            "edit",
            "-R",
            "o/r",
            "5",
            "--add-assignee",
            "carol",
            "--remove-assignee",
            "bob",
        ])
        .assert()
        .success();
    let w = writes(&s);
    let b = json(&w.last().unwrap().1);
    assert_eq!(b["assignees"], serde_json::json!(["alice", "carol"]));

    // Removing everyone clears the assignees.
    let s = server();
    s.gtx()
        .args([
            "issue",
            "edit",
            "-R",
            "o/r",
            "5",
            "--remove-assignee",
            "alice,bob",
        ])
        .assert()
        .success();
    let w = writes(&s);
    let b = json(&w.last().unwrap().1);
    assert_eq!(b["assignee"], "", "{b}");
}

#[test]
fn issue_edit_several_issues() {
    let s = server();
    s.gtx()
        .args(["issue", "edit", "-R", "o/r", "5", "6", "--add-label", "bug"])
        .assert()
        .success();
    let posts: Vec<_> = writes(&s)
        .into_iter()
        .filter(|(r, _)| r.starts_with("POST"))
        .map(|(r, _)| r)
        .collect();
    assert_eq!(
        posts,
        [
            "POST /api/v1/repos/o/r/issues/5/labels",
            "POST /api/v1/repos/o/r/issues/6/labels"
        ]
    );
}

#[test]
fn issue_edit_requires_a_field() {
    server()
        .gtx()
        .args(["issue", "edit", "-R", "o/r", "5"])
        .assert()
        .failure();
}

#[test]
fn issue_list_filters() {
    let s = server();
    s.gtx()
        .args([
            "issue",
            "list",
            "-R",
            "o/r",
            "-l",
            "bug",
            "--label",
            "docs",
            "-a",
            "alice",
            "-A",
            "bob",
            "--mention",
            "carol",
            "-S",
            "foo",
            "-m",
            "v1",
        ])
        .assert()
        .success();
    let seen = s.seen();
    let q = seen
        .iter()
        .find(|r| r.starts_with("GET /api/v1/repos/o/r/issues?"))
        .unwrap_or_else(|| panic!("{seen:?}"));
    for want in [
        "labels=bug%2Cdocs",
        "assigned_by=alice",
        "created_by=bob",
        "mentioned_by=carol",
        "q=foo",
        "milestones=3",
    ] {
        assert!(q.contains(want), "{want} missing: {q}");
    }
}

#[test]
fn issue_list_at_me_is_current_user() {
    let s = server();
    s.gtx()
        .args(["issue", "list", "-R", "o/r", "-a", "@me"])
        .assert()
        .success();
    let seen = s.seen();
    assert!(
        seen.iter()
            .any(|r| r.contains("assigned_by=me&") || r.ends_with("assigned_by=me")),
        "{seen:?}"
    );
}

#[test]
fn issue_list_unknown_label_matches_nothing() {
    let s = server();
    s.gtx()
        .args([
            "issue", "list", "-R", "o/r", "-l", "nope", "--json", "number",
        ])
        .assert()
        .success()
        .stdout("[]\n");
    assert!(
        !s.seen()
            .iter()
            .any(|r| r.starts_with("GET /api/v1/repos/o/r/issues?")),
        "{:?}",
        s.seen()
    );
}

#[test]
fn issue_list_unknown_milestone_errors() {
    server()
        .gtx()
        .args(["issue", "list", "-R", "o/r", "-m", "nope"])
        .assert()
        .failure()
        .stderr(contains("nope"));
}

#[test]
fn pr_edit_uses_gh_label_flags() {
    let s = server();
    s.gtx()
        .args([
            "pr",
            "edit",
            "-R",
            "o/r",
            "5",
            "--add-label",
            "bug",
            "-m",
            "v1",
        ])
        .assert()
        .success();
    let w = writes(&s);
    assert!(
        w.iter()
            .any(|(r, b)| r == "POST /api/v1/repos/o/r/issues/5/labels"
                && json(b)["labels"] == serde_json::json!([7])),
        "{w:?}"
    );
    assert!(
        w.iter()
            .any(|(r, b)| r.starts_with("PATCH") && json(b)["milestone"] == 3),
        "{w:?}"
    );

    server()
        .gtx()
        .args(["pr", "edit", "-R", "o/r", "5", "-l", "bug"])
        .assert()
        .failure();
}
