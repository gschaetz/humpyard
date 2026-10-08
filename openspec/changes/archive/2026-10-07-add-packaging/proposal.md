# Proposal

## Why

humpyard works in tests and live runs but cannot be installed: no release binaries, no image, no
service definition. Using it as the daily gateway needs all three, and a release path that is
deliberate (nothing publishes by accident) and reproducible.

## What Changes

- A **release workflow** that runs only when a `vX.Y.Z` tag is pushed (never on pushes to `main`,
  never from the UI): it verifies the tag matches `Cargo.toml` and points at a commit on `main`,
  builds on **native runners** (macOS arm64, Linux x86_64, Linux arm64), and publishes a GitHub
  release (marked pre-release while the version is 0.x) with tarballs and SHA-256 checksums.
- A **container image** (multi-stage, distroless, non-root, `/data` volume for the ledger),
  published to `ghcr.io/gschaetz/humpyard` as a two-architecture manifest tagged with the version
  (and `latest` only for non-pre-releases), built natively per architecture.
- A **PR check** that builds the Dockerfile (no push) so it cannot rot unnoticed.
- A **launchd service** template and guide for running on macOS, loading provider keys from the
  env file without printing them, with an exit timeout that covers graceful shutdown.
- `docs/deployment.md`, linked from the README.
- A release profile (thin LTO, stripped) in `Cargo.toml`.

Out of scope: signing/notarization of macOS binaries, Homebrew tap, Windows, SBOM/provenance
attestations (candidates for a later change), publishing to crates.io.

## Capabilities

### New Capabilities
- `release-packaging`: how releases, images and service definitions are produced.

### Modified Capabilities

## Impact

New: `.github/workflows/release.yml`, `Dockerfile`, `.dockerignore`, `deploy/launchd/*`,
`docs/deployment.md`, ADR 0010. Changed: `ci.yml` (image build job), `Cargo.toml` (profile),
README, AGENTS.md, architecture.md.
