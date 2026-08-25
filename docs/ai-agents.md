# Using kq with AI agents

`kq` ships an agent contract so Cursor, Claude Code, GitHub Copilot, Gemini
CLI, OpenCode, and similar tools query a KotOR install instead of extracting
archives or inventing flags.

Canonical contract: [`AGENTS.md`](../AGENTS.md) at the repo root.
Installable plugin: [`plugin/`](../plugin/).

## What agents get

- **Rules** — always-on CLI contract (`-i` is install, not ignore-case)
- **Skills** — `kq-query` (how to run the CLI), `kq-formats` (projections)
- **Subagent** — `kq-explorer` for install questions
- **Slash command** — `/kq` (Claude Code plugin)
- **Session hook** — injects the contract at conversation start

## Cursor

Opening this repository is enough. Cursor loads:

- `.cursor/rules/*.mdc`
- `.cursor/skills/`
- `.cursor/agents/`
- `.cursor/hooks.json`

To use the same plugin in **other** Cursor workspaces:

```bash
mkdir -p ~/.cursor/plugins/local
ln -sfn "$(pwd)/plugin" ~/.cursor/plugins/local/kq
```

Then enable **kq** under Cursor Settings → Plugins if it does not appear
automatically. Restart Cursor if hooks do not fire.

Say things like: “which appearance.2da loads?”, “grep Bastila in dlg”,
“cat n_bastila as JSON”.

## Claude Code

From a checkout:

```bash
claude --plugin-dir "$(pwd)/plugin"
```

Or add a local marketplace / `/plugin install` pointing at `./plugin`.
`CLAUDE.md` imports `AGENTS.md`. After the plugin loads, `/kq` and the
`kq-query` skill are available.

```bash
# optional: project-scoped
# claude --plugin-dir /path/to/kq/plugin
```

## GitHub Copilot (VS Code / github.com)

Copilot reads:

- `.github/copilot-instructions.md`
- `.github/instructions/kq.instructions.md` (`applyTo: **`)

No extra install. In VS Code, keep the repo open so workspace instructions
apply.

## Gemini CLI

Gemini CLI reads `GEMINI.md`, which points at `AGENTS.md`. Clone the repo
and run Gemini from the project root (or add `GEMINI.md` to the files the
CLI includes).

## OpenCode and other AGENTS.md clients

OpenCode, Codex, and similar tools load `AGENTS.md` automatically when the
project root is the workspace. No plugin install required.

## Windsurf / Cascade

A short [`.windsurfrules`](../.windsurfrules) file points at `AGENTS.md`.

## Aider and generic assistants

Point the tool at `AGENTS.md` (Aider: `--read AGENTS.md`, or add it to the
convention file). The same contract applies.

## Local install script

From the repo root:

```bash
./scripts/install-agent-plugin.sh
```

This symlinks `plugin/` to `~/.cursor/plugins/local/kq` and prints Claude
Code / Copilot notes. It does not copy KotOR game files.

## Verify

```bash
# hook (Cursor-style stdin JSON)
echo '{}' | python3 plugin/hooks/scripts/session-context.py

# hook (Claude-style empty stdin)
python3 plugin/hooks/scripts/session-context.py </dev/null

# CLI still works
kq --help
```

Expect JSON containing the kq contract and, if `kq` is on `PATH`, its path.

## Prompting tips

Be specific about the **target** and the **question**:

> Using kq on `$KQ_INSTALL`, which copy of `appearance.2da` loads, then print
> the row for appearance 8 as gron.

Avoid: “search the bif files for Bastila” (that implies extract). Prefer:
“`kq grep Bastila -t dlg`”.
