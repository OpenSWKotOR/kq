//! Binary Odyssey MDL/MDX reader.
//!
//! Retail files start with a 12-byte preamble; every offset in the file is
//! relative to the first byte after that preamble. Mesh vertex attributes
//! usually live in a companion MDX buffer (same ResRef, type `mdx`).
//!
//! This is an original implementation of the published on-disk layout. It is
//! not a port of mdlops (GPL-3.0).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::model::*;
use crate::error::{FormatError, Result};
use crate::reader::{decode_cstr, Reader};

const PREAMBLE: usize = 12;

/// Geometry-header layout tokens used by retail K1 / K2 compilers.
const K1_LAYOUT0: u32 = 4_273_776;
const K2_LAYOUT0: u32 = 4_285_200;
const K1_LAYOUT1: u32 = 4_216_096;
const K2_LAYOUT1: u32 = 4_216_320;

const NODE_HEADER: usize = 80;
const FACE_SIZE: usize = 32;
const CONTROLLER_SIZE: usize = 16;
const TRIMESH_K1: usize = 332;
const TRIMESH_K2: usize = 340;

const FLAG_HEADER: u16 = 0x0001;
const FLAG_LIGHT: u16 = 0x0002;
const FLAG_EMITTER: u16 = 0x0004;
const FLAG_REFERENCE: u16 = 0x0010;
const FLAG_MESH: u16 = 0x0020;
const FLAG_SKIN: u16 = 0x0040;
const FLAG_DANGLY: u16 = 0x0100;
const FLAG_AABB: u16 = 0x0200;
const FLAG_SABER: u16 = 0x0800;

const MDX_VERTEX: u32 = 0x0000_0001;
const MDX_TEX0: u32 = 0x0000_0002;
const MDX_TEX1: u32 = 0x0000_0004;
const MDX_NORMAL: u32 = 0x0000_0020;

const MAX_ANIMS: usize = 4_096;
const MAX_NAMES: usize = 50_000;
const MAX_CHILDREN: usize = 4_096;
const MAX_VERTS: usize = 200_000;
const MAX_FACES: usize = 200_000;
const MAX_CONTROLLERS: usize = 4_096;
const MAX_AABB: usize = 50_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Game {
    K1,
    K2,
}

pub fn sniff(data: &[u8]) -> bool {
    if data.len() < PREAMBLE + 8 {
        return false;
    }
    if data[..4] != [0, 0, 0, 0] {
        return false;
    }
    let t0 = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);
    t0 == K1_LAYOUT0 || t0 == K2_LAYOUT0
}

pub fn read(mdl: &[u8], mdx: Option<&[u8]>, path: &Path) -> Result<Model> {
    if mdl.len() < PREAMBLE + NODE_HEADER {
        return Err(FormatError::Truncated {
            path: path.to_path_buf(),
            offset: 0,
            needed: PREAMBLE + NODE_HEADER,
            len: mdl.len(),
        });
    }
    let geom = &mdl[PREAMBLE..];
    let mut ctx = Ctx {
        geom,
        mdx: mdx.unwrap_or(&[]),
        path: path.to_path_buf(),
        names: Vec::new(),
        game: Game::K1,
        compress_quaternions: 0,
        warnings: Vec::new(),
        visited: HashSet::new(),
        missing_mdx: false,
    };
    ctx.load()
}

struct Ctx<'a> {
    geom: &'a [u8],
    mdx: &'a [u8],
    path: PathBuf,
    names: Vec<String>,
    game: Game,
    compress_quaternions: i32,
    warnings: Vec<String>,
    visited: HashSet<usize>,
    missing_mdx: bool,
}

impl<'a> Ctx<'a> {
    fn reader(&self) -> Reader<'a> {
        Reader::new(self.geom, &self.path)
    }

    fn at(&self, off: usize) -> Result<Reader<'a>> {
        let mut r = self.reader();
        r.seek(off)?;
        Ok(r)
    }

    fn load(&mut self) -> Result<Model> {
        let mut r = self.reader();
        let layout0 = r.u32()?;
        let layout1 = r.u32()?;
        self.game = if layout0 == K2_LAYOUT0 && layout1 == K2_LAYOUT1 {
            Game::K2
        } else if layout0 == K1_LAYOUT0 && layout1 == K1_LAYOUT1 {
            Game::K1
        } else if layout0 == K2_LAYOUT0 {
            Game::K2
        } else {
            Game::K1
        };
        let name = r.fixed_string_cased(32)?;
        let root_off = r.u32()? as usize;
        let _node_count = r.u32()?;
        r.take(28)?; // unknown
        let _geom_type = r.u8()?;
        r.take(3)?; // padding

        // Model header follows the 80-byte geometry header.
        let model_type = r.u8()?;
        let subclass = r.u8()?;
        let _pad = r.u8()?;
        let fog = r.u8()?;
        let _child_models = r.u32()?;
        let anims_off = r.u32()? as usize;
        let anim_count = sane(r.u32()?, MAX_ANIMS);
        let _anim_count2 = r.u32()?;
        let _parent_ptr = r.u32()?;
        let bmin = read_vec3(&mut r)?;
        let bmax = read_vec3(&mut r)?;
        let radius = r.f32()?;
        let anim_scale = r.f32()?;
        let supermodel = r.fixed_string_cased(32)?;
        let super_root = r.u32()? as usize;
        let _mdx_buf = r.u32()?;
        let _mdx_size = r.u32()?;
        let _mdx_off = r.u32()?;
        let name_offs_at = r.u32()? as usize;
        let name_count = sane(r.u32()?, MAX_NAMES);
        let _name_count2 = r.u32()?;

        self.names = self.load_names(name_offs_at, name_count)?;
        let root = self.load_node(root_off, None)?;

        let mut animations = Vec::with_capacity(anim_count);
        if anim_count > 0 && ptr_ok(anims_off, self.geom.len(), anim_count * 4) {
            let mut ar = self.at(anims_off)?;
            let mut offs = Vec::with_capacity(anim_count);
            for _ in 0..anim_count {
                offs.push(ar.u32()? as usize);
            }
            for off in offs {
                if let Some(a) = self.load_anim(off)? {
                    animations.push(a);
                }
            }
        }

        let mut headlink = String::new();
        if super_root != root_off && has_named(&root, "neck_g") {
            headlink = "1".into();
        }

        if self.missing_mdx {
            self.warnings.push(
                "mesh vertices live in a companion .mdx; none was provided \
                 (pass the same-ResRef MDX to read_with_mdx)"
                    .into(),
            );
        }

        Ok(Model {
            name,
            supermodel,
            classification: classification_from_byte(model_type).to_string(),
            classification_unk1: subclass as i32,
            ignorefog: if fog == 0 { 1 } else { 0 },
            compress_quaternions: self.compress_quaternions,
            headlink,
            animation_scale: anim_scale,
            bmin,
            bmax,
            radius,
            game: match self.game {
                Game::K1 => "k1".into(),
                Game::K2 => "k2".into(),
            },
            root: Some(Box::new(root)),
            animations,
            warnings: std::mem::take(&mut self.warnings),
            ..Model::default()
        })
    }

    fn load_names(&self, table: usize, count: usize) -> Result<Vec<String>> {
        if count == 0 || !ptr_ok(table, self.geom.len(), count * 4) {
            return Ok(Vec::new());
        }
        let mut r = self.at(table)?;
        let mut offs = Vec::with_capacity(count);
        for _ in 0..count {
            offs.push(r.u32()? as usize);
        }
        let mut names = Vec::with_capacity(count);
        for off in offs {
            if off < self.geom.len() {
                names.push(decode_cstr(self.geom, off));
            } else {
                names.push(String::new());
            }
        }
        Ok(names)
    }

    fn load_anim(&mut self, off: usize) -> Result<Option<Animation>> {
        if !ptr_ok(off, self.geom.len(), 80 + 56) {
            return Ok(None);
        }
        let mut r = self.at(off)?;
        let _l0 = r.u32()?;
        let _l1 = r.u32()?;
        let name = r.fixed_string_cased(32)?;
        let root_off = r.u32()? as usize;
        let _node_count = r.u32()?;
        r.take(28)?;
        let _geom_type = r.u8()?;
        r.take(3)?;
        let length = r.f32()?;
        let transtime = r.f32()?;
        let root_model = r.fixed_string_cased(32)?;
        let events_off = r.u32()? as usize;
        let event_count = sane(r.u32()?, 1_024);
        let _event_count2 = r.u32()?;
        let _unknown = r.u32()?;

        let mut events = Vec::new();
        if event_count > 0 && ptr_ok(events_off, self.geom.len(), event_count * 36) {
            let mut er = self.at(events_off)?;
            for _ in 0..event_count {
                events.push(Event {
                    time: er.f32()?,
                    name: er.fixed_string_cased(32)?,
                });
            }
        }

        // Animation nodes are a separate tree; do not treat their offsets as
        // already-visited geometry nodes.
        let saved = std::mem::take(&mut self.visited);
        let root = self.load_node(root_off, None)?;
        self.visited = saved;

        Ok(Some(Animation {
            name,
            root_model,
            length,
            transtime,
            events,
            nodes: vec![root],
        }))
    }

    fn load_node(&mut self, off: usize, parent: Option<&str>) -> Result<Node> {
        if !self.visited.insert(off) {
            return Err(FormatError::Malformed {
                path: self.path.clone(),
                message: format!("MDL node cycle at offset {off}"),
            });
        }
        if !ptr_ok(off, self.geom.len(), NODE_HEADER) {
            return Err(FormatError::Truncated {
                path: self.path.clone(),
                offset: off,
                needed: NODE_HEADER,
                len: self.geom.len(),
            });
        }

        let mut r = self.at(off)?;
        let flags = r.u16()?;
        let _pad = r.u16()?;
        let node_id = r.u16()? as i32;
        let name_id = r.u16()? as i32;
        let _root_ptr = r.u32()?;
        let _parent_ptr = r.u32()?;
        let position = read_vec3(&mut r)?;
        let ow = r.f32()?;
        let ox = r.f32()?;
        let oy = r.f32()?;
        let oz = r.f32()?;
        let children_off = r.u32()? as usize;
        let child_count = sane(r.u32()?, MAX_CHILDREN);
        let _child_count2 = r.u32()?;
        let controllers_off = r.u32()? as usize;
        let controller_count = sane(r.u32()?, MAX_CONTROLLERS);
        let _controller_count2 = r.u32()?;
        let controller_data_off = r.u32()? as usize;
        let _controller_data_len = r.u32()?;
        let _controller_data_len2 = r.u32()?;

        let kind = NodeKind::from_flags(flags);
        let name = self
            .names
            .get(node_id as usize)
            .cloned()
            .or_else(|| self.names.get(name_id as usize).cloned())
            .unwrap_or_default();

        let mut node = Node {
            kind,
            name: name.clone(),
            parent: parent.map(str::to_string),
            node_id,
            position,
            orientation: Quat {
                x: ox,
                y: oy,
                z: oz,
                w: ow,
            },
            ..Node::default()
        };

        let mut trimesh = None;
        if flags & FLAG_MESH != 0 {
            trimesh = Some(self.read_trimesh(&mut r)?);
        }
        let mut skin = None;
        if flags & FLAG_SKIN != 0 {
            skin = Some(self.read_skin(&mut r)?);
        }
        if flags & FLAG_LIGHT != 0 {
            node.light = Some(self.read_light(&mut r)?);
        }
        if flags & FLAG_EMITTER != 0 {
            node.emitter = Some(self.read_emitter(&mut r)?);
        }
        if flags & FLAG_REFERENCE != 0 {
            node.reference = Some(self.read_reference(&mut r)?);
        }
        let mut dangly = None;
        if flags & FLAG_DANGLY != 0 {
            dangly = Some(self.read_dangly(&mut r)?);
        }
        let mut aabb_off = 0usize;
        if flags & FLAG_AABB != 0 {
            let raw = r.i32()?;
            if raw > 0 {
                aabb_off = raw as usize;
            }
        }

        if let Some(tm) = trimesh {
            let mut mesh = self.build_mesh(&tm)?;
            if let Some(d) = dangly {
                mesh.displacement = d.displacement;
                mesh.tightness = d.tightness;
                mesh.period = d.period;
                mesh.constraints = self.read_f32s(d.constraints_off, d.constraints_count)?;
            }
            if let Some(s) = skin {
                self.apply_skin(&mut mesh, &tm, &s)?;
            }
            if tm.dirt_enabled != 0 {
                node.extras.insert(
                    "dirt_enabled".into(),
                    Property::Number(tm.dirt_enabled as f64),
                );
            }
            if tm.hologram_donotdraw != 0 {
                node.extras.insert(
                    "hologram_donotdraw".into(),
                    Property::Number(tm.hologram_donotdraw as f64),
                );
            }
            node.mesh = Some(mesh);
        }

        if aabb_off > 0 {
            node.aabb = self.read_aabb(aabb_off)?;
        }

        if controller_count > 0
            && ptr_ok(
                controllers_off,
                self.geom.len(),
                controller_count * CONTROLLER_SIZE,
            )
        {
            node.controllers = self.read_controllers(
                controllers_off,
                controller_data_off,
                controller_count,
                flags,
            )?;
        }

        if child_count > 0 && ptr_ok(children_off, self.geom.len(), child_count * 4) {
            let mut cr = self.at(children_off)?;
            let mut child_offs = Vec::with_capacity(child_count);
            for _ in 0..child_count {
                child_offs.push(cr.u32()? as usize);
            }
            for child_off in child_offs {
                if ptr_ok(child_off, self.geom.len(), NODE_HEADER) {
                    node.children.push(self.load_node(child_off, Some(&name))?);
                }
            }
        }

        let _ = FLAG_HEADER;
        let _ = FLAG_SABER;
        Ok(node)
    }

    fn read_trimesh(&mut self, r: &mut Reader<'_>) -> Result<TriMesh> {
        let start = r.position();
        let _l0 = r.u32()?;
        let _l1 = r.u32()?;
        let faces_off = r.u32()? as usize;
        let faces_count = sane(r.u32()?, MAX_FACES);
        let _faces_count2 = r.u32()?;
        let bmin = read_vec3(r)?;
        let bmax = read_vec3(r)?;
        let radius = r.f32()?;
        let average = read_vec3(r)?;
        let diffuse = read_vec3(r)?;
        let ambient = read_vec3(r)?;
        let transparencyhint = r.u32()? as i32;
        let bitmap = r.fixed_string_cased(32)?;
        let lightmap = r.fixed_string_cased(32)?;
        r.take(24)?; // unknown
        let _idx_counts_off = r.u32()?;
        let _idx_counts = r.u32()?;
        let _idx_counts2 = r.u32()?;
        let _idx_offs_off = r.u32()?;
        let _idx_offs = r.u32()?;
        let _idx_offs2 = r.u32()?;
        let _counters_off = r.u32()?;
        let _counters = r.u32()?;
        let _counters2 = r.u32()?;
        r.take(12)?; // unknown {-1,-1,0}
        r.take(8)?; // saber unknowns
        let _unknown2 = r.i32()?;
        let _uv_dir_x = r.f32()?;
        let _uv_dir_y = r.f32()?;
        let _uv_jitter = r.f32()?;
        let _uv_speed = r.f32()?;
        let mdx_data_size = i32_as_u32(r.i32()?);
        let mdx_data_bitmap = i32_as_u32(r.i32()?);
        let mdx_vertex_off = i32_as_u32(r.i32()?);
        let mdx_normal_off = i32_as_u32(r.i32()?);
        let _mdx_color_off = i32_as_u32(r.i32()?);
        let mdx_tex0_off = i32_as_u32(r.i32()?);
        let mdx_tex1_off = i32_as_u32(r.i32()?);
        r.take(4 * 6)?; // remaining MDX offsets
        let vertex_count = r.u16()? as usize;
        let _texture_count = r.u16()?;
        let lightmapped = r.u8()? as i32;
        let rotatetexture = r.u8()? as i32;
        let backgroundgeometry = r.u8()? as i32;
        let shadow = r.u8()? as i32;
        let beaming = r.u8()? as i32;
        let render = r.u8()? as i32;
        let mut dirt_enabled = 0i32;
        let mut hologram_donotdraw = 0i32;
        if self.game == Game::K2 {
            dirt_enabled = r.u8()? as i32;
            let _pad = r.u8()?;
            let _dirt_tex = r.i16()?;
            let _dirt_ws = r.i16()?;
            hologram_donotdraw = if r.u32()? == 1 { 1 } else { 0 };
            let _k2a = r.u32()?;
            let _k2b = r.u32()?;
        } else {
            let _tail_short = r.u16()?;
        }
        let area = r.f32()?;
        let _tail_long = r.u32()?;
        let mdx_data_offset = r.u32()? as usize;
        let vertices_offset = r.u32()? as usize;

        let expect = if self.game == Game::K1 {
            TRIMESH_K1
        } else {
            TRIMESH_K2
        };
        r.seek(start + expect)?;

        Ok(TriMesh {
            faces_off,
            faces_count,
            bmin,
            bmax,
            radius,
            average,
            diffuse,
            ambient,
            transparencyhint,
            bitmap,
            lightmap,
            mdx_data_size,
            mdx_data_bitmap,
            mdx_vertex_off,
            mdx_normal_off,
            mdx_tex0_off,
            mdx_tex1_off,
            vertex_count: vertex_count.min(MAX_VERTS),
            lightmapped,
            rotatetexture,
            backgroundgeometry,
            shadow,
            beaming,
            render,
            area,
            mdx_data_offset,
            vertices_offset,
            dirt_enabled,
            hologram_donotdraw,
        })
    }

    fn build_mesh(&mut self, tm: &TriMesh) -> Result<Mesh> {
        let mut faces = Vec::new();
        if tm.faces_count > 0 && ptr_ok(tm.faces_off, self.geom.len(), tm.faces_count * FACE_SIZE) {
            let mut r = self.at(tm.faces_off)?;
            for _ in 0..tm.faces_count {
                let _n = read_vec3(&mut r)?;
                let _plane = r.f32()?;
                let material = r.u32()? as i32;
                let _a1 = r.u16()?;
                let _a2 = r.u16()?;
                let _a3 = r.u16()?;
                let v1 = r.u16()? as i32;
                let v2 = r.u16()? as i32;
                let v3 = r.u16()? as i32;
                faces.push(Face {
                    v1,
                    v2,
                    v3,
                    smooth: 1,
                    t1: v1,
                    t2: v2,
                    t3: v3,
                    material,
                });
            }
        }

        let vcount = tm.vertex_count;
        let mut verts = vec![Vertex::default(); vcount];
        let mut tverts = Vec::new();
        let mut tverts1 = Vec::new();
        let mut got_positions = false;

        let want_mdx = tm.mdx_data_bitmap & MDX_VERTEX != 0
            && tm.mdx_data_size > 0
            && tm.mdx_data_offset != 0
            && tm.mdx_data_offset != u32::MAX as usize
            && vcount > 0;

        if want_mdx && self.mdx.is_empty() {
            self.missing_mdx = true;
        }

        if want_mdx && !self.mdx.is_empty() {
            let stride = tm.mdx_data_size as usize;
            let base = tm.mdx_data_offset;
            let voff = if tm.mdx_vertex_off == u32::MAX {
                0
            } else {
                tm.mdx_vertex_off as usize
            };
            for (i, vert) in verts.iter_mut().enumerate() {
                let row = base.saturating_add(i.saturating_mul(stride));
                if let Some(p) = mdx_vec3(self.mdx, row.saturating_add(voff)) {
                    vert.position = p;
                    got_positions = true;
                }
                if tm.mdx_data_bitmap & MDX_NORMAL != 0 {
                    if let Some(n) =
                        mdx_vec3(self.mdx, row.saturating_add(tm.mdx_normal_off as usize))
                    {
                        vert.normal = Some(n);
                    }
                }
                if tm.mdx_data_bitmap & MDX_TEX0 != 0 {
                    if let Some(uv) =
                        mdx_vec2(self.mdx, row.saturating_add(tm.mdx_tex0_off as usize))
                    {
                        vert.uv = Some(uv);
                        tverts.push(uv);
                    }
                }
                if tm.mdx_data_bitmap & MDX_TEX1 != 0 {
                    if let Some(uv) =
                        mdx_vec2(self.mdx, row.saturating_add(tm.mdx_tex1_off as usize))
                    {
                        vert.uv2 = Some(uv);
                        tverts1.push(uv);
                    }
                }
            }
        }

        if !got_positions && vcount > 0 && ptr_ok(tm.vertices_offset, self.geom.len(), vcount * 12)
        {
            let mut r = self.at(tm.vertices_offset)?;
            for v in &mut verts {
                v.position = read_vec3(&mut r)?;
            }
        }

        Ok(Mesh {
            bmin: Some(tm.bmin),
            bmax: Some(tm.bmax),
            radius: tm.radius,
            average: Some(tm.average),
            area: tm.area,
            ambient: Some(tm.ambient),
            diffuse: Some(tm.diffuse),
            transparencyhint: tm.transparencyhint,
            bitmap: tm.bitmap.clone(),
            lightmap: tm.lightmap.clone(),
            render: tm.render,
            shadow: tm.shadow,
            beaming: tm.beaming,
            backgroundgeometry: tm.backgroundgeometry,
            rotatetexture: tm.rotatetexture,
            lightmapped: tm.lightmapped,
            verts,
            faces,
            tverts,
            tverts1,
            ..Mesh::default()
        })
    }

    fn read_skin(&self, r: &mut Reader<'_>) -> Result<SkinHead> {
        let _u0 = r.i32()?;
        let _u1 = r.i32()?;
        let _u2 = r.i32()?;
        let mdx_weights = r.u32()? as usize;
        let mdx_bones = r.u32()? as usize;
        let bonemap_off = r.u32()? as usize;
        let bonemap_count = sane(r.u32()?, MAX_VERTS);
        let qbones_off = r.u32()? as usize;
        let qbones_count = sane(r.u32()?, 1_024);
        let _qc2 = r.u32()?;
        let tbones_off = r.u32()? as usize;
        let tbones_count = sane(r.u32()?, 1_024);
        let _tc2 = r.u32()?;
        r.take(12)?; // unknown array
        let mut palette = [0i32; 16];
        for p in &mut palette {
            *p = r.u16()? as i32;
        }
        let _pad = r.u32()?;
        Ok(SkinHead {
            mdx_weights,
            mdx_bones,
            bonemap_off,
            bonemap_count,
            qbones_off,
            qbones_count,
            tbones_off,
            tbones_count,
            palette,
        })
    }

    fn apply_skin(&self, mesh: &mut Mesh, tm: &TriMesh, skin: &SkinHead) -> Result<()> {
        let mut bonemap = Vec::new();
        if skin.bonemap_count > 0
            && ptr_ok(skin.bonemap_off, self.geom.len(), skin.bonemap_count * 4)
        {
            let mut r = self.at(skin.bonemap_off)?;
            for _ in 0..skin.bonemap_count {
                bonemap.push(r.f32()? as i32);
            }
        }

        let mut qbones = Vec::new();
        if skin.qbones_count > 0 && ptr_ok(skin.qbones_off, self.geom.len(), skin.qbones_count * 16)
        {
            let mut r = self.at(skin.qbones_off)?;
            for _ in 0..skin.qbones_count {
                qbones.push(Quat {
                    x: r.f32()?,
                    y: r.f32()?,
                    z: r.f32()?,
                    w: r.f32()?,
                });
            }
        }
        let mut tbones = Vec::new();
        if skin.tbones_count > 0 && ptr_ok(skin.tbones_off, self.geom.len(), skin.tbones_count * 12)
        {
            let mut r = self.at(skin.tbones_off)?;
            for _ in 0..skin.tbones_count {
                tbones.push(read_vec3(&mut r)?);
            }
        }
        for i in 0..qbones.len().max(tbones.len()) {
            mesh.bones.push(Bone {
                index: i as i32,
                bone: skin.palette.get(i).copied().unwrap_or(-1),
                orientation: qbones.get(i).cloned().unwrap_or_default(),
                translation: tbones.get(i).cloned().unwrap_or_default(),
            });
        }

        if self.mdx.is_empty() || tm.mdx_data_size == 0 {
            return Ok(());
        }
        let stride = tm.mdx_data_size as usize;
        let base = tm.mdx_data_offset;
        for i in 0..tm.vertex_count {
            let row = base.saturating_add(i.saturating_mul(stride));
            let mut influences = Vec::new();
            for k in 0..4 {
                let w =
                    mdx_f32(self.mdx, row.saturating_add(skin.mdx_weights + k * 4)).unwrap_or(0.0);
                let raw =
                    mdx_f32(self.mdx, row.saturating_add(skin.mdx_bones + k * 4)).unwrap_or(-1.0);
                if w <= 0.0 {
                    continue;
                }
                let idx = raw as i32;
                let mapped = bonemap.get(idx as usize).copied().unwrap_or(idx);
                let bone_name = self
                    .names
                    .get(mapped as usize)
                    .cloned()
                    .unwrap_or_else(|| format!("bone_{mapped}"));
                influences.push((bone_name, w));
            }
            if !influences.is_empty() {
                mesh.weights.push(Weight { influences });
            }
        }
        Ok(())
    }

    fn read_dangly(&self, r: &mut Reader<'_>) -> Result<DanglyHead> {
        Ok(DanglyHead {
            constraints_off: r.u32()? as usize,
            constraints_count: sane(r.u32()?, MAX_VERTS),
            _c2: r.u32()?,
            displacement: r.f32()?,
            tightness: r.f32()?,
            period: r.f32()?,
            _verts: r.u32()?,
        })
    }

    fn read_light(&self, r: &mut Reader<'_>) -> Result<Light> {
        let _unk_off = r.u32()?;
        let _unk_n = r.u32()?;
        let _unk_n2 = r.u32()?;
        let sizes_off = r.u32()? as usize;
        let sizes_n = sane(r.u32()?, 64);
        let _s2 = r.u32()?;
        let pos_off = r.u32()? as usize;
        let pos_n = sane(r.u32()?, 64);
        let _p2 = r.u32()?;
        let col_off = r.u32()? as usize;
        let col_n = sane(r.u32()?, 64);
        let _c2 = r.u32()?;
        let tex_off = r.u32()? as usize;
        let tex_n = sane(r.u32()?, 64);
        let _t2 = r.u32()?;
        let flareradius = r.f32()?;
        let lightpriority = r.u32()? as i32;
        let ambientonly = r.u32()? as i32;
        let ndynamictype = r.u32()? as i32;
        let affectdynamic = r.u32()? as i32;
        let shadow = r.u32()? as i32;
        let flare = r.u32()? as i32;
        let fadinglight = r.u32()? as i32;

        Ok(Light {
            flareradius,
            lightpriority,
            ambientonly,
            ndynamictype,
            affectdynamic,
            shadow,
            flare,
            fadinglight,
            flaresizes: self.read_f32s(sizes_off, sizes_n)?,
            flarepositions: self.read_f32s(pos_off, pos_n)?,
            flarecolorshifts: self.read_vec3s(col_off, col_n)?,
            texturenames: self.read_flare_textures(tex_off, tex_n)?,
        })
    }

    fn read_emitter(&self, r: &mut Reader<'_>) -> Result<Emitter> {
        let mut fields = std::collections::BTreeMap::new();
        let put_f = |m: &mut std::collections::BTreeMap<String, Property>, k: &str, v: f32| {
            m.insert(k.into(), Property::Number(v as f64));
        };
        let put_i = |m: &mut std::collections::BTreeMap<String, Property>, k: &str, v: i32| {
            m.insert(k.into(), Property::Number(v as f64));
        };
        let put_s = |m: &mut std::collections::BTreeMap<String, Property>, k: &str, v: String| {
            if !v.is_empty() {
                m.insert(k.into(), Property::Text(v));
            }
        };
        put_f(&mut fields, "deadSpace", r.f32()?);
        put_f(&mut fields, "blastRadius", r.f32()?);
        put_f(&mut fields, "blastLength", r.f32()?);
        put_i(&mut fields, "numBranches", r.u32()? as i32);
        put_i(&mut fields, "controlptsmoothing", r.u32()? as i32);
        put_i(&mut fields, "xgrid", r.u32()? as i32);
        put_i(&mut fields, "ygrid", r.u32()? as i32);
        put_i(&mut fields, "spawntype", r.u32()? as i32);
        put_s(&mut fields, "update", r.fixed_string_cased(32)?);
        put_s(&mut fields, "render", r.fixed_string_cased(32)?);
        put_s(&mut fields, "blend", r.fixed_string_cased(32)?);
        put_s(&mut fields, "texture", r.fixed_string_cased(32)?);
        put_s(&mut fields, "chunkname", r.fixed_string_cased(16)?);
        put_i(&mut fields, "twosidedtex", r.u32()? as i32);
        put_i(&mut fields, "loop", r.u32()? as i32);
        put_i(&mut fields, "renderorder", r.u16()? as i32);
        put_i(&mut fields, "frameBlending", r.u8()? as i32);
        put_s(&mut fields, "depth_texture", r.fixed_string_cased(32)?);
        let _pad = r.u8()?;
        put_i(&mut fields, "flags", r.u32()? as i32);
        Ok(Emitter { fields })
    }

    fn read_reference(&self, r: &mut Reader<'_>) -> Result<Reference> {
        Ok(Reference {
            refmodel: r.fixed_string_cased(32)?,
            reattachable: r.u32()? as i32,
        })
    }

    fn read_f32s(&self, off: usize, n: usize) -> Result<Vec<f32>> {
        if n == 0 || !ptr_ok(off, self.geom.len(), n * 4) {
            return Ok(Vec::new());
        }
        let mut r = self.at(off)?;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(r.f32()?);
        }
        Ok(out)
    }

    fn read_vec3s(&self, off: usize, n: usize) -> Result<Vec<Vec3>> {
        if n == 0 || !ptr_ok(off, self.geom.len(), n * 12) {
            return Ok(Vec::new());
        }
        let mut r = self.at(off)?;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(read_vec3(&mut r)?);
        }
        Ok(out)
    }

    fn read_flare_textures(&self, off: usize, n: usize) -> Result<Vec<String>> {
        if n == 0 || !ptr_ok(off, self.geom.len(), n * 4) {
            return Ok(Vec::new());
        }
        let mut r = self.at(off)?;
        let mut ptrs = Vec::with_capacity(n);
        for _ in 0..n {
            ptrs.push(r.u32()? as usize);
        }
        Ok(ptrs
            .into_iter()
            .filter(|&p| p < self.geom.len())
            .map(|p| decode_cstr(self.geom, p))
            .filter(|s| !s.is_empty())
            .collect())
    }

    fn read_aabb(&self, off: usize) -> Result<Vec<AabbNode>> {
        let mut out = Vec::new();
        let mut stack = vec![off];
        let mut seen = HashSet::new();
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur) || out.len() >= MAX_AABB {
                continue;
            }
            if !ptr_ok(cur, self.geom.len(), 40) {
                continue;
            }
            let mut r = self.at(cur)?;
            let bmin = read_vec3(&mut r)?;
            let bmax = read_vec3(&mut r)?;
            let left = r.i32()?;
            let right = r.i32()?;
            let face = r.i32()?;
            let most_significant = r.i32()?;
            out.push(AabbNode {
                bmin,
                bmax,
                face,
                most_significant,
                left,
                right,
            });
            if face == -1 {
                if left > 0 {
                    stack.push(left as usize);
                }
                if right > 0 {
                    stack.push(right as usize);
                }
            }
        }
        Ok(out)
    }

    fn read_controllers(
        &mut self,
        table: usize,
        data: usize,
        count: usize,
        flags: u16,
    ) -> Result<Vec<Controller>> {
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let mut r = self.at(table + i * CONTROLLER_SIZE)?;
            let type_id = r.u32()?;
            let _unk = r.u16()?;
            let row_count = r.u16()? as usize;
            let key_off = r.u16()? as usize;
            let data_off = r.u16()? as usize;
            let column_count = r.u8()? as usize;
            r.take(3)?;
            if type_id > 1_000 {
                continue;
            }
            let bezier = column_count & 0x10 != 0;
            let mut keys = Vec::new();
            let key_at = data.saturating_add(key_off.saturating_mul(4));
            if row_count > 0 && ptr_ok(key_at, self.geom.len(), row_count * 4) {
                let mut kr = self.at(key_at)?;
                for _ in 0..row_count.min(8_192) {
                    keys.push(kr.f32()?);
                }
            }
            let data_at = data.saturating_add(data_off.saturating_mul(4));
            let mut rows = Vec::new();
            if type_id == 20 && column_count == 2 {
                self.compress_quaternions = 1;
                if ptr_ok(data_at, self.geom.len(), keys.len() * 4) {
                    let mut dr = self.at(data_at)?;
                    for time in keys {
                        let packed = dr.u32()?;
                        let q = decompress_quat(packed);
                        rows.push(ControllerRow {
                            time,
                            data: vec![q.x, q.y, q.z, q.w],
                        });
                    }
                }
            } else {
                let cols = if bezier {
                    (column_count & !0x10).saturating_mul(3)
                } else {
                    column_count
                };
                let cols = cols.max(1);
                if ptr_ok(data_at, self.geom.len(), keys.len() * cols * 4) {
                    let mut dr = self.at(data_at)?;
                    for time in keys {
                        let mut vals = Vec::with_capacity(cols);
                        for _ in 0..cols {
                            vals.push(dr.f32()?);
                        }
                        rows.push(ControllerRow { time, data: vals });
                    }
                }
            }
            out.push(Controller {
                name: controller_name(flags, type_id),
                controller_type: type_id,
                bezier,
                rows,
            });
        }
        Ok(out)
    }
}

struct TriMesh {
    faces_off: usize,
    faces_count: usize,
    bmin: Vec3,
    bmax: Vec3,
    radius: f32,
    average: Vec3,
    diffuse: Vec3,
    ambient: Vec3,
    transparencyhint: i32,
    bitmap: String,
    lightmap: String,
    mdx_data_size: u32,
    mdx_data_bitmap: u32,
    mdx_vertex_off: u32,
    mdx_normal_off: u32,
    mdx_tex0_off: u32,
    mdx_tex1_off: u32,
    vertex_count: usize,
    lightmapped: i32,
    rotatetexture: i32,
    backgroundgeometry: i32,
    shadow: i32,
    beaming: i32,
    render: i32,
    area: f32,
    mdx_data_offset: usize,
    vertices_offset: usize,
    dirt_enabled: i32,
    hologram_donotdraw: i32,
}

struct SkinHead {
    mdx_weights: usize,
    mdx_bones: usize,
    bonemap_off: usize,
    bonemap_count: usize,
    qbones_off: usize,
    qbones_count: usize,
    tbones_off: usize,
    tbones_count: usize,
    palette: [i32; 16],
}

struct DanglyHead {
    constraints_off: usize,
    constraints_count: usize,
    _c2: u32,
    displacement: f32,
    tightness: f32,
    period: f32,
    _verts: u32,
}

fn read_vec3(r: &mut Reader<'_>) -> Result<Vec3> {
    Ok(Vec3 {
        x: r.f32()?,
        y: r.f32()?,
        z: r.f32()?,
    })
}

fn ptr_ok(off: usize, len: usize, need: usize) -> bool {
    off != 0 && off != u32::MAX as usize && off.saturating_add(need) <= len
}

fn sane(n: u32, max: usize) -> usize {
    (n as usize).min(max)
}

fn i32_as_u32(v: i32) -> u32 {
    if v < 0 {
        u32::MAX
    } else {
        v as u32
    }
}

fn mdx_f32(mdx: &[u8], off: usize) -> Option<f32> {
    let end = off.checked_add(4)?;
    let b = mdx.get(off..end)?;
    Some(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn mdx_vec2(mdx: &[u8], off: usize) -> Option<Vec2> {
    Some(Vec2 {
        x: mdx_f32(mdx, off)?,
        y: mdx_f32(mdx, off + 4)?,
    })
}

fn mdx_vec3(mdx: &[u8], off: usize) -> Option<Vec3> {
    Some(Vec3 {
        x: mdx_f32(mdx, off)?,
        y: mdx_f32(mdx, off + 4)?,
        z: mdx_f32(mdx, off + 8)?,
    })
}

fn has_named(node: &Node, name: &str) -> bool {
    node.name == name || node.children.iter().any(|c| has_named(c, name))
}

/// Packed orientation: X 11 bits, Y 11 bits, Z 10 bits; W from |q| = 1.
fn decompress_quat(packed: u32) -> Quat {
    let x = ((packed & 0x7FF) as f32 / 1023.0) - 1.0;
    let y = (((packed >> 11) & 0x7FF) as f32 / 1023.0) - 1.0;
    let z = ((packed >> 22) as f32 / 511.0) - 1.0;
    let mag2 = x * x + y * y + z * z;
    let w = if mag2 < 1.0 { (1.0 - mag2).sqrt() } else { 0.0 };
    Quat { x, y, z, w }
}
