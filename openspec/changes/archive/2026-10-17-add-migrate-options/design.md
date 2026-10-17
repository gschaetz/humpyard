# Design

## Decisions

- **Rename the provider, derive the rest.** The environment variable and the target prefix come from the
  provider name today, so renaming the name renames all three consistently and the fragment still loads on its
  own. A rename that collides with another generated provider, or is not a valid bare name, is an error before
  anything is printed.
- **The weight is written, not implied.** `quality_weight` appears as a line on each `fastest` route, so the
  output says what the route does and an operator can read or change it. Absent the flag, nothing is written
  and routes keep the pure speed order.
