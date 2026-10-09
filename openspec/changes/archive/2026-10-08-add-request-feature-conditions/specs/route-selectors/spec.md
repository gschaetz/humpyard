# Spec Delta

## ADDED Requirements

### Requirement: Content conditions
A rule MAY require `prompt_tokens` within inclusive `min` and `max` bounds of the estimated prompt size, `tools` to be true or false according to whether the request defines tools, and `images` according to whether it carries images. The prompt-size estimate SHALL be computed only when a rule needs it, and an unknown size SHALL NOT match a `prompt_tokens` rule.

#### Scenario: Long prompts
- **WHEN** a rule requires `prompt_tokens` of at least 2000 and the request's estimate is 2500
- **THEN** the rule matches

#### Scenario: Bounds are inclusive
- **WHEN** the estimate equals `min` or `max`
- **THEN** the rule matches

#### Scenario: Tools and images
- **WHEN** a rule requires `tools = true` and the request defines tools, or `images = true` and a message carries an image
- **THEN** the rule matches

### Requirement: Content conditions are explainable and validated
`explain` SHALL accept `prompt_tokens`, `tools` and `images` and report which content condition stopped a rule; configuration SHALL be rejected when a `prompt_tokens` range has neither bound or a `min` above its `max`.

#### Scenario: Reason shown
- **WHEN** explain is asked with `prompt_tokens` 10 against a rule requiring at least 2000
- **THEN** the rule's mismatch reads "prompt_tokens: wanted at least 2000, got 10"

#### Scenario: Invalid range
- **WHEN** `prompt_tokens = { min = 10, max = 5 }`
- **THEN** loading fails saying min is above max
