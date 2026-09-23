# gtx CLI — TODO

Command roadmap modeled after `gh` (GitHub CLI), adapted for Gitea.

## Currently Implemented

- [x] `gtx issue list` — list issues (state filter, limit, JSON output, table output)
- [x] `gtx issue view <number>` — view issue details (comments via `-c`, JSON output)
- [x] Config system — `~/.config/gtx/config.toml`, env vars `GITEA_URL` / `GITEA_TOKEN`

## Priority 1: Core Commands

### Git-remote-aware repo detection
Foundation for all commands — auto-detect repo from git remote so users
don't need `-R owner/repo` on every command.

- [x] Auto-detect `owner/repo` from git remote `origin`
- [x] Match remote URL to configured Gitea instance URL
- [x] Support `-R owner/repo` override (like `gh -R`)
- [x] Support `GITEA_REPO` env var

### `gtx api` — Raw API access (backdoor to everything)
The universal escape hatch. Lets users hit any Gitea API endpoint
without waiting for dedicated subcommands.

- [x] `gtx api <endpoint>` — GET request to Gitea API
- [x] `gtx api -X POST <endpoint>` — specify HTTP method
- [x] `gtx api -f key=value` — add form/JSON fields
- [x] `gtx api -F key=value` — typed fields (int, bool, @file)
- [x] `gtx api -H key:value` — custom headers
- [x] `gtx api --jq <expr>` — filter JSON output with jq syntax
- [x] `gtx api --paginate` — follow pagination links
- [x] `gtx api -i` — include response headers
- [x] `{owner}` / `{repo}` placeholder expansion from git remote

### `gtx pr` — Pull request management
- [x] `gtx pr list` — list PRs (state, label, assignee filters)
- [x] `gtx pr view <number>` — view PR details, diff stats, CI status
- [x] `gtx pr create` — create PR (title, body, base branch, head branch auto-detected)
- [x] `gtx pr checkout <number>` — fetch and checkout PR branch
- [x] `gtx pr merge <number>` — merge PR (merge/rebase/squash, --delete-branch)
- [x] `gtx pr close <number>` — close PR
- [x] `gtx pr reopen <number>` — reopen PR
- [x] `gtx pr comment <number>` — add comment
- [x] `gtx pr diff <number>` — view diff
- [x] `gtx pr review <number>` — approve/request changes
- [x] `gtx pr checks <number>` — show CI status

### `gtx issue` — Complete issue management
- [x] `gtx issue create` — create issue (title, body, assignees)
- [x] `gtx issue close <number>` — close issue
- [x] `gtx issue reopen <number>` — reopen issue
- [x] `gtx issue comment <number>` — add comment
- [x] `gtx issue edit <number>` — edit title, body
- [x] `gtx issue status` — show open issues in repo

## Priority 2: Repository & Navigation

### `gtx repo` — Repository management
- [x] `gtx repo view` — show repo info (description, stats, default branch)
- [x] `gtx repo clone <owner/repo>` — clone a Gitea repo
- [x] `gtx repo list` — list repos for user/org
- [x] `gtx repo create` — create new repo
- [x] `gtx repo fork` — fork a repo

### `gtx browse` — Open in browser
- [x] `gtx browse` — open repo in browser
- [x] `gtx browse <number>` — open issue/PR in browser
- [x] `gtx browse --settings` — open repo settings

## Priority 3: Projects, Labels, Milestones, Releases

### `gtx project` — Project board management
- [x] `gtx project list` — list projects
- [x] `gtx project view <id>` — view project details
- [x] `gtx project create` — create project
- [x] `gtx project close` / `gtx project reopen`
- [x] `gtx project column list` — list columns
- [x] `gtx project column create` — add column

### `gtx label` — Label management
- [x] `gtx label list` — list labels
- [x] `gtx label create` — create label (name, color, description)
- [x] `gtx label edit` — edit label
- [x] `gtx label delete` — delete label

### `gtx milestone` — Milestone management
- [x] `gtx milestone list` — list milestones
- [x] `gtx milestone create` — create milestone
- [x] `gtx milestone view` — view milestone details
- [x] `gtx milestone close` / `gtx milestone reopen`

### `gtx release` — Release management
- [x] `gtx release list` — list releases
- [x] `gtx release create` — create release with assets
- [x] `gtx release view` — view release
- [x] `gtx release download` — download release assets
- [x] `gtx release delete` — delete release

## Priority 4: Actions, Org

### `gtx run` — Actions/CI
- [x] `gtx run list` — list workflow runs
- [x] `gtx run view <id>` — view run details and logs
- [x] `gtx run rerun <id>` — rerun a workflow
- [x] `gtx run watch [<id>]` — poll until complete (`--exit-status`, `--compact`, `--interval`)
- [x] `gtx run download <id>` — download artifacts (`-d <dir>`)
- [ ] `gtx run cancel <id>` — cancel an in-progress run (mirrors `gh run cancel`)
- [ ] `gtx run delete <id>` — delete a run from history (mirrors `gh run delete`)

### `gtx secret` — Actions secrets (gap, hit during 2026-05 drawbar eval)
Mirrors `gh secret`. Today the only path is `gtx api --method PUT
repos/{owner}/{repo}/actions/secrets/<NAME> --field 'data=<value>'`,
which is awkward enough that it surprised me twice in one session.

- [ ] `gtx secret list` — list repo/org actions secrets (names only, never values)
- [ ] `gtx secret set <NAME>` — create or update; read value from `--body`, `--body-file`, or stdin
- [ ] `gtx secret delete <NAME>` — remove a secret
- [ ] `--org <ORG>` flag so each subcommand can target org secrets instead of repo secrets

### `gtx org` — Organization management
- [x] `gtx org list` — list orgs the user belongs to
- [x] `gtx org view <name>` — view org details

## Priority 5: Auth, Config, Completions

### `gtx auth` — Authentication
- [x] `gtx auth login` — interactive login (token input or browser OAuth)
- [x] `gtx auth status` — show current auth state
- [x] `gtx auth logout` — remove credentials
- [x] Multi-instance support (multiple Gitea servers via `[servers.NAME]` + `GITEA_SERVER` env)

### `gtx config` — Configuration
- [x] `gtx config get <key>` — read config value
- [x] `gtx config set <key> <value>` — write config value
- [x] `gtx config list` — show all config

### `gtx completion` — Shell completions
- [x] `gtx completion bash`
- [x] `gtx completion zsh`
- [x] `gtx completion fish`

## Architecture Notes

### `gtx api` design
The `api` command is the universal escape hatch. It should:
1. Accept any Gitea API path (e.g., `repos/{owner}/{repo}/issues`)
2. Expand `{owner}` and `{repo}` from git remote
3. Handle pagination transparently with `--paginate`
4. Support jq-style filtering for scripting
5. Use the same auth config as other commands

This means every Gitea API feature is accessible immediately, even before
we write a dedicated subcommand.

### Output conventions
- **Table output** (default for TTY): human-readable columns, truncated to terminal width
- **JSON output** (`--json`): machine-readable, full data
- **jq filtering** (`--jq`): for scripting pipelines
- All commands should support `--json` for scriptability

### Testing
Set up after repo detection is implemented:

**Unit tests** (`#[cfg(test)]` modules in each source file):
- Config parsing — TOML loading, env var overrides, missing values
- Repo detection — parse git remote URLs (SSH, HTTPS, `git@`, custom ports) into owner/repo
- Output formatting — relative time, table truncation
- API command arg parsing — field types, placeholder expansion

**Integration tests** (`gtx/tests/integration.rs`, uses `assert_cmd` crate):
- Start a real Gitea instance (temp dir, SQLite, ephemeral port)
- Create test data via API (user, repo, issues, labels, project)
- Run `gtx` binary and assert on stdout/stderr/exit code
- Test commands: `gtx issue list`, `gtx issue view`, `gtx api`, `gtx pr list`
- Tear down after each test

**Workflow**: run `cargo test` and commit only after ALL tests pass (including
pre-existing ones). Fix any pre-existing test failures before moving on.

### Config hierarchy
1. CLI flags (highest priority)
2. Environment variables (`GITEA_URL`, `GITEA_TOKEN`, `GITEA_REPO`)
3. Repo-local config (`.gtx.toml` in repo root — future)
4. User config (`~/.config/gtx/config.toml`)
