# Tasks

## 1. Endpoint

- [ ] 1.1 Make the counting handler protocol-aware and add `POST /v1/responses/input_tokens` with the OpenAI success shape and error format; tests: shape and header, more input counts more, 401 and 403 and 404 in the OpenAI error format, 400 for `previous_response_id`, bad JSON
- [ ] 1.2 Test that Responses counting makes no provider call, writes no ledger entry and works for a key over budget

## 2. Docs

- [ ] 2.1 Update `docs/clients.md` (the Codex section: `input_tokens` is now answered, `compact` is intentionally not implemented and why), the README endpoint list and `docs/architecture.md`; verify the claims match the code
