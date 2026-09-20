//! KotOR / Neverwinter Nights MDL — ASCII, JSON, and binary (with MDX).
//!
//! The interchange IR is [`Model`]. ASCII is the mdlops / NWN text dialect
//! (`newmodel`, `node trimesh`, `newanim`). JSON is a 1:1 serde of that tree.
//! Binary files are decoded into the same IR so `kq cat` / `kq export` no
//! longer stop at an inventory of names.
//!
//! This crate implements the published formats in original Rust. It is not a
//! port of [mdlops](https://github.com/ndixUR/mdlops) (GPL-3.0).

mod ascii;
mod binary;
mod model;

pub use model::*;

use std::path::Path;

use crate::error::{FormatError, Result};

pub fn sniff(data: &[u8]) -> bool {
    sniff_ascii(data) || sniff_binary(data)
}

pub fn sniff_ascii(data: &[u8]) -> bool {
    ascii::sniff(data)
}

pub fn sniff_binary(data: &[u8]) -> bool {
    binary::sniff(data)
}

/// Collect texture / supermodel name strings without decoding meshes or reading MDX.
pub fn texture_refs(bytes: &[u8]) -> Vec<String> {
    if sniff_ascii(bytes) {
        texture_refs_ascii(bytes)
    } else if sniff_binary(bytes) {
        texture_refs_binary(bytes)
    } else {
        Vec::new()
    }
}

fn texture_refs_ascii(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let mut out = Vec::new();
    for (key, idx) in [
        ("bitmap ", 7usize),
        ("lightmap ", 9),
        ("setsupermodel ", 14),
        ("texture0 ", 9),
        ("texture1 ", 9),
        ("texture2 ", 9),
    ] {
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix(key) {
                // setsupermodel <model> <supermodel> — take the last token, not the model name.
                let name = if key == "setsupermodel " {
                    rest.split_whitespace().last().unwrap_or("")
                } else {
                    rest.split_whitespace().next().unwrap_or("")
                };
                if !name.is_empty() && !name.eq_ignore_ascii_case("null") {
                    out.push(name.to_ascii_lowercase());
                }
            }
        }
        let _ = idx;
    }
    out.sort();
    out.dedup();
    out
}

fn texture_refs_binary(bytes: &[u8]) -> Vec<String> {
    // binary::read(mdx=None) still allocates vertex/face arrays (up to MAX_VERTS).
    // Walk node headers and trimesh bitmap/lightmap fields only.
    binary::texture_refs(bytes)
}

/// Read ASCII or binary MDL. Binary meshes that store verts in MDX will
/// record a warning unless [`read_with_mdx`] is used.
pub fn read(data: &[u8], path: &Path) -> Result<Model> {
    if sniff_ascii(data) {
        ascii::read(data, path)
    } else {
        binary::read(data, None, path)
    }
}

/// Read a binary MDL together with its companion MDX vertex buffer.
pub fn read_with_mdx(mdl: &[u8], mdx: &[u8], path: &Path) -> Result<Model> {
    if sniff_ascii(mdl) {
        ascii::read(mdl, path)
    } else {
        binary::read(mdl, Some(mdx), path)
    }
}

pub fn read_ascii(data: &[u8], path: &Path) -> Result<Model> {
    ascii::read(data, path)
}

pub fn write_ascii(model: &Model) -> String {
    ascii::write(model)
}

pub fn read_json(data: &[u8], path: &Path) -> Result<Model> {
    serde_json::from_slice(data).map_err(|e| FormatError::Malformed {
        path: path.to_path_buf(),
        message: format!("MDL JSON: {e}"),
    })
}

pub fn write_json(model: &Model) -> Result<String> {
    serde_json::to_string_pretty(model).map_err(|e| FormatError::Malformed {
        path: Path::new("<mdl-json>").to_path_buf(),
        message: format!("MDL JSON: {e}"),
    })
}

pub fn to_json_value(model: &Model) -> serde_json::Value {
    serde_json::to_value(model).unwrap_or(serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const ASCII: &str = r#"
filedependancy test NULL.mlk
newmodel test
setsupermodel test NULL
classification character
ignorefog 1
setanimationscale 1
beginmodelgeom test
  bmin -1 -1 -1
  bmax 1 1 1
  radius 1.732
  node dummy test
  {
    parent NULL
    position 0 0 0
    orientation 0 0 0 1
  }
  node trimesh mesh
  {
    parent test
    position 0 1 0
    orientation 0 0 0 1
    bitmap cm_baremetal
    render 1
    verts 3
      0 0 0 0
      1 1 0 0
      2 0 1 0
    faces 1
      0 1 2 1 0 1 2 0
  }
endmodelgeom test
newanim pause test
  length 1
  transtime 0.25
  animroot test
  event 0.5 blast
  node dummy test
  {
    parent NULL
    positionkey
      0 0 0 0
      1 0 1 0
    endlist
  }
doneanim pause test
donemodel test
"#;

    #[test]
    fn sniff_ascii_newmodel() {
        assert!(sniff_ascii(ASCII.as_bytes()));
        assert!(!sniff_binary(ASCII.as_bytes()));
    }

    #[test]
    fn sniff_binary_layout_token() {
        let mut buf = vec![0u8; 20];
        buf[12..16].copy_from_slice(&4_273_776u32.to_le_bytes());
        assert!(sniff_binary(&buf));
        buf[12..16].copy_from_slice(&1u32.to_le_bytes());
        assert!(!sniff_binary(&buf));
    }

    #[test]
    fn ascii_roundtrip_preserves_tree() {
        let path = Path::new("test.mdl");
        let first = read_ascii(ASCII.as_bytes(), path).unwrap();
        assert_eq!(first.name, "test");
        assert_eq!(first.classification, "character");
        assert_eq!(first.supermodel, "NULL");
        let root = first.root.as_deref().expect("root");
        assert_eq!(root.name, "test");
        assert_eq!(root.children.len(), 1);
        let mesh = root.children[0].mesh.as_ref().expect("trimesh");
        assert_eq!(mesh.verts.len(), 3);
        assert_eq!(mesh.faces.len(), 1);
        assert_eq!(mesh.bitmap, "cm_baremetal");
        assert_eq!(first.animations.len(), 1);
        assert_eq!(first.animations[0].name, "pause");
        assert_eq!(first.animations[0].events[0].name, "blast");
        assert_eq!(first.animations[0].nodes.len(), 1);
        assert_eq!(first.animations[0].nodes[0].controllers.len(), 1);
        assert_eq!(first.animations[0].nodes[0].controllers[0].name, "position");

        let written = write_ascii(&first);
        let second = read_ascii(written.as_bytes(), path).unwrap();
        assert_eq!(second.name, first.name);
        assert_eq!(second.node_names(), first.node_names());
        assert_eq!(second.animation_names(), first.animation_names());
        let mesh2 = second.root.as_ref().unwrap().children[0]
            .mesh
            .as_ref()
            .unwrap();
        assert_eq!(mesh2.verts.len(), 3);
        assert_eq!(mesh2.faces[0].v3, 2);
    }

    #[test]
    fn json_roundtrip() {
        let path = Path::new("test.mdl.json");
        let first = read_ascii(ASCII.as_bytes(), Path::new("test.mdl")).unwrap();
        let json = write_json(&first).unwrap();
        let second = read_json(json.as_bytes(), path).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn texture_refs_ascii_reads_bitmap_without_mdx() {
        let ascii = br#"
newmodel test
setsupermodel test NULL
beginmodelgeom test
  node trimesh mesh
  {
    bitmap cm_baremetal
    lightmap m01aa_lm
  }
endmodelgeom test
"#;
        let refs = texture_refs(ascii);
        assert!(refs.iter().any(|s| s == "cm_baremetal"));
        assert!(refs.iter().any(|s| s == "m01aa_lm"));
    }
}
