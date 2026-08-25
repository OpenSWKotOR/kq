//! `kq which` — which copy of a resource actually loads, and what it shadows.
//!
//! KotOR resolves a ResRef by walking locations in a fixed order, so the same
//! name can exist in Override, in a module, and in a BIF, with only one of
//! them ever loading. Getting that wrong is the most common way a mod appears
//! not to work, so the whole chain is shown, not just the winner.

use anyhow::Result;
use serde::Serialize;

use crate::{exit, Ctx};

#[derive(clap::Args)]
pub struct Args {
    /// ResRef to resolve, with or without an extension.
    #[arg(value_name = "RESREF")]
    resref: String,

    /// Restrict to one resource type when the name is ambiguous.
    #[arg(short = 't', long = "type", value_name = "EXT")]
    restype: Option<String>,
}

#[derive(Serialize)]
struct Hit {
    name: String,
    #[serde(rename = "type")]
    restype: String,
    source: &'static str,
    container: String,
    module: Option<String>,
    file: String,
    offset: u64,
    size: u64,
    /// True for the copy the engine loads.
    active: bool,
}

#[derive(Serialize)]
struct Report {
    resref: String,
    matches: usize,
    hits: Vec<Hit>,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<i32> {
    let (_install, index) = ctx.index()?;

    // `kq which nwscript.nss` should work as well as `kq which nwscript`.
    let (name, ext_from_name) = match args.resref.rsplit_once('.') {
        Some((base, ext)) if kq_format::ResType::from_extension(ext).is_some() => {
            (base.to_string(), Some(ext.to_string()))
        }
        _ => (args.resref.clone(), None),
    };
    let ext = args.restype.or(ext_from_name);
    let want = match ext.as_deref() {
        Some(e) => match kq_format::ResType::from_extension(e) {
            Some(t) => Some(t),
            None => anyhow::bail!("unknown resource type: {e}"),
        },
        None => None,
    };

    let ids = index.lookup(&name);
    let mut hits = Vec::new();
    let mut seen_active: std::collections::HashSet<kq_format::ResType> = Default::default();

    for &i in ids {
        let r = &index.resources[i as usize];
        if want.is_some_and(|t| r.restype != t) {
            continue;
        }
        let source = index.source(r);
        // `ids` is already precedence-ordered, so the first of each type wins.
        let active = seen_active.insert(r.restype);
        hits.push(Hit {
            name: r.filename(),
            restype: r.restype.to_string(),
            source: source.kind.as_str(),
            container: source.label.clone(),
            module: source.module_root.clone(),
            file: index.file(r).display().to_string(),
            offset: r.offset,
            size: r.size,
            active,
        });
    }

    if ctx.out.json {
        ctx.out.json_value(&Report {
            resref: name,
            matches: hits.len(),
            hits,
        })?;
        return Ok(if ids.is_empty() {
            exit::NO_MATCH
        } else {
            exit::OK
        });
    }

    if hits.is_empty() {
        eprintln!("kq: no resource named {name}");
        return Ok(exit::NO_MATCH);
    }

    let o = &ctx.out;
    for h in &hits {
        let marker = if h.active { o.bold("*") } else { o.dim(" ") };
        println!(
            "{} {:<22} {:<13} {:<28} {}",
            marker,
            o.accent(&h.name),
            h.source,
            h.container,
            o.dim(&format!("{} bytes", h.size))
        );
    }
    if hits.len() > 1 {
        println!(
            "\n{}",
            o.dim("* is the copy the game loads; the rest are shadowed.")
        );
    }
    Ok(exit::OK)
}
