# gateway-config Specification

## Purpose
Defines how operators configure and start the gateway, and how credentials are supplied.

## Requirements

### Requirement: TOML configuration
The gateway SHALL load a TOML file defining the listen address, any number of providers (name, base URL, API key env var name, timeout), targets with ordered endpoints, and routes with a built-in algorithm over named targets.

#### Scenario: Valid config
- **WHEN** `serve --config <path>` is run with a valid file
- **THEN** the server listens on the configured address

#### Scenario: Invalid config
- **WHEN** the file is missing a required field or has an unknown key
- **THEN** the process exits non-zero with a message naming the problem

#### Scenario: Dangling reference
- **WHEN** an endpoint names an unknown provider, or a route names an unknown target
- **THEN** the process exits non-zero naming the missing reference

#### Scenario: Duplicate names
- **WHEN** two providers, targets or routes share a name, or a route and a target share a name
- **THEN** the process exits non-zero naming the duplicate

### Requirement: API keys come from the environment
The configuration SHALL name an environment variable for each provider's API key and SHALL reject an inline key.

#### Scenario: Missing key variable
- **WHEN** a provider's named environment variable is unset at startup
- **THEN** startup fails with an error naming the provider and the variable

### Requirement: Config validation command
The CLI SHALL provide `check-config <path>` that validates a file without starting the server.

#### Scenario: Check a good file
- **WHEN** `check-config` runs on a valid file
- **THEN** it exits 0

### Requirement: Provider headers
The configuration SHALL allow each provider an optional `headers` table of additional HTTP headers, and SHALL reject header names or values that are not valid HTTP and names that would override authentication.

#### Scenario: Valid headers
- **WHEN** a provider declares `headers = { "x-app" = "conductor" }`
- **THEN** the configuration loads

#### Scenario: Authentication header rejected
- **WHEN** a provider declares a header named `authorization` or `x-api-key`
- **THEN** the process exits non-zero naming the provider and the header

#### Scenario: Invalid header name
- **WHEN** a provider declares a header whose name contains whitespace
- **THEN** the process exits non-zero naming the provider and the header

### Requirement: Keys, ledger and prices in configuration
The configuration SHALL allow `[keys.<id>]` entries (hash, optional route allowlist, budgets, `over_budget`), a `[ledger]` section naming the SQLite file, a `[budget]` section with the restricted fraction and price ceiling, and an optional `price` (input, output, optional cached input, USD per million tokens) on each endpoint.

#### Scenario: Valid budget config
- **WHEN** the file declares a key with `daily_usd`, a ledger path and priced endpoints
- **THEN** it loads

#### Scenario: Budget without ledger
- **WHEN** a key declares a budget but no ledger path is configured
- **THEN** startup fails explaining that budgets need a ledger

#### Scenario: Invalid limits
- **WHEN** a limit is negative or `restricted_at` is outside 0 to 1
- **THEN** startup fails naming the field

### Requirement: Shutdown grace period
The configuration SHALL accept an optional top-level `shutdown_grace_secs`, a whole number of seconds from 0 to 3600 with a default of 30, bounding how long the gateway waits for in-flight requests after a termination signal.

#### Scenario: Default
- **WHEN** the key is absent
- **THEN** the grace period is 30 seconds

#### Scenario: Out of range
- **WHEN** the value is above 3600
- **THEN** the process exits non-zero naming the key

### Requirement: Health configuration
The config MAY contain a `[health]` section with `failure_threshold` (default 3), `cooldown_secs` (default 30) and `max_cooldown_secs` (default 300). A threshold of 0 SHALL disable endpoint health tracking. Invalid values (cooldown 0 while enabled, max below cooldown, values above one day) SHALL be rejected naming the key.

#### Scenario: Defaults
- **WHEN** the section is absent
- **THEN** health tracking is on with 3, 30 and 300

#### Scenario: Disabled
- **WHEN** `failure_threshold = 0`
- **THEN** no endpoint is ever skipped

#### Scenario: Invalid
- **WHEN** `max_cooldown_secs` is below `cooldown_secs`
- **THEN** loading fails naming `max_cooldown_secs`

### Requirement: Target strengths
A target MAY declare `strengths`: a list of at most 8 labels, each 1 to 32 characters of lowercase letters, digits, `-` and `_`, with no duplicates. Strengths SHALL NOT affect any request except through the pool routes that name them, and SHALL NOT remove a target from a request.

#### Scenario: Declared strengths
- **WHEN** a target declares `strengths = ["coding", "general"]`
- **THEN** the configuration loads and the target is a candidate for pools named `coding` and `general`

#### Scenario: Invalid strengths
- **WHEN** a strength is empty, longer than 32 characters, contains an uppercase letter or a space, is repeated on a target, or a target lists more than 8
- **THEN** the configuration is rejected with a message naming the target and the strength

### Requirement: Pool routes
A `passthrough`, `fastest` or `random` route SHALL accept `pool = { strength = "...", min_context_window = N }` in place of `targets`, with `min_context_window` optional. The route's targets SHALL be every target that carries the strength and, when a window is given, has at least one endpoint declaring a `context_window` of at least that value, resolved when the configuration is loaded or reloaded and ordered by target name. `pool` and `targets` SHALL NOT be combined, and `weights` SHALL NOT be combined with `pool`.

#### Scenario: Resolved at load
- **WHEN** targets `a`, `b` and `c` carry `general` and `d` does not, and a route has `pool = { strength = "general" }`
- **THEN** the route's targets are `a`, `b`, `c` and a request to the route never considers `d`

#### Scenario: Minimum context window
- **WHEN** a pool sets `min_context_window = 32000` and target `a` declares a 64000 window, `b` declares 8000 and `c` declares none
- **THEN** only `a` is a member, and `b` and `c` are reported as left out with the reason

#### Scenario: Empty pool
- **WHEN** no target carries the pool's strength, or none qualifies
- **THEN** the configuration is rejected naming the strength and listing the strengths that exist

#### Scenario: Reload changes membership
- **WHEN** a reload adds the strength to another target
- **THEN** the route is rebuilt with the new membership and the reload log names the pool whose membership changed

#### Scenario: Strengths never refuse a request
- **WHEN** a request is served by a pool route
- **THEN** the only removals are the usual per-request ones (class, policy, capability, health); strengths remove nothing

### Requirement: Two switches
Discovery SHALL be off unless `[discovery] enabled` is true (which requires `file`) and the provider has `discover` with `enabled` true (the default). With the master switch off the generated file SHALL NOT be read and `models sync` SHALL refuse to write; a provider with `enabled = false` SHALL have its generated targets skipped by the loader at once and removed by the next sync.

#### Scenario: Master switch off
- **WHEN** `[discovery] file` is set and `enabled` is false or absent
- **THEN** the file is not merged, the state is reported as disabled, and `models sync` exits 2 naming the switch

#### Scenario: One provider opts out
- **WHEN** two providers have generated targets and one sets `discover.enabled = false`
- **THEN** the other provider's targets still load and the opted-out provider's are skipped

### Requirement: Discovery opt-in and settings
A provider MAY set `discover = { strengths = [...], confirm = true }` (requires `price_source`), and the configuration MAY have `[discovery]` with `file`, `class` (default `public`), `max_added_per_run` (default 10), `max_removed_fraction` (default 0.5) and `exclude` (a list of `provider/model` ids).

#### Scenario: Opting in without a source
- **WHEN** a provider sets `discover` but not `price_source`
- **THEN** the configuration is rejected naming the provider

### Requirement: The generated file is merged, strictly, and fails safe
When `[discovery] file` names a file, `serve` SHALL parse it at load and on every reload and merge its targets, accepting it only if it contains nothing but `[targets.auto-*]` on providers that exist and have `discover` set, no more than 500 targets, no (provider, model) pair that a hand-written target already uses, and every endpoint priced at zero. A file that does not meet this, or is missing or empty, SHALL be ignored with a log line and a gauge, and the gateway SHALL serve with the main configuration alone. `serve` SHALL NOT fetch any data to do this.

#### Scenario: A valid generated file
- **WHEN** the file holds `auto-groq-x` with the strength `general`
- **THEN** a pool of `general` includes it after the next reload

#### Scenario: An invalid generated file
- **WHEN** the file contains a table other than a target, or a priced endpoint
- **THEN** it is ignored in full and the gateway serves with the main configuration

#### Scenario: Reload sees the file
- **WHEN** only the generated file changes
- **THEN** the gateway reloads

### Requirement: Generated targets are visible
Generated targets SHALL be named `auto-*`, SHALL be marked as generated in `route/explain`, and the number loaded and the file's state SHALL be exposed as metrics.

#### Scenario: Explain marks them
- **WHEN** a client explains a route whose pool includes a generated target
- **THEN** the target is reported with `generated = true`

### Requirement: Declared quality
A target MAY declare `quality`, a number from 0 to 1. A pool MAY set `min_quality`; its members SHALL be the targets that carry the strength, satisfy any `min_context_window`, and declare a `quality` of at least `min_quality`. A target with no declared quality SHALL NOT qualify, and SHALL be reported as left out with the reason `quality_undeclared` (or `quality_below`).

#### Scenario: A quality floor
- **WHEN** targets declare qualities 0.7, 0.5 and none, and a pool sets `min_quality = 0.6`
- **THEN** only the first is a member, and explain lists the others with `quality_below` and `quality_undeclared`

#### Scenario: Out of range
- **WHEN** a quality is below 0 or above 1, or `min_quality` is
- **THEN** the configuration is rejected naming the target or route

### Requirement: Override settings and the manage_overrides key setting
The configuration MAY set `[overrides] default_secs` (default 14400) and `max_secs` (default 86400, at most 604800), with the default not above the maximum, and a key MAY set `manage_overrides = true` (default false). Invalid values SHALL be rejected when the configuration loads.

#### Scenario: A default above the maximum
- **WHEN** `default_secs` is greater than `max_secs`
- **THEN** the configuration is refused
