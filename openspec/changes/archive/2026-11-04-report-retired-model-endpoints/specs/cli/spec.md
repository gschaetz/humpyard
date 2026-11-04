## ADDED Requirements

### Requirement: models check --retired
The CLI SHALL accept `humpyard models check --retired` which, for each provider in the configuration (or the one named by `--provider`) that has an API key available, fetches that provider's own `/models` list and reports every configured endpoint, hand-written or generated, whose model that list does not include, naming the target, provider and model. It SHALL NOT require `discover` or `price_source`, SHALL NOT change any file, SHALL send a provider's key only to that provider's own host, and SHALL treat the response as untrusted data with a size and time limit. It SHALL exit 1 when it reports at least one endpoint, 0 when it reports none and 2 when a provider's list could not be used, and in the latter case SHALL still check the other providers.

#### Scenario: A model the provider stopped listing
- **WHEN** a target's model is not in its provider's `/models` list
- **THEN** the report names the target, provider and model and the exit code is 1

#### Scenario: Every model is listed
- **WHEN** every configured endpoint's model is in its provider's list
- **THEN** nothing is reported and the exit code is 0

#### Scenario: A provider whose list cannot be fetched
- **WHEN** one provider's `/models` request fails or returns something that is not a model list
- **THEN** that provider is reported as unusable, the other providers are still checked, and the exit code is 2

#### Scenario: No discover setting needed
- **WHEN** a provider has no `discover` and no `price_source`
- **THEN** its endpoints are still checked

#### Scenario: Nothing is changed
- **WHEN** the check reports retired endpoints
- **THEN** no configuration or generated file is modified
