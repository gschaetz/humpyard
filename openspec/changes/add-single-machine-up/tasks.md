# Tasks

## 1. The key store

- [x] 1.1 Add a `keystore` module (reads and atomically replaces the `0600` file in a `0700` directory; `Secret` values that print nothing; refuses a looser file or directory with how to fix it) with `tests/up_keystore.rs` covering modes, atomic replace, refusal on a loose file, and that no value appears in `Debug` or an error. Verify: `cargo test --test up_keystore`.
- [x] 1.2 Add the environment overlay (file values under the real environment) and a `serve` test that a variable only the file holds is still a missing-variable failure. Verify: the new tests pass and the existing ADR 0004 config tests are unchanged.
- [x] 1.3 Add the architecture rule that only `keystore` reads the file and only `up` calls it (`tests/architecture.rs` `rules()`), and register the new modules in `fitness/register.toml` and `docs/fitness-functions.md`. Verify: `cargo test --test architecture` and the fitness register check pass.

## 2. `humpyard up`

- [x] 2.1 Add the state directory (`$XDG_CONFIG_HOME/humpyard` or `~/.config/humpyard`, `--state-dir`) and the per-run directory (`0700`, removed on exit and on the next start after a crash). Verify: `tests/up_run.rs` covers creation, modes and cleanup after a simulated crash.
- [x] 2.2 Generate the per-run admin credential and the run config (the user's text plus an `[admin.credentials.up]` hash block), refuse a configuration that already has `[admin]` with a clear message, and never print or log the credential or its hash. Verify: tests for a fresh credential per run, the refusal, and a log-capture test.
- [x] 2.3 Spawn `humpyard serve` as a child of the current executable with a fixed argument list and the overlay environment; stop it when `up` stops; report why and exit non-zero when it dies; refuse a taken address naming the flag. Verify: `tests/up_run.rs` covers start, stop on interrupt, child death, a taken port, and that the child's argv holds no key.
- [x] 2.4 Serve the GUI from `up` using the router `humpyard ui` uses (shared code, not a copy), forwarding `/admin/v1` with the run's credential, and open the browser with the per-run token in the URL fragment (falling back to printing the link). Verify: a test that the page, the token exchange and `/admin/v1` work from `up`, and that the gateway's own address serves no page.
- [x] 2.5 Run the `init` flow as a library call when no configuration exists (providers whose keys are found; the GUI setup state when none; never overwrite; never start on an unproven configuration). Verify: tests for first run with one key, no key, and an existing configuration, each against a mock provider.
- [x] 2.6 Add `humpyard up` to the CLI with `--listen`, `--ui-listen` and `--state-dir`, and document it in `docs/deployment.md`, a new single-machine quick start page (added to `mkdocs.yml`), and the README quick start (lead with `humpyard up`; the `init` and container paths stay). Verify: `humpyard up --help` matches the docs, the docs build strictly, and following the quick start on a clean state directory reaches a signed-in page.

## 3. The local surface

- [x] 3.1 Add the `/local` router behind the GUI's existing checks (session cookie, loopback Host, no cross-site, JSON content type, body limit, strict headers): `GET /local/connect`, `GET /local/keys`, `POST /local/keys`, `DELETE /local/keys/{provider}`, `POST /local/test`. Verify: `tests/up_local.rs` extends the `ui` boundary tests to every route (no session, cross-site, rebound Host, wrong content type, oversize body are all refused and write nothing).
- [x] 3.2 Key changes: validate the key live like `init` does, accept only a name the configuration's `api_key_env` uses, save only when asked, restart the child with the new environment, and roll back the file and child if it does not come up. Verify: tests for add, replace, remove, a rejected key (nothing saved, no restart), a failed restart (rolled back), and that no response, log or argv contains the key.
- [x] 3.3 Disable key entry (`403 key_entry_disabled`) when `up` listens beyond loopback. Verify: a test with a non-loopback `--ui-listen` and `--allow-host`.
- [x] 3.4 `POST /local/test`: the fixed prompt to the gateway's loopback address on `fast` (else the first route), returning the answering target, latency and outcome or the failing step in words. Verify: tests against a mock provider for success and for every provider failing.

## 4. The first-run GUI

- [x] 4.1 Add the home screen to `ui/src`: the *Connect a tool* panel from one table of client settings, with a test that the table matches `docs/clients.md`, and exact-copy behavior. Verify: `npm test` covers the panel, the placeholder instead of a key, and copy.
- [x] 4.2 Add the empty state with *Send a test request*, and the switch to the query view with the savings strip first after the first answered call. Verify: component tests for before traffic, after one call, and a failing test request.
- [x] 4.3 Add the write-only key form (*use for this run only* the default, *save on this machine* separate and unchecked, field cleared at once, status only, absent when entry is disabled), and make the GUI send `/local` requests only when served by `up`. Verify: component tests, including that no key reaches any rendered text, storage or URL, and that served by `humpyard ui` there are no `/local` calls.
- [x] 4.4 Update `docs/admin-gui.md` and `docs/clients.md` for the home screen, and capture a fresh screenshot and the demo GIF's first frame from it for the README. Verify: the docs build strictly and the screenshot shows no key.

## 5. Security and architecture checks

- [x] 5.1 Check the design's threat model against what was built (each row still true, each named proof exists) and add boundary B12 and the two accepted gaps to `docs/threat-model.md`; update `docs/invariants.md` with the key-file rule and its enforcement. Verify: every row's test is named and runs.
- [x] 5.2 Update `docs/architecture.md` (the supervisor, the `/local` surface, the state directory, its status table and "Last updated" line) and check its Mermaid diagrams against the code. Verify: the docs build and the diagrams match the process layout `up` really starts.
- [x] 5.3 Mark ADR 0039 accepted when the change lands and update `docs/roadmap.md` (shipped, next). Verify: the ADR index and roadmap agree.

## 6. Integration

- [ ] 6.1 On a clean machine account with no config and one real free-tier key: `humpyard up`, save the key in the page, connect Claude Code from the panel, send a request, and see it in the query view. Verify: record the steps and the elapsed time in the change's notes and fix anything that took more than a few minutes.
- [ ] 6.2 Run `cargo fmt`, clippy with `-D warnings`, the full test suite and the UI's lint and tests, and confirm the coverage floor holds. Verify: the CI `check`, `docs`, `msrv`, `deny` and `coverage` jobs pass on the PR.

## Workflow follow-up

- Archive the change in the PR that finishes its last task (`openspec archive add-single-machine-up --yes`) and move any deferred item to `docs/roadmap.md` first.
- A release PR bumps `Cargo.toml`, `docs/deployment.md` and the README image tag; the tag is pushed only when asked for.
