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

use crate::gff::{Gff, Struct, Value};
use crate::tlk::Tlk;
use crate::twoda::TwoDa;

// ---------------------------------------------------------------- to JSON --

/// Convert a GFF tree to JSON.
///
/// Scalars become plain JSON scalars so `jq` queries read naturally
/// (`.Appearance_Type`, not `.Appearance_Type.value`). The types that carry
/// more than a number — localized strings, byte blobs, vectors — keep a small
/// tagged object, because collapsing those would lose the StrRef that makes
/// them meaningful.
pub fn gff_to_json(gff: &Gff) -> J {
    struct_to_json(&gff.root)
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
