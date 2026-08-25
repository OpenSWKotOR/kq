//! GFF — the tree format behind creatures, doors, dialog, areas, journals and
//! about twenty other extensions.
//!
//! One parser covers all of them: the extension only names the schema, never
//! the encoding. Layout (V3.2) is six (offset, count) pairs pointing at the
//! struct, field, label, field-data, field-index and list-index arrays.

use std::collections::BTreeMap;
use std::path::Path;

use crate::error::{FormatError, Result};
use crate::reader::Reader;

/// A GFF value, decoded into something a query language can walk.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    UInt(u64),
    Float(f64),
    /// CExoString and ResRef both land here; the distinction is schema, not
    /// data, and keeping them apart would only complicate every query.
    Str(String),
    /// A localized string: a talk-table reference plus any inline overrides.
    LocString { strref: i64, substrings: BTreeMap<u32, String> },
    /// Raw bytes, kept as-is.
    Void(Vec<u8>),
    Struct(Struct),
    List(Vec<Struct>),
    Vector([f32; 3]),
    Orientation([f32; 4]),
    /// A talk-table index. Resolvable against dialog.tlk.
    StrRef(i64),
}

/// One GFF struct: an id plus ordered named fields.
#[derive(Clone, Debug, PartialEq)]
pub struct Struct {
    pub id: u32,
    /// Field order is the file's order, which is meaningful for diffing.
    pub fields: Vec<(String, Value)>,
}

impl Struct {
    pub fn get(&self, label: &str) -> Option<&Value> {
        self.fields.iter().find(|(k, _)| k.eq_ignore_ascii_case(label)).map(|(_, v)| v)
    }
}

/// A parsed GFF file.
#[derive(Clone, Debug)]
pub struct Gff {
    /// The four-character type tag, e.g. `UTC `, trimmed.
    pub file_type: String,
    pub version: String,
    pub root: Struct,
}

/// True when the bytes start with any GFF-family signature.
///
/// GFF has no single magic number — the first four bytes are the content
/// type — so the version field four bytes in is what identifies it.
pub fn sniff(data: &[u8]) -> bool {
    data.len() >= 8 && (&data[4..8] == b"V3.2" || &data[4..8] == b"V3.3")
}

const MAX_DEPTH: usize = 64;

pub fn read(data: &[u8], path: &Path) -> Result<Gff> {
    let mut r = Reader::new(data, path);
    let file_type = String::from_utf8_lossy(r.slice_at(0, 4)?).trim().to_string();
    let version = String::from_utf8_lossy(r.slice_at(4, 4)?).trim().to_string();
    if version != "V3.2" && version != "V3.3" {
        return Err(FormatError::BadVersion {
            path: path.to_path_buf(),
            format: "GFF",
            version,
        });
    }

    r.seek(8)?;
    let h = Header {
        struct_offset: r.u32()? as usize,
        struct_count: r.u32()? as usize,
        field_offset: r.u32()? as usize,
        field_count: r.u32()? as usize,
        label_offset: r.u32()? as usize,
        label_count: r.u32()? as usize,
        field_data_offset: r.u32()? as usize,
        _field_data_bytes: r.u32()? as usize,
        field_indices_offset: r.u32()? as usize,
        _field_indices_bytes: r.u32()? as usize,
        list_indices_offset: r.u32()? as usize,
        _list_indices_bytes: r.u32()? as usize,
    };

    let mut labels = Vec::with_capacity(h.label_count);
    for i in 0..h.label_count {
        r.seek(h.label_offset + i * 16)?;
        labels.push(r.fixed_string_cased(16)?);
    }

    let mut ctx = Ctx { r, h, labels };
    if ctx.h.struct_count == 0 {
        return Err(ctx.r.malformed("GFF has no structs"));
    }
    let root = read_struct(&mut ctx, 0, 0)?;
    Ok(Gff { file_type, version, root })
}

struct Header {
    struct_offset: usize,
    struct_count: usize,
    field_offset: usize,
    field_count: usize,
    label_offset: usize,
    label_count: usize,
    field_data_offset: usize,
    _field_data_bytes: usize,
    field_indices_offset: usize,
    _field_indices_bytes: usize,
    list_indices_offset: usize,
    _list_indices_bytes: usize,
}

struct Ctx<'a> {
    r: Reader<'a>,
    h: Header,
    labels: Vec<String>,
}

fn read_struct(ctx: &mut Ctx, index: usize, depth: usize) -> Result<Struct> {
    if depth > MAX_DEPTH {
        return Err(ctx.r.malformed(format!("struct nesting deeper than {MAX_DEPTH}")));
    }
    if index >= ctx.h.struct_count {
        return Err(ctx.r.malformed(format!("struct index {index} out of range")));
    }
    ctx.r.seek(ctx.h.struct_offset + index * 12)?;
    let id = ctx.r.u32()?;
    let data_or_offset = ctx.r.u32()? as usize;
    let field_count = ctx.r.u32()? as usize;

    // One field stores its index directly; more than one stores a byte
    // offset into the field-index array.
    let indices: Vec<usize> = if field_count == 1 {
        vec![data_or_offset]
    } else {
        let mut v = Vec::with_capacity(field_count);
        for i in 0..field_count {
            ctx.r.seek(ctx.h.field_indices_offset + data_or_offset + i * 4)?;
            v.push(ctx.r.u32()? as usize);
        }
        v
    };

    let mut fields = Vec::with_capacity(field_count);
    for fi in indices {
        fields.push(read_field(ctx, fi, depth)?);
    }
    Ok(Struct { id, fields })
}

fn read_field(ctx: &mut Ctx, index: usize, depth: usize) -> Result<(String, Value)> {
    if index >= ctx.h.field_count {
        return Err(ctx.r.malformed(format!("field index {index} out of range")));
    }
    ctx.r.seek(ctx.h.field_offset + index * 12)?;
    let kind = ctx.r.u32()?;
    let label_index = ctx.r.u32()? as usize;
    let raw = ctx.r.u32()?;

    let label = ctx
        .labels
        .get(label_index)
        .cloned()
        .unwrap_or_else(|| format!("_label{label_index}"));

    let at = |ctx: &Ctx, off: u32| ctx.h.field_data_offset + off as usize;

    let value = match kind {
        0 => Value::UInt(raw as u8 as u64),
        1 => Value::Int(raw as u8 as i8 as i64),
        2 => Value::UInt(raw as u16 as u64),
        3 => Value::Int(raw as u16 as i16 as i64),
        4 => Value::UInt(raw as u64),
        5 => Value::Int(raw as i32 as i64),
        6 => {
            ctx.r.seek(at(ctx, raw))?;
            let lo = ctx.r.u32()? as u64;
            let hi = ctx.r.u32()? as u64;
            Value::UInt(lo | (hi << 32))
        }
        7 => {
            ctx.r.seek(at(ctx, raw))?;
            let lo = ctx.r.u32()? as u64;
            let hi = ctx.r.u32()? as u64;
            Value::Int((lo | (hi << 32)) as i64)
        }
        8 => Value::Float(f32::from_bits(raw) as f64),
        9 => {
            ctx.r.seek(at(ctx, raw))?;
            let lo = ctx.r.u32()? as u64;
            let hi = ctx.r.u32()? as u64;
            Value::Float(f64::from_bits(lo | (hi << 32)))
        }
        10 => {
            ctx.r.seek(at(ctx, raw))?;
            let len = ctx.r.u32()? as usize;
            let bytes = ctx.r.take(len)?;
            Value::Str(decode_text(bytes))
        }
        11 => {
            ctx.r.seek(at(ctx, raw))?;
            let len = ctx.r.u8()? as usize;
            let bytes = ctx.r.take(len)?;
            Value::Str(decode_text(bytes))
        }
        12 => {
            ctx.r.seek(at(ctx, raw))?;
            let _total = ctx.r.u32()?;
            let strref = ctx.r.u32()? as i32 as i64;
            let count = ctx.r.u32()? as usize;
            let mut substrings = BTreeMap::new();
            for _ in 0..count {
                let id = ctx.r.u32()?;
                let len = ctx.r.u32()? as usize;
                let bytes = ctx.r.take(len)?;
                substrings.insert(id, decode_text(bytes));
            }
            Value::LocString { strref, substrings }
        }
        13 => {
            ctx.r.seek(at(ctx, raw))?;
            let len = ctx.r.u32()? as usize;
            Value::Void(ctx.r.take(len)?.to_vec())
        }
        14 => Value::Struct(read_struct(ctx, raw as usize, depth + 1)?),
        15 => {
            ctx.r.seek(ctx.h.list_indices_offset + raw as usize)?;
            let count = ctx.r.u32()? as usize;
            let mut ids = Vec::with_capacity(count);
            for _ in 0..count {
                ids.push(ctx.r.u32()? as usize);
            }
            let mut items = Vec::with_capacity(count);
            for id in ids {
                items.push(read_struct(ctx, id, depth + 1)?);
            }
            Value::List(items)
        }
        16 => {
            ctx.r.seek(at(ctx, raw))?;
            let mut q = [0f32; 4];
            for slot in &mut q {
                *slot = ctx.r.f32()?;
            }
            Value::Orientation(q)
        }
        17 => {
            ctx.r.seek(at(ctx, raw))?;
            let mut v = [0f32; 3];
            for slot in &mut v {
                *slot = ctx.r.f32()?;
            }
            Value::Vector(v)
        }
        18 => {
            ctx.r.seek(at(ctx, raw))?;
            let _size = ctx.r.u32()?;
            Value::StrRef(ctx.r.u32()? as i32 as i64)
        }
        other => return Err(ctx.r.malformed(format!("unknown GFF field type {other}"))),
    };
    Ok((label, value))
}

/// KotOR strings are Windows-1252, not UTF-8.
///
/// Decoding as UTF-8 would mangle every accented character in the European
/// releases; Windows-1252 maps every byte to something, so this cannot fail.
fn decode_text(bytes: &[u8]) -> String {
    if bytes.is_ascii() {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    bytes.iter().map(|&b| cp1252_char(b)).collect()
}

/// Map one Windows-1252 byte to its Unicode code point.
pub fn cp1252_char(b: u8) -> char {
    // 0x80..0x9F is where Windows-1252 differs from Latin-1; everything else
    // is identity.
    const HIGH: [char; 32] = [
        '\u{20AC}', '\u{81}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{8D}',
        '\u{017D}', '\u{8F}', '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}',
        '\u{2022}', '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}',
        '\u{0153}', '\u{9D}', '\u{017E}', '\u{0178}',
    ];
    if (0x80..0xA0).contains(&b) {
        HIGH[(b - 0x80) as usize]
    } else {
        b as char
    }
}
