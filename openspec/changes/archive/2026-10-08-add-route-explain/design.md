# Design

- **Same code, not a copy.** Explain calls `select_route`, `check_budget` and `plan_route`, the
  functions the request path uses, and only adds a trace of every rule (`Selectors::trace`). The
  test suite compares the explanation with real responses across a matrix of keys and headers.
- **Facts come from the body, the key from authentication.** There is no admin credential yet, so
  a caller can only ask about their own key; explaining as another key waits for the admin API.
  The route allowlist is not secret to the key's holder.
- **No side effects.** No provider call, no ledger entry, no budget change.
- **Always 200 for a well-formed question.** A refusal is a *finding*, reported in `outcome`, so
  tooling can ask about many hypothetical requests without handling errors; malformed bodies are
  400 and missing keys 401 as elsewhere.
- Lives in `src/server/explain.rs` to keep `server.rs` from growing further.
