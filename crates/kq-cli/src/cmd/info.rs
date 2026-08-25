//! `kq info` — what is this installation?

use std::collections::BTreeMap;

use anyhow::Result;
use serde::Serialize;

use crate::{exit, Ctx};

#[derive(clap::Args)]
pub struct Args {
    /// Break the resource count down by type instead of by source.
    #[arg(long)]
    by_type: bool,
}

#[derive(Serialize)]
struct Report {
    root: String,
    game: &'static str,
    title: &'static str,
    resources: usize,
    modules: usize,
    containers: usize,
    files: usize,
    by_source: BTreeMap<String, usize>,
    by_type: BTreeMap<String, usize>,
    warnings: Vec<String>,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<i32> {
    let (install, index) = ctx.index()?;

    let mut by_source: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_type: BTreeMap<String, usize> = BTreeMap::new();
    for r in &index.resources {
        *by_source
            .entry(index.source(r).kind.as_str().to_string())
            .or_default() += 1;
        *by_type.entry(r.restype.to_string()).or_default() += 1;
    }

    let report = Report {
        root: install.root.display().to_string(),
        game: index.game.as_str(),
        title: index.game.title(),
        resources: index.resources.len(),
        modules: index.module_roots().len(),
        containers: index.sources.len(),
        files: index.files.len(),
        by_source,
        by_type,
        warnings: index.warnings.clone(),
    };

    if ctx.out.json {
        ctx.out.json_value(&report)?;
        return Ok(exit::OK);
    }

    let o = &ctx.out;
    println!("{}", o.bold(report.title));
    println!("  {:<12} {}", o.dim("path"), report.root);
    println!("  {:<12} {}", o.dim("game"), report.game);
    println!("  {:<12} {}", o.dim("resources"), report.resources);
    println!("  {:<12} {}", o.dim("modules"), report.modules);
    println!("  {:<12} {}", o.dim("containers"), report.containers);

    let table = if args.by_type {
        &report.by_type
    } else {
        &report.by_source
    };
    let heading = if args.by_type { "by type" } else { "by source" };
    println!("\n{}", o.bold(heading));

    let mut rows: Vec<(&String, &usize)> = table.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (name, count) in rows.iter().take(if args.by_type { 20 } else { usize::MAX }) {
        println!("  {:<14} {:>7}", o.accent(name), count);
    }
    if args.by_type && rows.len() > 20 {
        println!(
            "  {}",
            o.dim(&format!("... and {} more types", rows.len() - 20))
        );
    }
    Ok(exit::OK)
}
