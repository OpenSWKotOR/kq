//! `kq ls` — list resources.

use std::io::{BufWriter, Write};

use anyhow::Result;
use serde::Serialize;

use crate::filter::Filter;
use crate::{exit, Ctx};

#[derive(clap::Args)]
pub struct Args {
    /// Name pattern. Bare text is a substring match; `*` and `?` are globs.
    #[arg(value_name = "PATTERN")]
    pattern: Option<String>,

    #[command(flatten)]
    filter: Filter,

    /// Show only the copy the game would actually load.
    #[arg(long)]
    winners: bool,

    /// Stop after this many results. 0 means no limit.
    #[arg(short = 'n', long, default_value_t = 0, value_name = "N")]
    limit: usize,

    /// Print only names, one per line.
    #[arg(short = 'q', long)]
    quiet: bool,
}

#[derive(Serialize)]
struct Row<'a> {
    name: String,
    resref: &'a str,
    #[serde(rename = "type")]
    restype: String,
    size: u64,
    source: &'a str,
    container: &'a str,
    module: Option<&'a str>,
    file: String,
    offset: u64,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<i32> {
    let index = ctx.index()?;

    let mut selected = args
        .filter
        .select(&index, args.pattern.as_deref().unwrap_or(""))?;
    if args.winners {
        Filter::dedup_winners(&index, &mut selected);
    }

    let total = selected.len();
    if args.limit > 0 {
        selected.truncate(args.limit);
    }

    let stdout = std::io::stdout();
    let mut w = BufWriter::new(stdout.lock());

    for &i in &selected {
        let r = &index.resources[i as usize];
        let source = index.source(r);
        if ctx.out.json {
            let row = Row {
                name: r.filename(),
                resref: &r.resref,
                restype: r.restype.to_string(),
                size: r.size,
                source: source.kind.as_str(),
                container: &source.label,
                module: source.module_root.as_deref(),
                file: index.file(r).display().to_string(),
                offset: r.offset,
            };
            ctx.out.json_line(&mut w, &row)?;
        } else if args.quiet {
            writeln!(w, "{}", r.filename())?;
        } else {
            writeln!(
                w,
                "{:<24} {:>10}  {:<13} {}",
                ctx.out.accent(&r.filename()),
                r.size,
                source.kind.as_str(),
                ctx.out.dim(&source.label)
            )?;
        }
    }

    if !ctx.out.json && !args.quiet && args.limit > 0 && total > args.limit {
        writeln!(
            w,
            "{}",
            ctx.out.dim(&format!(
                "... {} more (use -n 0 for all)",
                total - args.limit
            ))
        )?;
    }
    w.flush()?;

    Ok(if total == 0 { exit::NO_MATCH } else { exit::OK })
}
