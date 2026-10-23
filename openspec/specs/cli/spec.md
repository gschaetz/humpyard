# cli Specification

## Purpose
TBD - created by archiving change add-migrate-modelrelay. Update Purpose after archive.

## Requirements

### Requirement: migrate-modelrelay command
The CLI SHALL provide `humpyard migrate-modelrelay` that reads a ModelRelay setup from its admin API (`--url`, with the admin token taken from the environment variable named by `--token-env`) or from saved responses (`--config FILE --models FILE`), and prints a humpyard configuration fragment containing providers, one target per usable model, and pool routes. It SHALL NOT modify any configuration file, SHALL NOT overwrite an existing `--out` file, and SHALL NOT request or write any API key value.

#### Scenario: From saved responses
- **WHEN** the command is run with `--config` and `--models` files
- **THEN** it prints a fragment and needs no network and no token

#### Scenario: Keys are never copied
- **WHEN** a provider has a key in ModelRelay
- **THEN** the fragment contains `api_key_env = "<PROVIDER>_API_KEY"` and no key value, and the key endpoint is never requested

#### Scenario: Existing output file
- **WHEN** `--out` names a file that exists
- **THEN** the command fails without writing

### Requirement: Drop-in routes
The fragment SHALL define routes named `auto-fastest` and `tag:<strength>` for each strength present, and a `+min_ctx:N` variant of each for every `--min-ctx N`, so existing clients keep their model strings. `auto-fastest` SHALL be a `fastest` route over every generated target and `tag:<strength>` a `fastest` pool of that strength.

#### Scenario: Tag routes
- **WHEN** generated targets carry `general` and `coding` and `--min-ctx 32000` is given
- **THEN** the fragment defines `tag:general`, `tag:coding`, `tag:general+min_ctx:32000`, `tag:coding+min_ctx:32000`, `auto-fastest` and `auto-fastest+min_ctx:32000`

### Requirement: Targets from models
Each model that is enabled, not banned and on an enabled provider that has a key SHALL become a target with `class = "public"` (overridable), `strengths` limited to the shared vocabulary, and its reported context window as a declared `context_window`.

#### Scenario: Banned and unkeyed models are skipped
- **WHEN** a model is banned, or its provider is disabled or has no key
- **THEN** it has no target and appears in the report with the reason

### Requirement: Report and self-check
The command SHALL report what did not carry over (extra keys of a multi-account provider, unknown providers, unknown tags, skipped models) and SHALL load the fragment through the configuration parser with placeholder keys, failing with the parser's message if it does not load.

#### Scenario: A fragment that would not load
- **WHEN** the generated fragment fails to load
- **THEN** the command exits non-zero and prints the error instead of declaring success

### Requirement: models check
The CLI SHALL provide `humpyard models check --config FILE [--provider NAME] [--json]` that, for each provider with `discover` set, fetches its models.dev source and (unless `confirm = false`) the provider's own `/models`, and reports models that both list as free chat models and that no target uses, configured endpoints the source no longer lists, and declared context windows that differ. It SHALL NOT change any file, and SHALL exit 0 when nothing is to review, 1 when something is, and 2 when a source or the config could not be used.

#### Scenario: A new free model
- **WHEN** both sources list a free chat model that no target uses
- **THEN** the report names it with its window and the exit code is 1

#### Scenario: A model only one source lists
- **WHEN** models.dev lists a free model that the provider's `/models` does not
- **THEN** it is reported as unconfirmed and is not proposed

### Requirement: models sync
The CLI SHALL provide `humpyard models sync --config FILE [--every DURATION]` that rebuilds the generated file named by `[discovery] file` from the models both sources confirm, writing it atomically only when its content changes and keeping the previous version as `.prev`. It SHALL NOT edit the main configuration, SHALL refuse a run that adds more than `max_added_per_run` targets or removes more than `max_removed_fraction` of a non-trivial generated set (writing nothing and exiting 1), and SHALL skip and report a provider whose source returns no models. With `--every` it SHALL repeat, surviving a failed run.

#### Scenario: Idempotent
- **WHEN** a run finds nothing changed
- **THEN** the file is not rewritten

#### Scenario: Adding needs both sources, keeping needs the provider
- **WHEN** a model is new and only one source lists it as free
- **THEN** it is not added
- **WHEN** a model already in the generated file is still listed by the provider but models.dev has dropped or lost it
- **THEN** it is kept

#### Scenario: Removal
- **WHEN** the provider no longer lists a generated model, or models.dev now records a nonzero cost for it
- **THEN** it is removed at the next run

#### Scenario: A flapping dataset
- **WHEN** models.dev omits a generated model for one run and lists it again the next, and the provider lists it throughout
- **THEN** the file is unchanged for both runs and the gateway does not reload

#### Scenario: A source outage
- **WHEN** a source returns an empty list for a provider that has generated targets
- **THEN** nothing is written, the guardrail is reported, and the existing file keeps serving

#### Scenario: Keys
- **WHEN** the command confirms models with a provider
- **THEN** the provider's key is sent only to that provider's own host and none is sent to models.dev

### Requirement: Scheduling is documented
The documentation SHALL state that the gateway does not run discovery itself, and SHALL give recipes for running `models sync` on a schedule, including for migrators from ModelRelay, which refreshed itself every 30 minutes.

#### Scenario: A migrator reads the guide
- **WHEN** a user follows the ModelRelay migration guide
- **THEN** it tells them to schedule `models sync` and links the recipes

#### Scenario: No Kubernetes needed
- **WHEN** a user runs the gateway on one machine or in one container
- **THEN** the guide's first recipe is a cron or other host scheduler running the one-shot command against the file the gateway reads, with a copy-pasteable crontab line, a systemd timer pair, a launchd plist and a Task Scheduler line

### Requirement: The migration carries measured quality
`humpyard migrate-modelrelay` SHALL write `quality` on a generated target from the model's score only when ModelRelay's data marks the score as measured; an estimated score SHALL be written as a comment and SHALL become a declared `quality` only with `--estimated-quality`. The report SHALL count the targets with a declared quality.

#### Scenario: Measured and estimated
- **WHEN** one model has a measured score of 0.74 and another an estimated 0.58
- **THEN** the first target declares `quality = 0.74`, the second carries the estimate only as a comment, and the report says 1 of 2 declare a quality

### Requirement: Rename providers and set a quality weight in the migration
`humpyard migrate-modelrelay` SHALL accept `--rename FROM=TO` (repeatable) to name a ModelRelay provider differently in the fragment, applying the new name to the provider table, its `api_key_env` (`<TO>_API_KEY`) and its targets' name prefix, and `--quality-weight W` (0 to 1) to write `quality_weight = W` on every generated `fastest` route. A rename whose `TO` is not lowercase letters, digits and dashes, or repeats another generated provider's name, and a weight outside 0 to 1, SHALL be refused before anything is printed.

#### Scenario: Renaming a provider
- **WHEN** the command is run with `--rename opencode=opencode-zen`
- **THEN** the fragment has `[providers.opencode-zen]` with `api_key_env = "OPENCODE_ZEN_API_KEY"`, its targets are named `opencode-zen-...` and use that provider, and no `opencode` provider appears

#### Scenario: A colliding rename
- **WHEN** two providers are renamed to the same name, or one is renamed to another provider's name
- **THEN** the command fails naming the conflict and prints no fragment

#### Scenario: A quality weight
- **WHEN** the command is run with `--quality-weight 0.5`
- **THEN** every generated `fastest` route has `quality_weight = 0.5` and the fragment loads

### Requirement: init command
The CLI SHALL provide `humpyard init`, which writes a new configuration file chosen interactively on a terminal or from flags otherwise. It SHALL read each provider key only from the environment variable the user names, SHALL NOT print, log or write a key value, and SHALL NOT overwrite an existing output file. With `--out -` it SHALL print the configuration to standard output instead of creating a file, and send every other message to standard error.

#### Scenario: Existing output file
- **WHEN** the output path already exists
- **THEN** the command fails without writing

#### Scenario: A key is never revealed
- **WHEN** a provider's variable is set
- **THEN** the command reports only that it is present and no output or file contains its value

#### Scenario: A missing key
- **WHEN** a provider's variable is unset
- **THEN** the provider is written without live checks and the command says which variable to set

#### Scenario: Configuration to standard output
- **WHEN** `--out -` is given
- **THEN** standard output holds only the configuration, which loads with the configuration parser, no file is created or checked for, and the progress, the test reply and any message go to standard error

#### Scenario: An exposed listener to standard output
- **WHEN** `--out -` is given with a non-loopback `--listen`
- **THEN** the key is printed once on standard error only, and the configuration on standard output holds its hash and never the key

### Requirement: Models are checked live
The command SHALL write a model only when the provider lists it and it answers one small request. A model that is not listed or does not answer SHALL be left out and reported with the reason.

#### Scenario: A retired model
- **WHEN** a preferred model is no longer listed by its provider
- **THEN** it is not written and the command picks another listed model or reports that none is available

### Requirement: The written file loads and is proven
The command SHALL load the file with the configuration parser before writing it and SHALL NOT write a file that does not load. It SHALL write a loopback listener by default, send one request through the gateway in process, and print the reply, the target that served it and the time it took.

#### Scenario: A working setup
- **WHEN** a provider key is set and a model answers
- **THEN** the file is written, a test request succeeds, and the command prints the serve command and a client snippet

### Requirement: Non-loopback needs a key
A non-loopback listen address SHALL be refused unless the command also creates a virtual key with a budget, which it SHALL print once.

#### Scenario: Exposing the gateway
- **WHEN** `--listen` is a non-loopback address
- **THEN** a key with a budget is created and printed once, and the file includes its hash

### Requirement: ctl is a client for a running gateway
The CLI SHALL provide `humpyard ctl` with the subcommands `status`, `models`, `health`, `stats`, `decisions`, `quota`, `explain`, `config`, `whoami` and `overrides`, each calling an existing gateway endpoint with the caller's key. Every subcommand except `overrides` SHALL be read-only and SHALL NOT change the gateway's configuration, keys or state; `overrides` MAY create and remove runtime overrides when the key may, and changes nothing else. It SHALL print the response body unchanged with `--json`.

#### Scenario: A summary
- **WHEN** `humpyard ctl status` is run against a reachable gateway
- **THEN** it prints whether the gateway is reachable, the loaded configuration version, any endpoints being skipped and the key's spend

#### Scenario: JSON for scripts
- **WHEN** `--json` is given
- **THEN** the output is the endpoint's response body without reformatting

### Requirement: The key is read from the environment and protected
The gateway address SHALL come from `--url` or `HUMPYARD_URL`. The key SHALL be read only from the environment variable named by `--key-env` (default `HUMPYARD_API_KEY`), SHALL NOT be accepted as a flag value, and SHALL NOT be printed or logged. Redirects SHALL NOT be followed.

#### Scenario: Key variable unset
- **WHEN** the named variable is unset or empty
- **THEN** the command fails naming the variable and sends no request

#### Scenario: A redirect
- **WHEN** the gateway answers with a redirect
- **THEN** the command does not follow it and reports the redirect as an error

### Requirement: Plain HTTP only to this machine
A `http://` address SHALL be accepted only for a loopback host unless `--allow-insecure-http` is given; `https://` SHALL be accepted for any host.

#### Scenario: Plain HTTP to another host
- **WHEN** the address is `http://` and the host is not loopback and the flag is absent
- **THEN** the command refuses before sending anything and says why

#### Scenario: A port-forward
- **WHEN** the address is `http://127.0.0.1:8080`
- **THEN** the request is sent

### Requirement: Exit codes distinguish refusal from failure
The command SHALL exit 0 when the gateway answers successfully, 1 when it answers 401, 403, 404 or 429, and 2 when it cannot be reached or its answer cannot be understood.

#### Scenario: A wrong key
- **WHEN** the gateway answers 401
- **THEN** the command exits 1 and prints the status and the gateway's message

#### Scenario: Gateway down
- **WHEN** the connection fails
- **THEN** the command exits 2

### Requirement: ctl config shows the effective configuration
`humpyard ctl config` SHALL show the gateway's effective configuration as text with the source of each target, print the response body unchanged with `--json`, and print only the fingerprint with `--fingerprint`. A refusal because the key may not read the configuration SHALL exit 1.

#### Scenario: Fingerprint for a drift check
- **WHEN** `humpyard ctl config --fingerprint` is run against a gateway
- **THEN** it prints the fingerprint of the configuration that gateway is serving and nothing else

#### Scenario: A key that may not read it
- **WHEN** the key lacks `read_config`
- **THEN** the command prints the gateway's message and exits 1

### Requirement: check-config prints the fingerprint
`humpyard check-config` SHALL print the fingerprint the configuration would have when loaded, computed from the same files by the same function the gateway uses, so a repository can be compared with a running gateway.

#### Scenario: Matching files
- **WHEN** `check-config` is run on the files a gateway loaded
- **THEN** the fingerprint it prints equals the one the gateway reports

### Requirement: ctl overrides manages overrides
`humpyard ctl overrides` SHALL provide `list` (with `--history`), `add` (with exactly one of `--disable-target`, `--drain-provider` or `--pin ROUTE=TARGET`, a required `--reason`, and an optional `--for` duration) and `remove ID`. `add` SHALL print the override's id and expiry. A refusal because the key may not manage overrides SHALL exit 1.

#### Scenario: Taking a target out for two hours
- **WHEN** `humpyard ctl overrides add --disable-target T --reason "slow" --for 2h` is run with a permitted key
- **THEN** it prints the new override's id and its expiry about two hours ahead

#### Scenario: A reason is required
- **WHEN** `add` is run without `--reason`
- **THEN** it fails before sending anything

#### Scenario: The history
- **WHEN** `humpyard ctl overrides list --history` is run
- **THEN** it also lists overrides that have ended (revoked or expired) from the last 30 days, each with its status
