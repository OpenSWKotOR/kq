//! `kq cat` — print a resource as text.

use std::io::Write;

use anyhow::Result;

use crate::render::{self, Format};
use crate::resource_json;
use crate::{exit, read, Ctx};

#[derive(clap::Args)]
pub struct Args {
    /// ResRef to print, with or without an extension.
    #[arg(value_name = "RESREF")]
    resref: String,

    /// Restrict to one resource type when the name is ambiguous.
    #[arg(short = 't', long = "type", value_name = "EXT")]
    restype: Option<String>,

    /// How to render the resource (`json` is the default structured form).
    #[arg(short = 'f', long, value_name = "FORMAT", default_value = "json")]
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

    let format = if args.raw {
        Format::Raw
    } else if ctx.out.text {
        match args.format {
            Format::Json => Format::Outline,
            other => other,
        }
    } else {
        Format::Json
    };

    if format == Format::Raw {
        w.write_all(&bytes)?;
        return Ok(exit::OK);
    }

    let decoded = render::decode_resource(&index, resource, &bytes)?;

    if format == Format::Json {
        let report = resource_json::build_resource_json(&index, resource, &decoded);
        ctx.out.json_value(&report)?;
        return Ok(exit::OK);
    }

    w.write_all(render::render(&decoded, format, &resource.filename())?.as_bytes())?;
    Ok(exit::OK)
}
