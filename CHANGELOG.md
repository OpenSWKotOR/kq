# Changelog

## 0.3.3

`kq graph` — live mention hierarchy and leftovers in one report. Text mode
shows seeds, an ASCII tree of reachable resources (with ResRef edges), then
every leftover path and talk-table string. JSON mode returns a nested `tree`,
`used_by_module`, and `leftovers` arrays.

Structured JSON improvements: `kq cat --json` wraps decoded content in a
resource envelope (`path`, `source`, `module`, `content`). GFF JSON includes
`_file_type` and `_version` (PyKotor-style metadata on nested trees).

## 0.3.2

Install-relative paths everywhere: archives (`.mod` / `.rim` / `.erf` /
`.bif` / …) display as folders (`modules/end_m01aa.mod/m01aa.git`).

Live-graph fixes: nodes are resource ids (not shared ResRef strings),
modules are entered via `module.ifo` / area GIT (not a fake
`end_m01aa` ResRef), and composite rim/erf trios share one module root.

`kq unused -q` prints every leftover path, one per line — the pipe-friendly
list of resources the live graph never reaches.

## 0.3.1

Standalone installer one-liners (`curl | sh`, `wget`, `irm | iex`), matching
the uv-style quickstart. Linux x86_64 downloads a release binary; other
platforms fall back to `cargo install --git`.

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
