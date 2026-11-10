#!/bin/sh
# Smoke test for a build that must carry the admin GUI (ADR 0032): runs `humpyard ui` and fails when it serves the
# placeholder page instead of the GUI, or when the page's script or the security headers are missing.
# Usage: scripts/smoke-ui.sh path/to/humpyard
set -eu

bin="${1:?usage: smoke-ui.sh path/to/humpyard}"
dir="$(mktemp -d)"
pid=""
cleanup() {
  [ -z "$pid" ] || kill "$pid" 2>/dev/null || true
  rm -rf "$dir"
}
trap cleanup EXIT

# Port 0 lets the system pick a free port; the command prints the address it bound.
"$bin" ui --url http://127.0.0.1:9 --listen 127.0.0.1:0 >"$dir/out" 2>&1 &
pid=$!

addr=""
for _ in $(seq 1 50); do
  addr="$(sed -n 's|^humpyard ui: the admin GUI is at http://||p' "$dir/out" | head -1)"
  [ -n "$addr" ] && break
  kill -0 "$pid" 2>/dev/null || { echo "humpyard ui exited early:"; cat "$dir/out"; exit 1; }
  sleep 0.2
done
[ -n "$addr" ] || { echo "humpyard ui did not start:"; cat "$dir/out"; exit 1; }

fail() { echo "smoke-ui: $1"; exit 1; }

if grep -q "no GUI files" "$dir/out"; then fail "the binary reports it has no GUI files"; fi

curl -fsS -D "$dir/headers" -o "$dir/index.html" "http://$addr/" || fail "GET / failed"
if grep -q "not in this build" "$dir/index.html"; then fail "GET / served the placeholder page"; fi
grep -q 'id="app"' "$dir/index.html" || fail "GET / is not the GUI page"
grep -qi '^content-security-policy:' "$dir/headers" || fail "GET / has no Content-Security-Policy"

script="$(sed -n 's|.*src="\(/assets/[^"]*\.js\)".*|\1|p' "$dir/index.html" | head -1)"
[ -n "$script" ] || fail "the page references no script under /assets/"
curl -fsS -D "$dir/script-headers" -o /dev/null "http://$addr$script" || fail "GET $script failed"
grep -qi '^content-type: text/javascript' "$dir/script-headers" || fail "$script is not served as JavaScript"

echo "smoke-ui: ok ($script)"
