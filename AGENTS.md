# gtx — project notes for Claude

## Stability

This is alpha software. The only users are the author and Claude. **No one
should expect a stable interface.** Don't preserve old exit codes, output
formats, flag spellings, or config keys for backwards compatibility — if the
new behavior is better (especially: closer to `gh` parity per README), just
change it. Don't add deprecation shims, `--legacy-foo` flags, or compat
warnings.

## Versioning

Bump `gtx/Cargo.toml`'s `version` on every commit, as you (Claude) feel
appropriate:

- patch (`0.2.0` → `0.2.1`): bug fix, doc tweak, internal refactor with no
  user-visible change.
- minor (`0.2.0` → `0.3.0`): new subcommand, new flag, new behavior, anything
  the user could notice in `--help` output or tab completion.
- major: breaking changes to existing flags / output formats / config schema.

Version + `--version` output also embeds the short git SHA and build date
(see `gtx/build.rs`); no need to touch those by hand.

`gitea-api/Cargo.toml` doesn't get bumped per-commit — only when its public
surface meaningfully changes.

## Formatting

Keep the workspace rustfmt-clean: run `cargo fmt --all` before committing.
`gtx/tests/fmt.rs` fails `cargo test` otherwise (no CI). Formatting-only
commits go in `.git-blame-ignore-revs`.
