//! mdlops-compatible ASCII MDL reader and writer.
//!
//! The dialect is the Neverwinter Nights / Odyssey text model: `newmodel`,
//! `beginmodelgeom`, `node <kind> <name> { … }`, `newanim`, `donemodel`.
//!
//! This is an original implementation of that public text format. It is not a
//! port of mdlops (GPL-3.0). See the [mdlops README](https://github.com/ndixUR/mdlops)
//! for the reference tool; typical retail copies of `rims/` 2DAs are unrelated.

use std::collections::BTreeMap;
use std::path::Path;

use super::model::*;
use crate::error::{FormatError, Result};

pub fn sniff(data: &[u8]) -> bool {
    let text = strip_bom(data);
    let start = text
        .iter()
        .position(|&b| !b.is_ascii_whitespace() && b != b'#')
        .unwrap_or(0);
    let head = &text[start..];
    starts_ignore_ascii(head, b"newmodel")
        || starts_ignore_ascii(head, b"filedependancy")
        || starts_ignore_ascii(head, b"filedependency")
}

fn strip_bom(data: &[u8]) -> &[u8] {
    data.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(data)
}

fn starts_ignore_ascii(data: &[u8], prefix: &[u8]) -> bool {
    data.len() >= prefix.len() && data[..prefix.len()].eq_ignore_ascii_case(prefix)
}

pub fn read(data: &[u8], path: &Path) -> Result<Model> {
    let text = String::from_utf8_lossy(strip_bom(data));
    parse(&text, path)
}

pub fn write(model: &Model) -> String {
    let mut out = String::new();
    out.push_str("# ASCII MDL\n");
    if !model.filedependancy.is_empty() {
        out.push_str(&format!("filedependancy {}\n", model.filedependancy));
    } else {
        out.push_str(&format!("filedependancy {} NULL.mlk\n", model.name));
    }
    out.push_str(&format!("newmodel {}\n\n", model.name));
    out.push_str(&format!(
        "setsupermodel {} {}\n",
        model.name,
        if model.supermodel.is_empty() {
            "NULL"
        } else {
            &model.supermodel
        }
    ));
    let class = if model.classification.is_empty() {
        "other"
    } else {
        &model.classification
    };
    out.push_str(&format!("classification {class}\n"));
    out.push_str(&format!(
        "classification_unk1 {}\n",
        model.classification_unk1
    ));
    out.push_str(&format!("ignorefog {}\n", model.ignorefog));
    out.push_str(&format!(
        "compress_quaternions {}\n",
        model.compress_quaternions
    ));
    if !model.headlink.is_empty() {
        out.push_str(&format!("headlink {}\n", model.headlink));
    }
    out.push_str(&format!(
        "\nsetanimationscale {}\n\n",
        model.animation_scale
    ));
    out.push_str(&format!("beginmodelgeom {}\n", model.name));
    write_indent(
        &mut out,
        1,
        &format!("bmin {} {} {}", model.bmin.x, model.bmin.y, model.bmin.z),
    );
    write_indent(
        &mut out,
        1,
        &format!("bmax {} {} {}", model.bmax.x, model.bmax.y, model.bmax.z),
    );
    write_indent(&mut out, 1, &format!("radius {}", model.radius));
    out.push('\n');
    if let Some(root) = &model.root {
        write_node(&mut out, 1, root);
    }
    out.push_str(&format!("endmodelgeom {}\n\n", model.name));
    for anim in &model.animations {
        write_anim(&mut out, anim, &model.name);
    }
    out.push_str(&format!("donemodel {}\n", model.name));
    out
}

fn write_indent(out: &mut String, indent: usize, line: &str) {
    for _ in 0..indent {
        out.push_str("  ");
    }
    out.push_str(line);
    out.push('\n');
}

fn write_node(out: &mut String, indent: usize, node: &Node) {
    write_indent(
        out,
        indent,
        &format!("node {} {}", node.kind.as_str(), node.name),
    );
    write_indent(out, indent, "{");
    let inner = indent + 1;
    match &node.parent {
        Some(p) => write_indent(out, inner, &format!("parent {p}")),
        None => write_indent(out, inner, "parent NULL"),
    }
    write_indent(
        out,
        inner,
        &format!(
            "position  {:.7} {:.7} {:.7}",
            node.position.x, node.position.y, node.position.z
        ),
    );
    write_indent(
        out,
        inner,
        &format!(
            "orientation  {:.7} {:.7} {:.7} {:.7}",
            node.orientation.x, node.orientation.y, node.orientation.z, node.orientation.w
        ),
    );
    if let Some(mesh) = &node.mesh {
        write_mesh(out, inner, mesh);
    }
    if let Some(light) = &node.light {
        write_light(out, inner, light);
    }
    if let Some(emitter) = &node.emitter {
        for (k, v) in &emitter.fields {
            write_indent(out, inner, &format!("{k} {}", property_to_tokens(v)));
        }
    }
    if let Some(r) = &node.reference {
        if !r.refmodel.is_empty() {
            write_indent(out, inner, &format!("refmodel {}", r.refmodel));
        }
        write_indent(out, inner, &format!("reattachable {}", r.reattachable));
    }
    if !node.aabb.is_empty() {
        write_indent(out, inner, &format!("aabb {}", node.aabb.len()));
        for a in &node.aabb {
            write_indent(
                out,
                inner + 1,
                &format!(
                    "{} {} {} {} {} {} {} {} {} {}",
                    a.bmin.x,
                    a.bmin.y,
                    a.bmin.z,
                    a.bmax.x,
                    a.bmax.y,
                    a.bmax.z,
                    a.face,
                    a.most_significant,
                    a.left,
                    a.right
                ),
            );
        }
    }
    for (k, v) in &node.extras {
        write_indent(out, inner, &format!("{k} {}", property_to_tokens(v)));
    }
    for c in &node.controllers {
        write_controller(out, inner, c);
    }
    write_indent(out, indent, "}");
    for child in &node.children {
        write_node(out, indent, child);
    }
}

fn write_mesh(out: &mut String, indent: usize, mesh: &Mesh) {
    if let Some(b) = &mesh.bmin {
        write_indent(
            out,
            indent,
            &format!("bmin  {:.7} {:.7} {:.7}", b.x, b.y, b.z),
        );
    }
    if let Some(b) = &mesh.bmax {
        write_indent(
            out,
            indent,
            &format!("bmax  {:.7} {:.7} {:.7}", b.x, b.y, b.z),
        );
    }
    write_indent(out, indent, &format!("radius  {:.7}", mesh.radius));
    if let Some(a) = &mesh.average {
        write_indent(
            out,
            indent,
            &format!("average  {:.7} {:.7} {:.7}", a.x, a.y, a.z),
        );
    }
    write_indent(out, indent, &format!("area {:.7}", mesh.area));
    if let Some(a) = &mesh.ambient {
        write_indent(out, indent, &format!("ambient {} {} {}", a.x, a.y, a.z));
    }
    if let Some(d) = &mesh.diffuse {
        write_indent(out, indent, &format!("diffuse {} {} {}", d.x, d.y, d.z));
    }
    write_indent(
        out,
        indent,
        &format!("transparencyhint {}", mesh.transparencyhint),
    );
    if !mesh.bitmap.is_empty() {
        write_indent(out, indent, &format!("bitmap {}", mesh.bitmap));
    }
    if !mesh.lightmap.is_empty() {
        write_indent(out, indent, &format!("lightmap {}", mesh.lightmap));
    }
    write_indent(out, indent, &format!("render {}", mesh.render));
    write_indent(out, indent, &format!("shadow {}", mesh.shadow));
    write_indent(out, indent, &format!("beaming {}", mesh.beaming));
    write_indent(
        out,
        indent,
        &format!("backgroundgeometry {}", mesh.backgroundgeometry),
    );
    write_indent(
        out,
        indent,
        &format!("rotatetexture {}", mesh.rotatetexture),
    );
    write_indent(out, indent, &format!("lightmapped {}", mesh.lightmapped));
    if mesh.displacement != 0.0 {
        write_indent(out, indent, &format!("displacement {}", mesh.displacement));
    }
    if mesh.tightness != 0.0 {
        write_indent(out, indent, &format!("tightness {}", mesh.tightness));
    }
    if mesh.period != 0.0 {
        write_indent(out, indent, &format!("period {}", mesh.period));
    }
    if !mesh.bones.is_empty() {
        write_indent(out, indent, &format!("bones {}", mesh.bones.len()));
        for b in &mesh.bones {
            write_indent(
                out,
                indent + 1,
                &format!(
                    "{} {} {} {} {} {} {} {} {}",
                    b.index,
                    b.bone,
                    b.orientation.x,
                    b.orientation.y,
                    b.orientation.z,
                    b.orientation.w,
                    b.translation.x,
                    b.translation.y,
                    b.translation.z
                ),
            );
        }
    }
    if !mesh.weights.is_empty() {
        write_indent(out, indent, &format!("weights {}", mesh.weights.len()));
        for w in &mesh.weights {
            let mut line = String::new();
            for (i, (name, wt)) in w.influences.iter().enumerate() {
                if i > 0 {
                    line.push(' ');
                }
                line.push_str(&format!("{name} {wt}"));
            }
            write_indent(out, indent + 1, &line);
        }
    }
    if !mesh.constraints.is_empty() {
        write_indent(
            out,
            indent,
            &format!("constraints {}", mesh.constraints.len()),
        );
        for c in &mesh.constraints {
            write_indent(out, indent + 1, &format!("{c}"));
        }
    }
    write_indent(out, indent, &format!("verts {}", mesh.verts.len()));
    for (i, v) in mesh.verts.iter().enumerate() {
        let mut line = format!("{} {} {} {}", i, v.position.x, v.position.y, v.position.z);
        if let Some(n) = &v.normal {
            line.push_str(&format!(" {} {} {}", n.x, n.y, n.z));
        }
        if let Some(uv) = &v.uv {
            line.push_str(&format!(" {} {}", uv.x, uv.y));
        }
        if let Some(uv) = &v.uv2 {
            line.push_str(&format!(" {} {}", uv.x, uv.y));
        }
        write_indent(out, indent + 1, &line);
    }
    if !mesh.tverts.is_empty() {
        write_indent(out, indent, &format!("tverts {}", mesh.tverts.len()));
        for (i, t) in mesh.tverts.iter().enumerate() {
            write_indent(out, indent + 1, &format!("{} {} {}", i, t.x, t.y));
        }
    }
    if !mesh.tverts1.is_empty() {
        write_indent(out, indent, &format!("tverts1 {}", mesh.tverts1.len()));
        for (i, t) in mesh.tverts1.iter().enumerate() {
            write_indent(out, indent + 1, &format!("{} {} {}", i, t.x, t.y));
        }
    }
    write_indent(out, indent, &format!("faces {}", mesh.faces.len()));
    for f in &mesh.faces {
        write_indent(
            out,
            indent + 1,
            &format!(
                "{} {} {} {} {} {} {} {}",
                f.v1, f.v2, f.v3, f.smooth, f.t1, f.t2, f.t3, f.material
            ),
        );
    }
}

fn write_light(out: &mut String, indent: usize, light: &Light) {
    write_indent(out, indent, &format!("flareradius {}", light.flareradius));
    write_indent(
        out,
        indent,
        &format!("lightpriority {}", light.lightpriority),
    );
    write_indent(out, indent, &format!("ambientonly {}", light.ambientonly));
    write_indent(out, indent, &format!("ndynamictype {}", light.ndynamictype));
    write_indent(
        out,
        indent,
        &format!("affectdynamic {}", light.affectdynamic),
    );
    write_indent(out, indent, &format!("shadow {}", light.shadow));
    write_indent(out, indent, &format!("flare {}", light.flare));
    write_indent(out, indent, &format!("fadinglight {}", light.fadinglight));
    if !light.flaresizes.is_empty() {
        write_indent(
            out,
            indent,
            &format!("flaresizes {}", light.flaresizes.len()),
        );
        for v in &light.flaresizes {
            write_indent(out, indent + 1, &format!("{v}"));
        }
    }
    if !light.flarepositions.is_empty() {
        write_indent(
            out,
            indent,
            &format!("flarepositions {}", light.flarepositions.len()),
        );
        for v in &light.flarepositions {
            write_indent(out, indent + 1, &format!("{v}"));
        }
    }
    if !light.flarecolorshifts.is_empty() {
        write_indent(
            out,
            indent,
            &format!("flarecolorshifts {}", light.flarecolorshifts.len()),
        );
        for c in &light.flarecolorshifts {
            write_indent(out, indent + 1, &format!("{} {} {}", c.x, c.y, c.z));
        }
    }
    if !light.texturenames.is_empty() {
        write_indent(
            out,
            indent,
            &format!("texturenames {}", light.texturenames.len()),
        );
        for t in &light.texturenames {
            write_indent(out, indent + 1, t);
        }
    }
}

fn write_controller(out: &mut String, indent: usize, c: &Controller) {
    let key = if c.bezier {
        format!("{}bezierkey", c.name)
    } else if c.rows.len() > 1 {
        format!("{}key", c.name)
    } else {
        c.name.clone()
    };
    if c.rows.len() <= 1 && !c.bezier {
        if let Some(row) = c.rows.first() {
            let nums = row
                .data
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(" ");
            write_indent(out, indent, &format!("{key} {nums}"));
            return;
        }
    }
    write_indent(out, indent, &key);
    for row in &c.rows {
        let mut line = format!("{}", row.time);
        for n in &row.data {
            line.push_str(&format!(" {n}"));
        }
        write_indent(out, indent + 1, &line);
    }
    write_indent(out, indent, "endlist");
}

fn write_anim(out: &mut String, anim: &Animation, model: &str) {
    let root = if anim.root_model.is_empty() {
        model
    } else {
        &anim.root_model
    };
    out.push_str(&format!("newanim {} {}\n", anim.name, root));
    write_indent(out, 1, &format!("length {}", anim.length));
    write_indent(out, 1, &format!("transtime {}", anim.transtime));
    write_indent(out, 1, &format!("animroot {root}"));
    for e in &anim.events {
        write_indent(out, 1, &format!("event {} {}", e.time, e.name));
    }
    for node in &anim.nodes {
        write_node(out, 1, node);
    }
    out.push_str(&format!("doneanim {} {}\n\n", anim.name, root));
}

fn property_to_tokens(p: &Property) -> String {
    match p {
        Property::Number(n) => n.to_string(),
        Property::Text(s) => s.clone(),
        Property::List(xs) => xs
            .iter()
            .map(property_to_tokens)
            .collect::<Vec<_>>()
            .join(" "),
    }
}

struct Parser {
    model: Model,
    nodes: Vec<PendingNode>,
    current: Option<usize>,
    in_geom: bool,
    in_anim: bool,
    anim_idx: usize,
    table: Table,
}

struct PendingNode {
    node: Node,
    parent: Option<String>,
    anim: Option<usize>,
}

#[derive(Clone, Debug, Default)]
enum Table {
    #[default]
    None,
    Counted {
        kind: TableKind,
        remain: usize,
    },
    Controller {
        name: String,
        bezier: bool,
    },
}

#[derive(Clone, Copy, Debug)]
enum TableKind {
    Verts,
    Faces,
    Tverts,
    Tverts1,
    Bones,
    Weights,
    Constraints,
    Aabb,
    FlareSizes,
    FlarePositions,
    FlareColorShifts,
    TextureNames,
}

fn parse(text: &str, path: &Path) -> Result<Model> {
    let mut p = Parser {
        model: Model {
            supermodel: "NULL".into(),
            classification: "other".into(),
            animation_scale: 0.971,
            bmin: Vec3 {
                x: -5.0,
                y: -5.0,
                z: -1.0,
            },
            bmax: Vec3 {
                x: 5.0,
                y: 5.0,
                z: 10.0,
            },
            radius: 7.0,
            ignorefog: 1,
            ..Model::default()
        },
        nodes: Vec::new(),
        current: None,
        in_geom: false,
        in_anim: false,
        anim_idx: 0,
        table: Table::None,
    };
    let mut saw = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        saw = true;
        p.feed(line);
    }
    if !saw {
        return Err(FormatError::Malformed {
            path: path.to_path_buf(),
            message: "empty ASCII MDL".into(),
        });
    }
    p.finish()
}

impl Parser {
    fn feed(&mut self, line: &str) {
        let tokens = tokenize(line);
        if tokens.is_empty() {
            return;
        }
        let key = tokens[0].to_ascii_lowercase();

        if matches!(&self.table, Table::None) && key == "endlist" {
            return;
        }

        match &self.table {
            Table::Counted { kind, remain } if *remain > 0 => {
                let kind = *kind;
                self.ingest_table_row(kind, &tokens);
                if let Table::Counted { remain, .. } = &mut self.table {
                    *remain = remain.saturating_sub(1);
                    if *remain == 0 {
                        self.table = Table::None;
                    }
                }
                return;
            }
            Table::Controller { name, bezier } => {
                if key == "endlist" || key == "}" {
                    self.table = Table::None;
                    if key == "}" {
                        self.close_node();
                    }
                    return;
                }
                let name = name.clone();
                let bezier = *bezier;
                self.ingest_controller_row(&name, bezier, &tokens);
                return;
            }
            _ => {}
        }

        if key == "}" {
            self.close_node();
            return;
        }

        match key.as_str() {
            "newmodel" => {
                if let Some(n) = tokens.get(1) {
                    self.model.name = n.to_string();
                }
            }
            "filedependancy" | "filedependency" => {
                self.model.filedependancy = tokens[1..].join(" ");
            }
            "setsupermodel" => {
                if let Some(n) = tokens.get(2) {
                    self.model.supermodel = n.to_string();
                }
            }
            "classification" => {
                if let Some(n) = tokens.get(1) {
                    self.model.classification = n.to_ascii_lowercase();
                }
            }
            "classification_unk1" => {
                self.model.classification_unk1 = int_tok(&tokens, 1);
            }
            "ignorefog" => self.model.ignorefog = int_tok(&tokens, 1),
            "compress_quaternions" => self.model.compress_quaternions = int_tok(&tokens, 1),
            "headlink" => {
                if let Some(n) = tokens.get(1) {
                    self.model.headlink = n.to_string();
                }
            }
            "setanimationscale" => self.model.animation_scale = float_tok(&tokens, 1),
            "beginmodelgeom" => {
                self.in_geom = true;
                self.in_anim = false;
            }
            "endmodelgeom" => self.in_geom = false,
            "newanim" => {
                let name = tokens.get(1).cloned().unwrap_or_default();
                let root = tokens.get(2).cloned().unwrap_or_default();
                self.model.animations.push(Animation {
                    name,
                    root_model: root,
                    ..Animation::default()
                });
                self.in_anim = true;
                self.anim_idx = self.model.animations.len() - 1;
            }
            "doneanim" | "donemodel" => self.in_anim = false,
            "length" if self.in_anim && self.current.is_none() => {
                if let Some(a) = self.model.animations.get_mut(self.anim_idx) {
                    a.length = float_tok(&tokens, 1);
                }
            }
            "transtime" if self.in_anim && self.current.is_none() => {
                if let Some(a) = self.model.animations.get_mut(self.anim_idx) {
                    a.transtime = float_tok(&tokens, 1);
                }
            }
            "animroot" if self.in_anim && self.current.is_none() => {
                if let Some(a) = self.model.animations.get_mut(self.anim_idx) {
                    if let Some(n) = tokens.get(1) {
                        a.root_model = n.to_string();
                    }
                }
            }
            "event" if self.in_anim && self.current.is_none() => {
                if let Some(a) = self.model.animations.get_mut(self.anim_idx) {
                    a.events.push(Event {
                        time: float_tok(&tokens, 1),
                        name: tokens.get(2).cloned().unwrap_or_default(),
                    });
                }
            }
            "bmin" if self.current.is_none() => self.model.bmin = vec3_tok(&tokens, 1),
            "bmax" if self.current.is_none() => self.model.bmax = vec3_tok(&tokens, 1),
            "radius" if self.current.is_none() => self.model.radius = float_tok(&tokens, 1),
            "node" => self.open_node(&tokens),
            "{" => {}
            _ if self.current.is_some() => self.node_property(&key, &tokens),
            _ => {}
        }
    }

    fn open_node(&mut self, tokens: &[String]) {
        let mut kind = tokens
            .get(1)
            .map(|s| NodeKind::parse(s))
            .unwrap_or_default();
        let mut name = tokens.get(2).cloned().unwrap_or_default();
        if name.starts_with("2081__") {
            kind = NodeKind::Lightsaber;
            name = name[6..].to_string();
        }
        let mut node = Node {
            kind,
            name,
            node_id: self.nodes.len() as i32,
            ..Node::default()
        };
        match kind {
            NodeKind::Trimesh | NodeKind::Skin | NodeKind::Danglymesh | NodeKind::Lightsaber => {
                node.mesh = Some(Mesh {
                    render: 1,
                    ..Mesh::default()
                });
            }
            NodeKind::Light => node.light = Some(Light::default()),
            NodeKind::Emitter => node.emitter = Some(Emitter::default()),
            NodeKind::Reference => node.reference = Some(Reference::default()),
            _ => {}
        }
        self.nodes.push(PendingNode {
            node,
            parent: None,
            anim: if self.in_anim {
                Some(self.anim_idx)
            } else {
                None
            },
        });
        self.current = Some(self.nodes.len() - 1);
    }

    fn close_node(&mut self) {
        self.current = None;
        self.table = Table::None;
    }

    fn node_mut(&mut self) -> Option<&mut PendingNode> {
        self.nodes.get_mut(self.current?)
    }

    fn ensure_mesh(&mut self) -> Option<&mut Mesh> {
        let n = self.node_mut()?;
        if n.node.mesh.is_none() {
            n.node.mesh = Some(Mesh::default());
        }
        n.node.mesh.as_mut()
    }

    fn node_property(&mut self, key: &str, tokens: &[String]) {
        if key.ends_with("bezierkey") || key.ends_with("key") {
            let bezier = key.ends_with("bezierkey");
            let name = key
                .trim_end_matches("bezierkey")
                .trim_end_matches("key")
                .to_string();
            if tokens.len() > 1 {
                self.ingest_controller_row(&name, bezier, &tokens[1..]);
            }
            self.table = Table::Controller { name, bezier };
            return;
        }

        match key {
            "parent" => {
                if let Some(n) = self.node_mut() {
                    let p = tokens.get(1).map(|s| s.as_str()).unwrap_or("NULL");
                    n.parent = if p.eq_ignore_ascii_case("null") {
                        None
                    } else {
                        Some(p.to_string())
                    };
                    n.node.parent = n.parent.clone();
                }
            }
            "position" => {
                if let Some(n) = self.node_mut() {
                    n.node.position = vec3_tok(tokens, 1);
                }
            }
            "orientation" => {
                if let Some(n) = self.node_mut() {
                    n.node.orientation = quat_tok(tokens, 1);
                }
            }
            "verts" => self.begin_table(TableKind::Verts, int_tok(tokens, 1) as usize),
            "faces" => self.begin_table(TableKind::Faces, int_tok(tokens, 1) as usize),
            "tverts" => self.begin_table(TableKind::Tverts, int_tok(tokens, 1) as usize),
            "tverts1" | "lightmaptverts" => {
                self.begin_table(TableKind::Tverts1, int_tok(tokens, 1) as usize)
            }
            "bones" => self.begin_table(TableKind::Bones, int_tok(tokens, 1) as usize),
            "weights" => self.begin_table(TableKind::Weights, int_tok(tokens, 1) as usize),
            "constraints" => self.begin_table(TableKind::Constraints, int_tok(tokens, 1) as usize),
            "aabb" => {
                if tokens.len() > 1 && tokens[1].chars().next().is_some_and(|c| c.is_ascii_digit())
                {
                    self.begin_table(TableKind::Aabb, int_tok(tokens, 1) as usize);
                }
            }
            "flaresizes" => self.begin_table(TableKind::FlareSizes, int_tok(tokens, 1) as usize),
            "flarepositions" => {
                self.begin_table(TableKind::FlarePositions, int_tok(tokens, 1) as usize)
            }
            "flarecolorshifts" => {
                self.begin_table(TableKind::FlareColorShifts, int_tok(tokens, 1) as usize)
            }
            "texturenames" => {
                self.begin_table(TableKind::TextureNames, int_tok(tokens, 1) as usize)
            }
            "bitmap" => {
                if let Some(m) = self.ensure_mesh() {
                    m.bitmap = tokens.get(1).cloned().unwrap_or_default();
                }
            }
            "lightmap" => {
                if let Some(m) = self.ensure_mesh() {
                    m.lightmap = tokens.get(1).cloned().unwrap_or_default();
                }
            }
            "ambient" => {
                if let Some(m) = self.ensure_mesh() {
                    m.ambient = Some(vec3_tok(tokens, 1));
                }
            }
            "diffuse" => {
                if let Some(m) = self.ensure_mesh() {
                    m.diffuse = Some(vec3_tok(tokens, 1));
                }
            }
            "bmin" => {
                if let Some(m) = self.ensure_mesh() {
                    m.bmin = Some(vec3_tok(tokens, 1));
                }
            }
            "bmax" => {
                if let Some(m) = self.ensure_mesh() {
                    m.bmax = Some(vec3_tok(tokens, 1));
                }
            }
            "average" => {
                if let Some(m) = self.ensure_mesh() {
                    m.average = Some(vec3_tok(tokens, 1));
                }
            }
            "radius" => {
                if let Some(m) = self.ensure_mesh() {
                    m.radius = float_tok(tokens, 1);
                }
            }
            "area" => {
                if let Some(m) = self.ensure_mesh() {
                    m.area = float_tok(tokens, 1);
                }
            }
            "transparencyhint" => {
                if let Some(m) = self.ensure_mesh() {
                    m.transparencyhint = int_tok(tokens, 1);
                }
            }
            "render" => {
                if let Some(m) = self.ensure_mesh() {
                    m.render = int_tok(tokens, 1);
                }
            }
            "shadow" => {
                if let Some(m) = self.ensure_mesh() {
                    m.shadow = int_tok(tokens, 1);
                }
            }
            "beaming" => {
                if let Some(m) = self.ensure_mesh() {
                    m.beaming = int_tok(tokens, 1);
                }
            }
            "backgroundgeometry" => {
                if let Some(m) = self.ensure_mesh() {
                    m.backgroundgeometry = int_tok(tokens, 1);
                }
            }
            "rotatetexture" => {
                if let Some(m) = self.ensure_mesh() {
                    m.rotatetexture = int_tok(tokens, 1);
                }
            }
            "lightmapped" => {
                if let Some(m) = self.ensure_mesh() {
                    m.lightmapped = int_tok(tokens, 1);
                }
            }
            "displacement" => {
                if let Some(m) = self.ensure_mesh() {
                    m.displacement = float_tok(tokens, 1);
                }
            }
            "tightness" => {
                if let Some(m) = self.ensure_mesh() {
                    m.tightness = float_tok(tokens, 1);
                }
            }
            "period" => {
                if let Some(m) = self.ensure_mesh() {
                    m.period = float_tok(tokens, 1);
                }
            }
            "refmodel" => {
                if let Some(n) = self.node_mut() {
                    n.node
                        .reference
                        .get_or_insert_with(Reference::default)
                        .refmodel = tokens.get(1).cloned().unwrap_or_default();
                }
            }
            "reattachable" => {
                if let Some(n) = self.node_mut() {
                    n.node
                        .reference
                        .get_or_insert_with(Reference::default)
                        .reattachable = int_tok(tokens, 1);
                }
            }
            "flareradius" => {
                if let Some(n) = self.node_mut() {
                    n.node.light.get_or_insert_with(Light::default).flareradius =
                        float_tok(tokens, 1);
                }
            }
            "lightpriority" => {
                if let Some(n) = self.node_mut() {
                    n.node
                        .light
                        .get_or_insert_with(Light::default)
                        .lightpriority = int_tok(tokens, 1);
                }
            }
            "ambientonly" => {
                if let Some(n) = self.node_mut() {
                    n.node.light.get_or_insert_with(Light::default).ambientonly =
                        int_tok(tokens, 1);
                }
            }
            "ndynamictype" => {
                if let Some(n) = self.node_mut() {
                    n.node.light.get_or_insert_with(Light::default).ndynamictype =
                        int_tok(tokens, 1);
                }
            }
            "affectdynamic" => {
                if let Some(n) = self.node_mut() {
                    n.node
                        .light
                        .get_or_insert_with(Light::default)
                        .affectdynamic = int_tok(tokens, 1);
                }
            }
            "flare" => {
                if let Some(n) = self.node_mut() {
                    n.node.light.get_or_insert_with(Light::default).flare = int_tok(tokens, 1);
                }
            }
            "fadinglight" => {
                if let Some(n) = self.node_mut() {
                    n.node.light.get_or_insert_with(Light::default).fadinglight =
                        int_tok(tokens, 1);
                }
            }
            other => {
                let value = tokens_to_property(&tokens[1..]);
                if let Some(n) = self.node_mut() {
                    if n.node.kind == NodeKind::Emitter {
                        n.node
                            .emitter
                            .get_or_insert_with(Emitter::default)
                            .fields
                            .insert(other.to_string(), value);
                    } else {
                        n.node.extras.insert(other.to_string(), value);
                    }
                }
            }
        }
    }

    fn begin_table(&mut self, kind: TableKind, count: usize) {
        self.table = Table::Counted {
            kind,
            remain: count,
        };
    }

    fn ingest_table_row(&mut self, kind: TableKind, tokens: &[String]) {
        match kind {
            TableKind::Verts => {
                if let Some(m) = self.ensure_mesh() {
                    let start = if tokens.first().is_some_and(|t| t.parse::<i32>().is_ok())
                        && tokens.len() > 3
                    {
                        1
                    } else {
                        0
                    };
                    let mut v = Vertex {
                        position: vec3_tok(tokens, start),
                        ..Vertex::default()
                    };
                    if tokens.len() >= start + 6 {
                        v.normal = Some(vec3_tok(tokens, start + 3));
                    }
                    if tokens.len() >= start + 8 {
                        v.uv = Some(vec2_tok(tokens, start + 6));
                    }
                    if tokens.len() >= start + 10 {
                        v.uv2 = Some(vec2_tok(tokens, start + 8));
                    }
                    m.verts.push(v);
                }
            }
            TableKind::Faces => {
                if let Some(m) = self.ensure_mesh() {
                    let nums: Vec<i32> = tokens.iter().filter_map(|t| t.parse().ok()).collect();
                    for chunk in nums.chunks(8) {
                        if chunk.len() >= 8 {
                            m.faces.push(Face {
                                v1: chunk[0],
                                v2: chunk[1],
                                v3: chunk[2],
                                smooth: chunk[3],
                                t1: chunk[4],
                                t2: chunk[5],
                                t3: chunk[6],
                                material: chunk[7],
                            });
                        }
                    }
                }
            }
            TableKind::Tverts => {
                if let Some(m) = self.ensure_mesh() {
                    let start = if tokens.len() >= 3 { 1 } else { 0 };
                    m.tverts.push(vec2_tok(tokens, start));
                }
            }
            TableKind::Tverts1 => {
                if let Some(m) = self.ensure_mesh() {
                    let start = if tokens.len() >= 3 { 1 } else { 0 };
                    m.tverts1.push(vec2_tok(tokens, start));
                }
            }
            TableKind::Bones => {
                if let Some(m) = self.ensure_mesh() {
                    m.bones.push(Bone {
                        index: int_tok(tokens, 0),
                        bone: int_tok(tokens, 1),
                        orientation: quat_tok(tokens, 2),
                        translation: vec3_tok(tokens, 6),
                    });
                }
            }
            TableKind::Weights => {
                if let Some(m) = self.ensure_mesh() {
                    let mut influences = Vec::new();
                    let mut i = 0;
                    while i + 1 < tokens.len() {
                        if let Ok(w) = tokens[i + 1].parse::<f32>() {
                            influences.push((tokens[i].clone(), w));
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }
                    m.weights.push(Weight { influences });
                }
            }
            TableKind::Constraints => {
                if let Some(m) = self.ensure_mesh() {
                    m.constraints.push(float_tok(tokens, 0));
                }
            }
            TableKind::Aabb => {
                if let Some(n) = self.node_mut() {
                    n.node.aabb.push(AabbNode {
                        bmin: vec3_tok(tokens, 0),
                        bmax: vec3_tok(tokens, 3),
                        face: int_tok(tokens, 6),
                        most_significant: int_tok(tokens, 7),
                        left: int_tok(tokens, 8),
                        right: int_tok(tokens, 9),
                    });
                }
            }
            TableKind::FlareSizes => {
                if let Some(n) = self.node_mut() {
                    n.node
                        .light
                        .get_or_insert_with(Light::default)
                        .flaresizes
                        .push(float_tok(tokens, 0));
                }
            }
            TableKind::FlarePositions => {
                if let Some(n) = self.node_mut() {
                    n.node
                        .light
                        .get_or_insert_with(Light::default)
                        .flarepositions
                        .push(float_tok(tokens, 0));
                }
            }
            TableKind::FlareColorShifts => {
                if let Some(n) = self.node_mut() {
                    n.node
                        .light
                        .get_or_insert_with(Light::default)
                        .flarecolorshifts
                        .push(vec3_tok(tokens, 0));
                }
            }
            TableKind::TextureNames => {
                if let Some(n) = self.node_mut() {
                    n.node
                        .light
                        .get_or_insert_with(Light::default)
                        .texturenames
                        .push(tokens.first().cloned().unwrap_or_default());
                }
            }
        }
    }

    fn ingest_controller_row(&mut self, name: &str, bezier: bool, tokens: &[String]) {
        let Some(n) = self.node_mut() else {
            return;
        };
        let row = if tokens.len() == 1 {
            ControllerRow {
                time: 0.0,
                data: vec![float_tok(tokens, 0)],
            }
        } else {
            ControllerRow {
                time: float_tok(tokens, 0),
                data: tokens[1..].iter().filter_map(|t| t.parse().ok()).collect(),
            }
        };
        if let Some(c) = n.node.controllers.iter_mut().find(|c| c.name == name) {
            c.rows.push(row);
        } else {
            n.node.controllers.push(Controller {
                name: name.to_string(),
                bezier,
                rows: vec![row],
                ..Controller::default()
            });
        }
    }

    fn finish(self) -> Result<Model> {
        let mut model = self.model;
        let mut geom = Vec::new();
        let mut by_anim: BTreeMap<usize, Vec<PendingNode>> = BTreeMap::new();
        for p in self.nodes {
            match p.anim {
                Some(i) => by_anim.entry(i).or_default().push(p),
                None => geom.push(p),
            }
        }
        model.root = build_tree(geom).map(Box::new);
        for (i, nodes) in by_anim {
            if let Some(a) = model.animations.get_mut(i) {
                a.nodes = match build_tree(nodes) {
                    Some(root) => vec![root],
                    None => Vec::new(),
                };
            }
        }
        Ok(model)
    }
}

fn build_tree(pending: Vec<PendingNode>) -> Option<Node> {
    if pending.is_empty() {
        return None;
    }
    let mut by_name: BTreeMap<String, Node> = BTreeMap::new();
    let mut parents: BTreeMap<String, Option<String>> = BTreeMap::new();
    let order: Vec<String> = pending.iter().map(|p| p.node.name.clone()).collect();
    for p in pending {
        parents.insert(p.node.name.clone(), p.parent.clone());
        by_name.insert(p.node.name.clone(), p.node);
    }
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut roots = Vec::new();
    for name in &order {
        match parents.get(name).and_then(|p| p.as_ref()) {
            Some(parent) if by_name.contains_key(parent) => {
                children
                    .entry(parent.clone())
                    .or_default()
                    .push(name.clone());
            }
            _ => roots.push(name.clone()),
        }
    }
    fn assemble(
        name: &str,
        by_name: &mut BTreeMap<String, Node>,
        children: &BTreeMap<String, Vec<String>>,
    ) -> Option<Node> {
        let mut node = by_name.remove(name)?;
        if let Some(kids) = children.get(name) {
            for k in kids {
                if let Some(child) = assemble(k, by_name, children) {
                    node.children.push(child);
                }
            }
        }
        Some(node)
    }
    if roots.is_empty() {
        roots.push(order[0].clone());
    }
    let mut root = assemble(&roots[0], &mut by_name, &children)?;
    for extra in roots.into_iter().skip(1) {
        if let Some(n) = assemble(&extra, &mut by_name, &children) {
            root.children.push(n);
        }
    }
    Some(root)
}

fn tokenize(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_string).collect()
}

fn float_tok(tokens: &[String], i: usize) -> f32 {
    tokens.get(i).and_then(|t| t.parse().ok()).unwrap_or(0.0)
}

fn int_tok(tokens: &[String], i: usize) -> i32 {
    tokens
        .get(i)
        .and_then(|t| t.parse::<f64>().ok())
        .map(|n| n as i32)
        .unwrap_or(0)
}

fn vec2_tok(tokens: &[String], i: usize) -> Vec2 {
    Vec2 {
        x: float_tok(tokens, i),
        y: float_tok(tokens, i + 1),
    }
}

fn vec3_tok(tokens: &[String], i: usize) -> Vec3 {
    Vec3 {
        x: float_tok(tokens, i),
        y: float_tok(tokens, i + 1),
        z: float_tok(tokens, i + 2),
    }
}

fn quat_tok(tokens: &[String], i: usize) -> Quat {
    Quat {
        x: float_tok(tokens, i),
        y: float_tok(tokens, i + 1),
        z: float_tok(tokens, i + 2),
        w: float_tok(tokens, i + 3),
    }
}

fn tokens_to_property(tokens: &[String]) -> Property {
    if tokens.is_empty() {
        return Property::Text(String::new());
    }
    if tokens.len() == 1 {
        if let Ok(n) = tokens[0].parse::<f64>() {
            return Property::Number(n);
        }
        return Property::Text(tokens[0].clone());
    }
    Property::List(
        tokens
            .iter()
            .map(|t| {
                t.parse::<f64>()
                    .map(Property::Number)
                    .unwrap_or_else(|_| Property::Text(t.clone()))
            })
            .collect(),
    )
}
