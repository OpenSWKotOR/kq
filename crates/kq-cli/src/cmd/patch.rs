//! `kq patch` — apply a `kq-delta-1` document to a resource.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use kq_format::delta::{self, Op};

use crate::side;
use crate::{exit, Ctx};

#[derive(clap::Args)]
pub struct Args {
    /// Resource to start from: ResRef, resource file, or JSON (`-` = stdin).
    #[arg(value_name = "TARGET")]
    target: String,

    /// Delta document (`kq delta` JSON). `-` = stdin (not if TARGET is also `-`).
    #[arg(value_name = "DELTA")]
    delta: String,

    /// Target copy from this container label.
    #[arg(long, value_name = "NAME")]
    from: Option<String>,

    /// Include the `cat` envelope when TARGET is a ResRef.
    #[arg(long)]
    envelope: bool,

    /// Write the patched JSON here instead of stdout.
    #[arg(short = 'o', long, value_name = "FILE")]
    output: Option<PathBuf>,
}

#[derive(Deserialize)]
struct DeltaDoc {
    #[serde(default)]
    ops: Vec<Op>,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<i32> {
    if args.target == "-" && args.delta == "-" {
        bail!("TARGET and DELTA cannot both be stdin");
    }
    let loaded = side::load(
        ctx,
        &args.target,
        args.from.as_deref(),
        !args.envelope,
        None,
    )?;
    let raw = if args.delta == "-" {
        std::io::read_to_string(std::io::stdin()).context("read delta from stdin")?
    } else {
        std::fs::read_to_string(&args.delta)
            .with_context(|| format!("cannot read {}", args.delta))?
    };
    let doc: DeltaDoc = serde_json::from_str(&raw).context("delta is not a kq-delta-1 document")?;
    let mut value = loaded.value;
    delta::apply(&mut value, &doc.ops).map_err(|e| anyhow::anyhow!("{e}"))?;

    if let Some(path) = &args.output {
        let file = std::fs::File::create(path)?;
        let mut w = std::io::BufWriter::new(file);
        serde_json::to_writer_pretty(&mut w, &value)?;
        use std::io::Write;
        w.write_all(b"\n")?;
    } else if ctx.out.text {
        let mut s = String::new();
        kq_format::text::outline(&value, &mut s)?;
        print!("{s}");
    } else {
        ctx.out.json_value(&value)?;
    }
    Ok(exit::OK)
}
