//! Turning a resource's bytes into text, whatever format it is.
//!
//! Format is decided by sniffing the bytes, not by the extension. A `.utc`
//! and a `.dlg` are both GFF; a mod can ship anything under any name; and
//! `git textconv` hands us a temp file called `git-blob-XXXX` with no
//! extension at all.

use anyhow::Result;
use serde_json::Value as J;

use kq_format::{bwm, gff, lip, ltr, mdl, ncs, ssf, text, tlk, tpc, twoda, wav, ResType};
use kq_index::{Index, Resource};

use crate::read;

/// How to print a decoded resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    /// One `path = value` line per leaf. Every line carries its address.
    Gron,
    /// Indented tree, for reading.
    Outline,
    /// The decoded value as JSON.
    Json,
    /// The resource's exact bytes.
    Raw,
}

/// What a resource decoded into.
pub enum Decoded {
    /// A structured value: GFF tree, 2DA rows, TLK entries.
    Value(J),
    /// The resource was already text.
    Text(String),
    /// No decoder for this format yet.
    Opaque { kind: &'static str, len: usize },
}

/// Decode a resource from the index, pairing binary MDL with its MDX companion.
pub fn decode_resource(index: &Index, r: &Resource, bytes: &[u8]) -> Result<Decoded> {
    let mdx = companion_mdx(index, r);
    decode_ex(bytes, mdx.as_deref(), Some(r.restype), &r.filename())
}

fn companion_mdx(index: &Index, r: &Resource) -> Option<Vec<u8>> {
    if r.restype.extension() != Some("mdl") {
        return None;
    }
    for ext in ["mdx", "mdx2"] {
        let Some(ty) = ResType::from_extension(ext) else {
            continue;
        };
        if let Some(companion) = index.resolve(&r.resref, Some(ty)) {
            if let Ok(bytes) = read::read(index, companion) {
                return Some(bytes);
            }
        }
    }
    None
}

/// Decode a resource. `restype` is a hint used only when sniffing is
/// inconclusive.
pub fn decode(bytes: &[u8], restype: Option<ResType>, name: &str) -> Result<Decoded> {
    decode_ex(bytes, None, restype, name)
}

pub fn decode_with_mdx(
    bytes: &[u8],
    mdx: Option<&[u8]>,
    restype: Option<ResType>,
    name: &str,
) -> Result<Decoded> {
    decode_ex(bytes, mdx, restype, name)
}

fn decode_ex(
    bytes: &[u8],
    mdx: Option<&[u8]>,
    restype: Option<ResType>,
    name: &str,
) -> Result<Decoded> {
    let path = std::path::Path::new(name);

    if gff::sniff(bytes) {
        let g = gff::read(bytes, path)?;
        return Ok(Decoded::Value(text::gff_to_json(&g)));
    }
    if twoda::sniff(bytes) {
        let t = twoda::read_or_salvage(bytes, path)?;
        return Ok(Decoded::Value(text::twoda_to_json(&t)));
    }
    if tlk::sniff(bytes) {
        let t = tlk::read(bytes, path)?;
        return Ok(Decoded::Value(text::tlk_to_json(&t)));
    }
    if ssf::sniff(bytes) {
        let s = ssf::read(bytes, path)?;
        return Ok(Decoded::Value(text::ssf_to_json(&s)));
    }
    if lip::sniff(bytes) {
        let l = lip::read(bytes, path)?;
        return Ok(Decoded::Value(text::lip_to_json(&l)));
    }
    if ncs::sniff(bytes) {
        let n = ncs::read(bytes, path)?;
        return Ok(Decoded::Value(text::ncs_to_json(&n)));
    }
    if bwm::sniff(bytes) {
        let w = bwm::read(bytes, path)?;
        return Ok(Decoded::Value(text::bwm_to_json(&w)));
    }
    if ltr::sniff(bytes) {
        let l = ltr::read(bytes, path)?;
        return Ok(Decoded::Value(text::ltr_to_json(&l)));
    }
    if wav::sniff(bytes) {
        let w = wav::read(bytes, path)?;
        return Ok(Decoded::Value(text::wav_to_json(&w)));
    }
    // TPC and binary MDL have no reliable magic. Restype is the hint.
    // ASCII MDL is text and must be claimed before `looks_like_text`.
    if mdl::sniff_ascii(bytes)
        || (restype.is_some_and(|t| t.extension() == Some("mdl")) && mdl::sniff_binary(bytes))
    {
        let m = match mdx {
            Some(ext) => mdl::read_with_mdx(bytes, ext, path)?,
            None => mdl::read(bytes, path)?,
        };
        return Ok(Decoded::Value(text::mdl_to_json(&m)));
    }
    if restype.is_some_and(|t| matches!(t.extension(), Some("mdx") | Some("mdx2"))) {
        return Ok(Decoded::Value(serde_json::json!({
            "kind": "mdx",
            "bytes": bytes.len(),
            "note": "companion vertex buffer for the same-ResRef .mdl",
        })));
    }
    if restype.is_some_and(|t| t.extension() == Some("tpc")) && tpc::sniff(bytes) {
        let t = tpc::read(bytes, path)?;
        return Ok(Decoded::Value(text::tpc_to_json(&t)));
    }
    if restype.is_some_and(ResType::is_plain_text) || looks_like_text(bytes) {
        return Ok(Decoded::Text(decode_cp1252(bytes)));
    }
    Ok(Decoded::Opaque {
        kind: restype.and_then(|t| t.extension()).unwrap_or("binary"),
        len: bytes.len(),
    })
}

/// Render a decoded resource in the requested format.
///
/// `root` is the address prefix for gron lines — normally the resource's
/// `name.ext`, so a line stays locatable after leaving the pipe.
pub fn render(decoded: &Decoded, format: Format, root: &str) -> Result<String> {
    use std::fmt::Write;
    let mut s = String::new();
    match (decoded, format) {
        (Decoded::Value(value), Format::Gron) => text::gron(root, value, &mut s)?,
        (Decoded::Value(value), Format::Outline) => text::outline(value, &mut s)?,
        (Decoded::Value(value), Format::Json) => {
            s = serde_json::to_string_pretty(value)?;
            s.push('\n');
        }
        (Decoded::Text(t), Format::Gron) => {
            // Plain text has no field paths, so the address is the line
            // number — still self-locating, still one leaf per line.
            for (i, line) in t.lines().enumerate() {
                writeln!(s, "{root}[{}] = {}", i + 1, J::String(line.to_string()))?;
            }
        }
        (Decoded::Text(t), Format::Json) => {
            s = serde_json::to_string_pretty(&J::String(t.clone()))?;
            s.push('\n');
        }
        (Decoded::Text(t), _) => {
            s = t.clone();
            if !s.ends_with('\n') {
                s.push('\n');
            }
        }
        (Decoded::Opaque { kind, len }, Format::Json) => {
            s = serde_json::to_string_pretty(
                &serde_json::json!({ "kind": kind, "bytes": len, "decoded": false }),
            )?;
            s.push('\n');
        }
        (Decoded::Opaque { kind, len }, _) => {
            writeln!(s, "{root} = <{kind}, {len} bytes, no text form yet>")?;
        }
        (_, Format::Raw) => unreachable!("raw is handled before decoding"),
    }
    Ok(s)
}

/// Heuristic for "this is already text".
///
/// A NUL means binary. Anything else that is mostly printable is treated as
/// text, which covers the several KotOR formats that are plain text with no
/// signature at all: LYT, VIS, TXI, NSS.
fn looks_like_text(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(8192)];
    if sample.is_empty() || sample.contains(&0) {
        return false;
    }
    let printable = sample
        .iter()
        .filter(|&&b| b == b'\n' || b == b'\r' || b == b'\t' || (0x20..0x7F).contains(&b))
        .count();
    printable * 100 / sample.len() >= 95
}

fn decode_cp1252(bytes: &[u8]) -> String {
    if bytes.is_ascii() {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        bytes.iter().map(|&b| gff::cp1252_char(b)).collect()
    }
}

/// Force any bytes to a searchable string, for a format with no decoder yet.
///
/// Every byte maps to a character — control bytes included — so nothing is
/// dropped and a pattern can still find an embedded ASCII string inside an
/// otherwise binary resource (an NCS constant pool, MDL node names). Not a
/// text *decoding*: it exists so `kq grep --include-binary` searches real
/// bytes instead of a one-line placeholder that can never match.
pub fn raw_as_text(bytes: &[u8]) -> String {
    decode_cp1252(bytes)
}
