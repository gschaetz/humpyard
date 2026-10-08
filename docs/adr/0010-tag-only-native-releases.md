# 0010. Releases are tag-only and built on native runners

- Status: accepted
- Date: 2026-10-08
- Supersedes: none
- Change: [add-packaging](../../openspec/changes/archive/2026-10-07-add-packaging)

## Context

humpyard needs installable artifacts, but publishing is outward-facing and hard to undo: a binary
or image tag that has been pulled cannot be recalled. The code also compiles C (SQLite) and links
TLS native code.

## Decision

A release exists only when someone pushes a `vX.Y.Z` tag; the workflow has no other trigger. It
refuses to run unless the tag equals the `Cargo.toml` version and the commit is on `main`. Each
target (macOS arm64, Linux x86_64, Linux arm64) is built on a runner of its own architecture, and
the container image is built per architecture on native runners and merged into one manifest.
Binaries go to GitHub releases with SHA-256 sums; images go to `ghcr.io/gschaetz/humpyard`.
Versions below 1.0 are marked pre-release and do not move `latest`.

Alternatives considered: releasing on every merge to `main` (publishes unreviewed states, noisy
versions); `workflow_dispatch` (invites accidental releases from the UI); cross-compiling or QEMU
emulation (fragile with C dependencies and slow); static musl binaries (a different libc and
allocator behavior for SQLite and TLS; revisit if portability of the tarballs matters).

## Consequences

Releasing is a deliberate two-step: bump the version in a PR, then tag. Linux tarballs need a
glibc as new as the runner's (2.39); the image is the portable Linux option. There is no signing
or provenance yet; both can be added without changing this decision.
