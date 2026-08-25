//! MDL — a model inventory: name, supermodel, classification, node names,
//! animation names. Not a mesh dumper.
//!
//! Binary files start with a 12-byte size header and have no magic number.
//! Offsets inside the model are relative to byte 12. ASCII MDLs (`newmodel`)
//! are already plain text and never reach this reader.

use std::path::Path;

use crate::error::Result;
use crate::reader::{decode_cstr, Reader};

const FILE_HEADER: usize = 12;
const GEOM_HEADER: usize = 80;
const NAME_WIDTH: usize = 32;

#[derive(Clone, Debug)]
pub struct Mdl {
    pub name: String,
    pub supermodel: String,
    pub classification: &'static str,
    pub node_count: u32,
    pub names: Vec<String>,
    pub animations: Vec<String>,
}

pub fn sniff(data: &[u8]) -> bool {
    // Binary MDL: first dword is usually 0, then two size fields.
    data.len() >= FILE_HEADER + GEOM_HEADER + 4 && data[..4] == [0, 0, 0, 0]
}

pub fn read(data: &[u8], path: &Path) -> Result<Mdl> {
    let mut r = Reader::new(data, path);
    if data.len() < FILE_HEADER + GEOM_HEADER + 80 {
        return Err(r.malformed("MDL file is shorter than a model header"));
    }

    r.seek(FILE_HEADER)?;
    let _layout0 = r.u32()?;
    let _layout1 = r.u32()?;
    let name = r.fixed_string_cased(NAME_WIDTH)?;
    let _root_node = r.u32()?;
    let node_count = r.u32()?;

    // model_type sits at the start of the model-header block (geom + 0).
    r.seek(FILE_HEADER + GEOM_HEADER)?;
    let model_type = r.u8()?;
    r.seek(FILE_HEADER + GEOM_HEADER + 4)?;
    let _child_model_count = r.u32()?;
    let offset_to_animations = r.u32()? as usize;
    let animation_count = r.u32()? as usize;
    let _animation_count2 = r.u32()?;
    let _parent = r.u32()?;
    // bbox min, bbox max, radius, anim_scale
    r.seek(r.position() + 12 + 12 + 4 + 4)?;
    let supermodel = r.fixed_string(NAME_WIDTH)?;
    let _super_root = r.u32()?;
    let _mdx_buf = r.u32()?;
    let _mdx_size = r.u32()?;
    let _mdx_offset = r.u32()?;
    let offset_to_name_offsets = r.u32()? as usize;
    let name_count = r.u32()? as usize;

    let names = read_name_table(&r, data, offset_to_name_offsets, name_count)?;
    let animations = read_animation_names(&r, data, offset_to_animations, animation_count)?;

    Ok(Mdl {
        name,
        supermodel,
        classification: classification(model_type),
        node_count,
        names,
        animations,
    })
}

fn rel(offset: usize) -> usize {
    FILE_HEADER + offset
}

fn read_name_table(
    r: &Reader<'_>,
    data: &[u8],
    offset: usize,
    count: usize,
) -> Result<Vec<String>> {
    if count == 0 {
        return Ok(Vec::new());
    }
    if count > 4096 {
        return Err(r.malformed(format!("implausible MDL name count {count}")));
    }
    let table_at = rel(offset);
    let mut names = Vec::with_capacity(count);
    for i in 0..count {
        let entry = table_at + i * 4;
        let raw = r.slice_at(entry, 4)?;
        let name_off = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
        names.push(decode_cstr(data, rel(name_off)));
    }
    Ok(names)
}

fn read_animation_names(
    r: &Reader<'_>,
    _data: &[u8],
    offset: usize,
    count: usize,
) -> Result<Vec<String>> {
    if count == 0 {
        return Ok(Vec::new());
    }
    if count > 2048 {
        return Err(r.malformed(format!("implausible MDL animation count {count}")));
    }
    let table_at = rel(offset);
    let mut names = Vec::with_capacity(count);
    for i in 0..count {
        let entry = table_at + i * 4;
        let raw = r.slice_at(entry, 4)?;
        let anim_off = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
        // Geometry header: two u32 tokens, then a 32-byte name.
        let name_at = rel(anim_off) + 8;
        let bytes = r.slice_at(name_at, NAME_WIDTH)?;
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        names.push(String::from_utf8_lossy(&bytes[..end]).trim().to_string());
    }
    Ok(names)
}

fn classification(model_type: u8) -> &'static str {
    match model_type {
        0x00 => "other",
        0x01 => "effect",
        0x02 => "tile",
        0x04 => "character",
        0x08 => "door",
        0x10 => "lightsaber",
        0x20 => "placeable",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_rejects_ascii() {
        assert!(!sniff(b"newmodel p_bastila\n"));
    }
}
