# Local Gitea for `gtx` smoke testing

A docker-compose setup that runs `ghcr.io/myers/gitea` on `localhost:3333` so we can smoke-test `gtx` without depending on a remote instance.

## Quick start

```bash
./up.sh
```

This will:

1. Pull and start the Gitea container.
2. Wait for it to come up.
3. Create an admin user (`gtx-admin` / `gtx-admin-pw`).
4. Mint an API token with `all` scopes.
5. Write `gtx-config.toml` here (gitignored) with `[default]` pointing at the local instance.

To point `gtx` at it for the current shell:

```bash
source <(grep -E '^(url|token)' gtx-config.toml | sed 's/url *= */export GITEA_URL=/; s/token *= */export GITEA_TOKEN=/; s/"//g')
cargo run -p gtx -- repo list ${USER}
```

To stop and wipe:

```bash
./down.sh
```

## Notes

- Port 3333 is hardcoded — matches the existing `scripts/fetch-spec.sh` convention.
- Data lives in `./data/` (gitignored). The DB is sqlite, so persistence "just works" across `up.sh` invocations.
- The image tag is pinned in `docker-compose.yml`. Update when the upstream fork ships a new build.
- The token has `all` scopes (admin, read/write across the API). This is fine for local smoke testing; do not reuse this config against a real instance.
