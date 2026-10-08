# Deployment

Three ways to run humpyard: a release binary, the container image, or the macOS launchd service
(which uses the binary). Releases are described at the end.

Whatever you use, you need a config file (see [examples/config.toml](../examples/config.toml),
checked with `humpyard check-config`) and the provider API keys in **environment variables** named
by the config (`api_key_env`). Keys are never stored in the config.

## Release binary

Each [GitHub release](https://github.com/gschaetz/humpyard/releases) has tarballs for macOS arm64
(`aarch64-apple-darwin`) and Linux x86_64 / arm64 (`*-unknown-linux-gnu`), plus `SHA256SUMS`.

```sh
shasum -a 256 -c SHA256SUMS --ignore-missing      # sha256sum -c on Linux
tar xzf humpyard-v0.1.0-aarch64-apple-darwin.tar.gz
install -m 755 humpyard-v0.1.0-aarch64-apple-darwin/humpyard ~/.local/bin/
```

- The binaries are not signed. If macOS quarantines a downloaded one:
  `xattr -d com.apple.quarantine ~/.local/bin/humpyard`.
- Linux binaries are built on Ubuntu 24.04 and need glibc 2.39 or newer. On an older distribution
  use the container image.

## Container image

```sh
docker run -d --name humpyard -p 8080:8080 \
  -v "$PWD/config.toml:/etc/humpyard/config.toml:ro" \
  -v humpyard-data:/data \
  -e GROQ_API_KEY -e OPENROUTER_API_KEY \
  ghcr.io/gschaetz/humpyard:0.1.0
```

In the config, set `listen = "0.0.0.0:8080"` (inside the container the default loopback address is
unreachable) and the ledger path to `/data/ledger.db` so usage survives upgrades.

- The image is distroless, runs as a non-root user (65532), has no shell, config or secrets, and is
  about 70 MB. `docker stop` triggers the graceful shutdown (the default stop timeout of 10 s is
  shorter than the 30 s grace period; use `docker stop -t 45` or lower `shutdown_grace_secs`).
- There is no built-in container health check (no shell or curl in the image); probe
  `GET /healthz` from outside.
- Tags: `X.Y.Z` for every release; `latest` only moves for releases at 1.0 or later.
- A bind-mounted `/data` must be writable by uid 65532.
- First publish only: GitHub creates the package private. The owner sets it to public once under
  *Package settings* so anyone can pull.

## macOS service (launchd)

Layout (all under your home directory, nothing needs root):

| Path | What |
|---|---|
| `~/.local/bin/humpyard` | the binary |
| `~/.config/humpyard/config.toml` | the config |
| `~/.config/humpyard/env` | provider keys, `chmod 600`: lines like `OPENCODE_API_KEY=...` or `export ...` |
| `~/Library/Logs/humpyard.log` | stdout and stderr |

```sh
deploy/launchd/install.sh          # checks the files exist, installs the agent, starts it
tail -f ~/Library/Logs/humpyard.log
launchctl bootout gui/$(id -u)/dev.humpyard.gateway    # stop and unload
```

The service starts at login, restarts if it dies (10 s throttle) and loads the env file in the
shell that launches it, so keys never appear in the plist, in `launchctl print` or in logs.
`ExitTimeOut` is 50 s: launchd waits that long after SIGTERM, which covers the default 30 s grace
period plus the ledger flush. Change the environment file, then
`launchctl kickstart -k gui/$(id -u)/dev.humpyard.gateway` to restart.

## Operating notes

- `GET /healthz` answers 200 while the process is up; `GET /v1/health` (with a key when the
  gateway has keys) shows which provider endpoints are being skipped
  ([routing.md](routing.md#endpoint-health)).
- Ledger and budgets live in the SQLite file; back it up by copying the `.db` together with its
  `-wal` file, or stop the service first.
- Upgrading: replace the binary (or pull the new tag) and restart. The ledger schema is created on
  open; before 1.0, read the release notes for schema changes.

## Releases (maintainers)

Releases are made only by pushing a version tag (ADR
[0010](adr/0010-tag-only-native-releases.md)); nothing publishes from branches or pull requests.

1. In a PR, bump `version` in `Cargo.toml` and merge it.
2. From an up-to-date `main`: `git tag v0.1.0 && git push origin v0.1.0`.
3. The `release` workflow verifies that the tag equals the `Cargo.toml` version and that the commit
   is on `main`, builds each target on a runner of its own architecture, builds the two image
   architectures, and only when all of that has succeeded creates the GitHub release
   (pre-release while the version is 0.x) and the multi-architecture image manifest.

A failed run before the last job publishes nothing public except per-architecture image tags
(`X.Y.Z-amd64`, `X.Y.Z-arm64`) in the registry. To retry, delete the tag and push it again.
