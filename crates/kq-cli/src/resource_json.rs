//! Shared JSON envelope for decoded resources (`cat`).

use serde::Serialize;
use serde_json::Value as J;

use kq_index::{Index, Resource};

use crate::render::Decoded;

#[derive(Serialize)]
pub struct ResourceJson<'a> {
    pub name: String,
    pub resref: &'a str,
    #[serde(rename = "type")]
    pub restype: String,
    pub path: String,
    pub source: &'static str,
    pub container: &'a str,
    pub module: Option<&'a str>,
    pub file: String,
    pub offset: u64,
    pub size: u64,
    pub content: J,
}

pub fn decoded_to_json(decoded: &Decoded) -> J {
    match decoded {
        Decoded::Value(v) => v.clone(),
        Decoded::Text(t) => J::String(t.clone()),
        Decoded::Opaque { kind, len } => {
            serde_json::json!({ "kind": kind, "bytes": len, "decoded": false })
        }
    }
}

pub fn build_resource_json<'a>(
    index: &'a Index,
    r: &'a Resource,
    decoded: &Decoded,
) -> ResourceJson<'a> {
    let source = index.source(r);
    ResourceJson {
        name: r.filename(),
        resref: &r.resref,
        restype: r.restype.to_string(),
        path: index.virt_path(r),
        source: source.kind.as_str(),
        container: &source.label,
        module: source.module_root.as_deref(),
        file: index.rel_file(r),
        offset: r.offset,
        size: r.size,
        content: decoded_to_json(decoded),
    }
}
