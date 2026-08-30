# kq text projections

Trees below are what `cat --json` / gron emit. Outline is the same tree,
indented.

## GFF

Object of fields. Localized strings look like
`{ "strref": N, "substrings": … }`. Vectors are arrays.

## 2DA

Array of row objects. Row label is `_row` (not always the numeric index).

## TLK

Array of `{ strref, text, sound? }` in StrRef order.

## NCS

`{ declared_size, instructions: [{ offset, op, … }] }`. ACTION rows include
`routine`, `name`, `argc`. TSL names are a superset of K1; ids 0–767 match.

## SSF

Map of 28 event names → StrRef. `-1` means none.

## LIP

Duration plus mouth-shape keyframes. Truncated files fail `cat` with exit 1.

## LTR

Single-letter name-generation probabilities in JSON (full 28³ tables are
parsed internally).

## BWM (`wok` / `dwk` / `pwk`)

Vertices, faces, materials, area-transition edges.

## TPC

Size, format, mipmaps, trailing TXI. No pixel payload.

## MDL

Full model IR: name, supermodel, classification, node tree (meshes,
controllers, lights, emitters), animations. Binary files include vertices
from the companion `.mdx` when present. JSON is a 1:1 serde of that tree.

## WAV / BMU

Kind, rate, channels. RIFF, 470-byte SFX header, or BMU. No samples.

## Plain text

`nss` / `lyt` / `vis` / `txi` / `ini` / `txt`: JSON string; gron is
`file.ext[N] = "line"`.
