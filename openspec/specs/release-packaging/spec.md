# release-packaging Specification

## Purpose
Defines how humpyard releases are cut, built, checksummed and published (binaries and container image), and that nothing publishes except by an explicit version tag.

## Requirements

### Requirement: Releases happen only by tag
The release workflow SHALL run only when a tag matching `v*.*.*` is pushed. It SHALL fail before building anything if the tag does not equal `v` plus the `Cargo.toml` version, or if the tagged commit is not on `main`.

#### Scenario: No accidental release
- **WHEN** commits are pushed to `main` or a pull request is opened
- **THEN** nothing is published

#### Scenario: Mismatched tag
- **WHEN** the tag is `v0.2.0` but `Cargo.toml` says 0.1.0
- **THEN** the workflow fails at verification and publishes nothing

### Requirement: Native builds with checksums
The release SHALL contain a tarball per target (macOS arm64, Linux x86_64, Linux arm64), each built on a runner of that architecture, plus a SHA-256 checksum file covering them. Versions below 1.0 SHALL be marked as pre-releases.

#### Scenario: Release assets
- **WHEN** a valid tag is pushed
- **THEN** the release lists three tarballs and `SHA256SUMS`, and is a pre-release while the version is 0.x

### Requirement: Container image
The image SHALL run as a non-root user, contain no secrets or config, expose the ledger directory as `/data`, and be published as a multi-architecture manifest at `ghcr.io/gschaetz/humpyard` tagged with the version; `latest` SHALL move only for non-pre-releases.

#### Scenario: Pre-release image
- **WHEN** a 0.x tag is released
- **THEN** the image is tagged `0.x.y` and `latest` is not moved

### Requirement: The image build is checked on pull requests
CI SHALL build the Dockerfile on pull requests without publishing.

#### Scenario: Broken Dockerfile
- **WHEN** a pull request breaks the image build
- **THEN** the CI image job fails
