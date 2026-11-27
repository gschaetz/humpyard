# ADR review

ADR review completed. Reviewed in force: 0004 (no plaintext secrets), 0025 (the UI has no state of its own), 0027 (admin credentials), 0032 (the GUI runs as its own process), 0036 (admin and UI work must not change the request path's performance). None is superseded by this change; 0004 and 0025 are refined, and the design stays within 0032 and 0036.

New ADR:

- [0039](../../../docs/adr/0039-a-local-key-file-is-read-only-by-up-and-never-shown-again.md): a local key file is read only by `humpyard up` and never shown again. It qualifies because it is the first stored secret, a durable boundary (one reader, write-only, loopback only) that later changes must not loosen without a new decision, and it diverges from ADR 0004's "environment only" for this one case.

Not an ADR (tactical, in design.md): the supervisor shape (`up` serves the GUI and runs `serve` as a child), the state directory layout, the per-run admin credential, the restart on a key change, and the first-run screens.
