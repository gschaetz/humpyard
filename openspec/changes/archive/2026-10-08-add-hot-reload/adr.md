# ADR Review

## Reviewed

ADR-0003 (config-first keys behind KeyStore): keys now reload with the snapshot; a managed store
would plug in the same way. ADR-0009 (endpoint health in the pool): breakers now survive reloads
by identity. ADR-0011 (selectors): rules reload with the snapshot.

## Outcome

A durable decision on how configuration changes reach a running gateway.

## New ADRs

- 0012 The configuration is an atomically swapped snapshot, reloaded on SIGHUP.
