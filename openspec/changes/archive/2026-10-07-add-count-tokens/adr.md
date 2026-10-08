# ADR Review

## Reviewed

ADR-0001 (Switchyard in-process) and ADR-0002 (dispatch on Switchyard's client; providers speak
OpenAI Chat), both in force and unaffected: this endpoint adds no dispatch path. ADR-0004 to
ADR-0008 scanned; nothing relevant.

## Outcome

No major durable architectural decision was introduced. Estimating locally rather than probing is
a tactical choice recorded in design.md and reversible by adding an option.

## New ADRs

None.
