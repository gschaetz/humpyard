# Tasks

## 1. Configuration

- [x] 1.1 `tool_call_extra_content` on a provider: schema, `Provider`, validation (must be a table)

## 2. Request path

- [x] 2.1 Add the content to replayed assistant tool calls per attempt, after `drop_fields`

## 3. Tests and docs

- [x] 3.1 `tests/tool_call_extra_content.rs`: Responses replay, a provider without the setting, no tool call, existing value kept
- [x] 3.2 Config validation unit tests
- [x] 3.3 Document the setting in docs/configuration.md, docs/providers.md and the Codex section of docs/clients.md
- [x] 3.4 Verify docs/architecture.md still matches (request flow unchanged; expect no edit)
