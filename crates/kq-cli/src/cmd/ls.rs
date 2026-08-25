//! `kq ls` — list resources.

use std::io::{BufWriter, Write};

use anyhow::Result;
use serde::Serialize;

use crate::{exit, glob, Ctx};

#[derive(clap::Args)]
pub struct Args {
    /// Name pattern. Bare text is a substring match; `*` and `?` are globs.
    #[arg(value_name = "PATTERN")]
    pattern: Option<String>,

    /// Only this resource type, e.g. `utc`, `2da`, `dlg`. Repeatable.
    #[arg(short = 't', long = "type", value_name = "EXT")]
    types: Vec<String>,

    /// Only resources from this module root, e.g. `danm13`. Repeatable.
    #[arg(short = 'm', long = "module", value_name = "ROOT")]
    modules: Vec<String>,

    /// Only this source kind: override, module-mod, module-rim, lips,
    /// texturepack, rims, stream, chitin.
    #[arg(short = 's', long = "source", value_name = "KIND")]
    sources: Vec<String>,

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
    let (_install, index) = ctx.index()?;

    let want_types: Vec<kq_format::ResType> = args
        .types
        .iter()
        .filter_map(|t| kq_format::ResType::from_extension(t))
        .collect();
    if want_types.len() != args.types.len() {
        let bad: Vec<&String> = args
            .types
            .iter()
            .filter(|t| kq_format::ResType::from_extension(t).is_none())
            .collect();
        anyhow::bail!("unknown resource type(s): {}", join(&bad));
    }

    let pattern = args.pattern.as_deref().unwrap_or("");
    let mut selected: Vec<u32> = Vec::new();

    for (i, r) in index.resources.iter().enumerate() {
        if !glob::matches(pattern, &r.resref) {
            continue;
        }
        if !want_types.is_empty() && !want_types.contains(&r.restype) {
            continue;
        }
        let source = index.source(r);
        if !args.sources.is_empty()
            && !args.sources.iter().any(|s| s.eq_ignore_ascii_case(source.kind.as_str()))
        {
            continue;
        }
        if !args.modules.is_empty() {
            let Some(root) = source.module_root.as_deref() else { continue };
            if !args.modules.iter().any(|m| m.eq_ignore_ascii_case(root)) {
                continue;
            }
        }
        selected.push(i as u32);
    }

    // Deterministic order: name, then precedence, so two runs agree and a
    // diff of two installs lines up.
    selected.sort_by(|&a, &b| {
        let (ra, rb) = (&index.resources[a as usize], &index.resources[b as usize]);
        ra.resref
            .cmp(&rb.resref)
            .then(ra.restype.cmp(&rb.restype))
            .then(index.sources[ra.source as usize].precedence.cmp(
                &index.sources[rb.source as usize].precedence,
            ))
    });

    if args.winners {
        selected.dedup_by(|&mut a, &mut b| {
            let (ra, rb) = (&index.resources[a as usize], &index.resources[b as usize]);
            ra.resref == rb.resref && ra.restype == rb.restype
        });
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
        writeln!(w, "{}", ctx.out.dim(&format!("... {} more (use -n 0 for all)", total - args.limit)))?;
    }
    w.flush()?;

    Ok(if total == 0 { exit::NO_MATCH } else { exit::OK })
}

fn join(items: &[&String]) -> String {
    items.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
}
