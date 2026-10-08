# Design

## Decisions

- **Tag-push trigger only.** No `workflow_dispatch`, no branch trigger: the only way to release is
  to push a tag, which is a deliberate, reviewable act. Verification (tag == Cargo.toml version,
  commit on `main`) catches the two usual mistakes before anything is built.
- **Native runners, not cross-compilation.** `macos-14`, `ubuntu-24.04` and `ubuntu-24.04-arm`
  each build their own target; SQLite is compiled from C and TLS has native code, so cross
  toolchains would be the fragile part. Cost: Linux tarballs link glibc of the runner (2.39), so
  they need a comparable distro; the image builds in a Debian 12 container instead and is the
  portable Linux option. Documented.
- **Per-architecture image builds on native runners, merged into one manifest** with
  `docker buildx imagetools create`, instead of QEMU emulation (slow, flaky for Rust).
- **Distroless `cc` runtime, nonroot.** The binary is dynamically linked against glibc; distroless
  provides glibc and CA certificates (TLS uses the platform verifier) and nothing else. `/data` is
  pre-created and owned by the nonroot user so a fresh named volume is writable.
- **No third-party release actions.** Release creation uses the preinstalled `gh` CLI; only
  Docker's official actions are used for login and buildx.
- **GitHub release is the source of truth for binaries; GHCR for images.** Both use the
  workflow's `GITHUB_TOKEN` with the minimum permissions per job.
- **First version 0.1.0**, matching `Cargo.toml`, marked pre-release.

## Risks / Trade-offs

- The release workflow cannot be fully tested without publishing; it is linted with
  `actionlint`, and its building blocks (release build, image build) are run locally. The first
  tag is the real test, and a failed run publishes nothing until its last job.
- GHCR packages start private: the first publish needs the package set to public once, by the
  owner, in the package settings.
- No signing: macOS may quarantine a downloaded binary (`xattr -d com.apple.quarantine`).
