# Proposal

## Why

A real 404 from a non-JSON upstream (2026-10-08) reached the client, and the log, as a raw HTML page. Only the `error.message` JSON shape was recognized.

## What Changes

- Upstream error text is taken from `error.message`, a string `error`, or `message`. Otherwise it is reduced to `upstream returned <status>`; short plain text is kept, collapsed to one line and clipped to 200 characters; HTML or JSON of unknown shape is dropped.

## Capabilities

### New Capabilities

### Modified Capabilities
- `upstream-proxy`: upstream error messages are normalized.

## Impact

`src/error.rs` and its unit tests, docs/providers.md.
