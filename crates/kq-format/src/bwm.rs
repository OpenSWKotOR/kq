//! BWM / WOK / DWK / PWK — walkmesh collision geometry.
//!
//! Layout (V1.0): `"BWM "`, `"V1.0"`, a type id, five hook/position float3s,
//! then a table of counts and offsets into vertices, faces, materials and
//! area-transition edges.

use std::path::Path;

use crate::error::Result;
use crate::reader::Reader;

#[derive(Clone, Copy, Debug)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            x: r.f32()?,
            y: r.f32()?,
            z: r.f32()?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Face {
    pub vertices: [u32; 3],
    pub material: u32,
    /// Area-transition ids per edge, `None` when the edge stays in-area.
    pub transitions: [Option<u32>; 3],
}

#[derive(Clone, Debug)]
pub struct Bwm {
    pub walkmesh_type: u32,
    pub relative_hook1: Vec3,
    pub relative_hook2: Vec3,
    pub absolute_hook1: Vec3,
    pub absolute_hook2: Vec3,
    pub position: Vec3,
    pub vertices: Vec<Vec3>,
    pub faces: Vec<Face>,
}

pub fn sniff(data: &[u8]) -> bool {
    data.starts_with(b"BWM ")
}

pub fn read(data: &[u8], path: &Path) -> Result<Bwm> {
    let mut r = Reader::new(data, path);
    r.expect_signature("BWM V1.0")?;
    r.seek(8)?;
    let walkmesh_type = r.u32()?;
    let relative_hook1 = Vec3::read(&mut r)?;
    let relative_hook2 = Vec3::read(&mut r)?;
    let absolute_hook1 = Vec3::read(&mut r)?;
    let absolute_hook2 = Vec3::read(&mut r)?;
    let position = Vec3::read(&mut r)?;

    let vertices_count = r.u32()? as usize;
    let vertices_offset = r.u32()? as usize;
    let face_count = r.u32()? as usize;
    let indices_offset = r.u32()? as usize;
    let materials_offset = r.u32()? as usize;
    for _ in 0..7 {
        let _ = r.u32()?;
    }
    let edges_count = r.u32()? as usize;
    let edges_offset = r.u32()? as usize;
    let _ = r.u32()?;
    let _ = r.u32()?;

    r.seek(vertices_offset)?;
    let mut vertices = Vec::with_capacity(vertices_count);
    for _ in 0..vertices_count {
        vertices.push(Vec3::read(&mut r)?);
    }

    r.seek(indices_offset)?;
    let mut faces = Vec::with_capacity(face_count);
    for _ in 0..face_count {
        faces.push(Face {
            vertices: [r.u32()?, r.u32()?, r.u32()?],
            material: 0,
            transitions: [None, None, None],
        });
    }

    r.seek(materials_offset)?;
    for face in &mut faces {
        face.material = r.u32()?;
    }

    if edges_count > 0 && edges_offset < data.len() {
        r.seek(edges_offset)?;
        for _ in 0..edges_count {
            let edge_index = r.u32()? as usize;
            let transition = r.u32()?;
            if transition == 0xFFFF_FFFF {
                continue;
            }
            let face_index = edge_index / 3;
            let edge = edge_index % 3;
            if let Some(face) = faces.get_mut(face_index) {
                face.transitions[edge] = Some(transition);
            }
        }
    }

    Ok(Bwm {
        walkmesh_type,
        relative_hook1,
        relative_hook2,
        absolute_hook1,
        absolute_hook2,
        position,
        vertices,
        faces,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn empty_walkmesh() {
        let mut data = vec![0u8; 136];
        data[..8].copy_from_slice(b"BWM V1.0");
        // vertices/faces at 136, counts 0 — already zeroed
        let w = read(&data, Path::new("empty.wok")).unwrap();
        assert!(w.vertices.is_empty());
        assert!(w.faces.is_empty());
    }
}
