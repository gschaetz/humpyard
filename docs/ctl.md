# Looking at a running gateway: `humpyard ctl`

`humpyard ctl` is a client for a gateway that is already running, on this machine or another. Each subcommand is
one request to an endpoint the gateway already serves, made with your own key, so it shows exactly what `curl`
would, minus the header and JSON wrangling. Every command only reads, except [`overrides`](#overrides), which can
also add and remove temporary overrides if your key is allowed to.

```sh
export HUMPYARD_URL=https://gateway.example      # or pass --url
export HUMPYARD_API_KEY=...                      # your key; read from the environment only
humpyard ctl status
```

## Commands

| Command | Shows |
|---|---|
| `status` | one screen: reachable, the loaded config version, endpoints being skipped, your key's spend |
| `models` | the routes and targets your key may use |
| `health` | which provider endpoints are being skipped, their cooldowns, and the loaded config version |
| `stats [--window 24h] [--group-by route\|target\|endpoint] [--route R] [--target T]` | how routes, targets or endpoints have performed: failure and fallback rates, latency percentiles, cost per request |
| `decisions [--limit N] [--route R] [--rule R] [--outcome O] [--request-id ID]` | what the router did for your recent requests |
| `quota` | provider quota state |
| `explain --model M [--prompt-tokens N] [--tools] [--images] [--subagent]` | what a request would do, without sending it; `--body FILE` takes a full description as JSON |
| `config [--fingerprint]` | the configuration the gateway is serving, secrets omitted, with each target's source (`file` or `generated`), the pool routes' resolved members and the config fingerprint; `--fingerprint` prints only the fingerprint. Needs a key with `read_config = true` |
| `whoami` | your key's limits, spend and budget state |
| `overrides list [--history]` / `add ...` / `remove ID` | the [runtime overrides](routing.md#runtime-overrides) in force, and adding or removing one; see [Overrides](#overrides) |

Output is plain text. `--json` prints the gateway's answer unchanged, for scripts.

## Overrides

A quick, temporary switch for when a model or provider misbehaves: stop using it for a while, with no config edit and
no reload. It always expires, and every change is recorded with your key's id. See
[Runtime overrides](routing.md#runtime-overrides) for what each kind does.

```sh
# stop using one target for two hours
humpyard ctl overrides add --disable-target deep --reason "bad answers" --for 2h
# stop using every endpoint of a provider (the default is 4 hours)
humpyard ctl overrides add --drain-provider groq --reason "provider outage"
# cut a route to one of its targets
humpyard ctl overrides add --pin auto=fast --reason "keep it local" --for 30m

humpyard ctl overrides list              # what is in force, and how long each has left
humpyard ctl overrides list --history    # also the ones that ended in the last 30 days, and how
humpyard ctl overrides remove 3
```

- `add` takes exactly one of `--disable-target TARGET`, `--drain-provider PROVIDER` or `--pin ROUTE=TARGET`, a required
  `--reason`, and an optional `--for` like `90s`, `30m`, `2h` or `1d`. It prints the new override's id and how long it
  lasts. A missing reason, a missing or doubled kind and a malformed `--for` or `--pin` are refused before anything
  is sent.
- Changing overrides needs a key with `manage_overrides = true`, and listing needs that or `read_config`; any other
  key is refused (exit 1). A gateway without a `[ledger]` cannot hold overrides (it answers 409, exit 2).
- The gateway enforces the rules: the subject must exist, a pin never adds a target, and the time is capped at
  `[overrides] max_secs`.
- An override that names something a reload has since removed is shown as `(stale)` and does nothing.

## Connecting

- **Address:** `--url`, or `HUMPYARD_URL`.
- **Key:** only from the environment variable named by `--key-env` (default `HUMPYARD_API_KEY`). It is never a flag
  value, which would land in shell history and process listings, and it is never printed.
- **Plain HTTP only to this machine.** `http://` is accepted for `localhost` and loopback addresses, so a
  `kubectl port-forward` works; any other host needs `https://`, or `--allow-insecure-http` if you accept that the
  key crosses the network unprotected.
- **Redirects are never followed**, so the key cannot be carried to another host.

```sh
# A gateway in a cluster, reached through a port-forward
kubectl -n <namespace> port-forward svc/humpyard 8080:8080 &
HUMPYARD_URL=http://127.0.0.1:8080 humpyard ctl decisions --limit 5
```

A key sees what its own requests did: `decisions` and `stats` are scoped to the key you use.

## Is the running gateway the repository's?

`humpyard check-config FILE` prints the fingerprint the configuration would have (the same function the gateway uses,
over the config file and the generated targets file). `humpyard ctl config --fingerprint` prints the one the gateway
is serving. If they differ, the running gateway is not on the files you checked.

```sh
humpyard check-config config.toml | grep fingerprint
humpyard ctl config --fingerprint
```

## Exit codes

| Code | Meaning |
|---|---|
| 0 | the gateway answered |
| 1 | the gateway refused: 401, 403, 404 or 429 (a wrong key, a key that may not do that, a rate limit) |
| 2 | it could not be used: unreachable, a bad address, the key variable unset, an answer that was not understood |

Scripts can tell "the key is wrong" (1) from "the gateway is down" (2).

## What it will not do

`ctl` never edits configuration, keys or state. Changes belong to the configuration model in
[ADR 0025](adr/0025-the-ui-has-no-state-of-its-own.md): configuration stays in the file (in a cluster, a
ConfigMap), and later write commands will print a reviewable patch or set an override that expires.
