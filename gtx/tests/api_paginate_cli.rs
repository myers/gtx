mod common;

use common::FakeGitea;
use predicates::str::contains;

/// Every page is an empty array, so a paginated GET stops after one request.
fn empty_list_server() -> FakeGitea {
    FakeGitea::start_with(|_, _| (200, "[]".into()))
}

#[test]
fn paginate_rejects_input() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("body.json");
    std::fs::write(&path, "{}").unwrap();
    let server = empty_list_server();
    server
        .gtx()
        .args(["api", "repos/o/r/contents", "--paginate", "--input"])
        .arg(&path)
        .assert()
        .failure()
        .stderr(contains(
            "the `--paginate` option is not supported with `--input`",
        ));
    assert!(server.seen().is_empty(), "sent: {:?}", server.seen());
}

#[test]
fn paginate_rejects_input_even_with_get() {
    let server = empty_list_server();
    server
        .gtx()
        .args(["api", "repos/o/r/contents", "--paginate", "-X", "GET"])
        .args(["--input", "-"])
        .write_stdin("{}")
        .assert()
        .failure()
        .stderr(contains(
            "the `--paginate` option is not supported with `--input`",
        ));
    assert!(server.seen().is_empty(), "sent: {:?}", server.seen());
}

#[test]
fn paginate_rejects_explicit_non_get_method() {
    let server = empty_list_server();
    server
        .gtx()
        .args(["api", "repos/o/r/issues", "--paginate", "-X", "post"])
        .args(["-f", "title=x"])
        .assert()
        .failure()
        .stderr(contains(
            "the `--paginate` option is not supported for non-GET requests",
        ));
    assert!(server.seen().is_empty(), "sent: {:?}", server.seen());
}

#[test]
fn paginate_accepts_lowercase_get() {
    let server = empty_list_server();
    server
        .gtx()
        .args(["api", "repos/o/r/issues", "--paginate", "-X", "get"])
        .assert()
        .success();
    assert_eq!(
        server.seen(),
        ["GET /api/v1/repos/o/r/issues?page=1&limit=50"]
    );
}

/// gh lets this through (it checks only `-X`, which defaults to GET) and
/// then sends a POST per page; gtx rejects the effective POST.
#[test]
fn paginate_rejects_fields_without_method() {
    let server = empty_list_server();
    server
        .gtx()
        .args(["api", "repos/o/r/issues", "--paginate", "-f", "title=x"])
        .assert()
        .failure()
        .stderr(contains(
            "the `--paginate` option is not supported for non-GET requests",
        ))
        .stderr(contains("-X GET"));
    assert!(server.seen().is_empty(), "sent: {:?}", server.seen());
}
