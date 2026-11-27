# Proposal

## Why

humpyard is hard to reach for the person it suits best: a developer on one machine with a free-tier key or two. Today that
person exports a key, runs `init`, runs `serve` in a terminal, then (for the GUI) runs `keygen --admin`, edits the config,
restarts, starts `humpyard ui` in a second terminal and pastes a credential. The GUI is read-only and appears only after
all of it, so the first thing a newcomer sees is an empty chart, and there is no screenshot-worthy first minute. The repo is
getting little traffic, and the first five minutes are where a single-machine user decides.

## What Changes

- **`humpyard up`**: one command from install to a working, visible gateway on one machine. With no config it runs the
  `init` flow; it then starts the gateway and the admin GUI together, creates the admin credential for the run itself (no
  `keygen`), and opens the browser already signed in with the per-run access token the GUI already supports.
- **Two processes stay two** (ADR 0032): `up` is a supervisor that runs the gateway as a child (`serve`) and serves the GUI
  itself; the process that handles model requests serves no page and gains no setting.
- **A first-run GUI**: a *Connect a tool* panel with copy-paste settings for Claude Code, Codex and others, filled with the
  live address; an empty state with a *Send a test request* button; and the baseline savings strip moved to the front once
  there is traffic.
- **A deliberately small local key store** (the first secret humpyard keeps): in `up` only, the page can save a provider key
  to one `0600` file in the user's config directory. It is opt-in, write-only (add, replace or remove; never shown again,
  never returned by any call), accepted only over loopback, checked live before it is saved, and an environment variable
  always wins over the file. `serve` never reads the file; `up` hands its contents to the gateway child as environment.
- **Out of this change:** editing anything but provider keys from the GUI (routes, targets, prices), keys for non-loopback
  use, an OS keychain, running `up` as a service, Windows, and any change to `serve`, `ui` or the request path.

## Capabilities

### New Capabilities
- `single-machine-up`: the `up` command, its state directory and generated run files, the local key store and the loopback-only
  local surface the page uses.

### Modified Capabilities
- `admin-gui`: a first-run home (connect a tool, test request, empty state) when the page is served by `up`.

## Impact

- **Code:** a new `src/up.rs` (supervisor and local surface) and a small key-store module; `src/main.rs` (the `up` command);
  `ui/src` (home panel, key form, empty state, front-loaded strip) with tests; `docs/` (a new quick start, `admin-gui.md`,
  `clients.md`, `deployment.md`, `threat-model.md`, `architecture.md`, `roadmap.md`), `README.md` quick start, and
  `fitness/register.toml`.
- **API, ledger, request path:** none. `up` adds no work to the gateway process.
- **Security:** the first stored secret, and a write endpoint on a local web service. A new ADR states the limits and the
  threat model gains a boundary for them. ADR 0004 still holds for configuration and `serve`.
- **Compatibility:** nothing changes for existing `serve`, `ui` or container users.
