#!/bin/sh
# Installs (or reinstalls) humpyard as a per-user launchd service on macOS.
# Prerequisites: the binary at ~/.local/bin/humpyard, a config at ~/.config/humpyard/config.toml
# and provider keys in ~/.config/humpyard/env (chmod 600). See docs/deployment.md.
set -eu

label=dev.humpyard.gateway
dir=$(cd "$(dirname "$0")" && pwd)
target="$HOME/Library/LaunchAgents/$label.plist"

for f in "$HOME/.local/bin/humpyard" "$HOME/.config/humpyard/config.toml" "$HOME/.config/humpyard/env"; do
  [ -e "$f" ] || { echo "missing: $f" >&2; exit 1; }
done

mkdir -p "$HOME/Library/LaunchAgents" "$HOME/Library/Logs"
sed "s#__HOME__#$HOME#g" "$dir/$label.plist" > "$target"
plutil -lint "$target" >/dev/null

launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" "$target"
echo "installed $label; logs: ~/Library/Logs/humpyard.log"
echo "stop it with: launchctl bootout gui/$(id -u)/$label"
