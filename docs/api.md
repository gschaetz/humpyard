# API and endpoints

Clients can speak any of three protocols; providers are reached over OpenAI Chat Completions.
Set `"stream": true` for server-sent events.

```sh
# OpenAI Chat Completions
curl localhost:8080/v1/chat/completions -H 'content-type: application/json' \
  -d '{"model":"fast","messages":[{"role":"user","content":"hi"}]}'

# Anthropic Messages (e.g. Claude Code: ANTHROPIC_BASE_URL=http://localhost:8080)
curl localhost:8080/v1/messages -H 'content-type: application/json' \
  -d '{"model":"fast","max_tokens":100,"messages":[{"role":"user","content":"hi"}]}'

# OpenAI Responses (e.g. Codex CLI)
curl localhost:8080/v1/responses -H 'content-type: application/json' \
  -d '{"model":"fast","input":"hi"}'
```

## When a key is over its rate limit

A key with `max_requests_per_minute` or `max_concurrent` ([Limiting a key's speed](budgets.md#limiting-a-keys-speed))
that is over either gets HTTP 429 before any upstream call is made, with a `Retry-After` header in whole seconds
(the time until the oldest request leaves the window, at least 1; always 1 for the concurrency limit) and the
code `rate_limit_exceeded`:

```json
{"error": {"message": "key `agent` is limited to at most 60 requests per minute", "type": "rate_limit_error", "code": "rate_limit_exceeded"}}
```

On `/v1/messages` the body has the Anthropic shape (`{"type": "error", "error": {"type": "rate_limit_error", ...}}`).
The code tells it apart from a 429 that a provider sent and the gateway passed through. A client should wait
`Retry-After` seconds and try again; the refused request used no allowance.

## Everything else

| Endpoint | What it does |
|---|---|
| `GET /v1/models` | the routes and targets a key may use, plus the [aliases](configuration.md) for them (an `[aliases.<id>]` entry carries its `display_name` and `description` when set). A request may name an alias as its `model`; it is served, authorized and recorded as the real route or target. `model_aliases = false` removes aliases from both |
| `GET /healthz` | liveness |
| `GET /v1/key/info` | a key's actor and owner, limits, spend and budget state |
| `GET /v1/health` | which provider endpoints are being skipped, and which config version is loaded |
| `GET /v1/config` | the configuration the gateway is serving, secrets omitted, with where each target and route came from (`file` or `generated`) and the config fingerprint; needs a key with `read_config = true` ([Keys](budgets.md#virtual-keys)) |
| `POST /v1/overrides`, `GET /v1/overrides`, `DELETE /v1/overrides/{id}` | create, list (`?history=true` adds the ones that have ended in the last 30 days) and remove [runtime overrides](routing.md#runtime-overrides): an expiring instruction to stop using a target (`disable_target`) or a provider (`drain_provider`), or to cut a route to one target (`pin_route`). Creating and removing need a key with `manage_overrides = true`; listing needs that or `read_config` |
| `POST /v1/route/explain` | what a request would do, without sending it ([Routing](routing.md#selecting-the-route)) |
| `GET /v1/decisions` | what the router did for your past requests; every response carries an `x-humpyard-request-id` to look one up ([the decision log](budgets.md#the-decision-log)); `request.repaired_tool_arguments` counts tool calls whose cut-off arguments were repaired, and is absent when none were; `response_signals` lists the [quality signals](routing.md#reading-a-bad-answer) set on the answer, and is absent when none is set |
| `GET /v1/stats` | how routes, targets and endpoints have performed: failure mix, fallbacks, latency percentiles, cost per request, and per group the [response signals](routing.md#reading-a-bad-answer) as `observed` calls and a count per signal ([statistics](budgets.md#statistics)) |
| `GET /v1/stats/switches` | what changing endpoint in the middle of a session abandoned: switches, cached tokens left behind, the extra cost at the destination's prices, session coverage ([Cache use and sessions](budgets.md#cache-use-and-sessions)) |
| `GET /v1/quota` | the latest usage windows (percent used, status, reset time) of each provider that names a `quota_source`, with the age of the data, or why it is unknown ([Provider quota](routing.md#provider-quota)) |
| `GET /v1/quota/history` | stored quota samples (`window` such as `7d`, optional `provider` and `name`), oldest first, each with the cost of the provider's calls since the previous sample of its window; needs a ledger ([Provider quota](routing.md#provider-quota)) |
| `GET /admin/v1/whoami`, `/config`, `/health`, `/quota`, `/decisions`, `/stats`, `/stats/series`, `/credentials`, `/overrides`, the revoke endpoints, `POST /admin/v1/config/patch` and `/openapi.json` | the [admin API](#the-admin-api): who you are, and what the gateway is doing across every key |
| `GET /metrics` | Prometheus metrics, off by default; on a separate listener or behind its own credential ([Metrics and alerts](metrics.md)) |
| `POST /v1/messages/count_tokens`, `POST /v1/responses/input_tokens` | conservative local estimates: no provider call, no cost |

## The admin API

`/admin/v1` is the operator surface (ADR 0027, OpenSpec change `add-admin-api`). It is opened only by an **admin
credential**, defined in the configuration's [`[admin]` section](configuration.md), and by nothing else: a
client key never opens it, however many flags it carries, and an admin credential never serves an inference
or any other client endpoint. Send it as `Authorization: Bearer <secret>`.

- **Absent by default.** Without a credential in the configuration the whole namespace answers 404, as if it did
  not exist. With `[admin] listen` it is served only on that address and is a 404 on the public port.
- **One refusal.** A missing, unknown or expired credential, and a client key, all get the same 401 with the
  code `admin_credential_invalid` and `WWW-Authenticate: Bearer`, so the answer says nothing about which
  credentials exist.
- **Scopes.** An endpoint needs a scope (`read`, `overrides`, `credentials` or `config`); a valid credential without
  it gets 403 with the code `admin_scope_required` and the missing scope in `scope`. No scope implies another:
  `config`, which lets a credential ask for a patch to the configuration file, does not include `read`, so an
  editor holds both.
- **Reload.** Credentials are part of the configuration snapshot: a reload applies additions, removals and
  changed scopes or expiry at once, and a rejected reload leaves the old set in force. Moving `[admin] listen`
  needs a restart.
- **Counted.** `humpyard_admin_requests_total{result}` ([Metrics](metrics.md)).

Mint a credential with `humpyard keygen gary --admin --scopes read,overrides`: it prints the secret once and the
`[admin.credentials.gary]` block to paste into the configuration (add `--expires 2027-01-01T00:00:00Z` for an expiry).

### Overrides and revoking a credential

| Endpoint | Scope | What it does |
|---|---|---|
| `GET /admin/v1/overrides` | `read` | the [runtime overrides](routing.md#runtime-overrides) in force; `?history=true` adds the last 30 days |
| `POST /admin/v1/overrides` | `overrides` | creates one, under the same rules as `POST /v1/overrides` |
| `DELETE /admin/v1/overrides/{id}` | `overrides` | removes one in force |
| `POST /admin/v1/credentials/{id}/revoke` | `credentials` | revokes an admin credential: body `{"reason": "...", "expires_in_secs": 3600}` |
| `DELETE /admin/v1/credentials/{id}/revoke` | `credentials` | ends that revocation early |
| `POST /admin/v1/keys/{id}/revoke` | `credentials` | revokes a **client key** the same way, so a leaked one is retired without a deploy; same body |
| `DELETE /admin/v1/keys/{id}/revoke` | `credentials` | ends that revocation early |

An override made here is recorded as made by `admin:<id>` (a client key is recorded by its own id), so the audit
rows say who did it.

**Revoking a credential** is how a colleague's access is cut without a deploy. It takes effect on the credential's
very next request, with no reload: the answer is the same 401 an unknown secret gets. It lasts `expires_in_secs`
(the `[overrides]` default of four hours when omitted, never more than its maximum, a day by default), ends by
itself, and can be ended early with `DELETE`. It only ever removes access. A `reason` is required, the id must be
a credential in the loaded configuration (anything else is a 400 `invalid_revocation`), revoking one that is
already revoked is a 409, and it needs a `[ledger]` like any override (409 `overrides_need_ledger` without one).
An admin may revoke their own credential, which is how one is retired. To remove a credential for good, delete it
from the file and reload. `GET /admin/v1/credentials` shows `revoked: true` while one is in force, and
`humpyard_admin_revocations_active` ([Metrics](metrics.md)) counts them; the shipped alert rules include
`HumpyardAdminRevoked` for one that lasts over 30 minutes.

**Revoking a client key** works the same way and with the same body, rules and limits, at
`POST /admin/v1/keys/{id}/revoke` (the id is a key of `[keys.*]`; an admin credential's id is a 400). From the next
request the key is refused with exactly the 401 a key the gateway never knew gets, so nobody can tell a revoked key
from an unknown one. Other keys are untouched. `GET /admin/v1/config` shows `revoked: true` and `revoked_until_ms` on
the key while it is in force. It survives a restart, ends by itself or with `DELETE`, and deleting the key from the
file is the permanent step. Revoking an admin credential is counted by `humpyard_admin_revocations_active`; a key
revocation is not.

A revocation is stored with the overrides but is not one: it is not listed by `/v1/overrides` or
`/admin/v1/overrides`, is not counted in `humpyard_overrides_active`, never affects routing, and cannot be ended
through the override endpoints, so a client key that manages overrides cannot restore a revoked admin or a revoked key.

### Configuration patches

`POST /admin/v1/config/patch` (scope `config`) turns a list of typed edits into a reviewable patch against the
configuration file, and **writes nothing**: not the file, not the ledger, and the running gateway is not reloaded
or changed. It is how a tool (the admin GUI later) proposes a structural change; the operator applies it the way
the file is managed (a commit, a ConfigMap, a template). The credential also needs `read` to learn the file's
fingerprint, from `GET /admin/v1/config` (`config.fingerprint`).

```
curl -X POST https://gateway.example/admin/v1/config/patch \
  -H "Authorization: Bearer $ADMIN_SECRET" -H 'content-type: application/json' \
  -d '{"fingerprint": "f50a5670de4f",
       "edits": [{"op": "set", "path": ["targets", "groq-x", "endpoints", 0, "max_request_tokens"], "value": 8000}]}'
```

- **Edits.** `set` (a value at a key or array position; the key may be new, its table must exist), `remove` (a key
  or array element), `add_table` (a new table; `value` is an object of its keys) and `remove_table`. A path is table
  names, keys and array positions, such as `["targets", "t", "endpoints", 0, "model"]`, at most 8 segments; values
  are JSON (no `null`) nesting at most 4 deep. Edits apply in order, and none applies unless all do.
- **What can be edited.** `providers`, `targets`, `routes`, `profiles`, `aliases`, `budget`, `health`, `overrides`
  and `keys`. Under `keys` only `set` and `remove` work, never on a key's `sha256`, and a key cannot be added or
  removed here. The `admin` table, `metrics`, `ledger`, `discovery`, the top-level settings and the `[[select]]`
  rules cannot be edited this way.
- **Formatting survives.** The edits are made in the file's own text, so every line an edit does not touch comes
  back byte for byte, a replaced value keeps its trailing comment, and a new table is placed beside the others of
  its section.
- **The same check as a reload.** The edited text is loaded with the same loader, environment and generated
  targets file a reload uses, and the gateway also builds the serving state from it. A patch is stricter than a
  reload in one way: it is refused if it would make the gateway stop merging the generated targets file.
- **The answer.** `{"object": "config_patch", "changed": true, "base_fingerprint": "...", "fingerprint": "...",
  "edits": 1, "diff": "..."}`. `fingerprint` is what the file's fingerprint would be with the edits applied; with
  `changed: false` the diff is empty and the two fingerprints are equal. The diff is a unified diff with three
  lines of context. **It is for review, not for `patch(1)`:** a key hash, a credential hash and every provider
  header value are shown as `<redacted>` (`<redacted: changed>` when the value differs after the edits), so a diff
  that crosses such a line does not apply there. The edit list you sent is the reproducible form.
- **Refusals.** 400 `invalid_edit` names the edit (`error.index`, from 0) that is malformed, outside the editable
  sections or does not fit the file; 400 `invalid_request` is a malformed body or more than 64 edits; 409
  `stale_fingerprint` means the file has changed since you read it (`error.current_fingerprint` is its fingerprint
  now, including an edit the gateway has not reloaded yet); 413 `request_too_large` is a body over 64 KiB; 422
  `config_rejected` carries the loader's reason and no patch; 503 `config_file_unavailable` means the gateway has
  no configuration file it can read.

The `config` scope is separate from `read` on purpose: a diff shows the lines around an edit, so a credential
that holds it can learn the file's structure by probing (secrets are masked). Give it to a person or tool you
would let read the configuration file. Each request logs the admin id and the outcome, never the edits or the file.

### Reading

`GET /admin/v1/whoami` returns the caller's `id`, `kind` (`credential`), `scopes` and `expires` (a UTC time, or
`null`) and needs no particular scope. Every other read below needs `read`.

| Endpoint | What it returns |
|---|---|
| `GET /admin/v1/config` | the effective configuration, as `GET /v1/config` returns it, with the `admin` section in full: each credential's id, scopes and expiry (client keys see only a count) |
| `GET /admin/v1/health` | what `GET /v1/health` returns: skipped endpoints, latency, the loaded configuration, the last reload |
| `GET /admin/v1/quota` | what `GET /v1/quota` returns |
| `GET /admin/v1/decisions` | the decision log of **every key**, newest first, each failed attempt with the provider's `message`; `?key=alice` narrows it to one key, and the usual filters (`limit`, `route`, `outcome`, `since_ms`, ...) apply |
| `GET /admin/v1/stats` | the [statistics](budgets.md#statistics) of **every key**; `?key=alice` narrows them, and `window` and `group_by` work as on `/v1/stats`, which `group_by` extends with `actor` and `owner` ([below](#statistics-by-actor-and-owner)); a [heavy read](#heavy-reads) |
| `GET /admin/v1/stats/series` | the same statistics [over time](#statistics-over-time): equal buckets per group; a [heavy read](#heavy-reads) |
| `GET /admin/v1/credentials` | every admin credential: `id`, `kind`, `scopes`, `expires`, and `revoked` while a revocation is in force |

The client endpoints keep their rule that a key sees only its own decisions and statistics, and do not accept
`key`; only an admin reads across keys. Nothing here ever returns a secret or a hash of one.

```
curl -H "Authorization: Bearer $ADMIN_SECRET" "https://gateway.example/admin/v1/decisions?key=alice&limit=20"
```

### Statistics by actor and owner

`GET /admin/v1/stats` and `GET /admin/v1/stats/series` also accept `group_by=actor` and `group_by=owner`, which count by
who a key was for ([`[actors]`](configuration.md), ADR 0037). `actor` is the actor the key named when the request was
made, which is one agent or person; `owner` is the person it rolls up to, the human an agent acts for, or the human
itself for a human's own key. Each request counts once, by the same rules as the other groupings, with its cost and
tokens. Requests with no actor (or no owner) are the group `(none)`, so nothing is left out of the totals. The ids are
the ones recorded on each decision, so a key that changes owner later does not move its earlier requests. The client
`GET /v1/stats` does not accept them (`400`): a key sees only its own calls. Each decision in the log also carries
`actor` and `owner`.

### Statistics over time

`GET /admin/v1/stats/series` (scope `read`) returns the numbers of `GET /admin/v1/stats` in equal time buckets, for
the busiest groups, so a chart can show when something changed. It is not offered on the client API (`/v1/stats/series`
is a 404). It counts by the same rules and from the same decision rows as `/admin/v1/stats`, so each group's bucket
counts add up to the group's totals.

| Parameter | Meaning |
|---|---|
| `window` | How far back, such as `6h` or `7d`; default `24h`; at most the days of decisions kept |
| `interval` | Bucket width, such as `5m` or `1h`; at least `1m`, and no more than 400 buckets in the window. Default: the finest of 1m, 5m, 15m, 30m, 1h, 2h, 6h, 12h and 1d that gives at most 120 buckets |
| `group_by` | `route` (default), `target`, `endpoint`, `actor` or `owner` (see [Statistics by actor and owner](#statistics-by-actor-and-owner)) |
| `route`, `target` | Only this route, or only attempts on this target |
| `key` | Only this client key's requests; omit for every key |
| `limit` | The most groups returned, from 1 to 25; default 8 |

Any other parameter, and any value outside these bounds, is a `400` whose message names the parameter; nothing is read.

```json
{
  "object": "stats_series",
  "computed_at_ms": 1783000000000,
  "group_by": "target",
  "interval_ms": 900000,
  "window": {"since_ms": 1782913600000, "seconds": 86400},
  "now_ms": 1783000000000,
  "truncated": false,
  "covered_since_ms": null,
  "omitted_groups": 0,
  "data": [
    {"group": "fast", "buckets": [
      {"start_ms": 1782913200000, "requests": 12, "attempts": 13, "failures": 1, "failure_rate": 0.0769,
       "fallbacks": 1, "fallback_rate": 0.0833, "latency_ms": {"p50": 410, "p90": 900, "p99": 1450}},
      {"start_ms": 1782914100000, "requests": 0, "attempts": 0, "failures": 0, "failure_rate": null,
       "fallbacks": 0, "fallback_rate": null, "latency_ms": {"p50": null, "p90": null, "p99": null}}
    ]}
  ]
}
```

- **Buckets are aligned to the Unix epoch**: each starts at a whole multiple of `interval_ms` (UTC; a one-day interval
  starts at UTC midnight), so the same moment is in the same bucket on every read. Every bucket from the one
  holding the start of the window to the one holding `now_ms` is present, and the last is still filling.
- **A quiet bucket is not zero**: with no requests the counts are zero and the rates and percentiles are `null`.
- **`omitted_groups`** counts the groups with requests that `limit` left out; the groups kept are the busiest, by requests.
- **`truncated`** is true when the window held more than 50,000 decisions, so the oldest were not read;
  `covered_since_ms` is then the time of the oldest one counted. Buckets before it are unread, not quiet.

### Heavy reads

Statistics (`GET /v1/stats`, including `group_by=session`, `GET /admin/v1/stats` and `GET /admin/v1/stats/series`) and the switches report
(`GET /v1/stats/switches`) fold up to 50,000 ledger rows, so they are rationed to keep them from ever slowing the
requests the gateway serves ([ADR 0036](adr/0036-admin-and-ui-work-must-not-change-the-request-paths-performance.md)):

- **One at a time per surface.** The administrative surface (`/admin/v1`) and the client surface (`/v1`) each
  allow one heavy read at once; a second is answered `503` with `Retry-After: 5` and never queued. The error code is
  `admin_busy` on the admin surface and `stats_busy` on the client surface, and the same answer is given when the
  ledger could not be read in time. A client key reading its own statistics in a loop cannot make the operator's GUI
  answer busy, because the surfaces have gates of their own.
- **Identical reads share a result.** While a read is running, an identical one (same surface, same caller, same
  endpoint and parameters, in any order) waits for it instead of starting another. For ten seconds after, it is
  answered from that result. A caller never receives another caller's result: a client key's own id, or the admin `key`
  filter, is part of what makes two reads identical. Only a successful answer within 2 MiB is kept, at most 16 at
  once; an error or a `503` is never kept.
- **How old a number is.** Each of these answers carries `computed_at_ms`, the unix time in milliseconds when it was
  computed. A repeat within the ten seconds states the same time, so a number read from the shared result says
  how old it is; the figures reflect the ledger as of that time.
- **Before the gate.** Authentication and the scope check come first, so a credential without `read` is answered
  `403 admin_scope_required` and cannot occupy the gate or read a result a permitted caller made. Other admin
  reads (`/admin/v1/whoami`, `health`, `config`, overrides and the rest) are not heavy reads and are never held up by one.

A client that sees a `503` here should wait at least the `Retry-After` seconds before asking again (a tool that
polls should not shorten that), and can ask for the same read again afterwards.

### The contract

`GET /admin/v1/openapi.json` returns the API as an OpenAPI document to any valid admin credential, and the same
file is [`admin-v1.json`](openapi/admin-v1.json) in the repository. Each operation lists the scope it needs and
the error codes it can answer with. One table in the gateway builds the routes, and a test fails if the document
and the served routes differ in a path, a method, a scope or an error code, so a client typed against the
document can rely on it.

Verified with real Claude Code, Codex CLI and OpenClaw sessions: see [Clients](clients.md).
