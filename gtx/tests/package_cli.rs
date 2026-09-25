mod common;

use common::FakeGitea;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

const PKG: &str = r#"{"id":7,"type":"generic","name":"gtx-walk","version":"1.0","html_url":"http://x/chaos-inc/-/packages/generic/gtx-walk/1.0","created_at":"2026-09-01T10:00:00Z","owner":{"login":"chaos-inc"},"creator":{"login":"claude"},"repository":{"name":"r","full_name":"chaos-inc/r"}}"#;
const FILES: &str = r#"[{"id":1,"name":"walk.txt","size":12,"md5":"m","sha1":"s1","sha256":"s256","sha512":"s512"}]"#;

fn server() -> FakeGitea {
    FakeGitea::start_with(|method, target| {
        let path = target.split('?').next().unwrap();
        match (method, path) {
            ("GET", "/api/v1/packages/chaos-inc") | ("GET", "/api/v1/packages/o") => {
                (200, format!("[{PKG}]"))
            }
            ("GET", "/api/v1/packages/chaos-inc/generic/gtx-walk/1.0")
            | ("GET", "/api/v1/packages/chaos-inc/generic/gtx-walk/-/latest") => {
                (200, PKG.to_string())
            }
            ("GET", "/api/v1/packages/chaos-inc/generic/gtx-walk/1.0/files") => {
                (200, FILES.to_string())
            }
            ("DELETE", "/api/v1/packages/chaos-inc/generic/gtx-walk/1.0") => (204, String::new()),
            _ => (404, r#"{"message":"not found"}"#.to_string()),
        }
    })
}

#[test]
fn package_list_by_owner_with_filters() {
    let s = server();
    s.gtx()
        .args([
            "package",
            "list",
            "-o",
            "chaos-inc",
            "--type",
            "generic",
            "-S",
            "walk",
            "-L",
            "5",
        ])
        .assert()
        .success()
        .stdout(
            contains("gtx-walk")
                .and(contains("generic"))
                .and(contains("1.0")),
        );
    let seen = s.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    let req = &seen[0];
    assert!(req.starts_with("GET /api/v1/packages/chaos-inc?"), "{req}");
    for q in ["type=generic", "q=walk", "limit=5", "page=1"] {
        assert!(req.contains(q), "{req} lacks {q}");
    }
}

#[test]
fn package_list_defaults_owner_to_current_repo() {
    let s = server();
    s.gtx()
        .env("GITEA_REPO", "o/r")
        .args(["package", "list"])
        .assert()
        .success();
    assert!(s.seen()[0].starts_with("GET /api/v1/packages/o?"));
}

#[test]
fn package_list_json() {
    let s = server();
    s.gtx()
        .args([
            "package",
            "list",
            "-o",
            "chaos-inc",
            "--json",
            "name,type,version,owner,repository,createdAt,url",
        ])
        .assert()
        .success()
        .stdout(contains(r#""name":"gtx-walk""#))
        .stdout(contains(r#""owner":{"login":"chaos-inc"}"#))
        .stdout(contains(
            r#""repository":{"name":"r","nameWithOwner":"chaos-inc/r"}"#,
        ))
        .stdout(contains(r#""createdAt":"2026-09-01T10:00:00Z""#));
}

#[test]
fn package_list_rejects_unknown_type() {
    let s = server();
    s.gtx()
        .args(["package", "list", "-o", "chaos-inc", "--type", "bogus"])
        .assert()
        .failure()
        .stderr(contains("bogus"));
    assert!(s.seen().is_empty());
}

#[test]
fn package_view_shows_files() {
    let s = server();
    s.gtx()
        .args([
            "package",
            "view",
            "-o",
            "chaos-inc",
            "generic/gtx-walk",
            "1.0",
        ])
        .assert()
        .success()
        .stdout(
            contains("gtx-walk")
                .and(contains("1.0"))
                .and(contains("walk.txt"))
                .and(contains("s256")),
        );
    assert_eq!(
        s.seen(),
        [
            "GET /api/v1/packages/chaos-inc/generic/gtx-walk/1.0",
            "GET /api/v1/packages/chaos-inc/generic/gtx-walk/1.0/files",
        ]
    );
}

#[test]
fn package_view_without_version_uses_latest() {
    let s = server();
    s.gtx()
        .args(["package", "view", "-o", "chaos-inc", "generic/gtx-walk"])
        .assert()
        .success()
        .stdout(contains("walk.txt"));
    assert_eq!(
        s.seen(),
        [
            "GET /api/v1/packages/chaos-inc/generic/gtx-walk/-/latest",
            "GET /api/v1/packages/chaos-inc/generic/gtx-walk/1.0/files",
        ]
    );
}

#[test]
fn package_view_json_files() {
    let s = server();
    s.gtx()
        .args([
            "package",
            "view",
            "-o",
            "chaos-inc",
            "generic/gtx-walk",
            "1.0",
            "--json",
            "files",
            "--jq",
            ".files[0].sha256",
        ])
        .assert()
        .success()
        .stdout("s256\n");
}

#[test]
fn package_view_needs_type_slash_name() {
    let s = server();
    s.gtx()
        .args(["package", "view", "-o", "chaos-inc", "gtx-walk"])
        .assert()
        .failure()
        .stderr(contains("TYPE/NAME"));
    assert!(s.seen().is_empty());
}

#[test]
fn package_delete_with_yes() {
    let s = server();
    s.gtx()
        .args([
            "package",
            "delete",
            "-o",
            "chaos-inc",
            "generic/gtx-walk",
            "1.0",
            "--yes",
        ])
        .assert()
        .success();
    assert_eq!(
        s.seen(),
        ["DELETE /api/v1/packages/chaos-inc/generic/gtx-walk/1.0"]
    );
}

#[test]
fn package_delete_without_yes_non_interactive_fails() {
    let s = server();
    s.gtx()
        .args([
            "package",
            "delete",
            "-o",
            "chaos-inc",
            "generic/gtx-walk",
            "1.0",
        ])
        .assert()
        .failure()
        .stderr(contains("--yes required when not running interactively"));
    assert!(s.seen().is_empty());
}

#[test]
fn repo_delete_uses_yes_like_gh() {
    let s = FakeGitea::start_with(|_, _| (204, String::new()));
    s.gtx()
        .args(["repo", "delete", "-R", "o/r"])
        .assert()
        .failure()
        .stderr(contains("--yes required when not running interactively"));
    assert!(s.seen().is_empty());
    s.gtx()
        .args(["repo", "delete", "-R", "o/r", "--yes"])
        .assert()
        .success();
    assert_eq!(s.seen(), ["DELETE /api/v1/repos/o/r"]);
}
