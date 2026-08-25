# kq user guide

This is the end-user manual for `kq`: what it is for, how to point it at a
game, and how each command behaves. The [README](../README.md) is the short
version of the same story.

## Who this is for

Anyone who already has Knights of the Old Republic or The Sith Lords
installed and wants to *look* at that install without extracting archives
by hand:

- finding which copy of a resource the game will load
- reading a creature, dialogue, 2DA, or talk-table entry as text
- searching scripts and dialogue for a tag, a StrRef, or a function name
- piping that text into `rg`, `jq`, or a script

You do not need to know Rust. You do need a KotOR install (or a single
`.mod` / `.utc` / folder of loose files) and a `kq` binary.

## Install the tool

```bash
cargo install --git https://github.com/arrenkaetris/kq --locked
```

Or from a checkout: `cargo build --release` (binary at `target/release/kq`).
Requires [Rust](https://rustup.rs/) 1.82+.

`kq` never writes into the game folder. The only thing it writes is an
optional index cache (see [Caching](#caching)).

## Point it at something

`kq` needs a *target*. That can be:

- a full install (a directory that contains `chitin.key`)
- a standalone capsule (`.erf` / `.mod` / `.sav` / `.rim`)
- a folder of loose resource files
- one resource file (`.utc`, `.ncs`, `.2da`, …)

Resolution order:

1. `-i` / `--install PATH`
2. `KQ_INSTALL`
3. walk upward from the current directory until `chitin.key` is found

```bash
kq -i /path/to/Steam/steamapps/common/swkotor info
export KQ_INSTALL=/path/to/swkotor
kq ls -t dlg -n 5
```

If you `cd` into `Override/` (or any subdirectory of an install) and run
`kq` with no `-i`, it still finds the install root. That is deliberate:
`kq ls` from inside Override should list the *game*, not surprise-index
only that folder.

To index just one folder or file, name it with `-i`:

```bash
kq -i ./Override ls                  # still the whole install (directory inside one)
kq -i ./Override/n_bastila.utc cat n_bastila
kq -i ./modules/danm13.mod ls
```

A missing or bogus path exits with code **4**.

## The mental model

KotOR stores most content as *resources*: a 16-character ResRef plus a
type (`utc`, `dlg`, `2da`, `ncs`, …). Those resources live in:

| Place | Typical path | Wins over |
|-------|----------------|-----------|
| Override | `Override/*.utc` | everything |
| Module `.mod` | `modules/danm13.mod` | the rim trio |
| Module rims | `danm13.rim`, `danm13_s.rim`, `danm13_dlg.erf` | lips / packs / BIFs |
| Lips | `lips/*.mod` | texture packs |
| Texture packs | `texturepacks/swpc_tex_*.erf` | `rims/` |
| Global rims (K1) | `rims/*.rim` | streams |
| Streams | `streamwaves/`, `streammusic/`, … | chitin |
| Base game | `chitin.key` → `data/*.bif` | (nothing; lowest) |
| Talk table | `dialog.tlk` at the install root | (its own name only) |

`kq` indexes every one of those places. When two resources share a name,
`which` shows the whole chain and stars the winner. `ls --winners` and
`grep --winners` keep only that winner.

A *module* is usually `name.rim` + `name_s.rim` + `name_dlg.erf`, or one
`name.mod` that replaces the trio. Filter with `-m danm13`.

## Command recipes

### What is in this install?

```bash
kq info
kq info --by-type
kq info --json | jq '.resources, .modules'
```

### Where is appearance.2da, really?

```bash
kq which appearance.2da
```

`*` is the file the engine will open. The other lines are copies that
never load unless you delete or rename the winner.

### List Bastila-related creatures in Dantooine

```bash
kq ls bastila -t utc -m danm13
```

Bare `bastila` is a substring. `'n_bast*'` is a glob.

### Read a creature

```bash
kq cat n_bastila.utc
kq cat n_bastila.utc -f gron | rg Tag
kq cat n_bastila.utc --json | jq '.Tag'
```

`outline` is for people. `gron` is for `rg` (every line carries its own
address). `json` is for `jq`.

To read a *shadowed* copy, not the winner:

```bash
kq cat appearance.2da --from 'data/2da.bif'
```

`--from` matches the container label `which` prints.

### Search dialogue for a speaker

```bash
kq grep Bastila -t dlg -n 10
```

Hits look like:

```
22aa_zaalb01_01.dlg 22aa_zaalb01_01.dlg.EntryList[10].Speaker = "Bastila"
```

The first column is the resource name; the rest is a self-locating field
path. You can paste that path into a later `jq` query after `--json`.

### Search compiled scripts

```bash
kq grep GetObjectByTag -t ncs
kq grep --ignore-case cdx_il -t ncs
```

NCS is shown as a disassembly: constants, jumps, and engine calls with
names from `nwscript` (`GetObjectByTag` is routine 200). This is **not**
recovered source. It is enough to find a tag or a function.

`--ignore-case` has no short flag. `-i` is `--install` on every command.

### Search texture TXI strings

```bash
kq grep proceduretype -t tpc -l
```

TPC output is header metadata plus the trailing TXI text. Pixels are not
decoded.

### What does the game never reach?

```bash
kq unused --summary
kq unused -t utc
kq leftovers --summary
kq leftovers --what strings -n 40
```

Two commands share one graph:

1. Catalog every ResRef in the install, and every `dialog.tlk` row.
2. Scan GFF, 2DA, NCS, SSF, layouts (not textures or audio) for ResRef
   tokens and Holocron-style StrRefs (`CExoLocString`, 2DA name/desc
   columns, SSF events). NCS `CONSTS` strings count, so a script that
   hardcodes a `.dlg` name will mark it.
3. Seed the graph from names the engine itself opens — talk files, the
   2DAs hardcoded in the exe, K1 `end_m01aa` / Ebon Hawk / Taris, default
   `k_def_*` / `k_hen_*` scripts, TSL `001ebo`, plus `StartingModule=` in
   the ini. **Not** every folder under `modules/`, and **not** `rims/`.
4. Breadth-first walk. Leftovers are the catalog minus that reachable set.

`kq unused` prints leftover resources. `kq leftovers` prints leftover
talk-table rows (the usual “49k strings, which are unused?” question)
and a leftover-resource count. `--what resources` is the same list as
`unused`. `--catalog --json --summary` includes the seed list.

What this is good for: leftover creature/item/placeable templates,
unused dialogue files, scripts nothing live calls, and unused TLK rows
that might be worth restoring.

What it is not: a play-through. `GetObjectByTag("foo" + bar)` will not
mark `foobar` used. Fonts hardcoded only in the TSL exe are not seeded.
Textures, models and audio are **left out** of resource leftovers unless
you pass `--assets`.

`--summary` prints counts. `--json` is one object per leftover string
(or resource), or one summary object with `--summary --json`.

### One archive, no install

```bash
kq -i ~/kotor/modules/danm13.mod info
kq -i ~/kotor/modules/danm13.mod ls -t git
kq -i ~/kotor/modules/danm13.mod cat m13aa.git -f gron | rg Tag
```

## Output formats in more detail

`kq cat` and the projection `kq grep` searches are the same tree:

- **GFF** becomes a JSON object of fields. Localized strings keep
  `{ "strref": N, "substrings": … }`. Vectors stay arrays.
- **2DA** becomes an array of row objects. The row label is `_row`
  because it is not always the numeric index.
- **TLK** becomes an array of `{ strref, text, sound? }` in StrRef order.
- **NCS** becomes `{ declared_size, instructions: [{ offset, op, … }] }`.
  ACTION calls include `routine`, `name`, and `argc`.
- **SSF** is a map of event name → StrRef (`-1` means no sound).
- Plain text formats (`nss`, `lyt`, `vis`, `txi`) are line arrays in
  gron (`file.nss[12] = "…"`) and a single string in JSON.

Gron quotes strings as JSON, so a value that contains a newline cannot
break the one-leaf-per-line rule.

## Filters

`-t`, `-m`, and `-s` are repeatable and combine as AND across kinds
(type AND module AND source) and OR within a kind (`-t utc -t utd`).

Unknown `-t` values are an error (exit 1), not a silent empty list.

`--winners` drops shadowed duplicates after filtering. Use it when you
care about what the game would load, not about every copy.

## Caching

First `info` / `ls` / `which` / `cat` / `grep` on a full install builds
an index (a few seconds on a retail K1 tree). Later runs reuse it.

- Location: `$KQ_CACHE_DIR` if set, otherwise the platform cache dir
  (`~/.cache/kq` on Linux, similar on macOS/Windows).
- Invalidation: schema version + a fingerprint of file names, sizes, and
  modification times. Editing a file in Override invalidates the cache.
  Editing *inside* a BIF without changing its size or mtime will not
  (that is the same trade-off as “don't hash 1.3 GB on every run”).
- `--refresh` rebuilds and **writes** the new index (it used to discard
  the rebuild; it no longer does).
- `--no-cache` neither reads nor writes.
- `kq cache status` / `kq cache clear` inspect or wipe the directory.

Standalone targets skip the cache. One file or one `.mod` is already a
cheap parse.

## Exit codes

| Code | When |
|------|------|
| 0 | The command did what you asked (including “printed the resource”). |
| 1 | I/O or a malformed file the command could not recover from. |
| 2 | Clap rejected the command line. |
| 3 | The query was valid and matched nothing (`ls` empty, `grep` no hits, `which`/`cat` unknown name). |
| 4 | No install / file / folder could be resolved. |

`grep` of a large install can skip individual corrupt resources (they
do not crash the process). Retail K1 ships at least one 2DA PyKotor
itself also refuses; those become per-file errors, not a panic.

## Limits (honest)

- **Read-only.** There is no `kq write`, packer, or compiler.
- **NCS is disassembly.** You will see `CONSTS "cdx_il"` and
  `ACTION GetObjectByTag`, not the original `.nss`.
- **MDL is an inventory.** Node and animation names, not vertices.
- **TPC / WAV are metadata.** No PNG or PCM export.
- **TGA / DDS / MVE / BIK** have no decoder yet.
- **TSL ACTION names** are used as a superset of K1. Shared routine ids
  (0–767) match; TSL-only ids exist only in TSL.

## Environment reference

```
KQ_INSTALL=/path/to/swkotor     # default --install
KQ_CACHE_DIR=/tmp/kqcache       # override cache location
```

## Global flags

```
-i, --install <PATH>    target (also KQ_INSTALL)
    --json              JSON / JSONL instead of text
    --color <WHEN>      auto | always | never
    --no-cache          do not read or write the index cache
    --refresh           rebuild the index and replace the cache
-h, --help
-V, --version
```

These apply to every subcommand. Subcommand flags never reuse `-i`.

## AI assistants

Cursor, Claude Code, Copilot, Gemini CLI, and OpenCode can load the same
contract. See [Using kq with AI agents](ai-agents.md).
