## MODIFIED Requirements

### Requirement: models check --retired
The CLI SHALL accept `humpyard models check --retired` which, for each provider in the configuration (or the one named by `--provider`) that has an API key available, fetches that provider's own `/models` list and reports every configured endpoint, hand-written or generated, whose model that list does not include, naming the target, provider and model. It SHALL NOT require `discover` or `price_source`, SHALL NOT change any file, SHALL send a provider's key only to that provider's own host, and SHALL treat the response as untrusted data with a size and time limit. It SHALL tell apart three ways a provider's list can be missing: a provider whose `/models` answers 404, 405 or 501 has no model list and SHALL be reported as having none and skipped; a provider the check could not reach (the connection fails or times out, or the answer is 502, 503 or 504) SHALL be reported as unreachable; a provider that answered with something unusable (a rejected key, any other error status, a body that is not a model list, an empty list) SHALL be reported as unusable. It SHALL exit 1 when it reports at least one retired endpoint, 0 when it reports none and 2 when a provider is unusable; a provider with no list or one that is unreachable SHALL NOT change the exit code, and in every case the other providers SHALL still be checked.

#### Scenario: A model the provider stopped listing
- **WHEN** a target's model is not in its provider's `/models` list
- **THEN** the report names the target, provider and model and the exit code is 1

#### Scenario: Every model is listed
- **WHEN** every configured endpoint's model is in its provider's list
- **THEN** nothing is reported and the exit code is 0

#### Scenario: A provider whose list cannot be fetched
- **WHEN** one provider rejects the key, answers with an error other than the no-list and unreachable cases below, or returns something that is not a model list
- **THEN** that provider is reported as unusable, the other providers are still checked, and the exit code is 2

#### Scenario: A provider without a model list
- **WHEN** a provider's `/models` answers 404
- **THEN** it is reported as having no model list, its endpoints are not compared, the other providers are still checked and the exit code is not affected

#### Scenario: A provider that cannot be reached
- **WHEN** the connection to a provider fails, times out or is answered with 503
- **THEN** it is reported as unreachable, the other providers are still checked and the exit code is not affected

#### Scenario: Unreachable does not hide a retired endpoint
- **WHEN** one provider is unreachable and another lists none of a target's models
- **THEN** the retired endpoint is reported and the exit code is 1

#### Scenario: No discover setting needed
- **WHEN** a provider has no `discover` and no `price_source`
- **THEN** its endpoints are still checked

#### Scenario: Nothing is changed
- **WHEN** the check reports retired endpoints
- **THEN** no configuration or generated file is modified
