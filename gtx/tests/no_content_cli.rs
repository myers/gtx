//! Gitea answers several body-less operations with 204 No Content even where
//! its swagger documents only 201 (#5). Those must be reported as success.

mod common;

use predicates::prelude::*;

use common::FakeGitea;

fn no_content() -> FakeGitea {
    FakeGitea::start_with(|_, _| (204, String::new()))
}

#[test]
fn secret_set_update_204_is_success() {
    let server = no_content();
    server
        .gtx()
        .args(["secret", "set", "-R", "o/r", "S", "--value", "v"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Secret 'S' set"));
    assert_eq!(server.seen(), ["PUT /api/v1/repos/o/r/actions/secrets/S"]);
}

#[test]
fn variable_set_update_204_is_success() {
    let server = no_content();
    server
        .gtx()
        .args(["variable", "set", "-R", "o/r", "V", "x"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Variable 'V' updated"));
    assert_eq!(server.seen(), ["PUT /api/v1/repos/o/r/actions/variables/V"]);
}

#[test]
fn variable_delete_204_is_success() {
    let server = no_content();
    server
        .gtx()
        .args(["variable", "delete", "-R", "o/r", "V"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Variable 'V' deleted"));
}

#[test]
fn workflow_enable_disable_204_is_success() {
    let server = no_content();
    for action in ["enable", "disable"] {
        server
            .gtx()
            .args(["workflow", action, "-R", "o/r", "ci.yml"])
            .assert()
            .success();
    }
}

#[test]
fn variable_set_creates_on_404() {
    let server = FakeGitea::start_with(|method, _| match method {
        "PUT" => (404, r#"{"message":"not found"}"#.into()),
        _ => (201, String::new()),
    });
    server
        .gtx()
        .args(["variable", "set", "-R", "o/r", "V", "x"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Variable 'V' created"));
    assert_eq!(
        server.seen(),
        [
            "PUT /api/v1/repos/o/r/actions/variables/V",
            "POST /api/v1/repos/o/r/actions/variables/V",
        ]
    );
}

#[test]
fn workflow_run_requests_run_details() {
    let server = FakeGitea::start_with(|_, _| {
        (
            200,
            r#"{"workflow_run_id":42,"html_url":"http://x/runs/42"}"#.into(),
        )
    });
    server
        .gtx()
        .args(["workflow", "run", "-R", "o/r", "ci.yml", "--ref", "main"])
        .assert()
        .success()
        .stderr(predicate::str::contains("run #42"));
    let seen = server.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert!(seen[0].starts_with("POST /api/v1/repos/o/r/actions/workflows/ci.yml/dispatches?"));
    assert!(seen[0].contains("return_run_details=true"), "{seen:?}");
}

#[test]
fn workflow_run_204_is_success() {
    let server = no_content();
    server
        .gtx()
        .args(["workflow", "run", "-R", "o/r", "ci.yml", "--ref", "main"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Triggered workflow 'ci.yml'"));
    assert_eq!(server.seen().len(), 1, "must not retry the dispatch");
}

#[test]
fn real_errors_still_fail() {
    let server = FakeGitea::start_with(|_, _| (403, r#"{"message":"forbidden"}"#.into()));
    server
        .gtx()
        .args(["secret", "set", "-R", "o/r", "S", "--value", "v"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("HTTP 403"));
}
