# Tasks

## 1. Configuration

- [x] 1.1 `drop_fields` on a provider: schema, `Provider`, and validation (no empty name)

## 2. Request path

- [x] 2.1 Remove the listed fields from each attempt's request, including preserved same-format bodies
- [x] 2.2 Read an array-shaped provider error in `upstream_message`

## 3. Tests and docs

- [x] 3.1 `tests/drop_fields.rs`: Responses request, chat request, a provider without the setting, failover between the two
- [x] 3.2 Unit tests for the error array and config validation
- [x] 3.3 Document `drop_fields` in docs/configuration.md and the Google AI setting in docs/providers.md; add the Codex finding to docs/clients.md
- [x] 3.4 Verify docs/architecture.md still matches (request flow is unchanged; expect no edit)
