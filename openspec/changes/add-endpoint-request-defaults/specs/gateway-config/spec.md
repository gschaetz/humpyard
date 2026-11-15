# Spec Delta

## ADDED Requirements

### Requirement: An endpoint may declare request defaults
An endpoint MAY declare `request_defaults`, a table of request fields to add to a request sent to that endpoint when the request does not carry the field. An empty table SHALL be the same as none.

#### Scenario: A declared default
- **WHEN** an endpoint declares `request_defaults = { reasoning_effort = "none", temperature = 0.7 }`
- **THEN** the configuration loads and the effective configuration shows both fields on that endpoint

#### Scenario: Nothing declared
- **WHEN** an endpoint declares no `request_defaults`, or an empty table
- **THEN** requests to it are sent exactly as before

### Requirement: Only a short list of fields may be defaulted, each checked
For an endpoint of an OpenAI-compatible provider, `request_defaults` SHALL accept only `reasoning_effort`, `temperature`, `top_p`, `top_k`, `min_p`, `presence_penalty` and `frequency_penalty`, each with a value of its own type and range. Anything else SHALL be refused when the configuration loads and by `check-config`, naming the endpoint and the field.

#### Scenario: The ranges
- **WHEN** an endpoint declares `reasoning_effort` as a non-empty string, `temperature` from 0 to 2, `top_p` and `min_p` from 0 to 1, `top_k` as a whole number of at least 1, and `presence_penalty` and `frequency_penalty` from -2 to 2
- **THEN** the configuration loads

#### Scenario: An unknown field is refused
- **WHEN** an endpoint declares `request_defaults = { max_tokens = 100 }`
- **THEN** the configuration is refused, the message names the endpoint and `max_tokens`, and says which names are allowed

#### Scenario: A value out of range is refused
- **WHEN** an endpoint declares `request_defaults = { temperature = 5 }` or `{ top_k = 0 }` or `{ top_p = "high" }`
- **THEN** the configuration is refused and the message names the endpoint, the field and the allowed range or type

### Requirement: The list of defaultable fields belongs to the endpoint's protocol
The names allowed in `request_defaults` SHALL depend on the protocol the endpoint speaks. OpenAI chat is the only protocol an endpoint speaks today. An endpoint of another protocol added later SHALL declare its own names, and no name SHALL be assumed to carry over from another protocol's list.

#### Scenario: One protocol today
- **WHEN** any endpoint declares `request_defaults` in the current release
- **THEN** it is checked against the OpenAI chat list

### Requirement: The effective configuration shows request defaults
`GET /v1/config` and the admin configuration view SHALL report each endpoint's `request_defaults` when it has any, and SHALL omit the field when it has none. A reload SHALL apply an added, changed or removed `request_defaults` to the requests that start after it.

#### Scenario: Shown for the endpoint that has them
- **WHEN** one endpoint of a target declares defaults and another does not
- **THEN** the view shows the defaults on the first and no `request_defaults` field on the second

#### Scenario: Changed by a reload
- **WHEN** the file's `temperature` default changes from 0.7 to 0.5 and the gateway reloads
- **THEN** a request that starts after the reload is sent with 0.5
