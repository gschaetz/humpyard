#!/bin/sh
# Compares the gateway's throughput with the previous release's, measured alongside it (ADR 0036).
#
# One run compared with a figure written down on another day cannot resolve a 10% tolerance: the same binary varies
# by 7 to 10% between its own runs (docs/performance.md, Run 3). So this builds the previous release's overhead
# harness in a scratch directory and alternates it with the current one for several pairs in one session, reading
# the gateway's requests per second at 64 concurrent requests against an instant upstream from each run's table.
# The order alternates between pairs, so a machine that slows down or warms up does not favour either build.
#
# Usage:
#   scripts/compare-throughput.sh <previous tag or commit>
#   scripts/compare-throughput.sh --numbers OLD1,NEW1,OLD2,NEW2,OLD3,NEW3   # judge given figures, run nothing
#
# Environment:
#   PAIRS                   pairs to run (default 3, at least 3)
#   OVERHEAD_REQUESTS       passed to the harness (requests per instant-upstream cell; its default is 5,000)
#   HUMPYARD_BUDGET_ENFORCE=1   exit 1 unless the verdict is OK
#
# The verdict:
#   REGRESSION    the median of current/previous is below 0.90
#   INCONCLUSIVE  one build's own runs differ by more than 10%, so the machine is too noisy to decide
#   OK            neither
# Without HUMPYARD_BUDGET_ENFORCE=1 the verdict is printed and the exit status is 0.
set -eu

LIMIT_RATIO=0.90
LIMIT_SPREAD=0.10
MIN_PAIRS=3

usage() {
  sed -n '2,/^set -eu/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//' >&2
  exit 2
}

# Reads "previous current" pairs, one per line; prints a table and the verdict; exits 1 on a verdict other than OK
# when enforcing.
judge() {
  awk -v limit_ratio="$LIMIT_RATIO" -v limit_spread="$LIMIT_SPREAD" -v enforce="${HUMPYARD_BUDGET_ENFORCE:-0}" '
    function median(a, n,    i, j, t, b) {
      for (i = 1; i <= n; i++) b[i] = a[i]
      for (i = 2; i <= n; i++) { t = b[i]; for (j = i - 1; j >= 1 && b[j] > t; j--) b[j + 1] = b[j]; b[j + 1] = t }
      return (n % 2) ? b[(n + 1) / 2] : (b[n / 2] + b[n / 2 + 1]) / 2
    }
    function spread(a, n,    i, lo, hi) {
      lo = hi = a[1]
      for (i = 2; i <= n; i++) { if (a[i] < lo) lo = a[i]; if (a[i] > hi) hi = a[i] }
      return (hi - lo) / hi
    }
    { n++; old[n] = $1; cur[n] = $2; ratio[n] = $2 / $1 }
    END {
      printf "%-6s %12s %12s %10s\n", "pair", "previous", "current", "ratio"
      for (i = 1; i <= n; i++) printf "%-6d %12.0f %12.0f %10.3f\n", i, old[i], cur[i], ratio[i]
      m = median(ratio, n); so = spread(old, n); sc = spread(cur, n)
      printf "\nmedian ratio %.3f (at least %.2f)\n", m, limit_ratio
      printf "spread within one build: previous %.1f%%, current %.1f%% (at most %.0f%%)\n", so * 100, sc * 100, limit_spread * 100
      regression = (m < limit_ratio); noisy = (so > limit_spread || sc > limit_spread)
      if (regression && noisy) verdict = "REGRESSION (and the runs are noisy)"
      else if (regression) verdict = "REGRESSION"
      else if (noisy) verdict = "INCONCLUSIVE"
      else verdict = "OK"
      printf "verdict: %s\n", verdict
      if (verdict != "OK" && enforce == "1") exit 1
    }'
}

if [ "${1:-}" = "--numbers" ]; then
  [ $# -eq 2 ] || usage
  pairs=$(printf '%s' "$2" | awk -F, -v min="$MIN_PAIRS" '
    NF % 2 == 1 { print "error: --numbers takes previous,current pairs, so an even count of figures" > "/dev/stderr"; exit 2 }
    NF / 2 < min { print "error: at least " min " pairs are needed" > "/dev/stderr"; exit 2 }
    { for (i = 1; i <= NF; i += 2) print $i, $(i + 1) }')
  printf '%s\n' "$pairs" | judge
  exit $?
fi

[ $# -eq 1 ] || usage
previous="$1"
PAIRS="${PAIRS:-3}"
if [ "$PAIRS" -lt "$MIN_PAIRS" ]; then
  echo "error: PAIRS must be at least $MIN_PAIRS" >&2
  exit 2
fi

repo="$(cd "$(dirname "$0")/.." && pwd)"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT INT TERM

# The path of the overhead harness executable a build produced.
overhead_exe() {
  grep -o '"executable":"[^"]*overhead-[^"]*"' | tail -1 | cut -d'"' -f4
}

echo "building the previous release's harness ($previous) in a scratch directory..." >&2
mkdir "$scratch/previous"
git -C "$repo" archive "$previous" | tar -x -C "$scratch/previous"
if [ ! -f "$scratch/previous/tests/overhead.rs" ]; then
  echo "error: $previous has no overhead harness (tests/overhead.rs was added after 0.9.5)" >&2
  exit 2
fi
previous_exe="$(cd "$scratch/previous" && CARGO_TARGET_DIR="$scratch/target" \
  cargo test --release --test overhead --no-run --message-format=json 2>/dev/null | overhead_exe)"
echo "building the current harness..." >&2
current_exe="$(cd "$repo" && cargo test --release --test overhead --no-run --message-format=json 2>/dev/null | overhead_exe)"
if [ -z "$previous_exe" ] || [ -z "$current_exe" ]; then
  echo "error: could not build an overhead harness (previous: '$previous_exe', current: '$current_exe')" >&2
  exit 2
fi

# One harness run; prints the gateway's requests per second at concurrency 64 against the instant upstream.
measure() {
  "$1" --ignored --nocapture 2>&1 | awk '/^instant +64 +gateway/ { print $7; found = 1 } END { exit found ? 0 : 1 }' || {
    echo "error: $1 printed no instant-upstream concurrency-64 gateway row" >&2
    exit 2
  }
}

pairs_file="$scratch/pairs"
: >"$pairs_file"
i=1
while [ "$i" -le "$PAIRS" ]; do
  if [ $((i % 2)) -eq 1 ]; then
    echo "pair $i: previous, then current..." >&2
    old="$(measure "$previous_exe")"
    new="$(measure "$current_exe")"
  else
    echo "pair $i: current, then previous..." >&2
    new="$(measure "$current_exe")"
    old="$(measure "$previous_exe")"
  fi
  echo "$old $new" >>"$pairs_file"
  i=$((i + 1))
done

echo >&2
echo "gateway requests per second, 64 concurrent requests, instant upstream: $previous against the current build"
judge <"$pairs_file"
