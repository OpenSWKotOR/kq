//! Load one side of a compare / patch / merge: indexed resource or a file.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::Value as J;

use kq_format::ResType;
use kq_index::Index;

use crate::render;
use crate::resource_json;
use crate::{parse_ref, read, Ctx};

#[derive(Clone, Debug)]
pub struct Loaded {
    pub label: String,
    pub value: J,
}

pub fn load(
    ctx: &Ctx,
    spec: &str,
    from: Option<&str>,
    content_only: bool,
    other_root: Option<&Path>,
) -> Result<Loaded> {
    if spec == "-" {
        let raw = std::io::read_to_string(std::io::stdin()).context("read stdin")?;
        return from_json_text(raw, "<stdin>", content_only);
    }
    let path = PathBuf::from(spec);
    if path.exists() || spec.contains('/') || spec.contains('\\') {
        return load_file(&path, content_only);
    }
    if let Some(root) = other_root {
        let read_cache = ctx.use_cache && !ctx.refresh;
        let (_inst, idx, _) = kq_index::open(root, read_cache, false)?;
        load_indexed(&idx, spec, from, content_only)
    } else {
        let index = ctx.index()?;
        load_indexed(&index, spec, from, content_only)
    }
}

fn load_indexed(
    index: &Index,
    spec: &str,
    from: Option<&str>,
    content_only: bool,
) -> Result<Loaded> {
    let (name, want) = parse_ref(spec, None)?;
    let resource = match from {
        Some(container) => index
            .lookup(&name)
            .iter()
            .map(|&i| &index.resources[i as usize])
            .find(|r| {
                want.is_none_or(|t| r.restype == t)
                    && index.source(r).label.eq_ignore_ascii_case(container)
            }),
        None => index.resolve(&name, want),
    };
    let Some(resource) = resource else {
        bail!("no resource named {spec}");
    };
    let bytes = read::read(index, resource)?;
    let decoded = render::decode_resource(index, resource, &bytes)?;
    let env = resource_json::build_resource_json(index, resource, &decoded);
    let value = if content_only {
        resource_json::decoded_to_json(&decoded)
    } else {
        serde_json::to_value(&env)?
    };
    Ok(Loaded {
        label: index.virt_path(resource),
        value,
    })
}

fn load_file(path: &Path, content_only: bool) -> Result<Loaded> {
    let bytes = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string();
    if looks_like_json(&bytes) {
        let text = String::from_utf8_lossy(&bytes).into_owned();
        return from_json_text(text, &path.display().to_string(), content_only);
    }
    let ext = path.extension().and_then(|s| s.to_str());
    let restype = ext.and_then(ResType::from_extension);
    let mdx = if restype.is_some_and(|t| t.extension() == Some("mdl")) {
        fs::read(path.with_extension("mdx")).ok()
    } else {
        None
    };
    let decoded = match mdx.as_deref() {
        Some(ext) => render::decode_with_mdx(&bytes, Some(ext), restype, &name)?,
        None => render::decode(&bytes, restype, &name)?,
    };
    Ok(Loaded {
        label: path.display().to_string(),
        value: resource_json::decoded_to_json(&decoded),
    })
}

fn from_json_text(text: String, label: &str, content_only: bool) -> Result<Loaded> {
    let v: J = serde_json::from_str(&text).with_context(|| format!("{label}: not JSON"))?;
    let value = if content_only { unwrap_content(&v) } else { v };
    Ok(Loaded {
        label: label.to_string(),
        value,
    })
}

/// kq `cat` envelopes have `content` plus `resref` / `path`.
fn unwrap_content(v: &J) -> J {
    match v {
        J::Object(m)
            if m.contains_key("content")
                && (m.contains_key("resref") || m.contains_key("path")) =>
        {
            m.get("content").cloned().unwrap_or(J::Null)
        }
        other => other.clone(),
    }
}

fn looks_like_json(bytes: &[u8]) -> bool {
    let start = bytes
        .iter()
        .position(|&b| !b.is_ascii_whitespace())
        .map(|i| bytes[i]);
    matches!(start, Some(b'{') | Some(b'['))
}

/// The loaded copy vs the next same-type copy in the resolve chain.
pub fn load_shadow_pair(ctx: &Ctx, spec: &str, content_only: bool) -> Result<(Loaded, Loaded)> {
    let index = ctx.index()?;
    let (name, want) = parse_ref(spec, None)?;
    let copies: Vec<_> = index
        .lookup(&name)
        .iter()
        .map(|&i| &index.resources[i as usize])
        .filter(|r| want.is_none_or(|t| r.restype == t))
        .collect();
    if copies.len() < 2 {
        bail!("{spec}: no shadowed copy (only {} match)", copies.len());
    }
    let left = decode_one(&index, copies[0], content_only)?;
    let right = decode_one(&index, copies[1], content_only)?;
    Ok((left, right))
}

fn decode_one(index: &Index, r: &kq_index::Resource, content_only: bool) -> Result<Loaded> {
    let bytes = read::read(index, r)?;
    let decoded = render::decode_resource(index, r, &bytes)?;
    let env = resource_json::build_resource_json(index, r, &decoded);
    let value = if content_only {
        resource_json::decoded_to_json(&decoded)
    } else {
        serde_json::to_value(&env)?
    };
    Ok(Loaded {
        label: index.virt_path(r),
        value,
    })
}
