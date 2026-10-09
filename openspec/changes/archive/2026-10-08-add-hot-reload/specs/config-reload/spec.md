# Spec Delta

## ADDED Requirements

### Requirement: Reload on SIGHUP
On SIGHUP the gateway SHALL re-read its config file and, if the file loads and validates, atomically replace the configuration used for new requests. Requests already running SHALL finish on the configuration they started with.

#### Scenario: Changed rule takes effect
- **WHEN** a selector rule is edited and SIGHUP is sent
- **THEN** new requests follow the edited rule without a restart

#### Scenario: In-flight request unaffected
- **WHEN** a streaming request is running during a reload
- **THEN** it completes normally on the old configuration

### Requirement: Optional polling
With `reload_poll_secs` above zero the gateway SHALL re-read the config file at that interval and reload when its content differs from the last version it looked at. A file that fails to load SHALL be attempted once, not on every interval.

#### Scenario: Edited file, no signal
- **WHEN** polling is on and the file is edited
- **THEN** the new configuration takes effect within the interval without any signal

#### Scenario: Broken file
- **WHEN** polling is on and the file is broken
- **THEN** one failed reload is recorded, not one per interval, and the previous configuration keeps serving

### Requirement: A bad config never takes effect
If the file cannot be read, fails validation, or cannot be built, the gateway SHALL keep serving the previous configuration, log the error and report it.

#### Scenario: Invalid file
- **WHEN** the edited file names an unknown route
- **THEN** requests keep following the previous config and the health report shows the failed reload

### Requirement: Restart-only settings are refused
A reload that changes the listen address, the ledger path, `shutdown_grace_secs`, `reload_poll_secs`, or switches between having keys and having none SHALL be refused as a whole with a message naming the setting.

#### Scenario: Listen address changed
- **WHEN** the new file has a different `listen`
- **THEN** nothing is swapped and the message says a restart is required

### Requirement: State survives a reload
A reload SHALL keep the health state of endpoints whose target, provider, model, base URL and health settings are unchanged, the budget counters, and the session state of routes whose definition is unchanged.

#### Scenario: Cooling endpoint stays cooling
- **WHEN** an endpoint is cooling down and an unrelated rule is edited and reloaded
- **THEN** the endpoint is still cooling down

### Requirement: Reload is observable
`GET /v1/health` SHALL report the config fingerprint, when it was loaded, and the last reload attempt's time and outcome.

#### Scenario: Confirmation
- **WHEN** a reload succeeds
- **THEN** the reported fingerprint changes to that of the new file
