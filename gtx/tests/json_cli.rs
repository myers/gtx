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
            "issue", "list", "-R", "o/r", "--json",
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
            "pr", "list", "-R", "o/r", "--json",
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
        r#"[{"id":2,"name":"bug","color":"ee0701","description":"d","url":"https://g/api/l/2"}]"#.into()
    });
    let v = json_out(&server, &["label", "list", "-R", "o/r", "--json", "id,name,color,description,url"]);
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
        json_out(&server, &["secret", "list", "-R", "o/r", "--json", "name,updatedAt"]),
        json!([{"name": "TOKEN", "updatedAt": "2026-01-01T00:00:00Z"}])
    );
    assert_eq!(
        json_out(&server, &["variable", "list", "-R", "o/r", "--json", "name,value"]),
        json!([{"name": "V", "value": "x"}])
    );
    assert_eq!(
        json_out(&server, &["workflow", "list", "-R", "o/r", "--json", "id,name,path,state"]),
        json!([{"id": "ci.yml", "name": "CI", "path": ".gitea/workflows/ci.yml", "state": "active"}])
    );
    assert_eq!(
        json_out(
            &server,
            &["repo", "deploy-key", "list", "-R", "o/r", "--json", "id,title,key,readOnly,createdAt"]
        ),
        json!([{"id": 1, "title": "k", "key": "ssh-ed25519 AAA", "readOnly": true, "createdAt": "2026-01-01T00:00:00Z"}])
    );
}
