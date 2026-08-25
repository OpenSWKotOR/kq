//! `kq cat` — print a resource as text.

use std::io::Write;

use anyhow::Result;

use crate::render::{self, Format};
use crate::{exit, read, Ctx};

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
    let (_install, index) = ctx.index()?;
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
    w.write_all(render::render(&decoded, format, &filename)?.as_bytes())?;
    Ok(exit::OK)
}
