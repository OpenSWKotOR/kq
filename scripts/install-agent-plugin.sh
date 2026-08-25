#!/usr/bin/env bash
# Symlink the kq agent plugin into Cursor's local plugin directory.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
plugin="$root/plugin"
dest_dir="${HOME}/.cursor/plugins/local"
dest="$dest_dir/kq"

if [[ ! -f "$plugin/.cursor-plugin/plugin.json" ]]; then
  echo "install-agent-plugin: missing $plugin/.cursor-plugin/plugin.json" >&2
  exit 1
fi

mkdir -p "$dest_dir"
ln -sfn "$plugin" "$dest"
echo "Cursor local plugin: $dest -> $plugin"
echo "Claude Code: claude --plugin-dir $plugin"
echo "Other hosts: open the repo so AGENTS.md / CLAUDE.md / GEMINI.md load."
