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
