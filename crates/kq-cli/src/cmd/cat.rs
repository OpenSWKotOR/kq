//! `kq cat` — print a resource as text.

use std::io::Write;

use anyhow::Result;
use serde::Serialize;
use serde_json::Value as J;

use crate::render::{self, Decoded, Format};
use crate::{exit, read, Ctx};

#[derive(Serialize)]
struct Report<'a> {
    name: String,
    resref: &'a str,
    #[serde(rename = "type")]
    restype: String,
    path: String,
    source: &'static str,
    container: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    module: Option<&'a str>,
    file: String,
    offset: u64,
    size: u64,
    content: J,
}

#[derive(clap::Args)]
pub struct Args {
    /// ResRef to print, with or without an extension.
    #[arg(value_name = "RESREF")]
    resref: String,

    /// Restrict to one resource type when the name is ambiguous.
    #[arg(short = 't', long = "type", value_name = "EXT")]
    restype: Option<String>,

    /// How to render the resource.
    #[arg(short = 'f', long, value_name = "FORMAT", default_value = "outline")]
    format: Format,

    /// Write the resource's exact bytes. Same as `--format raw`.
    #[arg(long)]
    raw: bool,

    /// Read from this container instead of the highest-precedence copy.
    #[arg(long, value_name = "NAME")]
    from: Option<String>,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<i32> {
    let index = ctx.index()?;
    let (name, want) = crate::parse_ref(&args.resref, args.restype.as_deref())?;

    let resource = match &args.from {
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
        eprintln!("kq: no resource named {name}");
        return Ok(exit::NO_MATCH);
    };

    let bytes = read::read(&index, resource)?;
    let stdout = std::io::stdout();
    let mut w = stdout.lock();

    let format = if args.raw { Format::Raw } else { args.format };
    if format == Format::Raw {
        w.write_all(&bytes)?;
        return Ok(exit::OK);
    }

    let filename = resource.filename();
    let decoded = render::decode(&bytes, Some(resource.restype), &filename)?;
    // --json is a global flag; honor it even when --format was not given.
    let format = if ctx.out.json && format == Format::Outline {
        Format::Json
    } else {
        format
    };

    if format == Format::Json {
        let source = index.source(resource);
        let report = Report {
            name: filename.clone(),
            resref: &resource.resref,
            restype: resource.restype.to_string(),
            path: index.virt_path(resource),
            source: source.kind.as_str(),
            container: &source.label,
            module: source.module_root.as_deref(),
            file: index.rel_file(resource),
            offset: resource.offset,
            size: resource.size,
            content: decoded_to_json(&decoded),
        };
        ctx.out.json_value(&report)?;
        return Ok(exit::OK);
    }

    w.write_all(render::render(&decoded, format, &filename)?.as_bytes())?;
    Ok(exit::OK)
}

fn decoded_to_json(decoded: &Decoded) -> J {
    match decoded {
        Decoded::Value(v) => v.clone(),
        Decoded::Text(t) => J::String(t.clone()),
        Decoded::Opaque { kind, len } => {
            serde_json::json!({ "kind": kind, "bytes": len, "decoded": false })
        }
    }
}
