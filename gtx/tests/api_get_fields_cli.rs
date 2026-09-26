mod common;

use common::FakeGitea;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

fn empty_list_server() -> FakeGitea {
    FakeGitea::start_with(|_, _| (200, "[]".into()))
}

/// As in gh: a GET's fields go in the query string, since Gitea never
/// reads a GET body.
#[test]
fn get_fields_become_query_params() {
    let server = empty_list_server();
    server
        .gtx()
        .args(["api", "-X", "get", "repos/o/r/issues"])
        .args(["-f", "state=closed", "-F", "limit=5", "-F", "q=a b"])
        .assert()
        .success();
    assert_eq!(
        server.seen(),
        ["GET /api/v1/repos/o/r/issues?limit=5&q=a+b&state=closed"]
    );
    assert_eq!(server.bodies(), [""]);
    assert_eq!(server.content_types(), [None]);
}

#[test]
fn paginated_get_fields_precede_page() {
    let server = empty_list_server();
    server
        .gtx()
        .args(["api", "--paginate", "-X", "GET", "repos/o/r/issues"])
        .args(["-f", "state=closed"])
        .assert()
        .success();
    assert_eq!(
        server.seen(),
        ["GET /api/v1/repos/o/r/issues?state=closed&page=1&limit=50"]
    );
}

#[test]
fn paginated_limit_field_replaces_default_limit() {
    let server = empty_list_server();
    server
        .gtx()
        .args(["api", "--paginate", "-X", "GET", "repos/o/r/issues"])
        .args(["-F", "limit=10"])
        .assert()
        .success();
    assert_eq!(
        server.seen(),
        ["GET /api/v1/repos/o/r/issues?limit=10&page=1"]
    );
}

#[test]
fn curl_renders_get_fields_in_url() {
    let server = empty_list_server();
    server
        .gtx()
        .args(["api", "-X", "GET", "repos/o/r/issues", "-f", "state=closed"])
        .arg("--curl")
        .assert()
        .success()
        .stdout(contains("/api/v1/repos/o/r/issues?state=closed'"))
        .stdout(contains("--data-raw").not())
        .stdout(contains("Content-Type").not());
    assert!(server.seen().is_empty());
}

#[test]
fn non_get_fields_stay_json_body() {
    let server = FakeGitea::start_with(|_, _| (201, "{}".into()));
    server
        .gtx()
        .args(["api", "repos/o/r/issues", "-f", "title=hi", "-F", "n=1"])
        .assert()
        .success();
    assert_eq!(server.seen(), ["POST /api/v1/repos/o/r/issues"]);
    assert_eq!(server.bodies(), [r#"{"n":1,"title":"hi"}"#]);
}
