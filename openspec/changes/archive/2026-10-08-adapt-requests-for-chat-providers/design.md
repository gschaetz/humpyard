# Design

The adaptation lives in the pool's request path (`adapt_for_chat_providers`) because every
endpoint is a chat-completions backend today, so it applies uniformly and before failover (each
attempt reuses the adapted request). It works on the Switchyard IR rather than JSON, so it is
independent of the inbound protocol. Dropping Responses-shaped details loses nothing a chat
provider can use (the summary text survives as the reasoning text). Mapping `developer` to
`system` is the common compatibility choice; mid-conversation developer messages are hoisted to
the instruction list by the decoder, so their position is already not preserved. If a future
backend type accepts these shapes natively, the adaptation moves behind that backend.
