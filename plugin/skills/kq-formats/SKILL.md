---
name: kq-formats
description: Interpret kq cat/grep projections for KotOR formats (GFF including .utc/.dlg/.git, 2DA, TLK, NCS, SSF, LIP, TPC, MDL, BWM, WAV). Use when decoding resource contents, explaining gron/outline/JSON field paths or StrRefs, or deciding whether a type is searchable text vs opaque bytes.
---

# kq-formats

`cat` and `grep` share one projection. Format is chosen by **sniffing bytes**,
not by extension (`.utc` and `.dlg` are both GFF).

Read [references/projections.md](references/projections.md) when you need
field shapes.

## Searchable vs opaque

**Searchable by default:** GFF types, `2da`, `tlk`, `ssf`, `lip`, `ncs`,
`ltr`, `tpc`, `mdl`, `wok`/`dwk`/`pwk`, `wav`/`bmu`, plus plain text
(`nss`, `lyt`, `vis`, `txi`, `ini`, `txt`).

**Indexed, not dumped:** `key`, `bif`, `erf`, `mod`, `sav`, `rim`, `hak`.
List insides with `kq ls`.

**Opaque unless `--raw` / `grep --include-binary`:** TGA, DDS, MVE, BIK, and
other still-undecoded types.

## Honest limits

- NCS is a **disassembly** (`ACTION GetObjectByTag`), not recovered NSS.
- TPC/WAV are **metadata** (TPC includes trailing TXI text). No pixels/PCM.
- MDL is a **full model IR** (nodes, meshes, controllers, animations). Binary MDL pairs a same-ResRef `.mdx`.

## How to cite a hit

`grep` prints `resref  path = value`. The path is the address. After
`--json`, reuse that path in `jq`. Do not report byte offsets into a BIF
as if they were fields.
