# Changelog

## Unreleased

### Changed

* `kq graph` is the live inventory (used / unused / overshadowed). `kq unused`, `kq leftovers`, and `kq export` are gone. `--format` is `lists` | `tree` | `summary`.
* `kq graph` leftover list is one unused ResRef per type: packing copies of a reached script are not unused. NCS mentions come from decompiled NSS (and includes), not opcode names.
* `kq cat` of `.ncs` prints decompiled NSS unless `--disasm` / JSON.
* `kq ls --loaded` and `kq grep --loaded` keep only the copy the game loads. `kq which` marks that copy with `*` and the rest `(overshadowed)`.

### Bug Fixes

* accept `--json` and `--text` before or after every subcommand
* accept `find` as an idiomatic alias for resource discovery with `ls`
* decode padded and unpadded NUL-separated V2.b column headers
* accept `cat --module ROOT` and resolve the module's highest-precedence copy
* accept `cat --module ROOT --tag TAG --type EXT` for typed placed GIT objects
* accept standalone resource/archive paths in `cat --from` without installation discovery

## [0.2.0](https://github.com/holowan-biolabs/kq/compare/v0.1.0...v0.2.0) (2026-09-28)


### Features

* **cli:** default NCS projection to DeNCS NSS with --disasm ([0835a7f](https://github.com/holowan-biolabs/kq/commit/0835a7f76d50f8e5be4139967c15a847cfb283ad))
* **cli:** ls --loaded and which overshadowed labels ([004dde1](https://github.com/holowan-biolabs/kq/commit/004dde11a0201231b8a445a735c801f651d3d69a))
* **graph:** record missing ResRefs and accept kq graph NAME ([8071d23](https://github.com/holowan-biolabs/kq/commit/8071d235547ae57f9faf9ac8dc1796372cf83865))
* **graph:** used unused overshadowed inventory; drop unused leftovers export ([b839871](https://github.com/holowan-biolabs/kq/commit/b839871a3f376adbd8190a8dcfd40cadc8ce11d2))
* **live:** add module Scope and scoped_winners ([dd3e867](https://github.com/holowan-biolabs/kq/commit/dd3e867f2ca6fd26200728825d14659f6e09b1a6))


### Bug Fixes

* **cli:** default unused to scoped winners; add --shadowed ([6e4a7a9](https://github.com/holowan-biolabs/kq/commit/6e4a7a99f86f17f6163959478e1ad2d28d9483a7))
* **cli:** exclude override-shadowed copies from default candidates ([3d98df0](https://github.com/holowan-biolabs/kq/commit/3d98df06617e6d155b198d0bde554d9049aff3a9))
* **cli:** resolve Aspyr K2 installs via steamassets/ ([de41564](https://github.com/holowan-biolabs/kq/commit/de41564933526d7a1db7faf9f2c081728df68da5))
* **graph:** default inventory text unless --json ([8369beb](https://github.com/holowan-biolabs/kq/commit/8369beb803d76c57ef8407596447e1482e53bba8))
* **graph:** use inventory helpers in production builds ([3812c12](https://github.com/holowan-biolabs/kq/commit/3812c12875a31f42f3936823b1f5f196f26cae85))
* **live:** build module_entries from scoped winners ([701ccf6](https://github.com/holowan-biolabs/kq/commit/701ccf6d5fd48d9b0691187daa3b3d43b1c829b4))
* **live:** classify mentions against the referrer's scope ([279eefd](https://github.com/holowan-biolabs/kq/commit/279eefd98070680a46e181d3c17a95ac0e56572c))
* **live:** resolve BFS mentions in module scope ([24a24c6](https://github.com/holowan-biolabs/kq/commit/24a24c6c050ed1dd05f0aa6ef5c3f6bbf0d9f087))
* **live:** seed hen dialogue/retreat and CONSTS-only NCS mentions ([4b12a18](https://github.com/holowan-biolabs/kq/commit/4b12a18aa3115ed9e02163947ba2a7f23170b7a6))
* **live:** stop key/nss/numeric false-positive mentions ([26e3eb0](https://github.com/holowan-biolabs/kq/commit/26e3eb01fe7a0c593ab35aaaba39e3a90af15365))
* **live:** synthetic ARE→lyt/vis/pth edges ([0c4f601](https://github.com/holowan-biolabs/kq/commit/0c4f6018e6890a16c981e0f1dd1c2f6bb654cf3a))


### Performance Improvements

* **live:** extract MDL texture names without MDX/JSON ([222c67e](https://github.com/holowan-biolabs/kq/commit/222c67ef40b5343ae87ed3ab5b7c457361f9f9b6))
* **live:** scan archives sequentially via mmap by file ([224aa98](https://github.com/holowan-biolabs/kq/commit/224aa989a42d55c1e4505d957ba3d1455b113562))

## [0.5.0](https://github.com/holowan-biolabs/kq/compare/v0.4.0...v0.5.0) (2026-08-31)


### Features

* read formats through the shared kotor-formats crate, and generate changes.ini ([#1](https://github.com/holowan-biolabs/kq/issues/1)) ([6e65f67](https://github.com/holowan-biolabs/kq/commit/6e65f67bfdee992e9c78f0687b32a46a215b43e4))


### Bug Fixes

* make release-please able to run at all ([#2](https://github.com/holowan-biolabs/kq/issues/2)) ([8cdf88d](https://github.com/holowan-biolabs/kq/commit/8cdf88d930a8f9bf8754f182a6394e70f1f82729))

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
Overshadowed copies are included unless you ask only for the copy the game loads. `kq cat` defaults to
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
