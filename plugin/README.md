# kq agent plugin

Teaches AI agents how to use the [kq](https://github.com/arrenkaetris/kq) CLI
on a Knights of the Old Republic install: correct flags, resolve order, and
decoded-text workflows.

This directory is a **Claude Code** plugin (`.claude-plugin/plugin.json`) and a
**Cursor** plugin (`.cursor-plugin/plugin.json`).

## Contents

| Path | Role |
|------|------|
| `skills/kq-query/` | How to invoke `kq` |
| `skills/kq-formats/` | What `cat`/`grep` print per type |
| `agents/kq-explorer.md` | Subagent that answers from `kq` output |
| `commands/kq.md` | Slash command `/kq` |
| `hooks/hooks.json` | Claude Code SessionStart |
| `hooks/cursor-hooks.json` | Cursor sessionStart |
| `rules/` | Cursor rules bundled with the plugin |

## Install

See [docs/ai-agents.md](../docs/ai-agents.md) in the repo root for Cursor,
Claude Code, Copilot, Gemini CLI, OpenCode, and a local symlink.

Quick Cursor (this machine):

```bash
mkdir -p ~/.cursor/plugins/local
ln -sfn /absolute/path/to/kq/plugin ~/.cursor/plugins/local/kq
```

Quick Claude Code:

```bash
claude --plugin-dir /absolute/path/to/kq/plugin
```

## Requirements

- Python 3 (session hook)
- A `kq` binary on `PATH` or `./target/release/kq` in a checkout
- A KotOR / TSL install (not bundled)
