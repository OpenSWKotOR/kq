# Changelog

## 0.3.5

Full MDL/MDX model IR (ASCII + Odyssey binary + JSON) replaces the names-only
decoder. Binary MDL pairs companion MDX for vertex data. `kq export` dumps an
install as a JSON tree. 2DA salvage reads NUL-separated shadowed copies
(`rims/global.rim`). `kq delta` / `kq patch` / `kq merge` compare and combine
decoded resources (JSON out; game files stay untouched). Exit 5 means a
delta found changes or a merge still has conflicts.

## 0.3.4

JSON is the default output format (`--text` for human-readable views). No
truncation in graph/leftover reports: full reachability tree, complete
`catalog` with `status`/`mentions`/`parent_path` on every resource, all
talk-table rows, and all resource types included unless `--no-assets`.
Shadowed copies are included unless `--winners-only`. `kq cat` defaults to
nested JSON with a resource envelope and full GFF content.

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
