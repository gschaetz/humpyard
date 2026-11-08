# ADR Review

## Reviewed

- ADR-0025 The UI has no state of its own (this change is the patch half of its first rule)
- ADR-0012 The configuration is an atomically swapped snapshot, reloaded on SIGHUP (the check reuses the reload path)
- ADR-0022 Free-pool membership may be updated on a schedule without review (the generated file stays out of reach)
- ADR-0027 Admin authority is granted by configuration to named, scoped principals (the new scope follows it)
- ADR-0004 No plaintext secrets in config, logs or the ledger (hashes and header values are masked in answers)
- ADR-0032 The admin GUI runs as its own process from the same binary (the GUI will be a client of this endpoint)

## Outcome

One durable decision qualifies: what a configuration patch is made of, what it may touch, how it is validated, and
that producing one writes nothing. It is a contract the GUI, the direct write and database keys all build on, and
it is not covered by ADR 0025, which fixes only the principle.

## New ADRs

- [ADR-0034 A configuration patch is typed edits, checked by the reload path, and written by no one](../../../docs/adr/0034-a-configuration-patch-is-typed-edits-checked-by-the-reload-path-and-written-by-no-one.md) (accepted)
