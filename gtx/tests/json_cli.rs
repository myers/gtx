//! `--json` takes gh's field names and gh's value shapes.

mod common;

use common::FakeGitea;
use predicates::prelude::*;
use serde_json::json;

fn json_out(server: &FakeGitea, args: &[&str]) -> serde_json::Value {
    let out = server
        .gtx()
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap()
}

const USER: &str = r#"{"id":3,"login":"alice","full_name":"Alice A"}"#;

#[test]
fn issue_list_json_uses_gh_field_names() {
    let server = FakeGitea::start(|_| {
        format!(
            r#"[{{"id":11,"number":5,"title":"Bug","state":"closed","body":"b","user":{USER},
            "assignees":[{USER}],"labels":[{{"id":2,"name":"bug","color":"ee0701","description":"d"}}],
            "milestone":{{"id":9,"title":"M1","description":"","due_on":null}},
            "html_url":"https://g/o/r/issues/5","created_at":"2026-01-01T00:00:00Z",
            "updated_at":"2026-01-02T00:00:00Z","closed_at":"2026-01-03T00:00:00Z","pin_order":0}}]"#
        )
    });
    let v = json_out(
        &server,
        &[
            "issue",
            "list",
            "-R",
            "o/r",
            "--json",
            "number,title,state,closed,author,assignees,labels,milestone,url,createdAt,updatedAt,closedAt,id,isPinned,body",
        ],
    );
    let user = json!({"id": 3, "login": "alice", "name": "Alice A"});
    assert_eq!(
        v,
        json!([{
            "number": 5, "title": "Bug", "state": "CLOSED", "closed": true, "body": "b",
            "author": user, "assignees": [user],
            "labels": [{"id": 2, "name": "bug", "color": "ee0701", "description": "d"}],
            "milestone": {"number": 9, "title": "M1", "description": "", "dueOn": null},
            "url": "https://g/o/r/issues/5", "id": 11, "isPinned": false,
            "createdAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-01-02T00:00:00Z",
            "closedAt": "2026-01-03T00:00:00Z",
        }])
    );
    server
        .gtx()
        .args(["issue", "list", "-R", "o/r", "--json", "html_url"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown JSON field: \"html_url\""));
}

#[test]
fn pr_list_json_uses_gh_field_names() {
    let server = FakeGitea::start(|_| {
        format!(
            r#"[{{"id":21,"number":6,"title":"Feat","state":"closed","merged":true,"draft":false,
            "user":{USER},"merged_by":{USER},"mergeable":true,
            "head":{{"label":"feat","ref":"refs/pull/6/head","sha":"aaa","repo_id":2,"repo":{{"id":2,"name":"fork","owner":{USER}}}}},
            "base":{{"label":"main","ref":"main","sha":"bbb","repo_id":1}},
            "merge_commit_sha":"ccc","html_url":"https://g/o/r/pulls/6",
            "merged_at":"2026-01-03T00:00:00Z","additions":3,"deletions":1,"changed_files":2}}]"#
        )
    });
    let v = json_out(
        &server,
        &[
            "pr",
            "list",
            "-R",
            "o/r",
            "--json",
            "number,state,isDraft,headRefName,headRefOid,baseRefName,baseRefOid,isCrossRepository,mergeCommit,mergeable,mergedAt,mergedBy,url,additions,deletions,changedFiles,headRepository,headRepositoryOwner",
        ],
    );
    assert_eq!(
        v,
        json!([{
            "number": 6, "state": "MERGED", "isDraft": false,
            "headRefName": "feat", "headRefOid": "aaa",
            "baseRefName": "main", "baseRefOid": "bbb", "isCrossRepository": true,
            "mergeCommit": {"oid": "ccc"}, "mergeable": "MERGEABLE",
            "mergedAt": "2026-01-03T00:00:00Z",
            "mergedBy": {"id": 3, "login": "alice", "name": "Alice A"},
            "url": "https://g/o/r/pulls/6", "additions": 3, "deletions": 1, "changedFiles": 2,
            "headRepository": {"id": 2, "name": "fork"},
            "headRepositoryOwner": {"id": 3, "login": "alice", "name": "Alice A"},
        }])
    );
}

#[test]
fn label_list_json_uses_gh_field_names() {
    let server = FakeGitea::start(|_| {
        r#"[{"id":2,"name":"bug","color":"ee0701","description":"d","url":"https://g/api/l/2"}]"#
            .into()
    });
    let v = json_out(
        &server,
        &[
            "label",
            "list",
            "-R",
            "o/r",
            "--json",
            "id,name,color,description,url",
        ],
    );
    assert_eq!(
        v,
        json!([{"id": 2, "name": "bug", "color": "ee0701", "description": "d", "url": "https://g/api/l/2"}])
    );
}

#[test]
fn actions_lists_json_use_gh_field_names() {
    let server = FakeGitea::start(|target| {
        let path = target.split('?').next().unwrap();
        match path {
            "/api/v1/repos/o/r/actions/secrets" => {
                r#"[{"name":"TOKEN","created_at":"2026-01-01T00:00:00Z"}]"#.into()
            }
            "/api/v1/repos/o/r/actions/variables" => r#"[{"name":"V","data":"x"}]"#.into(),
            "/api/v1/repos/o/r/actions/workflows" => {
                r#"{"total_count":1,"workflows":[{"id":"ci.yml","name":"CI","path":".gitea/workflows/ci.yml","state":"active"}]}"#.into()
            }
            "/api/v1/repos/o/r/keys" => {
                r#"[{"id":1,"title":"k","key":"ssh-ed25519 AAA","read_only":true,"created_at":"2026-01-01T00:00:00Z"}]"#.into()
            }
            _ => "{}".into(),
        }
    });
    assert_eq!(
        json_out(
            &server,
            &["secret", "list", "-R", "o/r", "--json", "name,updatedAt"]
        ),
        json!([{"name": "TOKEN", "updatedAt": "2026-01-01T00:00:00Z"}])
    );
    assert_eq!(
        json_out(
            &server,
            &["variable", "list", "-R", "o/r", "--json", "name,value"]
        ),
        json!([{"name": "V", "value": "x"}])
    );
    assert_eq!(
        json_out(
            &server,
            &[
                "workflow",
                "list",
                "-R",
                "o/r",
                "--json",
                "id,name,path,state"
            ]
        ),
        json!([{"id": "ci.yml", "name": "CI", "path": ".gitea/workflows/ci.yml", "state": "active"}])
    );
    assert_eq!(
        json_out(
            &server,
            &[
                "repo",
                "deploy-key",
                "list",
                "-R",
                "o/r",
                "--json",
                "id,title,key,readOnly,createdAt"
            ]
        ),
        json!([{"id": 1, "title": "k", "key": "ssh-ed25519 AAA", "readOnly": true, "createdAt": "2026-01-01T00:00:00Z"}])
    );
}

/// Off a terminal, gh prints JSON compactly on one line.
#[test]
fn json_is_compact_when_stdout_is_not_a_terminal() {
    let server = FakeGitea::start(|_| r#"[{"id":2,"name":"bug","color":"ee0701"}]"#.into());
    server
        .gtx()
        .args(["label", "list", "-R", "o/r", "--json", "name,color"])
        .assert()
        .success()
        .stdout(r#"[{"color":"ee0701","name":"bug"}]"#.to_owned() + "\n");
}

const COMMENT: &str = r#"{"id":71,"body":"hi","user":{"id":3,"login":"alice","full_name":"Alice A"},
    "html_url":"https://g/o/r/issues/5#issuecomment-71","created_at":"2026-01-04T00:00:00Z",
    "updated_at":"2026-01-04T00:00:00Z"}"#;

fn gh_comment() -> serde_json::Value {
    json!({
        "id": 71, "author": {"login": "alice"}, "body": "hi",
        "createdAt": "2026-01-04T00:00:00Z", "includesCreatedEdit": false,
        "url": "https://g/o/r/issues/5#issuecomment-71",
    })
}

#[test]
fn issue_view_json_uses_gh_field_names() {
    let server = FakeGitea::start(|target| match target {
        "/api/v1/repos/o/r/issues/5/comments" => format!("[{COMMENT}]"),
        _ => format!(
            r#"{{"id":11,"number":5,"title":"Bug","state":"open","user":{USER},"html_url":"https://g/o/r/issues/5"}}"#
        ),
    });
    assert_eq!(
        json_out(
            &server,
            &[
                "issue",
                "view",
                "-R",
                "o/r",
                "5",
                "--json",
                "number,state,author,url,comments"
            ]
        ),
        json!({
            "number": 5, "state": "OPEN", "url": "https://g/o/r/issues/5",
            "author": {"id": 3, "login": "alice", "name": "Alice A"},
            "comments": [gh_comment()],
        })
    );
    // Comments are only fetched when asked for.
    let before = server.seen().len();
    json_out(
        &server,
        &["issue", "view", "-R", "o/r", "5", "--json", "title"],
    );
    assert_eq!(
        server.seen()[before..],
        ["GET /api/v1/repos/o/r/issues/5".to_string()]
    );
    server
        .gtx()
        .args(["issue", "view", "-R", "o/r", "5", "--json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Specify one or more comma-separated fields",
        ));
}

#[test]
fn pr_view_json_uses_gh_field_names() {
    let server = FakeGitea::start(|target| {
        let path = target.split('?').next().unwrap();
        match path {
            "/api/v1/repos/o/r/issues/6/comments" => format!("[{COMMENT}]"),
            "/api/v1/repos/o/r/pulls/6/commits" => format!(
                r#"[{{"sha":"aaa","author":{USER},"commit":{{"message":"Head line\n\nBody text",
                "author":{{"name":"Alice A","email":"a@x","date":"2026-01-01T00:00:00Z"}},
                "committer":{{"name":"Alice A","email":"a@x","date":"2026-01-02T00:00:00Z"}}}}}}]"#
            ),
            "/api/v1/repos/o/r/pulls/6/files" => {
                r#"[{"filename":"src/a.rs","additions":3,"deletions":1,"status":"modified"}]"#
                    .into()
            }
            "/api/v1/repos/o/r/pulls/6/reviews" => format!(
                r#"[{{"id":1,"user":{USER},"state":"COMMENT","body":"hm","commit_id":"aaa","submitted_at":"2026-01-02T00:00:00Z"}},
                {{"id":2,"user":{USER},"state":"APPROVED","body":"ok","commit_id":"aaa","submitted_at":"2026-01-03T00:00:00Z"}}]"#
            ),
            _ => format!(
                r#"{{"id":21,"number":6,"title":"Feat","state":"open","user":{USER},
                "head":{{"label":"feat","ref":"feat","sha":"aaa","repo_id":1}},
                "base":{{"label":"main","ref":"main","sha":"bbb","repo_id":1}}}}"#
            ),
        }
    });
    let v = json_out(
        &server,
        &[
            "pr",
            "view",
            "-R",
            "o/r",
            "6",
            "--json",
            "number,headRefName,comments,commits,files,reviews,latestReviews",
        ],
    );
    let alice = json!({"login": "alice"});
    let review = |id: i64, state: &str, body: &str, at: &str| json!({"id": id, "author": alice, "body": body, "state": state, "submittedAt": at, "commit": {"oid": "aaa"}});
    assert_eq!(
        v,
        json!({
            "number": 6, "headRefName": "feat",
            "comments": [gh_comment()],
            "commits": [{
                "oid": "aaa", "messageHeadline": "Head line", "messageBody": "Body text",
                "authoredDate": "2026-01-01T00:00:00Z", "committedDate": "2026-01-02T00:00:00Z",
                "authors": [{"email": "a@x", "id": 3, "login": "alice", "name": "Alice A"}],
            }],
            "files": [{"path": "src/a.rs", "additions": 3, "deletions": 1}],
            "reviews": [
                review(1, "COMMENTED", "hm", "2026-01-02T00:00:00Z"),
                review(2, "APPROVED", "ok", "2026-01-03T00:00:00Z"),
            ],
            "latestReviews": [review(2, "APPROVED", "ok", "2026-01-03T00:00:00Z")],
        })
    );
}

#[test]
fn pr_checks_json_uses_gh_field_names() {
    let server = FakeGitea::start(|target| {
        match target {
        "/api/v1/repos/o/r/pulls/6" => {
            r#"{"number":6,"head":{"sha":"aaa"},"base":{"sha":"bbb"}}"#.into()
        }
        _ => r#"{"state":"failure","statuses":[{"context":"ci / build (push)","status":"failure",
            "description":"Failing","target_url":"https://g/run/1","created_at":"2026-01-01T00:00:00Z",
            "updated_at":"2026-01-01T00:05:00Z"}]}"#
            .into(),
    }
    });
    assert_eq!(
        json_out(
            &server,
            &[
                "pr",
                "checks",
                "-R",
                "o/r",
                "6",
                "--json",
                "name,state,bucket,link,description,startedAt,completedAt"
            ]
        ),
        json!([{
            "name": "ci / build (push)", "state": "FAILURE", "bucket": "fail", "link": "https://g/run/1",
            "description": "Failing", "startedAt": "2026-01-01T00:00:00Z", "completedAt": "2026-01-01T00:05:00Z",
        }])
    );
}

#[test]
fn run_view_json_uses_gh_field_names() {
    let server = FakeGitea::start(|target| {
        match target {
        p if p.starts_with("/api/v1/repos/o/r/actions/runs/7/jobs") => {
            r#"{"total_count":1,"jobs":[{"id":70,"name":"build","status":"completed","conclusion":"success",
            "html_url":"https://g/o/r/actions/runs/7/jobs/0","started_at":"2026-01-01T00:00:00Z",
            "completed_at":"2026-01-01T00:01:00Z","steps":[{"number":1,"name":"checkout",
            "status":"completed","conclusion":"success","started_at":"2026-01-01T00:00:00Z",
            "completed_at":"2026-01-01T00:00:10Z"}]}]}"#
                .into()
        }
        _ => r#"{"id":7,"head_sha":"aaa","status":"completed","conclusion":"success"}"#.into(),
    }
    });
    assert_eq!(
        json_out(
            &server,
            &[
                "run",
                "view",
                "-R",
                "o/r",
                "7",
                "--json",
                "databaseId,headSha,conclusion,jobs"
            ]
        ),
        json!({
            "databaseId": 7, "headSha": "aaa", "conclusion": "success",
            "jobs": [{
                "databaseId": 70, "name": "build", "status": "completed", "conclusion": "success",
                "url": "https://g/o/r/actions/runs/7/jobs/0",
                "startedAt": "2026-01-01T00:00:00Z", "completedAt": "2026-01-01T00:01:00Z",
                "steps": [{
                    "number": 1, "name": "checkout", "status": "completed", "conclusion": "success",
                    "startedAt": "2026-01-01T00:00:00Z", "completedAt": "2026-01-01T00:00:10Z",
                }],
            }],
        })
    );
    // Jobs are only fetched when asked for.
    let before = server.seen().len();
    json_out(
        &server,
        &["run", "view", "-R", "o/r", "7", "--json", "databaseId"],
    );
    assert_eq!(
        server.seen()[before..],
        ["GET /api/v1/repos/o/r/actions/runs/7".to_string()]
    );
}

const REPO: &str = r#"{"id":4,"name":"r","full_name":"o/r","description":"d",
    "owner":{"id":1,"login":"o","full_name":""},"private":true,"fork":false,"archived":false,
    "mirror":false,"template":false,"empty":false,"default_branch":"main","stars_count":2,
    "forks_count":1,"watchers_count":3,"open_issues_count":5,"size":12,"language":"Rust",
    "website":"https://w","html_url":"https://g/o/r","ssh_url":"git@g:o/r.git",
    "clone_url":"https://g/o/r.git","has_issues":true,"has_wiki":false,"has_projects":true,
    "created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-02T00:00:00Z","topics":["cli"],
    "permissions":{"admin":true,"push":true,"pull":true}}"#;

#[test]
fn repo_view_and_list_json_use_gh_field_names() {
    let server = FakeGitea::start(|target| {
        if target.starts_with("/api/v1/user/repos") {
            format!("[{REPO}]")
        } else {
            REPO.into()
        }
    });
    let fields = "name,nameWithOwner,owner,description,url,sshUrl,isPrivate,isFork,isArchived,defaultBranchRef,stargazerCount,forkCount,createdAt,updatedAt,visibility,homepageUrl,primaryLanguage,repositoryTopics,watchers,diskUsage,viewerPermission";
    let want = json!({
        "name": "r", "nameWithOwner": "o/r", "owner": {"id": 1, "login": "o"}, "description": "d",
        "url": "https://g/o/r", "sshUrl": "git@g:o/r.git", "isPrivate": true, "isFork": false,
        "isArchived": false, "defaultBranchRef": {"name": "main"}, "stargazerCount": 2,
        "forkCount": 1, "createdAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-01-02T00:00:00Z",
        "visibility": "PRIVATE", "homepageUrl": "https://w", "primaryLanguage": {"name": "Rust"},
        "repositoryTopics": [{"name": "cli"}], "watchers": {"totalCount": 3}, "diskUsage": 12,
        "viewerPermission": "ADMIN",
    });
    assert_eq!(
        json_out(&server, &["repo", "view", "-R", "o/r", "--json", fields]),
        want
    );
    assert_eq!(
        json_out(&server, &["repo", "list", "--json", fields]),
        json!([want])
    );
    server
        .gtx()
        .args(["repo", "list", "--json", "full_name"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Unknown JSON field: \"full_name\"",
        ));
}

#[test]
fn search_json_uses_gh_field_names() {
    let server = FakeGitea::start(|target| {
        let path = target.split('?').next().unwrap();
        match path {
            "/api/v1/repos/search" => format!(r#"{{"ok":true,"data":[{REPO}]}}"#),
            "/api/v1/repos/issues/search" => format!(
                r#"[{{"id":11,"number":5,"title":"Bug","state":"open","body":"b","user":{USER},
                "assignees":[],"labels":[],"comments":2,"is_locked":false,
                "html_url":"https://g/o/r/issues/5","created_at":"2026-01-01T00:00:00Z",
                "repository":{{"id":4,"name":"r","owner":"o","full_name":"o/r"}}}}]"#
            ),
            "/api/v1/users/search" => format!(r#"{{"ok":true,"data":[{USER}]}}"#),
            _ => "{}".into(),
        }
    });
    assert_eq!(
        json_out(
            &server,
            &[
                "search",
                "repos",
                "x",
                "--json",
                "fullName,name,owner,isPrivate,stargazersCount,forksCount,defaultBranch,visibility,url,language,hasIssues,homepage,size,watchersCount,openIssuesCount"
            ]
        ),
        json!([{
            "fullName": "o/r", "name": "r", "isPrivate": true, "stargazersCount": 2, "forksCount": 1,
            "owner": {"id": 1, "is_bot": false, "login": "o", "type": "User", "url": ""},
            "defaultBranch": "main", "visibility": "private", "url": "https://g/o/r", "language": "Rust",
            "hasIssues": true, "homepage": "https://w", "size": 12, "watchersCount": 3, "openIssuesCount": 5,
        }])
    );
    let issue_fields =
        "number,title,state,author,repository,commentsCount,isLocked,isPullRequest,url,createdAt";
    let want = json!([{
        "number": 5, "title": "Bug", "state": "open", "commentsCount": 2, "isLocked": false,
        "isPullRequest": false, "url": "https://g/o/r/issues/5", "createdAt": "2026-01-01T00:00:00Z",
        "author": {"id": 3, "is_bot": false, "login": "alice", "type": "User", "url": ""},
        "repository": {"name": "r", "nameWithOwner": "o/r"},
    }]);
    assert_eq!(
        json_out(&server, &["search", "issues", "x", "--json", issue_fields]),
        want
    );
    assert_eq!(
        json_out(&server, &["search", "prs", "x", "--json", issue_fields]),
        want
    );
    let seen = server.seen();
    assert!(seen.iter().any(|s| s.starts_with("GET /api/v1/repos/issues/search?") && s.contains("type=issues")), "{seen:?}");
    assert!(
        seen.iter()
            .any(|s| s.starts_with("GET /api/v1/repos/issues/search?") && s.contains("type=pulls")),
        "{seen:?}"
    );
    assert_eq!(
        json_out(&server, &["search", "users", "a", "--json", "login,name"]),
        json!([{"login": "alice", "name": "Alice A"}])
    );
}

#[test]
fn gtx_only_lists_json_use_camel_case() {
    let server = FakeGitea::start(|target| {
        let path = target.split('?').next().unwrap();
        match path {
            "/api/v1/user/keys" => r#"[{"id":1,"title":"k","key":"ssh-ed25519 AAA","fingerprint":"SHA256:x","key_type":"user","read_only":false,"created_at":"2026-01-01T00:00:00Z"}]"#.into(),
            "/api/v1/user/gpg_keys" => r#"[{"id":2,"key_id":"ABC","emails":[{"email":"a@x","verified":true}],"can_sign":true,"created_at":"2026-01-01T00:00:00Z"}]"#.into(),
            "/api/v1/user/orgs" => r#"[{"id":3,"username":"chaos","full_name":"Chaos Inc","visibility":"public"}]"#.into(),
            "/api/v1/orgs/chaos" => r#"{"id":3,"username":"chaos","full_name":"Chaos Inc","visibility":"public"}"#.into(),
            "/api/v1/repos/o/r/milestones" => r#"[{"id":9,"title":"M1","state":"open","open_issues":2,"closed_issues":1,"due_on":null}]"#.into(),
            "/api/v1/repos/o/r/milestones/9" => r#"{"id":9,"title":"M1","state":"open","open_issues":2,"closed_issues":1}"#.into(),
            "/api/v1/notifications" => r#"[{"id":5,"unread":true,"pinned":false,"updated_at":"2026-01-01T00:00:00Z","url":"https://g/api/n/5",
                "subject":{"title":"Bug","type":"Issue","state":"open","html_url":"https://g/o/r/issues/5"},
                "repository":{"id":4,"name":"r","full_name":"o/r"}}]"#.into(),
            "/api/v1/repos/o/r/projects" => r#"[{"id":6,"title":"P","state":"open","open_issues":1}]"#.into(),
            "/api/v1/repos/o/r/projects/6" => r#"{"id":6,"title":"P","state":"closed"}"#.into(),
            "/api/v1/repos/o/r/projects/6/columns" => r##"[{"id":8,"title":"Todo","color":"#fff","default":true}]"##.into(),
            "/api/v1/repos/o/r/actions/runners" => r#"{"total_count":1,"runners":[{"id":12,"name":"r1","status":"online","busy":false,"labels":[{"id":1,"name":"ubuntu","type":"custom"}]}]}"#.into(),
            "/api/v1/repos/o/r/actions/runners/12" => r#"{"id":12,"name":"r1","status":"online","busy":true}"#.into(),
            _ => "[]".into(),
        }
    });
    let cases: &[(&[&str], serde_json::Value)] = &[
        (
            &["ssh-key", "list", "--json", "id,keyType,readOnly,createdAt"],
            json!([{"id": 1, "keyType": "user", "readOnly": false, "createdAt": "2026-01-01T00:00:00Z"}]),
        ),
        (
            &["gpg-key", "list", "--json", "keyId,emails,canSign"],
            json!([{"keyId": "ABC", "emails": [{"email": "a@x", "verified": true}], "canSign": true}]),
        ),
        (
            &["org", "list", "--json", "login,name,visibility"],
            json!([{"login": "chaos", "name": "Chaos Inc", "visibility": "public"}]),
        ),
        (
            &["org", "view", "chaos", "--json", "login,name"],
            json!({"login": "chaos", "name": "Chaos Inc"}),
        ),
        (
            &[
                "milestone",
                "list",
                "-R",
                "o/r",
                "--json",
                "number,title,state,openIssues,closedIssues,dueOn",
            ],
            json!([{"number": 9, "title": "M1", "state": "OPEN", "openIssues": 2, "closedIssues": 1, "dueOn": null}]),
        ),
        (
            &[
                "milestone",
                "view",
                "-R",
                "o/r",
                "9",
                "--json",
                "number,openIssues",
            ],
            json!({"number": 9, "openIssues": 2}),
        ),
        (
            &[
                "notification",
                "list",
                "--json",
                "id,unread,subject,repository,updatedAt",
            ],
            json!([{"id": 5, "unread": true, "updatedAt": "2026-01-01T00:00:00Z",
                 "subject": {"title": "Bug", "type": "Issue", "state": "open", "url": "https://g/o/r/issues/5"},
                 "repository": {"name": "r", "nameWithOwner": "o/r"}}]),
        ),
        (
            &[
                "project",
                "list",
                "-R",
                "o/r",
                "--json",
                "id,title,state,openIssues",
            ],
            json!([{"id": 6, "title": "P", "state": "OPEN", "openIssues": 1}]),
        ),
        (
            &["project", "view", "-R", "o/r", "6", "--json", "id,state"],
            json!({"id": 6, "state": "CLOSED"}),
        ),
        (
            &[
                "project",
                "column",
                "list",
                "-R",
                "o/r",
                "6",
                "--json",
                "id,title,color,isDefault",
            ],
            json!([{"id": 8, "title": "Todo", "color": "#fff", "isDefault": true}]),
        ),
        (
            &[
                "runner",
                "list",
                "-R",
                "o/r",
                "--json",
                "id,name,status,busy,labels",
            ],
            json!([{"id": 12, "name": "r1", "status": "online", "busy": false,
                 "labels": [{"id": 1, "name": "ubuntu", "type": "custom"}]}]),
        ),
        (
            &["runner", "view", "-R", "o/r", "12", "--json", "id,busy"],
            json!({"id": 12, "busy": true}),
        ),
    ];
    for (args, want) in cases {
        assert_eq!(&json_out(&server, args), want, "{args:?}");
    }
    server
        .gtx()
        .args(["ssh-key", "list", "--json", "key_type"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown JSON field: \"key_type\""));
}

/// `gtx api` takes gh's `-q` for `--jq` and prints compact JSON off a terminal.
#[test]
fn api_takes_q_and_prints_compact_json() {
    let server = FakeGitea::start(|_| r#"{"a": [1, 2], "b": "x"}"#.into());
    server
        .gtx()
        .args(["api", "repos/o/r", "-q", ".b"])
        .assert()
        .success()
        .stdout("x\n");
    server
        .gtx()
        .args(["api", "repos/o/r"])
        .assert()
        .success()
        .stdout("{\"a\":[1,2],\"b\":\"x\"}\n");
}

const TWO_ISSUES: &str = r#"[
  {"id":11,"number":5,"title":"Bug","state":"open","user":{"id":3,"login":"alice"},"labels":[{"id":1,"name":"bug"},{"id":2,"name":"ui"}],"created_at":"2026-01-01T10:20:30Z"},
  {"id":12,"number":123,"title":"Feature request","state":"open","user":{"id":4,"login":"bob"},"labels":[],"created_at":"2026-01-02T00:00:00Z"}
]"#;

fn stdout_of(server: &FakeGitea, args: &[&str]) -> String {
    let out = server
        .gtx()
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).unwrap()
}

#[test]
fn template_formats_json_fields() {
    let server = FakeGitea::start(|_| TWO_ISSUES.to_string());
    let out = stdout_of(
        &server,
        &[
            "issue",
            "list",
            "-R",
            "o/r",
            "--json",
            "number,title,author,labels,createdAt",
            "-t",
            r#"{{range .}}#{{.number}} {{.title}} by {{.author.login}} [{{join ", " (pluck "name" .labels)}}] {{timefmt "2006-01-02 15:04" .createdAt}}{{"\n"}}{{end}}"#,
        ],
    );
    assert_eq!(
        out,
        "#5 Bug by alice [bug, ui] 2026-01-01 10:20\n#123 Feature request by bob [] 2026-01-02 00:00\n"
    );
}

#[test]
fn template_tablerow_aligns_columns() {
    let server = FakeGitea::start(|_| TWO_ISSUES.to_string());
    let out = stdout_of(
        &server,
        &[
            "issue",
            "list",
            "-R",
            "o/r",
            "--json",
            "number,title",
            "--template",
            r#"{{range .}}{{tablerow .number .title (truncate 7 .title)}}{{end}}{{tablerender}}done{{"\n"}}"#,
        ],
    );
    assert_eq!(
        out,
        "5    Bug              Bug\n123  Feature request  Feat...\ndone\n"
    );
}

#[test]
fn template_requires_json_and_excludes_jq() {
    let server = FakeGitea::start(|_| TWO_ISSUES.to_string());
    server
        .gtx()
        .args(["issue", "list", "-R", "o/r", "-t", "{{.}}"])
        .assert()
        .failure();
    server
        .gtx()
        .args([
            "issue", "list", "-R", "o/r", "--json", "number", "-q", ".", "-t", "{{.}}",
        ])
        .assert()
        .failure();
    server
        .gtx()
        .args([
            "issue", "list", "-R", "o/r", "--json", "number", "-t", "{{.nope",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("template"));
}

#[test]
fn api_template_formats_response() {
    let server = FakeGitea::start(|_| r#"{"full_name":"o/r","stars_count":7}"#.to_string());
    let out = stdout_of(
        &server,
        &["api", "repos/o/r", "-t", "{{.full_name}}: {{.stars_count}}"],
    );
    assert_eq!(out, "o/r: 7");
}
