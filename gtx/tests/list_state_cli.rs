//! List commands send Gitea the `state` (and, for issues, `type`) the user asked for.

mod common;

use common::FakeGitea;

fn list_query(args: &[&str]) -> String {
    let server = FakeGitea::start(|_| "[]".into());
    server.gtx().args(args).assert().success();
    let seen = server.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    seen[0].clone()
}

#[test]
fn issue_list_sends_state_and_excludes_pulls() {
    for state in ["open", "closed", "all"] {
        let q = list_query(&["issue", "list", "-R", "o/r", "-s", state]);
        assert!(q.starts_with("GET /api/v1/repos/o/r/issues?"), "{q}");
        assert!(q.contains(&format!("state={state}")), "{q}");
        assert!(q.contains("type=issues"), "{q}");
    }
}

#[test]
fn issue_list_defaults_to_open_issues() {
    let q = list_query(&["issue", "list", "-R", "o/r"]);
    assert!(q.contains("state=open"), "{q}");
    assert!(q.contains("type=issues"), "{q}");
}

#[test]
fn issue_status_excludes_pulls() {
    let q = list_query(&["issue", "status", "-R", "o/r"]);
    assert!(q.contains("state=open"), "{q}");
    assert!(q.contains("type=issues"), "{q}");
}

#[test]
fn pr_list_sends_state() {
    for state in ["open", "closed", "all"] {
        let q = list_query(&["pr", "list", "-R", "o/r", "-s", state]);
        assert!(q.starts_with("GET /api/v1/repos/o/r/pulls?"), "{q}");
        assert!(q.contains(&format!("state={state}")), "{q}");
    }
}

#[test]
fn milestone_list_sends_state() {
    for state in ["open", "closed", "all"] {
        let q = list_query(&["milestone", "list", "-R", "o/r", "--state", state]);
        assert!(q.contains(&format!("state={state}")), "{q}");
    }
}
