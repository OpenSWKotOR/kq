//! `kq merge` — three-way merge of decoded resources.

use std::path::PathBuf;

use anyhow::Result;
use serde::Serialize;

use kq_format::delta::{self, ConflictSide};

use crate::side;
use crate::{exit, Ctx};

#[derive(clap::Args)]
pub struct Args {
    /// Common ancestor: ResRef, resource file, or JSON.
    #[arg(value_name = "BASE")]
    base: String,

    /// First changed side.
    #[arg(value_name = "OURS")]
    ours: String,

    /// Second changed side.
    #[arg(value_name = "THEIRS")]
    theirs: String,

    /// Container label for BASE.
    #[arg(long, value_name = "NAME")]
    from: Option<String>,

    /// Container label for OURS.
    #[arg(long = "from-ours", value_name = "NAME")]
    from_ours: Option<String>,

    /// Container label for THEIRS.
    #[arg(long = "from-theirs", value_name = "NAME")]
    from_theirs: Option<String>,

    /// Include the `cat` envelope.
    #[arg(long)]
    envelope: bool,

    /// On conflict, keep this side instead of a `_conflict` marker.
    #[arg(long, value_name = "SIDE", value_enum)]
    prefer: Option<Prefer>,

    /// Write the merge result here instead of stdout.
    #[arg(short = 'o', long, value_name = "FILE")]
    output: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum Prefer {
    Ours,
    Theirs,
    Base,
}

#[derive(Serialize)]
struct Report<'a> {
    schema: &'static str,
    base: &'a str,
    ours: &'a str,
    theirs: &'a str,
    conflicts: usize,
    value: serde_json::Value,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<i32> {
    let content_only = !args.envelope;
    let base = side::load(ctx, &args.base, args.from.as_deref(), content_only, None)?;
    let ours = side::load(
        ctx,
        &args.ours,
        args.from_ours.as_deref(),
        content_only,
        None,
    )?;
    let theirs = side::load(
        ctx,
        &args.theirs,
        args.from_theirs.as_deref(),
        content_only,
        None,
    )?;
    let (mut value, conflicts) = delta::merge(&base.value, &ours.value, &theirs.value);
    if let Some(prefer) = args.prefer {
        let side = match prefer {
            Prefer::Ours => ConflictSide::Ours,
            Prefer::Theirs => ConflictSide::Theirs,
            Prefer::Base => ConflictSide::Base,
        };
        value = delta::resolve_conflicts(&value, side);
    }
    let remaining = if args.prefer.is_some() { 0 } else { conflicts };

    let report = Report {
        schema: "kq-merge-1",
        base: &base.label,
        ours: &ours.label,
        theirs: &theirs.label,
        conflicts: remaining,
        value,
    };

    if let Some(path) = &args.output {
        let file = std::fs::File::create(path)?;
        serde_json::to_writer_pretty(std::io::BufWriter::new(file), &report.value)?;
    } else if ctx.out.text {
        print_text(ctx, &report);
    } else {
        ctx.out.json_value(&report)?;
    }

    Ok(if remaining == 0 {
        exit::OK
    } else {
        exit::DIFFER
    })
}

fn print_text(ctx: &Ctx, report: &Report<'_>) {
    println!("base   {}", report.base);
    println!("ours   {}", report.ours);
    println!("theirs {}", report.theirs);
    if report.conflicts > 0 {
        println!(
            "{}",
            ctx.out
                .minus(&format!("{} conflict(s)", report.conflicts))
        );
    } else {
        println!("(clean merge)");
    }
    let mut s = String::new();
    let _ = kq_format::text::outline(&report.value, &mut s);
    print!("{s}");
}
