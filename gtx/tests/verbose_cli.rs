//! HTTP transcripts, turned on like gh's: `GTX_DEBUG` (gh's `GH_DEBUG`) and
//! `gtx api --verbose`. There is no global `-v` (#33). Credentials are
//! redacted the way gh's `GH_DEBUG=api` does: nothing of the secret shows,
//! however short it is (#18).

mod common;

use common::{FakeGitea, Reply};
use predicates::prelude::*;

const REDACTED: &str = "████████████████████";

#[test]
fn verbose_redacts_short_token_and_cookies() {
    let server = FakeGitea::start_raw(|_, _| {
        let mut reply = Reply::json(200, r#"{"login":"me"}"#);
        reply.headers.push((
            "Set-Cookie".into(),
            "i_like_gitea=cookiesecret; Path=/".into(),
        ));
        reply
    });
    let out = server
        .gtx()
        .env("GITEA_TOKEN", "123456789")
        .env("GTX_DEBUG", "api")
        .args(["api", "/user"])
        .assert()
        .success()
        .get_output()
        .clone();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains(&format!("> authorization: token {REDACTED}\n")),
        "{stderr}"
    );
    assert!(
        stderr.contains(&format!("< set-cookie: i_like_gitea={REDACTED}; Path=/\n")),
        "{stderr}"
    );
    assert!(!stderr.contains("6789"), "token tail leaked: {stderr}");
    assert!(!stderr.contains("cookiesecret"), "cookie leaked: {stderr}");
}

#[test]
fn verbose_show_secrets_prints_token() {
    let server = FakeGitea::start(|_| r#"{"login":"me"}"#.into());
    server
        .gtx()
        .env("GITEA_TOKEN", "123456789")
        .args(["api", "--verbose", "--show-secrets", "/user"])
        .assert()
        .success()
        .stderr(predicates::str::contains(
            "> authorization: token 123456789\n",
        ));
}

/// gh has no global `-v`; neither does gtx.
#[test]
fn no_global_verbose_flag() {
    let server = FakeGitea::start(|_| r#"{"login":"me"}"#.into());
    server.gtx().args(["-v", "api", "/user"]).assert().failure();
}

/// `GTX_DEBUG=api` logs headers and bodies, like `GH_DEBUG=api`.
#[test]
fn debug_api_env_logs_bodies() {
    let server = FakeGitea::start(|_| r#"{"login":"me"}"#.into());
    server
        .gtx()
        .env("GTX_DEBUG", "api")
        .args(["api", "/user"])
        .assert()
        .success()
        .stderr(predicates::str::contains("> GET /api/v1/user HTTP/1.1\n"))
        .stderr(predicates::str::contains(r#"{"login":"me"}"#));
}

/// Any other truthy `GTX_DEBUG` logs headers only, like `GH_DEBUG=1`.
#[test]
fn debug_truthy_env_logs_headers_only() {
    let server = FakeGitea::start(|_| r#"{"login":"me"}"#.into());
    server
        .gtx()
        .env("GTX_DEBUG", "1")
        .args(["api", "/user"])
        .assert()
        .success()
        .stderr(predicates::str::contains("> GET /api/v1/user HTTP/1.1\n"))
        .stderr(predicates::str::contains("login").not());
}

/// `GTX_DEBUG=0`/`false` (and unset) log nothing.
#[test]
fn debug_falsy_env_logs_nothing() {
    for value in ["0", "false", "no", ""] {
        let server = FakeGitea::start(|_| r#"{"login":"me"}"#.into());
        server
            .gtx()
            .env("GTX_DEBUG", value)
            .args(["api", "/user"])
            .assert()
            .success()
            .stderr(predicates::str::is_empty());
    }
}

/// `gtx api --verbose` logs the full request and response, like gh's.
#[test]
fn api_verbose_logs_full_transcript() {
    let server = FakeGitea::start(|_| r#"{"login":"me"}"#.into());
    server
        .gtx()
        .env("GITEA_TOKEN", "123456789")
        .args(["api", "--verbose", "/user"])
        .assert()
        .success()
        .stderr(predicates::str::contains("> GET /api/v1/user HTTP/1.1\n"))
        .stderr(predicates::str::contains(format!(
            "> authorization: token {REDACTED}\n"
        )))
        .stderr(predicates::str::contains(r#"{"login":"me"}"#));
}
