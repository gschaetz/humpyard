# Tasks

## 1. Build artifacts

- [x] 1.1 Release profile in `Cargo.toml`; `Dockerfile` and `.dockerignore`; build and run the image locally against a mock-free config (healthz, non-root, ledger writable on a fresh volume)
- [x] 1.2 CI job that builds the image on pull requests

## 2. Release workflow

- [x] 2.1 `release.yml`: tag-only trigger, verification job, native build matrix, release job with checksums and pre-release flag, per-arch image jobs and manifest merge; lint with `actionlint`

## 3. Running it

- [x] 3.1 launchd template and wrapper behaviour verified locally (loads the env file, serves, stops gracefully on unload)
- [x] 3.2 `docs/deployment.md`; README, AGENTS.md (release process), architecture.md status; ADR 0010
