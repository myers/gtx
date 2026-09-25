# gtx — Gitea CLI

> **Early Alpha** — written by a coding agent, has not been used yet. YOU HAVE BEEN WARNED.

A command-line tool for Gitea, modeled after GitHub's `gh` CLI. Built in Rust.

This attempts to be a faithful copy of `gh` for GitHub. Any places it differs should be considered a bug.

## Install

```bash
cargo install --git https://github.com/myers/gtx gtx
```

## Setup

```bash
gtx auth login --url https://your-instance --token YOUR_TOKEN
gtx auth setup-git
```

The first command saves your Gitea instance URL and API token. The second configures git to authenticate using `gtx`, so clone/push/pull just work.

You can generate a token at `https://your-instance/user/settings/applications`.

## Configuration

`gtx auth login` writes `~/.config/gtx/config.toml`; set `GTX_CONFIG` to use another file. `GITEA_URL`, `GITEA_TOKEN`, `GITEA_SERVER` (selects a `[servers.NAME]` profile) and `GITEA_REPO` override it. A config left in `~/.config/gt` from before the rename is copied across on first run.

### Debugging

As with gh's `GH_DEBUG`, set `GTX_DEBUG=api` to print HTTP request/response transcripts, bodies included, on stderr; any other truthy value (`GTX_DEBUG=1`) prints headers only. `gtx api --verbose` does the same for a single API call. Tokens and cookies are masked; `gtx api --show-secrets` unmasks them.

## Usage

```
$ gtx --help
Gitea CLI

Usage: gtx [OPTIONS] <COMMAND>

Commands:
  issue         Manage issues
  pr            Manage pull requests
  repo          Manage repositories
  label         Manage labels
  milestone     Manage milestones
  release       Manage releases
  project       Manage projects
  run           Manage Actions workflow runs
  runner        Manage Actions runners
  search        Search repos, issues, users
  secret        Manage repository secrets
  variable      Manage repository variables
  notification  Manage notifications
  workflow      Manage Actions workflows
  org           Manage organizations
  package       Manage packages
  ssh-key       Manage your SSH keys
  gpg-key       Manage your GPG keys
  alias         Manage command aliases
  status        Show status dashboard (notifications, assigned, review requests)
  auth          Authentication commands
  config        Manage configuration
  browse        Open in browser
  api           Make an authenticated API request
  completion    Generate shell completions
  help          Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

## License

MIT
