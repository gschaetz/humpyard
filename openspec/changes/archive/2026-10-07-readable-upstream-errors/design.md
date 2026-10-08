# Design

Pure function change in `upstream_message`: recognized JSON shapes first, then a sanitized
fallback (whitespace collapsed, 200 characters, markup or JSON-looking text replaced by the
status). Status codes and failover behavior are unchanged.
