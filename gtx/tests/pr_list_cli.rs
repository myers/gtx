//! `pr list` with gh's filter flags, checked against the requests they send
//! and the PRs they keep.

mod common;

use common::FakeGitea;

/// PR #`n` by `author`, assigned to `assignee`, labeled `labels`, `head` →
/// `base`, `draft`.
fn pr(
    n: i64,
    author: &str,
    assignee: &str,
    labels: &[&str],
    base: &str,
    head: &str,
    draft: bool,
) -> String {
    let labels: Vec<String> = labels
        .iter()
        .map(|l| {
            format!(
                r#"{{"id":{},"name":"{l}"}}"#,
                if *l == "bug" { 7 } else { 8 }
            )
        })
        .collect();
    let assignees = if assignee.is_empty() {
        "[]".to_string()
    } else {
        format!(r#"[{{"login":"{assignee}"}}]"#)
    };
    // Gitea's head `ref` becomes `refs/pull/N/head` once the branch is
    // deleted; `label` keeps the branch name.
    let head_ref = if n == 3 {
        format!("refs/pull/{n}/head")
    } else {
        head.to_string()
    };
    format!(
        r#"{{"number":{n},"title":"PR {n}","state":"open","user":{{"login":"{author}"}},"assignees":{assignees},"labels":[{}],"base":{{"ref":"{base}","label":"{base}"}},"head":{{"ref":"{head_ref}","label":"{head}"}},"draft":{draft},"html_url":"http://h/o/r/pulls/{n}"}}"#,
        labels.join(",")
    )
}

/// A fake repo `o/r` with labels bug(7) and docs(8), current user `me`, and
/// three open PRs; `issues?type=pulls` (the keyword search) matches only #3.
fn server() -> FakeGitea {
    FakeGitea::start_with(|method, target| {
        let path = target.split('?').next().unwrap_or(target);
        let first_page =
            !target.contains("page=") || target.contains("page=1&") || target.ends_with("page=1");
        let body = match (method, path) {
            ("GET", "/api/v1/repos/o/r/labels") if first_page => {
                r#"[{"id":7,"name":"bug"},{"id":8,"name":"docs"}]"#.to_string()
            }
            ("GET", "/api/v1/user") => r#"{"login":"me"}"#.to_string(),
            ("GET", "/api/v1/repos/o/r/pulls") if first_page => format!(
                "[{},{},{}]",
                pr(1, "alice", "me", &["bug"], "main", "feat-a", false),
                pr(2, "me", "", &["bug", "docs"], "dev", "feat-b", true),
                pr(3, "bob", "carol", &[], "main", "feat-a", false),
            ),
            ("GET", "/api/v1/repos/o/r/issues") if first_page => {
                r#"[{"number":3,"pull_request":{"merged":false}}]"#.to_string()
            }
            _ => "[]".to_string(),
        };
        (200, body)
    })
}

/// Numbers `pr list -R o/r <args> --json number` prints, and the requests seen.
fn list(args: &[&str]) -> (Vec<i64>, Vec<String>) {
    let s = server();
    let out = s
        .gtx()
        .args(["pr", "list", "-R", "o/r", "--json", "number"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap();
    let nums = v.iter().map(|p| p["number"].as_i64().unwrap()).collect();
    (nums, s.seen())
}

fn pulls_query(seen: &[String]) -> &str {
    seen.iter()
        .find(|r| r.starts_with("GET /api/v1/repos/o/r/pulls?"))
        .unwrap_or_else(|| panic!("no pulls request: {seen:?}"))
}

#[test]
fn no_filters_lists_all() {
    let (nums, _) = list(&[]);
    assert_eq!(nums, [1, 2, 3]);
}

#[test]
fn label_filter_needs_every_label() {
    let (nums, seen) = list(&["-l", "bug,docs"]);
    assert_eq!(nums, [2]);
    let q = pulls_query(&seen);
    assert!(q.contains("labels=7") && q.contains("labels=8"), "{q}");
}

#[test]
fn unknown_label_matches_nothing() {
    let (nums, seen) = list(&["--label", "nope"]);
    assert!(nums.is_empty());
    assert!(!seen.iter().any(|r| r.contains("/pulls")), "{seen:?}");
}

#[test]
fn author_filter_resolves_me() {
    let (nums, seen) = list(&["-A", "@me"]);
    assert_eq!(nums, [2]);
    assert!(pulls_query(&seen).contains("poster=me"), "{seen:?}");
}

#[test]
fn assignee_filter() {
    assert_eq!(list(&["-a", "@me"]).0, [1]);
    assert_eq!(list(&["--assignee", "carol"]).0, [3]);
}

#[test]
fn base_filter() {
    let (nums, seen) = list(&["-B", "dev"]);
    assert_eq!(nums, [2]);
    assert!(pulls_query(&seen).contains("base_branch=dev"), "{seen:?}");
}

#[test]
fn head_filter() {
    assert_eq!(list(&["-H", "feat-a"]).0, [1, 3]);
    assert_eq!(list(&["--head", "nope"]).0, Vec::<i64>::new());
}

#[test]
fn draft_filter() {
    assert_eq!(list(&["-d"]).0, [2]);
    assert_eq!(list(&["--draft"]).0, [2]);
}

#[test]
fn search_uses_issue_keyword_search() {
    let (nums, seen) = list(&["-S", "needle", "-s", "all"]);
    assert_eq!(nums, [3]);
    let q = seen
        .iter()
        .find(|r| r.starts_with("GET /api/v1/repos/o/r/issues?"))
        .unwrap_or_else(|| panic!("{seen:?}"));
    for p in ["type=pulls", "q=needle", "state=all"] {
        assert!(q.contains(p), "{q}");
    }
}

#[test]
fn filters_combine() {
    assert_eq!(list(&["-H", "feat-a", "-a", "carol"]).0, [3]);
    assert_eq!(list(&["-H", "feat-a", "-d"]).0, Vec::<i64>::new());
}

/// With a client-side filter, `-L` counts matching PRs: gtx keeps paging
/// full pages until it has enough.
#[test]
fn limit_counts_filtered_prs_across_pages() {
    let s = FakeGitea::start(|target| {
        let page: i64 = target
            .split(['?', '&'])
            .find_map(|kv| kv.strip_prefix("page="))
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        let start = (page - 1) * 50;
        let prs: Vec<String> = (start..start + 50)
            .map(|i| pr(i + 1, "x", "", &[], "main", "h", i % 40 == 39))
            .collect();
        format!("[{}]", prs.join(","))
    });
    let out = s
        .gtx()
        .args([
            "pr", "list", "-R", "o/r", "-d", "-L", "2", "--json", "number",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap();
    let nums: Vec<i64> = v.iter().map(|p| p["number"].as_i64().unwrap()).collect();
    assert_eq!(nums, [40, 80]);
    let seen = s.seen();
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert!(seen.iter().all(|r| r.contains("limit=50")), "{seen:?}");
}

/// Every page is requested at the same size, so later pages don't overlap
/// earlier ones.
#[test]
fn limit_pages_at_a_fixed_size() {
    let s = FakeGitea::start(|target| {
        let _ = target;
        let prs: Vec<String> = (0..50)
            .map(|i| pr(i, "x", "", &[], "main", "h", false))
            .collect();
        format!("[{}]", prs.join(","))
    });
    s.gtx()
        .args(["pr", "list", "-R", "o/r", "-L", "120", "--json", "number"])
        .assert()
        .success();
    let seen = s.seen();
    assert_eq!(seen.len(), 3, "{seen:?}");
    assert!(seen.iter().all(|r| r.contains("limit=50")), "{seen:?}");
}
