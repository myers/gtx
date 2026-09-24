//! `-v` transcripts redact credentials the way gh's `GH_DEBUG=api` does:
//! nothing of the secret shows, however short it is (#18).

mod common;

use common::{FakeGitea, Reply};

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
        .args(["-v", "api", "/user"])
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
        .args(["-v", "--show-secrets", "api", "/user"])
        .assert()
        .success()
        .stderr(predicates::str::contains(
            "> authorization: token 123456789\n",
        ));
}
