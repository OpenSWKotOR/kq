//! Projecting binary resources into text.
//!
//! Three projections, one source of truth. `json` is the canonical value; the
//! other two are renderings of it.
//!
//! * **gron** — one `path = value` line per leaf. Every line carries its own
//!   address, which is what makes the output usable under `rg --pre`, where
//!   byte offsets and line numbers refer to the projection rather than the
//!   file. A grep hit is a complete, pasteable location.
//! * **outline** — indented, deduplicated, for reading.
//! * **json** — for `jq` and for anything that wants the tree back.

use base64::Engine as _;
use serde_json::{json, Map, Value as J};

use crate::bwm::Bwm;
use crate::gff::{Gff, Struct, Value};
use crate::lip::Lip;
use crate::ltr::{Ltr, LETTERS};
use crate::mdl::Mdl;
use crate::ncs::{Arg, Ncs};
use crate::ssf::{Ssf, EVENTS};
use crate::tlk::Tlk;
use crate::tpc::Tpc;
use crate::twoda::TwoDa;
use crate::wav::Wav;

// ---------------------------------------------------------------- to JSON --

/// Convert a GFF tree to JSON.
///
/// Scalars become plain JSON scalars so `jq` queries read naturally
/// (`.Appearance_Type`, not `.Appearance_Type.value`). The types that carry
/// more than a number — localized strings, byte blobs, vectors — keep a small
/// tagged object, because collapsing those would lose the StrRef that makes
/// them meaningful.
pub fn gff_to_json(gff: &Gff) -> J {
    let mut map = match struct_to_json(&gff.root) {
        J::Object(m) => m,
        other => {
            let mut m = Map::new();
            m.insert("_root".into(), other);
            m
        }
    };
    map.insert("_file_type".into(), json!(gff.file_type));
    map.insert("_version".into(), json!(gff.version));
    J::Object(map)
}

fn struct_to_json(s: &Struct) -> J {
    let mut map = Map::with_capacity(s.fields.len() + 1);
    if s.id != 0 && s.id != u32::MAX {
        map.insert("_struct_id".into(), json!(s.id));
    }
    for (label, value) in &s.fields {
        map.insert(label.clone(), value_to_json(value));
    }
    J::Object(map)
}

fn value_to_json(v: &Value) -> J {
    match v {
        Value::Int(i) => json!(i),
        Value::UInt(u) => json!(u),
        Value::Float(f) => json!(f),
        Value::Str(s) => json!(s),
        Value::StrRef(r) => json!({ "strref": r }),
        Value::LocString { strref, substrings } => {
            let mut m = Map::new();
            m.insert("strref".into(), json!(strref));
            if !substrings.is_empty() {
                let subs: Map<String, J> = substrings
                    .iter()
                    .map(|(k, v)| (k.to_string(), json!(v)))
                    .collect();
                m.insert("substrings".into(), J::Object(subs));
            }
            J::Object(m)
        }
        Value::Void(b) => {
            json!({ "bytes": base64::engine::general_purpose::STANDARD.encode(b), "len": b.len() })
        }
        Value::Struct(s) => struct_to_json(s),
        Value::List(items) => J::Array(items.iter().map(struct_to_json).collect()),
        Value::Vector([x, y, z]) => json!([x, y, z]),
        Value::Orientation([x, y, z, w]) => json!([x, y, z, w]),
    }
}

/// Convert a 2DA to an array of row objects, keyed by column label.
///
/// The row label is kept as `_row` because it is not always the row index —
/// some tables use names — and queries that assume otherwise get wrong
/// answers silently.
pub fn twoda_to_json(t: &TwoDa) -> J {
    let rows: Vec<J> = t
        .rows
        .iter()
        .enumerate()
        .map(|(i, cells)| {
            let mut m = Map::with_capacity(cells.len() + 1);
            m.insert(
                "_row".into(),
                json!(t.labels.get(i).cloned().unwrap_or_default()),
            );
            for (col, cell) in t.columns.iter().zip(cells) {
                m.insert(col.clone(), json!(cell));
            }
            J::Object(m)
        })
        .collect();
    J::Array(rows)
}

/// Convert a talk table to an array indexed by StrRef.
pub fn tlk_to_json(t: &Tlk) -> J {
    let entries: Vec<J> = t
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let mut m = Map::new();
            m.insert("strref".into(), json!(i));
            m.insert("text".into(), json!(e.text));
            if !e.sound.is_empty() {
                m.insert("sound".into(), json!(e.sound));
            }
            J::Object(m)
        })
        .collect();
    J::Array(entries)
}

/// Convert a sound set to a JSON object keyed by event name.
pub fn ssf_to_json(s: &Ssf) -> J {
    let mut m = Map::with_capacity(EVENTS.len());
    for (name, &strref) in EVENTS.iter().zip(s.sounds.iter()) {
        m.insert((*name).to_string(), json!(strref));
    }
    J::Object(m)
}

/// Convert lip-sync keyframes to a JSON object with the total length and the
/// keyframe array.
pub fn lip_to_json(l: &Lip) -> J {
    let keyframes: Vec<J> = l
        .keyframes
        .iter()
        .map(|k| json!({ "time": k.time, "shape": k.shape.0 }))
        .collect();
    json!({ "length": l.length, "keyframes": keyframes })
}

/// Convert compiled NWScript to a JSON array of located instructions.
pub fn ncs_to_json(n: &Ncs) -> J {
    let instructions: Vec<J> = n
        .instructions
        .iter()
        .map(|ins| {
            let mut m = Map::new();
            m.insert("offset".into(), json!(ins.offset));
            m.insert("op".into(), json!(ins.op));
            if let Some(routine) = ins.routine {
                m.insert("routine".into(), json!(routine));
            }
            if let Some(name) = ins.routine_name {
                m.insert("name".into(), json!(name));
            }
            if let Some(argc) = ins.argc {
                m.insert("argc".into(), json!(argc));
            }
            if !ins.args.is_empty() && ins.routine.is_none() {
                let args: Vec<J> = ins
                    .args
                    .iter()
                    .map(|a| match a {
                        Arg::Int(i) => json!(i),
                        Arg::Float(f) => json!(f),
                        Arg::Str(s) => json!(s),
                        Arg::Jump(t) => json!({ "jump": t }),
                    })
                    .collect();
                m.insert("args".into(), J::Array(args));
            }
            J::Object(m)
        })
        .collect();
    json!({ "declared_size": n.declared_size, "instructions": instructions })
}

/// Convert a texture header (and its trailing TXI) to JSON. Pixels stay out.
pub fn tpc_to_json(t: &Tpc) -> J {
    let mut m = Map::new();
    m.insert("width".into(), json!(t.width));
    m.insert("height".into(), json!(t.height));
    m.insert("format".into(), json!(t.format_name()));
    m.insert("mipmaps".into(), json!(t.mipmaps));
    m.insert("alpha_test".into(), json!(t.alpha_test));
    m.insert("cube_map".into(), json!(t.cube_map));
    if !t.txi.is_empty() {
        m.insert("txi".into(), json!(t.txi));
    }
    J::Object(m)
}

/// Convert a walkmesh to JSON: hooks, vertices, faces, materials.
pub fn bwm_to_json(w: &Bwm) -> J {
    let v3 = |v: &crate::bwm::Vec3| json!([v.x, v.y, v.z]);
    let vertices: Vec<J> = w.vertices.iter().map(v3).collect();
    let faces: Vec<J> = w
        .faces
        .iter()
        .map(|f| {
            let mut m = Map::new();
            m.insert("v".into(), json!(f.vertices));
            m.insert("material".into(), json!(f.material));
            if f.transitions.iter().any(|t| t.is_some()) {
                m.insert("transitions".into(), json!(f.transitions));
            }
            J::Object(m)
        })
        .collect();
    json!({
        "type": w.walkmesh_type,
        "position": v3(&w.position),
        "relative_hook1": v3(&w.relative_hook1),
        "relative_hook2": v3(&w.relative_hook2),
        "absolute_hook1": v3(&w.absolute_hook1),
        "absolute_hook2": v3(&w.absolute_hook2),
        "vertices": vertices,
        "faces": faces,
    })
}

/// Convert the single-letter name-generation table. Doubles/triples are
/// parsed for validity and then dropped — they are not greppable content.
pub fn ltr_to_json(l: &Ltr) -> J {
    let singles: Vec<J> = l
        .singles
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let ch = LETTERS.get(i).copied().unwrap_or(b'?') as char;
            json!({
                "letter": ch.to_string(),
                "start": c.start,
                "middle": c.middle,
                "end": c.end,
            })
        })
        .collect();
    json!({ "letters": l.letter_count, "singles": singles })
}

/// Convert a model inventory — names, not vertices.
pub fn mdl_to_json(m: &Mdl) -> J {
    json!({
        "name": m.name,
        "supermodel": m.supermodel,
        "classification": m.classification,
        "node_count": m.node_count,
        "names": m.names,
        "animations": m.animations,
    })
}

/// Convert audio metadata. Samples stay out.
pub fn wav_to_json(w: &Wav) -> J {
    let mut m = Map::new();
    m.insert("kind".into(), json!(w.kind));
    if let Some(c) = w.channels {
        m.insert("channels".into(), json!(c));
    }
    if let Some(r) = w.sample_rate {
        m.insert("sample_rate".into(), json!(r));
    }
    if let Some(b) = w.bits_per_sample {
        m.insert("bits_per_sample".into(), json!(b));
    }
    if let Some(e) = w.encoding {
        m.insert("encoding".into(), json!(e));
    }
    if let Some(n) = w.data_bytes {
        m.insert("data_bytes".into(), json!(n));
    }
    J::Object(m)
}

// ------------------------------------------------------------------ gron --

/// Write one `path = value` line per leaf.
///
/// `root` prefixes every path, so a line stays meaningful after it has been
/// piped through `rg` and separated from its file.
pub fn gron(root: &str, value: &J, out: &mut impl std::fmt::Write) -> std::fmt::Result {
    gron_inner(root, value, out)
}

fn gron_inner(path: &str, value: &J, out: &mut impl std::fmt::Write) -> std::fmt::Result {
    match value {
        J::Object(map) => {
            if map.is_empty() {
                writeln!(out, "{path} = {{}}")?;
            }
            for (k, v) in map {
                let child = if is_bare_key(k) {
                    format!("{path}.{k}")
                } else {
                    format!("{path}[{k:?}]")
                };
                gron_inner(&child, v, out)?;
            }
            Ok(())
        }
        J::Array(items) => {
            if items.is_empty() {
                writeln!(out, "{path} = []")?;
            }
            for (i, v) in items.iter().enumerate() {
                gron_inner(&format!("{path}[{i}]"), v, out)?;
            }
            Ok(())
        }
        other => writeln!(out, "{path} = {}", scalar(other)),
    }
}

/// A key that can appear after a dot without quoting.
fn is_bare_key(k: &str) -> bool {
    !k.is_empty()
        && !k.starts_with(|c: char| c.is_ascii_digit())
        && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn scalar(v: &J) -> String {
    match v {
        // Strings are JSON-quoted so a value containing a newline or a tab
        // cannot break the one-leaf-per-line rule the whole format rests on.
        J::String(s) => J::String(s.clone()).to_string(),
        other => other.to_string(),
    }
}

// --------------------------------------------------------------- outline --

/// Write an indented tree, for a human reading rather than grepping.
pub fn outline(value: &J, out: &mut impl std::fmt::Write) -> std::fmt::Result {
    outline_inner(value, 0, out)
}

fn outline_inner(value: &J, depth: usize, out: &mut impl std::fmt::Write) -> std::fmt::Result {
    let pad = "  ".repeat(depth);
    match value {
        J::Object(map) => {
            for (k, v) in map {
                match v {
                    J::Object(_) | J::Array(_) => {
                        writeln!(out, "{pad}{k}{}", shape(v))?;
                        outline_inner(v, depth + 1, out)?;
                    }
                    _ => writeln!(out, "{pad}{k:<26} {}", scalar(v))?,
                }
            }
            Ok(())
        }
        J::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                match v {
                    J::Object(_) | J::Array(_) => {
                        writeln!(out, "{pad}[{i}]{}", shape(v))?;
                        outline_inner(v, depth + 1, out)?;
                    }
                    _ => writeln!(out, "{pad}[{i}] {}", scalar(v))?,
                }
            }
            Ok(())
        }
        other => writeln!(out, "{pad}{}", scalar(other)),
    }
}

/// A short size hint so a collapsed branch still says how big it is.
fn shape(v: &J) -> String {
    match v {
        J::Array(a) => format!("[{}]", a.len()),
        J::Object(o) => format!(" ({} fields)", o.len()),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gron_lines_are_self_locating() {
        let v = json!({"ClassList": [{"Class": 3, "ClassLevel": 15}], "Tag": "bastila"});
        let mut s = String::new();
        gron("n_bastila.utc", &v, &mut s).unwrap();
        assert!(s.contains("n_bastila.utc.ClassList[0].Class = 3\n"));
        assert!(s.contains("n_bastila.utc.Tag = \"bastila\"\n"));
    }

    #[test]
    fn every_gron_line_holds_exactly_one_leaf() {
        let v = json!({"a": {"b": [1, 2]}, "c": "x\ny"});
        let mut s = String::new();
        gron("r", &v, &mut s).unwrap();
        assert_eq!(s.lines().count(), 3, "{s}");
        // A newline inside a value must not become a line break.
        assert!(s.contains(r#"r.c = "x\ny""#));
    }

    #[test]
    fn empty_containers_still_emit_a_line() {
        let mut s = String::new();
        gron("r", &json!({"a": [], "b": {}}), &mut s).unwrap();
        assert!(s.contains("r.a = []"));
        assert!(s.contains("r.b = {}"));
    }

    #[test]
    fn non_identifier_keys_are_bracketed() {
        let mut s = String::new();
        gron("r", &json!({"has space": 1, "2col": 2}), &mut s).unwrap();
        assert!(s.contains(r#"r["has space"] = 1"#));
        assert!(s.contains(r#"r["2col"] = 2"#));
    }
}
