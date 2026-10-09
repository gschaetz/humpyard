# Design

- **A pure feature record.** `Features { prompt_tokens, tools, images }` is built by the server
  from the decoded request and passed to the matcher with the other facts; the matcher stays free
  of Switchyard and HTTP types.
- **The estimate is the gateway's own** (`estimate_tokens` over the chat form of the request), so
  rules and `count_tokens` agree and a client can compute the number a rule will see. It errs
  high; thresholds should have headroom.
- **Lazy.** Encoding the request to estimate it costs a pass over the prompt, so it runs only if a
  rule mentions `prompt_tokens`.
- **Unknown never matches.** `explain` without `prompt_tokens` reports that the size is not known
  instead of guessing.
- Sizes are about the request as received; they do not include the model's reply (`max_tokens`).
