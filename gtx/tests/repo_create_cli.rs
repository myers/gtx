mod common;

use common::FakeGitea;
use serde_json::Value;

/// Run `gtx repo create NAME <extra>` against a fake Gitea and return the
/// JSON body it POSTed to /user/repos.
fn create_body(extra: &[&str]) -> Value {
    let server = FakeGitea::start_with(|method, target| match (method, target) {
        ("POST", "/api/v1/user/repos") => (
            201,
            r#"{"id":1,"full_name":"me/r","html_url":"http://x/me/r"}"#.into(),
        ),
        _ => (404, "{}".into()),
    });
    server
        .gtx()
        .args(["repo", "create", "r"])
        .args(extra)
        .assert()
        .success();
    assert_eq!(server.seen(), ["POST /api/v1/user/repos"]);
    serde_json::from_str(&server.bodies()[0]).unwrap()
}

#[test]
fn creates_an_empty_repo_by_default() {
    let body = create_body(&["--private"]);
    assert_eq!(body["name"], "r");
    assert_eq!(body["private"], true);
    assert!(body.get("auto_init").is_none(), "{body}");
    assert!(body.get("readme").is_none(), "{body}");
}

#[test]
fn add_readme_initializes_with_the_default_readme() {
    let body = create_body(&["--add-readme"]);
    assert_eq!(body["auto_init"], true);
    assert_eq!(body["readme"], "Default");
}

#[test]
fn gitignore_initializes_with_the_template() {
    let body = create_body(&["-g", "Rust"]);
    assert_eq!(body["auto_init"], true);
    assert_eq!(body["gitignores"], "Rust");
}

#[test]
fn license_initializes_with_the_template() {
    let body = create_body(&["--license", "MIT"]);
    assert_eq!(body["auto_init"], true);
    assert_eq!(body["license"], "MIT");
}
