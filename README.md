# cleanhouse — `kq`

Query a KotOR installation like it was plain text.

`kq` reads a *Star Wars: Knights of the Old Republic* installation — its
archives, modules and loose files — and answers questions about it, the way
`rg` answers questions about a source tree. It knows nothing about game
*data*; it knows the container formats (KEY, BIF, ERF, RIM) and the common
resource formats (GFF, 2DA, TLK) well enough to show you what's inside them
without you extracting anything first.

```
$ kq -i ~/kotor which appearance.2da
* appearance.2da         override      Override                     103273 bytes
  appearance.2da         rims          global.rim                   98610 bytes
  appearance.2da         rims          miniglobal.rim               98610 bytes
  appearance.2da         chitin        data/2da.bif                 98610 bytes

* is the copy the game loads; the rest are shadowed.
```

That's the thing KotOR modding gets wrong constantly: which copy of a
resource actually loads, out of everywhere it might be shadowed. `kq which`
answers it directly instead of making you reconstruct the search order by
hand.

## Install

```
cargo build --release
./target/release/kq --help
```

One binary, no interpreter or game-specific runtime — just the system's
usual dynamic libraries, like any other native tool.

## The five commands

Point every command at an install with `-i`/`--install`, the `KQ_INSTALL`
environment variable, or just run `kq` from inside one — it walks upward
looking for `chitin.key`, the way `git` walks upward looking for `.git`.

- **`kq info`** — what's in this installation: game, resource counts, a
  breakdown by source or by type.
- **`kq ls [pattern]`** — list resources. Bare text is a substring match;
  `*`/`?` are globs. Filter with `-t utc`, `-m danm13`, `-s override`.
- **`kq which <resref>`** — show every copy of a resource, in the order the
  game resolves them, marking the one that actually loads.
- **`kq cat <resref>`** — decode a resource to text. GFF-family formats
  (`.utc`, `.dlg`, `.are`, `.git`, `.ifo`, …), 2DA and TLK all decode; plain
  text formats (`.nss`, `.lyt`, `.vis`, `.txi`) pass through as-is.
- **`kq grep <pattern>`** — search decoded resource contents with a regex.

Every command takes `--json` for scripting and a stable, documented exit
code: `0` matched/succeeded, `1` a runtime error, `3` the query was valid but
matched nothing, `4` no installation or path could be resolved. (`2` is
reserved for a bad command line — clap owns that one before your code ever
runs.)

## Reading a resource

```
$ kq cat bastila00c.utc
TemplateResRef              "bastila00c"
Race                        6
FirstName (1 fields)
  strref                     31360
Appearance_Type              4
Tag                          "Bastila"
Conversation                 "k_hbas_dialog"
...
```

Add `-f gron` for one `path = value` line per field instead of a tree — each
line stands alone, so it survives being piped through `grep`/`rg`:

```
$ kq cat bastila00c.utc -f gron | grep Tag
bastila00c.utc.Tag = "Bastila"
```

Add `--json` for the same tree as JSON, for `jq`.

## Searching

```
$ kq grep 'Bastila' -t dlg -n 5
22aa_zaalb01_01.dlg 22aa_zaalb01_01.dlg.EntryList[10].Speaker = "Bastila"
22aa_zaalb01_01.dlg 22aa_zaalb01_01.dlg.EntryList[11].Speaker = "Bastila"
...
```

`kq grep` decodes each candidate resource the same way `kq cat -f gron`
would, then matches the pattern line by line — so a hit's address is a real
field path, not a meaningless byte offset into a binary blob. (That
distinction matters: piping binary formats through `rg --pre` destroys real
offsets, because `rg` numbers lines and bytes in the *preprocessor's output*,
not the original file. Every `kq grep` hit carries its own address instead of
relying on one.)

By default `grep` only reads resource types it can actually decode to text
(GFF, 2DA, TLK, and formats that are already plain text) — skipping the
~25,000 textures, models and sounds in a typical install. Pass
`--include-binary` to search everything else as raw bytes too (useful for
finding an embedded ASCII string constant inside an `.ncs` script, for
example).

## Beyond a full installation

Every command works the same way against a single resource file, a
standalone capsule, or a folder of loose files — not just a full
installation:

```
kq -i somefile.utc cat somefile          # one resource, no install needed
kq -i danm13.mod ls                      # everything inside one capsule
kq -i ./extracted_override/ grep Bastila # a folder of loose files
```

## Caching

Indexing a retail install means reading the header of every archive in
it — a few hundred files. The result is cached under `$KQ_CACHE_DIR` (or the
platform cache directory) and invalidated automatically when the
installation's file listing changes (name, size, and modification time —
not content, so a 1.3&nbsp;GB set of BIFs isn't hashed on every run). Pass
`--no-cache` to skip it entirely, or `--refresh` to force a rebuild.

Standalone targets (a lone file, capsule, or folder) are never cached —
each of those is already a single fast parse.

## What's inside

Three crates:

- **`kq-format`** — readers for the on-disk formats: KEY, BIF, ERF, RIM
  (containers), GFF, 2DA, TLK (data). No I/O beyond taking a byte slice; a
  caller mmaps or reads the file. Structural output is validated byte-exact
  against [PyKotor](https://github.com/OpenKotOR/PyKotor)'s readers.
- **`kq-index`** — discovers an installation (or a standalone target),
  builds the full resource index respecting KotOR's search-order precedence
  (Override beats a `.mod` beats the `.rim` trio beats a texture pack beats
  the base `chitin.key` BIFs), and caches it.
- **`kq-cli`** — the `kq` binary.

## What isn't here yet

Scope, deliberately: a representation of an installation, a file, a folder,
a capsule, or a module — decoded to text wherever a decoder exists.

Not yet decoded to text: `.ncs` (script bytecode — the bytes are readable
with `--include-binary`, but there's no disassembler/decompiler here),
`.mdl`/`.mdx` (model geometry), `.tpc`/`.tga`/`.dds` (textures), audio. Each
of those either has no natural text form or is a substantial project of its
own; `kq cat` says so explicitly (`no text form yet`) rather than silently
dumping bytes.
