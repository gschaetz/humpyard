# 0039. A local key file is read only by `humpyard up` and never shown again

- Status: accepted
- Date: 2026-11-26
- Supersedes: none (refines [0004](0004-no-plaintext-secrets.md): it said provider keys come only from environment variables named in the configuration; this says the one place a key may also be kept for a single-machine run, and that it still reaches the gateway only as environment)
- Change: `openspec/changes/add-single-machine-up`

## Context

ADR 0004 keeps every secret out of configuration, logs and the ledger, so an operator manages environment variables. That is the right rule for a deployment and the wrong first minute for a developer on one laptop with a free-tier key, who has to export a variable, in the shell that will start the gateway, before anything works. Asking for the key in the GUI that `humpyard up` opens is the largest single saving of effort, and it needs a place to keep the key between runs. This is the first time humpyard keeps a secret. ADR 0025 also says the GUI holds no state of its own, and ADR 0032 says the GUI runs apart from the gateway.

## Decision

`humpyard up` may keep provider keys in one file in the user's own config directory, and nothing else may.

- **One file, one reader.** The file is `0600` inside a `0700` directory, replaced atomically, and read only by `up`. `serve`, `ui`, `ctl` and every other command do not read it. `up` passes its values to the gateway it starts as environment variables the configuration already names in `api_key_env`; the gateway's loader, and ADR 0004, are unchanged: an inline key is still a startup error, and a missing variable is still a failure.
- **Opt-in.** A key entered in the page is used for the run unless the user chooses to save it on this machine.
- **Write-only.** The page can add, replace or remove a key. No call, log line, page or command-line argument ever returns or shows one afterwards; the page learns only whether a provider has a key and whether it comes from the environment or the file.
- **Only provider keys, only on loopback, only the configuration's own names.** The file holds no other secret (not the per-run admin credential, which is made fresh for each run and never stored, and not a client key). Key entry is disabled when `up` listens beyond loopback. A key can be set only for a variable the configuration already names.
- **The environment wins.** A variable set in the real environment overrides the file, so a user with a secret manager is unaffected.
- **The GUI still owns no configuration.** It writes the key through `up`, which owns the file; routes, targets and prices stay the configuration's business (ADR 0025).

Considered and rejected: an OS keychain (the right answer eventually, but three platform integrations and a dependency tree for a feature that must stay small; it can replace the file behind the same module later without a new decision); storing the key in the configuration file (breaks ADR 0004 for everyone who shares or commits a config); storing it in the ledger database (puts a secret in the file operators back up and copy); storing it in the browser (lost per profile, and then the page holds a credential); making `serve` read the file (spreads the secret to every deployment mode and to people who never opted in); asking for the key at every start (the friction this removes).

## Consequences

A single-machine user reaches a working gateway without opening a shell profile. humpyard now holds a secret on disk for the first time, so the file's mode, its single reader and its write-only page are commitments that a future change must not loosen without a new ADR; the threat model gains a boundary for them, and `tests/architecture.rs` pins that only the key-store module reads the file and only `up` calls it. A key change restarts the gateway child briefly. An OS keychain, a second secret in the file, or a reader other than `up` each need their own ADR.
