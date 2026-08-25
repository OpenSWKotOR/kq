# kq

**Query a Knights of the Old Republic installation like it was plain text.**

`kq` is a command-line tool that treats a KotOR (or KotOR II) game folder as
structured, searchable data. It reads the game's archives in place — you do
not extract anything first — and answers questions the way `rg` answers
questions about a source tree and `jq` answers questions about JSON.

```
$ kq -i ~/kotor which appearance.2da
* appearance.2da         override      Override                     103273 bytes
  appearance.2da         rims          global.rim                   98610 bytes
  appearance.2da         chitin        data/2da.bif                 98610 bytes
```

The starred line is the copy the game actually loads. Everything below it is
shadowed. That question is what KotOR modding gets wrong most often; `kq which`
answers it directly.

Full walkthrough, recipes, and format notes: **[User guide](docs/user-guide.md)**.

---

## What this is

A KotOR install is not a folder of files you can grep. Creature templates live
inside `.mod` archives, dialogue lives in GFF trees, strings live in
`dialog.tlk`, scripts are compiled bytecode, and the same name can exist in
five places with only one of them loading.

`kq` walks that layout, builds an index of every resource, and decodes the
formats it understands into text:

- **list** what the install contains (`kq ls`)
- **resolve** which copy of a name the game would load (`kq which`)
- **print** a resource as a readable tree, as `path = value` lines, or as JSON
  (`kq cat`)
- **search** decoded contents with a regex (`kq grep`)

It is not a save editor, a compiler, a GUI, or a replacement for the Holocron
Toolset. It is the `rg`/`jq` of a KotOR install.

---

## Install

You need a Rust toolchain **1.82 or newer** ([rustup](https://rustup.rs/)).

```bash
git clone https://github.com/arrenkaetris/kq.git
cd kq
cargo build --release
./target/release/kq --help
```

Put `target/release/kq` somewhere on your `PATH` if you want `kq` as a
normal command. There is no installer and no extra runtime — one native
binary, plus the system's usual dynamic libraries.

KotOR / KotOR II themselves are **not** bundled. Point `kq` at an install you
already have.

### AI agents (Cursor, Claude, Copilot, Gemini, OpenCode)

This repo includes an agent contract and a plugin so assistants use `kq`
instead of extracting archives. See **[Using kq with AI agents](docs/ai-agents.md)**.

```bash
./scripts/install-agent-plugin.sh   # Cursor local plugin symlink
```

---

## Point it at a game

Every command needs a target. In order:

1. `-i` / `--install <path>`
2. the `KQ_INSTALL` environment variable
3. walking upward from the current directory looking for `chitin.key`
   (the same idea as `git` walking upward for `.git`)

```bash
kq -i ~/kotor info
export KQ_INSTALL=~/kotor
kq ls -t utc -n 20
cd ~/kotor/Override && kq which appearance.2da
```

`-i` is not limited to a full install. The same commands work on a single
file, a standalone capsule, or a folder of loose resources:

```bash
kq -i somefile.utc cat somefile
kq -i danm13.mod ls
kq -i ./extracted_override/ grep Bastila
```

A directory *inside* an install (`-i Override` from the game root) still
means the whole install. A named *file* never does: `-i modules/danm13.mod`
means that one archive.

---

## Commands

### `kq info`

Summarize the target: game, resource counts, containers, modules.

```bash
kq info
kq info --by-type
kq info --json
```

### `kq ls [pattern]`

List resources. Bare text is a substring of the ResRef; `*` and `?` are
globs.

```bash
kq ls bastila
kq ls 'k_ai_*' -t ncs
kq ls -t utc -m danm13 --winners
kq ls -s override -q          # names only
kq ls -n 20                   # first 20
```

Filters (also work on `grep`):

| Flag | Meaning |
|------|---------|
| `-t`, `--type utc` | resource type (repeatable) |
| `-m`, `--module danm13` | module root (repeatable) |
| `-s`, `--source override` | source kind (repeatable) |
| `--winners` | only the copy the game would load |

Source kinds: `override`, `module-mod`, `module-rim`, `lips`,
`texturepack`, `rims`, `stream`, `chitin`, `talktable`, `loose`
(standalone files and folders).

### `kq which <resref>`

Show every copy of a name, in engine resolve order. `*` marks the winner.

```bash
kq which appearance.2da
kq which n_bastila -t utc
```

### `kq cat <resref>`

Print a resource as text. Default is an indented outline.

```bash
kq cat bastila00c.utc
kq cat appearance.2da -f gron
kq cat dialog.tlk --json
kq cat k_ai_master.ncs -f outline
kq cat n_bastila.utc --from Override
kq cat appearance.2da --raw > appearance.2da
```

Formats (`-f`):

| Value | What you get |
|-------|----------------|
| `outline` | indented tree, for reading (default) |
| `gron` | one `path = value` line per leaf — safe to pipe through `rg` |
| `json` | the same tree as JSON, for `jq` |
| `raw` | exact bytes (`--raw` is the same) |

`--json` on the command itself is a global flag and also selects JSON
output.

### `kq grep <pattern>`

Search decoded resource contents. Each hit is a field path, not a byte
offset into a binary blob.

```bash
kq grep Bastila -t dlg -n 5
kq grep --ignore-case 'getobjectbytag' -t ncs
kq grep -F 'cdx_il' -t ncs
kq grep 'proceduretype' -t tpc -l
```

There is no `-i` for case-insensitive match: `-i` is already `--install`.
Use `--ignore-case`.

By default `grep` only reads types it can decode. `--include-binary` also
searches everything else as raw bytes (slow on a full texture pack).

### `kq cache`

```bash
kq cache status
kq cache clear
```

---

## How it works

1. **Discover.** Find `chitin.key`, then the usual folders (`data/`,
   `modules/`, `Override/`, `lips/`, `texturepacks/`, `rims/`, stream
   directories) and `dialog.tlk` at the install root.
2. **Index.** Read every archive header and every loose file. Each resource
   becomes a `(name, type, file, offset, size, source)` row. Sources are
   ordered the way the engine resolves them: Override beats a `.mod` beats
   the `.rim` / `_s.rim` / `_dlg.erf` trio beats lips, texture packs,
   `rims/`, streams, then the base `chitin.key` BIFs.
3. **Cache.** The index is written under `$KQ_CACHE_DIR` or the platform
   cache directory (`~/.cache/kq` on Linux). The cache key is a fingerprint
   of names, sizes and mtimes — not file contents — so a 1.3 GB set of BIFs
   is not hashed on every run. `--refresh` rebuilds and replaces the cache;
   `--no-cache` skips it. Standalone files/folders are never cached.
4. **Decode.** `cat` and `grep` memory-map the owning file, slice out the
   resource, and project it to text. Format is chosen by sniffing bytes, not
   by extension (a `.utc` and a `.dlg` are both GFF).

A module in this index is the usual trio: `name.rim` + `name_s.rim` +
`name_dlg.erf`, or a single `name.mod` that replaces them.

---

## What decodes to text

| Kind | Extensions | What you see |
|------|------------|----------------|
| GFF | `utc` `utd` `ute` `uti` `utm` `utp` `uts` `utt` `utw` `dlg` `are` `git` `ifo` `jrl` `gui` `pth` `fac` … | field tree |
| 2DA | `2da` | rows as objects, row label in `_row` |
| TLK | `tlk` | `strref` + `text` (+ `sound` when present) |
| SSF | `ssf` | 28 named creature sound-event StrRefs |
| LIP | `lip` | duration + mouth-shape keyframes |
| NCS | `ncs` | disassembly: opcode, args, `GetObjectByTag`-style ACTION names |
| LTR | `ltr` | single-letter name-generation probabilities |
| BWM | `wok` `dwk` `pwk` | vertices, faces, materials, area-transition edges |
| TPC | `tpc` | size, format, mipmaps, trailing TXI text (not pixels) |
| MDL | `mdl` | name, supermodel, node names, animation names (not a mesh dump) |
| WAV | `wav` `bmu` | kind, rate, channels (not samples) |
| Already text | `nss` `lyt` `vis` `txi` `ini` `txt` | passed through |

Containers (`key` `bif` `erf` `mod` `sav` `rim` `hak`) are indexed, not
printed as a blob — `kq ls` lists what is inside them.

TGA, DDS and other still-opaque types print
`<type, N bytes, no text form yet>` unless you use `--raw` or
`grep --include-binary`.

NCS is a **disassembler**, not an NSS decompiler. Scripts become located
instructions you can grep (`….instructions[123].name = "GetObjectByTag"`),
not recovered source.

---

## Scripting

`--json` on any command. `ls` and `grep` emit one JSON object per line
(JSONL). `info`, `which` and `cache` emit one object.

Exit codes (stable; scripts should branch on these, not on stderr text):

| Code | Meaning |
|------|---------|
| 0 | succeeded / matched |
| 1 | runtime error (I/O, truncated file, …) |
| 2 | bad command line (clap) |
| 3 | valid query, nothing matched |
| 4 | no install or path could be resolved |

Broken pipes (`kq ls | head`) are silent success, not an error.

---

## Environment

| Variable | Role |
|----------|------|
| `KQ_INSTALL` | default target path (same as `--install`) |
| `KQ_CACHE_DIR` | where index files are stored |

---

## What this is not

- Not a writer. `kq` does not patch, compile, or pack archives.
- Not a decompiler. NCS is disassembled; it is not turned back into NSS.
- Not an image or audio exporter. TPC/WAV metadata only.
- Not a Windows-only tool. It is an ordinary Rust CLI; it does not launch
  the game.

---

## License

MIT. See [LICENSE](LICENSE).
