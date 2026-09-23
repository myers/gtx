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
        r#"{{"id":4,"tag_name":"{tag}","name":"Title {tag}","body":"notes","html_url":"{base}/o/r/releases/tag/{tag}","assets":[{}],"zipball_url":"{base}/o/r/archive/{tag}.zip","tarball_url":"{base}/o/r/archive/{tag}.tar.gz"}}"#,
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
    assert_eq!(
        seen[0], "GET /api/v1/repos/o/r/releases/tags/v1",
        "{seen:?}"
    );
    assert!(
        seen.contains(&"GET /attachments/a.txt".to_string()),
        "{seen:?}"
    );
    assert!(
        server
            .auth()
            .iter()
            .all(|a| a.as_deref() == Some("token t"))
    );
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
        .args([
            "release",
            "download",
            "-R",
            "o/r",
            "-p",
            "*.deb",
            "--pattern",
            "*.rpm",
        ])
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
        .args([
            "release", "download", "-R", "o/r", "v1", "-p", "a.*", "-O", "renamed",
        ])
        .assert()
        .success();
    assert_eq!(read(dir.join("renamed")), "a.txt bytes\n");

    server
        .gtx()
        .current_dir(&dir)
        .args([
            "release", "download", "-R", "o/r", "v1", "-p", "b.*", "-O", "-",
        ])
        .assert()
        .success()
        .stdout("b.deb bytes\n");

    server
        .gtx()
        .current_dir(&dir)
        .args(["release", "download", "-R", "o/r", "v1", "-O", "both"])
        .assert()
        .failure()
        .stderr(contains(
            "unable to write more than one asset with `--output`",
        ));
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
    assert!(
        server
            .seen()
            .contains(&"GET /api/v1/repos/o/r/releases/latest".to_string())
    );
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

/// A fake Gitea for the tag-addressed release commands: release `v1` (id 4,
/// also the latest) with asset `a.txt` (id 0); draft `v2` (id 7) that the
/// by-tag endpoint 404s on but the draft listing has; git tag `v1` exists.
fn tag_server() -> FakeGitea {
    let base: Arc<OnceLock<String>> = Arc::new(OnceLock::new());
    let server = FakeGitea::start_with({
        let base = Arc::clone(&base);
        move |method, target| {
            let base = base.get().unwrap();
            let v1 = release_json(base, "v1", &["a.txt"]);
            let draft = format!(
                r#"{{"id":7,"tag_name":"v2","name":"Draft","draft":true,"html_url":"{base}/o/r/releases/tag/v2"}}"#
            );
            let path = target.split('?').next().unwrap();
            match (method, path) {
                (
                    "GET",
                    "/api/v1/repos/o/r/releases/tags/v1" | "/api/v1/repos/o/r/releases/latest",
                ) => (200, v1),
                ("GET", "/api/v1/repos/o/r/releases") if target.contains("draft=true") => {
                    if target.contains("page=1") {
                        (200, format!("[{draft}]"))
                    } else {
                        (200, "[]".into())
                    }
                }
                ("GET", "/api/v1/repos/o/r/tags/v1") => (200, r#"{"name":"v1"}"#.into()),
                ("POST", "/api/v1/repos/o/r/releases") => (201, v1),
                ("PATCH", "/api/v1/repos/o/r/releases/4" | "/api/v1/repos/o/r/releases/7") => {
                    (200, v1)
                }
                ("POST", "/api/v1/repos/o/r/releases/4/assets") => (201, r#"{"id":9}"#.into()),
                ("DELETE", _) => (204, String::new()),
                _ => (404, r#"{"message":"not found"}"#.into()),
            }
        }
    });
    base.set(server.url.clone()).unwrap();
    server
}

/// The JSON body sent with the first request `METHOD target`.
fn sent_json(server: &FakeGitea, request: &str) -> serde_json::Value {
    let seen = server.seen();
    let i = seen
        .iter()
        .position(|s| s == request)
        .unwrap_or_else(|| panic!("no {request} in {seen:?}"));
    serde_json::from_str(&server.bodies()[i]).unwrap()
}

#[test]
fn release_view_by_tag_prints_gh_plain_format() {
    let server = tag_server();
    server
        .gtx()
        .args(["release", "view", "-R", "o/r", "v1"])
        .assert()
        .success()
        .stdout(contains(
            "title:\tTitle v1\ntag:\tv1\ndraft:\tfalse\nprerelease:\tfalse\n",
        ))
        .stdout(contains("asset:\ta.txt\n--\nnotes\n"));
    assert_eq!(server.seen(), ["GET /api/v1/repos/o/r/releases/tags/v1"]);
}

#[test]
fn release_view_without_tag_shows_latest() {
    let server = tag_server();
    server
        .gtx()
        .args(["release", "view", "-R", "o/r"])
        .assert()
        .success()
        .stdout(contains("tag:\tv1\n"));
    assert_eq!(server.seen(), ["GET /api/v1/repos/o/r/releases/latest"]);
}

#[test]
fn release_view_json_selects_fields_of_one_object() {
    let server = tag_server();
    let out = server
        .gtx()
        .args([
            "release",
            "view",
            "-R",
            "o/r",
            "v1",
            "--json",
            "id,tag_name",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v, serde_json::json!({"id": 4, "tag_name": "v1"}));

    server
        .gtx()
        .args([
            "release",
            "view",
            "-R",
            "o/r",
            "v1",
            "--json",
            "tag_name",
            "--jq",
            ".tag_name",
        ])
        .assert()
        .success()
        .stdout("v1\n");
}

/// Like gh, a draft the by-tag lookup can't see is found among the drafts.
#[test]
fn release_view_finds_draft_by_tag() {
    let server = tag_server();
    server
        .gtx()
        .args(["release", "view", "-R", "o/r", "v2"])
        .assert()
        .success()
        .stdout(contains("tag:\tv2\ndraft:\ttrue\n"));
    let seen = server.seen();
    assert_eq!(seen[0], "GET /api/v1/repos/o/r/releases/tags/v2");
    assert!(
        seen[1].starts_with("GET /api/v1/repos/o/r/releases?"),
        "{seen:?}"
    );
}

#[test]
fn release_view_unknown_tag_errors() {
    let server = tag_server();
    server
        .gtx()
        .args(["release", "view", "-R", "o/r", "nope"])
        .assert()
        .failure()
        .stderr(contains("release not found"));
}

#[test]
fn release_create_takes_tag_positionally_with_gh_flags() {
    let server = tag_server();
    server
        .gtx()
        .args([
            "release", "create", "-R", "o/r", "v1", "-t", "My title", "-n", "My notes", "--target",
            "dev", "-p",
        ])
        .assert()
        .success()
        .stdout(format!("{}/o/r/releases/tag/v1\n", server.url));
    let body = sent_json(&server, "POST /api/v1/repos/o/r/releases");
    assert_eq!(body["tag_name"], "v1");
    assert_eq!(body["name"], "My title");
    assert_eq!(body["body"], "My notes");
    assert_eq!(body["target_commitish"], "dev");
    assert_eq!(body["prerelease"], true);
    assert_eq!(body["draft"], false);
}

#[test]
fn release_create_reads_notes_file_and_stdin() {
    let server = tag_server();
    let dir = scratch("notes");
    std::fs::write(dir.join("notes.md"), "from file\n").unwrap();
    server
        .gtx()
        .current_dir(&dir)
        .args([
            "release", "create", "-R", "o/r", "v1", "-F", "notes.md", "-d",
        ])
        .assert()
        .success();
    let body = sent_json(&server, "POST /api/v1/repos/o/r/releases");
    assert_eq!(body["body"], "from file\n");
    assert_eq!(body["draft"], true);

    let server = tag_server();
    server
        .gtx()
        .args(["release", "create", "-R", "o/r", "v1", "--notes-file", "-"])
        .write_stdin("from stdin\n")
        .assert()
        .success();
    assert_eq!(
        sent_json(&server, "POST /api/v1/repos/o/r/releases")["body"],
        "from stdin\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Like gh, files are uploaded to a draft that is published afterwards.
#[test]
fn release_create_with_files_uploads_then_publishes() {
    let server = tag_server();
    let dir = scratch("create-files");
    std::fs::write(dir.join("b.bin"), "b").unwrap();
    server
        .gtx()
        .current_dir(&dir)
        .args(["release", "create", "-R", "o/r", "v1", "b.bin"])
        .assert()
        .success();
    let seen = server.seen();
    assert_eq!(
        seen,
        [
            "POST /api/v1/repos/o/r/releases",
            "POST /api/v1/repos/o/r/releases/4/assets?name=b.bin",
            "PATCH /api/v1/repos/o/r/releases/4",
        ]
    );
    assert_eq!(sent_json(&server, &seen[0])["draft"], true);
    assert_eq!(
        sent_json(&server, &seen[2]),
        serde_json::json!({"draft": false})
    );

    // --draft leaves it a draft.
    let server = tag_server();
    server
        .gtx()
        .current_dir(&dir)
        .args(["release", "create", "-R", "o/r", "v1", "b.bin", "--draft"])
        .assert()
        .success();
    assert_eq!(server.seen().len(), 2, "{:?}", server.seen());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn release_create_verify_tag_aborts_on_missing_tag() {
    let server = tag_server();
    server
        .gtx()
        .args(["release", "create", "-R", "o/r", "v9", "--verify-tag"])
        .assert()
        .failure()
        .stderr(contains("tag v9 doesn't exist in the repo o/r"));
    assert_eq!(server.seen(), ["GET /api/v1/repos/o/r/tags/v9"]);
}

#[test]
fn release_create_missing_file_fails_before_creating() {
    let server = tag_server();
    server
        .gtx()
        .args(["release", "create", "-R", "o/r", "v1", "/nonexistent/x.bin"])
        .assert()
        .failure();
    assert!(server.seen().is_empty());
}

#[test]
fn release_create_without_tag_needs_a_terminal() {
    let server = tag_server();
    server
        .gtx()
        .args(["release", "create", "-R", "o/r"])
        .assert()
        .failure()
        .stderr(contains("tag required when not running interactively"));
    assert!(server.seen().is_empty());
}

#[test]
fn release_edit_by_tag_sends_only_given_fields() {
    let server = tag_server();
    server
        .gtx()
        .args([
            "release",
            "edit",
            "-R",
            "o/r",
            "v1",
            "-t",
            "New",
            "--draft=false",
            "--prerelease",
            "--tag",
            "v1.0",
            "--target",
            "dev",
        ])
        .assert()
        .success()
        .stdout(format!("{}/o/r/releases/tag/v1\n", server.url));
    assert_eq!(server.seen()[0], "GET /api/v1/repos/o/r/releases/tags/v1");
    assert_eq!(
        sent_json(&server, "PATCH /api/v1/repos/o/r/releases/4"),
        serde_json::json!({
            "name": "New", "draft": false, "prerelease": true,
            "tag_name": "v1.0", "target_commitish": "dev",
        })
    );
}

#[test]
fn release_edit_publishes_a_draft_by_tag() {
    let server = tag_server();
    server
        .gtx()
        .args([
            "release",
            "edit",
            "-R",
            "o/r",
            "v2",
            "--draft=false",
            "-n",
            "N",
        ])
        .assert()
        .success();
    assert_eq!(
        sent_json(&server, "PATCH /api/v1/repos/o/r/releases/7"),
        serde_json::json!({"draft": false, "body": "N"})
    );
}

#[test]
fn release_delete_by_tag_keeps_git_tag_unless_cleanup() {
    let server = tag_server();
    server
        .gtx()
        .args(["release", "delete", "-R", "o/r", "v1", "--yes"])
        .assert()
        .success();
    assert_eq!(
        server.seen(),
        [
            "GET /api/v1/repos/o/r/releases/tags/v1",
            "DELETE /api/v1/repos/o/r/releases/4"
        ]
    );

    let server = tag_server();
    server
        .gtx()
        .args([
            "release",
            "delete",
            "-R",
            "o/r",
            "v1",
            "-y",
            "--cleanup-tag",
        ])
        .assert()
        .success();
    assert_eq!(
        server.seen(),
        [
            "GET /api/v1/repos/o/r/releases/tags/v1",
            "DELETE /api/v1/repos/o/r/releases/4",
            "DELETE /api/v1/repos/o/r/tags/v1",
        ]
    );
}

#[test]
fn release_upload_by_tag_refuses_existing_name_without_clobber() {
    let server = tag_server();
    let dir = scratch("upload");
    std::fs::write(dir.join("a.txt"), "new").unwrap();
    std::fs::write(dir.join("c d.txt"), "c").unwrap();

    server
        .gtx()
        .current_dir(&dir)
        .args(["release", "upload", "-R", "o/r", "v1", "c d.txt"])
        .assert()
        .success();
    assert_eq!(
        server.seen(),
        [
            "GET /api/v1/repos/o/r/releases/tags/v1",
            "POST /api/v1/repos/o/r/releases/4/assets?name=c+d.txt",
        ]
    );

    let server = tag_server();
    server
        .gtx()
        .current_dir(&dir)
        .args(["release", "upload", "-R", "o/r", "v1", "a.txt"])
        .assert()
        .failure()
        .stderr(contains(
            "asset under the same name already exists: [a.txt]",
        ));
    assert_eq!(server.seen(), ["GET /api/v1/repos/o/r/releases/tags/v1"]);

    let server = tag_server();
    server
        .gtx()
        .current_dir(&dir)
        .args(["release", "upload", "-R", "o/r", "v1", "a.txt", "--clobber"])
        .assert()
        .success();
    assert_eq!(
        server.seen(),
        [
            "GET /api/v1/repos/o/r/releases/tags/v1",
            "DELETE /api/v1/repos/o/r/releases/4/assets/0",
            "POST /api/v1/repos/o/r/releases/4/assets?name=a.txt",
        ]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn release_delete_asset_by_tag_and_name() {
    let server = tag_server();
    server
        .gtx()
        .args(["release", "delete-asset", "-R", "o/r", "v1", "a.txt", "-y"])
        .assert()
        .success();
    assert_eq!(
        server.seen(),
        [
            "GET /api/v1/repos/o/r/releases/tags/v1",
            "DELETE /api/v1/repos/o/r/releases/4/assets/0"
        ]
    );

    let server = tag_server();
    server
        .gtx()
        .args(["release", "delete-asset", "-R", "o/r", "v1", "zzz", "-y"])
        .assert()
        .failure()
        .stderr(contains("asset zzz not found in release v1"));
    assert_eq!(server.seen().len(), 1);
}
