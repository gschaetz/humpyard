# Spec Delta

## Purpose
Turn a list of typed edits into a reviewable patch against the gateway's configuration file, accepted by the real configuration loader before it is offered, without changing the file or the running gateway.

## ADDED Requirements

### Requirement: Edits become a patch only when the loader accepts the result
`POST /admin/v1/config/patch`, with the `config` scope, SHALL take the fingerprint the editor last read and a list of edits, apply them in order to the text of the configuration file the gateway loaded, and load the result with the same loader, environment and generated-targets file as a reload, including the checks a reload makes (a change that needs a restart is refused). A result the loader rejects SHALL return its reason and no patch.

#### Scenario: An edit the loader rejects
- **WHEN** an edit sets `max_request_tokens` to 0
- **THEN** the response is 422 `config_rejected` carrying the loader's message, with no diff and no new fingerprint

#### Scenario: An edit that needs a restart
- **WHEN** the edits would change a setting that a reload refuses to apply
- **THEN** the response is 422 `config_rejected` naming the setting, as a reload of the same text would

### Requirement: A valid patch is a diff, a change flag and the new fingerprint
When the result loads, the response SHALL be 200 with a unified diff, whether anything changed, and the fingerprint the new text would have.

#### Scenario: A valid edit
- **WHEN** an edit sets `targets.groq-x.endpoints[0].max_request_tokens` to 8000 and the result loads
- **THEN** the response is 200 with a diff that changes only that line, `changed` true, and the new fingerprint

#### Scenario: Edits that change nothing
- **WHEN** an edit sets a value to the value it already has
- **THEN** the response is 200 with `changed` false, an empty diff, and the unchanged fingerprint

### Requirement: Comments and ordering survive an edit
The patch SHALL be produced by editing the file's text in place, so a line that no edit touches is byte-identical in the result, and a comment attached to an edited table or key stays where it was.

#### Scenario: Untouched lines
- **WHEN** one value in a table is set
- **THEN** every other line of the file, including comments, blank lines and the order of tables, is unchanged in the diff's context

#### Scenario: A new table
- **WHEN** an edit adds a target
- **THEN** the table is added after the file's existing tables and nothing else in the diff changes

### Requirement: A stale fingerprint is refused
The request SHALL be refused with 409 `stale_fingerprint` carrying the current fingerprint when the fingerprint it names is not the fingerprint of the configuration file's text now (with the generated-targets file the gateway merges, as a reload reckons it), and nothing SHALL be computed from the old text.

#### Scenario: The file changed since the editor read it
- **WHEN** the file is edited and a request names the previous fingerprint
- **THEN** the response is 409 `stale_fingerprint` with the current fingerprint

#### Scenario: The file changed but has not reloaded yet
- **WHEN** the file differs from the running configuration (a reload has not run) and a request names the running configuration's fingerprint
- **THEN** the response is 409 `stale_fingerprint` with the file's fingerprint

### Requirement: A patch writes nothing
Producing a patch SHALL NOT write the configuration file, the generated-targets file or the ledger, SHALL NOT trigger or change a reload, and SHALL NOT change the running configuration, whatever the outcome.

#### Scenario: A valid patch
- **WHEN** a valid patch is returned
- **THEN** the file's bytes, the running configuration's fingerprint and the reload status are exactly as before

#### Scenario: Repeating a request
- **WHEN** the same request is sent twice
- **THEN** both responses are identical

### Requirement: Edits are typed
An edit SHALL be one of `set` (a value at a path), `remove` (a key at a path), `add_table` (a new table at a path with a value) or `remove_table`, with a path of table names, keys and array positions and a JSON value. An edit that is malformed, names a path that does not exist, or adds a table that exists SHALL be refused with 400 `invalid_edit` naming its position in the list, and nothing SHALL be applied.

#### Scenario: Adding a table that exists
- **WHEN** an edit adds `targets.groq-x` and that table exists
- **THEN** the response is 400 `invalid_edit` and nothing is applied

#### Scenario: A path that does not exist
- **WHEN** a `set` or `remove` names a table that is not in the file
- **THEN** the response is 400 `invalid_edit` naming the edit

### Requirement: Only named sections can be edited
A path SHALL lie inside `providers`, `targets`, `routes`, `profiles`, `aliases`, `budget`, `health`, `overrides` or `keys`. Under `keys`, only `set` and `remove` SHALL be allowed, never on a key's `sha256` and never adding or removing the key itself. The `admin`, `metrics`, `ledger` and `discovery` tables, the top-level settings and the `select` rules SHALL NOT be editable, and a refused edit SHALL return 400 `invalid_edit` naming its position.

#### Scenario: A credential section
- **WHEN** an edit's path begins with `admin`
- **THEN** the response is 400 `invalid_edit` naming that edit and no patch is computed

#### Scenario: A key's hash
- **WHEN** an edit sets `keys.alice.sha256`
- **THEN** the response is 400 `invalid_edit`

#### Scenario: Adding a key
- **WHEN** an edit adds a table under `keys`
- **THEN** the response is 400 `invalid_edit`

### Requirement: The request and the answer are bounded and carry no secret
A request body larger than 64 KiB, more than 64 edits, a path deeper than 8 or a value nested deeper than 4 SHALL be refused, naming the limit, before any edit is applied. The diff SHALL show the value of a key hash, a credential hash or a provider header as `<redacted>` wherever it appears, and no other response field SHALL contain one. A caller without the `config` scope SHALL receive 403 `admin_scope_required` naming `config`.

#### Scenario: Too many edits
- **WHEN** a request carries 65 edits
- **THEN** the response is 400 naming the limit and nothing is applied

#### Scenario: A removed provider
- **WHEN** an edit removes a provider whose table has header values
- **THEN** the diff shows the removed header lines with `<redacted>` in place of the values

#### Scenario: A read-only credential
- **WHEN** a credential with only `read` calls the endpoint
- **THEN** the response is 403 `admin_scope_required` naming `config` and the file is not read

### Requirement: No configuration file means no patch
When the gateway was started without a configuration file it can re-read, or the file cannot be read, the endpoint SHALL return 503 `config_file_unavailable` and compute nothing.

#### Scenario: An unreadable file
- **WHEN** the configuration file cannot be read when a request arrives
- **THEN** the response is 503 `config_file_unavailable`
