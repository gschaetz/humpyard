# Spec Delta

## RENAMED Requirements
- FROM: `### Requirement: ctl is a read-only client for a running gateway`
- TO: `### Requirement: ctl is a client for a running gateway`

## MODIFIED Requirements

### Requirement: ctl is a client for a running gateway
The CLI SHALL provide `humpyard ctl` with the subcommands `status`, `models`, `health`, `stats`, `decisions`, `quota`, `explain`, `config`, `whoami` and `overrides`, each calling an existing gateway endpoint with the caller's key. Every subcommand except `overrides` SHALL be read-only and SHALL NOT change the gateway's configuration, keys or state; `overrides` MAY create and remove runtime overrides when the key may, and changes nothing else. It SHALL print the response body unchanged with `--json`.

#### Scenario: A summary
- **WHEN** `humpyard ctl status` is run against a reachable gateway
- **THEN** it prints whether the gateway is reachable, the loaded configuration version, any endpoints being skipped and the key's spend

#### Scenario: JSON for scripts
- **WHEN** `--json` is given
- **THEN** the output is the endpoint's response body without reformatting

## ADDED Requirements

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
