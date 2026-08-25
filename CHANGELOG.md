# Changelog

## 0.3.0

Live mention graph and its inverse. `kq unused` lists leftover resources
reachable from engine-hardcoded seeds (not every module folder, not
`rims/`). `kq leftovers` catalogs every ResRef and `dialog.tlk` row, then
prints what that graph never reaches — unused talk-table strings by
default.

Isolated A↔B pairs stay unused. NCS `CONSTS` strings count. Texture and
audio files stay out of resource leftovers unless `--assets`.

## 0.2.0

Agent integration for Cursor, Claude Code, GitHub Copilot, Gemini CLI,
OpenCode, and Windsurf: `AGENTS.md`, host adapters, and a dual
`.claude-plugin` / `.cursor-plugin` under `plugin/`. Skills (`kq-query`,
`kq-formats`), `kq-explorer` subagent, `/kq` command, session hooks, and
`docs/ai-agents.md` install notes.

## 0.1.0

First public release: KEY/BIF/ERF/RIM index, GFF/2DA/TLK plus additional
text projections, `info`/`ls`/`which`/`cat`/`grep`/`cache`, and the
end-user manual.
