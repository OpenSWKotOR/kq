//! In-memory MDL/MDX model — the interchange IR for ASCII, JSON, and binary.
//!
//! Field names and ASCII keywords follow the mdlops / Neverwinter Nights
//! text model dialect (see `ascii.rs`). JSON is a 1:1 serde of this tree.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn skip_empty_map(m: &BTreeMap<String, Property>) -> bool {
    m.is_empty()
}

fn skip_empty_vec<T>(v: &[T]) -> bool {
    v.is_empty()
}

fn skip_none<T>(v: &Option<T>) -> bool {
    v.is_none()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Default for Quat {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Property {
    Number(f64),
    Text(String),
    List(Vec<Property>),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    #[default]
    Dummy,
    Trimesh,
    Skin,
    Danglymesh,
    Aabb,
    Lightsaber,
    Light,
    Emitter,
    Reference,
    Camera,
}

impl NodeKind {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "trimesh" => Self::Trimesh,
            "skin" => Self::Skin,
            "danglymesh" => Self::Danglymesh,
            "aabb" => Self::Aabb,
            "lightsaber" => Self::Lightsaber,
            "light" => Self::Light,
            "emitter" => Self::Emitter,
            "reference" => Self::Reference,
            "camera" => Self::Camera,
            _ => Self::Dummy,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dummy => "dummy",
            Self::Trimesh => "trimesh",
            Self::Skin => "skin",
            Self::Danglymesh => "danglymesh",
            Self::Aabb => "aabb",
            Self::Lightsaber => "lightsaber",
            Self::Light => "light",
            Self::Emitter => "emitter",
            Self::Reference => "reference",
            Self::Camera => "camera",
        }
    }

    pub fn from_flags(flags: u16) -> Self {
        const LIGHT: u16 = 0x0002;
        const EMITTER: u16 = 0x0004;
        const REFERENCE: u16 = 0x0010;
        const MESH: u16 = 0x0020;
        const SKIN: u16 = 0x0040;
        const DANGLY: u16 = 0x0100;
        const AABB: u16 = 0x0200;
        const SABER: u16 = 0x0800;
        if flags & SABER != 0 {
            Self::Lightsaber
        } else if flags & AABB != 0 {
            Self::Aabb
        } else if flags & DANGLY != 0 {
            Self::Danglymesh
        } else if flags & SKIN != 0 {
            Self::Skin
        } else if flags & EMITTER != 0 {
            Self::Emitter
        } else if flags & LIGHT != 0 {
            Self::Light
        } else if flags & REFERENCE != 0 {
            Self::Reference
        } else if flags & MESH != 0 {
            Self::Trimesh
        } else {
            // HEADER (0x0001), empty, or unknown bits — dummy is the safe node kind.
            Self::Dummy
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Vertex {
    pub position: Vec3,
    #[serde(default, skip_serializing_if = "skip_none")]
    pub normal: Option<Vec3>,
    #[serde(default, skip_serializing_if = "skip_none")]
    pub uv: Option<Vec2>,
    #[serde(default, skip_serializing_if = "skip_none")]
    pub uv2: Option<Vec2>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Face {
    pub v1: i32,
    pub v2: i32,
    pub v3: i32,
    pub smooth: i32,
    pub t1: i32,
    pub t2: i32,
    pub t3: i32,
    pub material: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Bone {
    pub index: i32,
    pub bone: i32,
    pub orientation: Quat,
    pub translation: Vec3,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Weight {
    pub influences: Vec<(String, f32)>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Mesh {
    #[serde(default, skip_serializing_if = "skip_none")]
    pub bmin: Option<Vec3>,
    #[serde(default, skip_serializing_if = "skip_none")]
    pub bmax: Option<Vec3>,
    #[serde(default)]
    pub radius: f32,
    #[serde(default, skip_serializing_if = "skip_none")]
    pub average: Option<Vec3>,
    #[serde(default)]
    pub area: f32,
    #[serde(default, skip_serializing_if = "skip_none")]
    pub ambient: Option<Vec3>,
    #[serde(default, skip_serializing_if = "skip_none")]
    pub diffuse: Option<Vec3>,
    #[serde(default)]
    pub transparencyhint: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bitmap: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub lightmap: String,
    #[serde(default)]
    pub render: i32,
    #[serde(default)]
    pub shadow: i32,
    #[serde(default)]
    pub beaming: i32,
    #[serde(default)]
    pub backgroundgeometry: i32,
    #[serde(default)]
    pub rotatetexture: i32,
    #[serde(default)]
    pub lightmapped: i32,
    #[serde(default, skip_serializing_if = "skip_empty_vec")]
    pub verts: Vec<Vertex>,
    #[serde(default, skip_serializing_if = "skip_empty_vec")]
    pub faces: Vec<Face>,
    #[serde(default, skip_serializing_if = "skip_empty_vec")]
    pub tverts: Vec<Vec2>,
    #[serde(default, skip_serializing_if = "skip_empty_vec")]
    pub tverts1: Vec<Vec2>,
    #[serde(default, skip_serializing_if = "skip_empty_vec")]
    pub bones: Vec<Bone>,
    #[serde(default, skip_serializing_if = "skip_empty_vec")]
    pub weights: Vec<Weight>,
    #[serde(default, skip_serializing_if = "skip_empty_vec")]
    pub constraints: Vec<f32>,
    #[serde(default)]
    pub displacement: f32,
    #[serde(default)]
    pub tightness: f32,
    #[serde(default)]
    pub period: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Light {
    #[serde(default)]
    pub flareradius: f32,
    #[serde(default)]
    pub lightpriority: i32,
    #[serde(default)]
    pub ambientonly: i32,
    #[serde(default)]
    pub ndynamictype: i32,
    #[serde(default)]
    pub affectdynamic: i32,
    #[serde(default)]
    pub shadow: i32,
    #[serde(default)]
    pub flare: i32,
    #[serde(default)]
    pub fadinglight: i32,
    #[serde(default, skip_serializing_if = "skip_empty_vec")]
    pub flaresizes: Vec<f32>,
    #[serde(default, skip_serializing_if = "skip_empty_vec")]
    pub flarepositions: Vec<f32>,
    #[serde(default, skip_serializing_if = "skip_empty_vec")]
    pub flarecolorshifts: Vec<Vec3>,
    #[serde(default, skip_serializing_if = "skip_empty_vec")]
    pub texturenames: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Emitter {
    #[serde(default, skip_serializing_if = "skip_empty_map")]
    pub fields: BTreeMap<String, Property>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Reference {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub refmodel: String,
    #[serde(default)]
    pub reattachable: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AabbNode {
    pub bmin: Vec3,
    pub bmax: Vec3,
    pub face: i32,
    pub most_significant: i32,
    pub left: i32,
    pub right: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Controller {
    pub name: String,
    #[serde(default)]
    pub controller_type: u32,
    #[serde(default)]
    pub bezier: bool,
    #[serde(default, skip_serializing_if = "skip_empty_vec")]
    pub rows: Vec<ControllerRow>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ControllerRow {
    pub time: f32,
    pub data: Vec<f32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub kind: NodeKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "skip_none")]
    pub parent: Option<String>,
    #[serde(default)]
    pub node_id: i32,
    pub position: Vec3,
    pub orientation: Quat,
    #[serde(default, skip_serializing_if = "skip_none")]
    pub mesh: Option<Mesh>,
    #[serde(default, skip_serializing_if = "skip_none")]
    pub light: Option<Light>,
    #[serde(default, skip_serializing_if = "skip_none")]
    pub emitter: Option<Emitter>,
    #[serde(default, skip_serializing_if = "skip_none")]
    pub reference: Option<Reference>,
    #[serde(default, skip_serializing_if = "skip_empty_vec")]
    pub aabb: Vec<AabbNode>,
    #[serde(default, skip_serializing_if = "skip_empty_vec")]
    pub controllers: Vec<Controller>,
    #[serde(default, skip_serializing_if = "skip_empty_map")]
    pub extras: BTreeMap<String, Property>,
    #[serde(default, skip_serializing_if = "skip_empty_vec")]
    pub children: Vec<Node>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub time: f32,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Animation {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub root_model: String,
    #[serde(default)]
    pub length: f32,
    #[serde(default)]
    pub transtime: f32,
    #[serde(default, skip_serializing_if = "skip_empty_vec")]
    pub events: Vec<Event>,
    #[serde(default, skip_serializing_if = "skip_empty_vec")]
    pub nodes: Vec<Node>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub name: String,
    #[serde(default)]
    pub supermodel: String,
    #[serde(default)]
    pub classification: String,
    #[serde(default)]
    pub classification_unk1: i32,
    #[serde(default)]
    pub ignorefog: i32,
    #[serde(default)]
    pub compress_quaternions: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub headlink: String,
    #[serde(default)]
    pub animation_scale: f32,
    #[serde(default)]
    pub bmin: Vec3,
    #[serde(default)]
    pub bmax: Vec3,
    #[serde(default)]
    pub radius: f32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub filedependancy: String,
    #[serde(default)]
    pub game: String,
    #[serde(default, skip_serializing_if = "skip_none")]
    pub root: Option<Box<Node>>,
    #[serde(default, skip_serializing_if = "skip_empty_vec")]
    pub animations: Vec<Animation>,
    /// Non-fatal notes (missing MDX companion, clamped counts, etc.).
    #[serde(default, skip_serializing_if = "skip_empty_vec")]
    pub warnings: Vec<String>,
}

impl Model {
    pub fn node_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        if let Some(root) = &self.root {
            collect_names(root, &mut names);
        }
        names
    }

    pub fn animation_names(&self) -> Vec<String> {
        self.animations.iter().map(|a| a.name.clone()).collect()
    }
}

fn collect_names(node: &Node, out: &mut Vec<String>) {
    out.push(node.name.clone());
    for child in &node.children {
        collect_names(child, out);
    }
}

pub fn classification_from_byte(model_type: u8) -> &'static str {
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

pub fn controller_name(node_flags: u16, type_id: u32) -> String {
    const LIGHT: u16 = 0x0002;
    const EMITTER: u16 = 0x0004;
    const MESH: u16 = 0x0020;
    if node_flags & EMITTER != 0 {
        if let Some(n) = emitter_controller(type_id) {
            return n.to_string();
        }
    }
    if node_flags & LIGHT != 0 {
        if let Some(n) = light_controller(type_id) {
            return n.to_string();
        }
    }
    if node_flags & MESH != 0 && type_id == 100 {
        return "selfillumcolor".into();
    }
    match type_id {
        8 => "position",
        20 => "orientation",
        36 => "scale",
        132 => "alpha",
        _ => return format!("controller_{type_id}"),
    }
    .into()
}

fn light_controller(id: u32) -> Option<&'static str> {
    Some(match id {
        76 => "color",
        88 => "radius",
        96 => "shadowradius",
        100 => "verticaldisplacement",
        140 => "multiplier",
        _ => return None,
    })
}

fn emitter_controller(id: u32) -> Option<&'static str> {
    Some(match id {
        80 => "alphaEnd",
        84 => "alphaStart",
        88 => "birthrate",
        92 => "bounce_co",
        96 => "combinetime",
        100 => "drag",
        104 => "fps",
        108 => "frameEnd",
        112 => "frameStart",
        116 => "grav",
        120 => "lifeExp",
        124 => "mass",
        128 => "p2p_bezier2",
        132 => "p2p_bezier3",
        136 => "particleRot",
        140 => "randvel",
        144 => "sizeStart",
        148 => "sizeEnd",
        152 => "sizeStart_y",
        156 => "sizeEnd_y",
        160 => "spread",
        164 => "threshold",
        168 => "velocity",
        172 => "xsize",
        176 => "ysize",
        180 => "blurlength",
        184 => "lightningDelay",
        188 => "lightningRadius",
        192 => "lightningScale",
        196 => "lightningSubDiv",
        200 => "lightningzigzag",
        216 => "alphaMid",
        220 => "percentStart",
        224 => "percentMid",
        228 => "percentEnd",
        232 => "sizeMid",
        236 => "sizeMid_y",
        240 => "m_fRandomBirthRate",
        252 => "targetsize",
        256 => "numcontrolpts",
        260 => "controlptradius",
        264 => "controlptdelay",
        268 => "tangentspread",
        272 => "tangentlength",
        284 => "colorMid",
        380 => "colorEnd",
        392 => "colorStart",
        502 => "detonate",
        _ => return None,
    })
}
