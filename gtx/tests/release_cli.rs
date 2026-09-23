mod common;

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use common::{FakeGitea, Reply};
use predicates::str::contains;

/// A fresh, empty scratch directory unique to this test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gtx-release-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A release JSON body whose assets live at `{base}/attachments/{name}`.
fn release_json(base: &str, tag: &str, assets: &[&str]) -> String {
    let assets: Vec<String> = assets
        .iter()
        .enumerate()
        .map(|(i, name)| {
            format!(
                r#"{{"id":{i},"name":"{name}","browser_download_url":"{base}/attachments/{name}"}}"#
            )
        })
        .collect();
    format!(
        r#"{{"id":4,"tag_name":"{tag}","assets":[{}],"zipball_url":"{base}/o/r/archive/{tag}.zip","tarball_url":"{base}/o/r/archive/{tag}.tar.gz"}}"#,
        assets.join(",")
    )
}

/// A fake Gitea with release `v1` (also the latest) carrying `assets`, each
/// served with body `"<name> bytes\n"`.
fn release_server(assets: &'static [&'static str]) -> FakeGitea {
    let base: Arc<OnceLock<String>> = Arc::new(OnceLock::new());
    let server = FakeGitea::start_raw({
        let base = Arc::clone(&base);
        move |_, target| {
            let base = base.get().unwrap();
            match target {
                "/api/v1/repos/o/r/releases/tags/v1" | "/api/v1/repos/o/r/releases/latest" => {
                    Reply::json(200, release_json(base, "v1", assets))
                }
                "/o/r/archive/v1.zip" => Reply {
                    status: 200,
                    headers: vec![(
                        "Content-Disposition".into(),
                        r#"attachment; filename="r-v1.zip""#.into(),
                    )],
                    body: b"zip bytes".to_vec(),
                },
                "/o/r/archive/v1.tar.gz" => Reply::bytes("application/gzip", b"tgz bytes".to_vec()),
                t => match t.strip_prefix("/attachments/") {
                    Some(name) if assets.contains(&name) => {
                        Reply::bytes("application/octet-stream", format!("{name} bytes\n").into())
                    }
                    _ => Reply::json(404, "{}"),
                },
            }
        }
    });
    base.set(server.url.clone()).unwrap();
    server
}

fn read(path: PathBuf) -> String {
    std::fs::read_to_string(path).unwrap()
}

/// Gitea's `browser_download_url` is a web route (`{root}/attachments/{uuid}`),
/// not an API path, and it 303s to the login page unless the token is sent.
#[test]
fn release_download_by_tag_fetches_every_asset_with_token() {
    let server = release_server(&["a.txt", "b.deb"]);
    let dir = scratch("tag");

    server
        .gtx()
        .current_dir(&dir)
        .args(["release", "download", "-R", "o/r", "v1"])
        .assert()
        .success();

    assert_eq!(read(dir.join("a.txt")), "a.txt bytes\n");
    assert_eq!(read(dir.join("b.deb")), "b.deb bytes\n");
    let seen = server.seen();
    assert_eq!(seen[0], "GET /api/v1/repos/o/r/releases/tags/v1", "{seen:?}");
    assert!(seen.contains(&"GET /attachments/a.txt".to_string()), "{seen:?}");
    assert!(server.auth().iter().all(|a| a.as_deref() == Some("token t")));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The token must not follow a download URL off to another host.
#[test]
fn release_download_withholds_token_from_foreign_host() {
    let other = FakeGitea::start_with(|_, _| (200, "elsewhere\n".into()));
    let other_url = other.url.clone();
    let server = FakeGitea::start_with(move |_, target| match target {
        "/api/v1/repos/o/r/releases/tags/v1" => (200, release_json(&other_url, "v1", &["dbg.txt"])),
        _ => (404, "{}".into()),
    });
    let dir = scratch("foreign");

    server
        .gtx()
        .current_dir(&dir)
        .args(["release", "download", "-R", "o/r", "v1"])
        .assert()
        .success();

    assert_eq!(other.seen(), ["GET /attachments/dbg.txt"]);
    assert_eq!(other.auth(), [None]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn release_download_without_tag_uses_latest_and_filters_by_patterns() {
    let server = release_server(&["a.txt", "b.deb", "c.rpm"]);
    let dir = scratch("latest");

    server
        .gtx()
        .current_dir(&dir)
        .args(["release", "download", "-R", "o/r", "-p", "*.deb", "--pattern", "*.rpm"])
        .assert()
        .success();

    assert_eq!(server.seen()[0], "GET /api/v1/repos/o/r/releases/latest");
    assert!(!dir.join("a.txt").exists());
    assert_eq!(read(dir.join("b.deb")), "b.deb bytes\n");
    assert_eq!(read(dir.join("c.rpm")), "c.rpm bytes\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn release_download_without_tag_needs_pattern_or_archive() {
    let server = release_server(&["a.txt"]);
    server
        .gtx()
        .args(["release", "download", "-R", "o/r"])
        .assert()
        .failure();
    assert!(server.seen().is_empty());
}

#[test]
fn release_download_errors_when_no_asset_matches() {
    let server = release_server(&["a.txt"]);
    let dir = scratch("nomatch");
    server
        .gtx()
        .current_dir(&dir)
        .args(["release", "download", "-R", "o/r", "v1", "-p", "*.deb"])
        .assert()
        .failure()
        .stderr(contains("no assets match the file pattern"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn release_download_refuses_to_overwrite_by_default() {
    let server = release_server(&["a.txt"]);
    let dir = scratch("exists");
    std::fs::write(dir.join("a.txt"), "old").unwrap();

    server
        .gtx()
        .current_dir(&dir)
        .args(["release", "download", "-R", "o/r", "v1"])
        .assert()
        .failure()
        .stderr(contains("already exists"));
    assert_eq!(read(dir.join("a.txt")), "old");

    server
        .gtx()
        .current_dir(&dir)
        .args(["release", "download", "-R", "o/r", "v1", "--skip-existing"])
        .assert()
        .success();
    assert_eq!(read(dir.join("a.txt")), "old");

    server
        .gtx()
        .current_dir(&dir)
        .args(["release", "download", "-R", "o/r", "v1", "--clobber"])
        .assert()
        .success();
    assert_eq!(read(dir.join("a.txt")), "a.txt bytes\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn release_download_dir_creates_and_writes_into_it() {
    let server = release_server(&["a.txt"]);
    let dir = scratch("dir");

    server
        .gtx()
        .current_dir(&dir)
        .args(["release", "download", "-R", "o/r", "v1", "-D", "out/nested"])
        .assert()
        .success();

    assert_eq!(read(dir.join("out/nested/a.txt")), "a.txt bytes\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn release_download_output_writes_single_asset_to_file_or_stdout() {
    let server = release_server(&["a.txt", "b.deb"]);
    let dir = scratch("output");

    server
        .gtx()
        .current_dir(&dir)
        .args(["release", "download", "-R", "o/r", "v1", "-p", "a.*", "-O", "renamed"])
        .assert()
        .success();
    assert_eq!(read(dir.join("renamed")), "a.txt bytes\n");

    server
        .gtx()
        .current_dir(&dir)
        .args(["release", "download", "-R", "o/r", "v1", "-p", "b.*", "-O", "-"])
        .assert()
        .success()
        .stdout("b.deb bytes\n");

    server
        .gtx()
        .current_dir(&dir)
        .args(["release", "download", "-R", "o/r", "v1", "-O", "both"])
        .assert()
        .failure()
        .stderr(contains("unable to write more than one asset with `--output`"));
    assert!(!dir.join("both").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn release_download_archive_fetches_source_archive() {
    let server = release_server(&["a.txt"]);
    let dir = scratch("archive");

    server
        .gtx()
        .current_dir(&dir)
        .args(["release", "download", "-R", "o/r", "v1", "-A", "zip"])
        .assert()
        .success();
    assert_eq!(read(dir.join("r-v1.zip")), "zip bytes");
    assert!(!dir.join("a.txt").exists());

    // No Content-Disposition: fall back to `<repo>-<tag>.<ext>`.
    server
        .gtx()
        .current_dir(&dir)
        .args(["release", "download", "-R", "o/r", "--archive", "tar.gz"])
        .assert()
        .success();
    assert_eq!(read(dir.join("r-v1.tar.gz")), "tgz bytes");
    assert!(server.seen().contains(&"GET /api/v1/repos/o/r/releases/latest".to_string()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn release_download_rejects_conflicting_flags() {
    let server = release_server(&["a.txt"]);
    for args in [
        &["v1", "--clobber", "--skip-existing"][..],
        &["v1", "-D", "d", "-O", "f"],
        &["v1", "-p", "*", "-A", "zip"],
        &["v1", "-A", "7z"],
    ] {
        server
            .gtx()
            .args(["release", "download", "-R", "o/r"])
            .args(args)
            .assert()
            .failure();
    }
    assert!(server.seen().is_empty());
}

/// An asset name that would escape the target directory is refused.
#[test]
fn release_download_refuses_unsafe_asset_names() {
    let base: Arc<OnceLock<String>> = Arc::new(OnceLock::new());
    let server = FakeGitea::start_with({
        let base = Arc::clone(&base);
        move |_, target| match target {
            "/api/v1/repos/o/r/releases/tags/v1" => {
                (200, release_json(base.get().unwrap(), "v1", &["../evil"]))
            }
            _ => (200, "evil".into()),
        }
    });
    base.set(server.url.clone()).unwrap();
    let dir = scratch("unsafe");
    std::fs::create_dir_all(dir.join("in")).unwrap();

    server
        .gtx()
        .current_dir(dir.join("in"))
        .args(["release", "download", "-R", "o/r", "v1"])
        .assert()
        .failure();
    assert!(!dir.join("evil").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
