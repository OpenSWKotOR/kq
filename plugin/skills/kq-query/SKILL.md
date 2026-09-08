---
name: kq-query
description: Run the kq CLI against a KotOR or KotOR II install, capsule, or loose file (kq info/ls/which/cat/grep). Use when the user asks to list, resolve, print, or search game resources, mentions ResRefs, Override, chitin.key, dialog.tlk, modules, or wants rg/jq-style queries over a KotOR tree. Also use when they propose extracting or unpacking chitin/BIFs instead of querying in place.
---

# kq-query

Query a KotOR install **in place**. Do not extract archives.

## Resolve the binary and the target

1. Binary: `kq` on `PATH`, else install with
   `curl -LsSf https://github.com/holowan-biolabs/kq/releases/latest/download/install.sh | sh`
   or use `./target/release/kq` / `./target/debug/kq`.
2. Target, in order: `-i PATH`, `$KQ_INSTALL`, walk-up for `chitin.key`.
3. If neither exists, ask for an install path. Do not invent one.

A directory *inside* an install still means the whole game. A named *file*
(`*.mod`, `*.utc`) is standalone.

## Pick the command

| User intent | Invoke |
|-------------|--------|
| What's here? | `kq info` then `kq info --by-type` if they want counts |
| Find a name | `kq ls <substr-or-glob>` |
| Which file loads? | `kq which <resref>` — `*` is the winner |
| Read it | `kq cat <resref>` |
| Compare two copies | `kq delta A B` / `kq delta RESREF --shadow` |
| Apply a delta | `kq patch TARGET delta.json` |
| Three-way merge | `kq merge BASE OURS THEIRS` |
| Search inside | `kq grep <pattern>` |
| Reachability tree + leftovers | `kq graph` / `kq graph --json` |
| Never reached (resources) | `kq unused -q` / `kq graph --what leftovers -q` |
| Leftover TLK strings | `kq leftovers` / `kq leftovers --what strings` |

Always pass `-i` unless `KQ_INSTALL` is set or cwd is inside the install.

## Flags you may use

**Global:** `-i/--install`, `--json`, `--color`, `--no-cache`, `--refresh`.

**ls / grep filters:** `-t/--type`, `-m/--module`, `-s/--source`, `--winners`.

**ls:** `-n <N>`, `-q`.

**cat:** `-t`, `-f outline|gron|json|raw`, `--raw`, `--from <container-label>`.

**grep:** `--ignore-case` (no `-i`), `-F`, `-l`, `--winners`, `--include-binary`, `-n`.

**graph / unused / leftovers:** `--summary`, `-q`, `-n`, `--assets`, `--depth`. graph also has `--what used|leftovers|both`.

If you need a flag not listed here, run `kq <cmd> --help`. Do not invent flags.

## Agent output defaults

- Parse with `--json` (`ls`/`grep` = JSONL).
- For field-level reading, `cat -f gron` or `cat --json`.
- After `which`, use `--from` with the printed container label to read a shadowed copy.

## Exit codes

`0` ok, `1` runtime, `2` usage, `3` no match, `4` no install, `5` delta differs or merge conflicts. Treat `3` as empty result, not a crash.

## Anti-patterns

- `kq grep -i pattern` — that sets `--install`, not ignore-case.
- Unpacking `data/*.bif` to search. Use `kq grep`.
- Assuming `kq` can write, pack, or decompile NSS.

## More detail

- Flag tables and recipes: [references/cli.md](references/cli.md)
- What each type prints: `../kq-formats/SKILL.md`
