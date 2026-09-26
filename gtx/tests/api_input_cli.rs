mod common;

use common::FakeGitea;

/// The multi-file contents body from #35: a nested array `-f`/`-F` can't express.
/// Odd spacing on purpose, so a re-serialized body would not match.
const FILES_BODY: &str = r#"{"branch": "main",  "message":"add",
"files":[{"operation":"create","path":"a.txt","content":"YQ=="},{"operation":"create","path":"b.txt","content":"Yg=="}]}"#;

fn contents_server() -> FakeGitea {
    FakeGitea::start_with(|_, _| (201, r#"{"ok":true}"#.into()))
}

#[test]
fn input_file_is_sent_verbatim_as_post_body() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("files.json");
    std::fs::write(&path, FILES_BODY).unwrap();
    let server = contents_server();
    server
        .gtx()
        .args(["api", "repos/o/r/contents", "--input"])
        .arg(&path)
        .assert()
        .success();
    assert_eq!(server.seen(), ["POST /api/v1/repos/o/r/contents"]);
    assert_eq!(server.bodies(), [FILES_BODY]);
    assert_eq!(
        server.content_types(),
        [Some("application/json".to_string())]
    );
}

#[test]
fn input_dash_reads_stdin() {
    let server = contents_server();
    server
        .gtx()
        .args(["api", "repos/o/r/contents", "--input", "-"])
        .write_stdin(FILES_BODY)
        .assert()
        .success();
    assert_eq!(server.seen(), ["POST /api/v1/repos/o/r/contents"]);
    assert_eq!(server.bodies(), [FILES_BODY]);
}

#[test]
fn fields_become_query_params_alongside_input() {
    let server = contents_server();
    server
        .gtx()
        .args(["api", "repos/o/r/contents", "--input", "-"])
        .args(["-f", "ref=dev branch", "-F", "force=true"])
        .write_stdin(FILES_BODY)
        .assert()
        .success();
    assert_eq!(
        server.seen(),
        ["POST /api/v1/repos/o/r/contents?force=true&ref=dev+branch"]
    );
    assert_eq!(server.bodies(), [FILES_BODY]);
}

#[test]
fn explicit_method_and_content_type_are_kept() {
    let server = contents_server();
    server
        .gtx()
        .args(["api", "-X", "put", "repos/o/r/raw", "--input", "-"])
        .args(["-H", "Content-Type: text/plain"])
        .write_stdin("not json")
        .assert()
        .success();
    assert_eq!(server.seen(), ["PUT /api/v1/repos/o/r/raw"]);
    assert_eq!(server.bodies(), ["not json"]);
    assert_eq!(server.content_types(), [Some("text/plain".to_string())]);
}

#[test]
fn curl_renders_input_as_data_binary() {
    let server = contents_server();
    server
        .gtx()
        .args([
            "api",
            "repos/o/r/contents",
            "--input",
            "files.json",
            "--curl",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("curl -X POST"))
        .stdout(predicates::str::contains("--data-binary '@files.json'"));
    assert!(server.seen().is_empty());
}
