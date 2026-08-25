//! `kq unused` — leftover *resources* the live graph never reaches.
//!
//! Install mode starts from engine-hardcoded names (2DAs, talk files, a
//! handful of modules and default scripts), then walks ResRef mentions.
//! Isolated A↔B pairs and everything sitting only in `rims/` stay unused
//! unless something live names them.
//!
//! `kq leftovers` is the same graph plus unused `dialog.tlk` rows.

use std::collections::HashMap;
use std::io::{BufWriter, Write};

use anyhow::Result;
use serde::Serialize;

use crate::filter::Filter;
use crate::live::{self, LiveGraph};
use crate::{exit, Ctx};

#[derive(clap::Args)]
pub struct Args {
    #[command(flatten)]
    filter: Filter,

    /// Include textures, models and audio in the unused report.
    ///
    /// Off by default: those types are often referenced from mesh data or
    /// the engine in ways `kq` does not yet decode, so the list would lie.
    #[arg(long)]
    assets: bool,

    /// Also consider shadowed copies, not only the winner of each name.
    #[arg(long)]
    all_copies: bool,

    /// Print counts by type instead of every name.
    #[arg(long)]
    summary: bool,

    /// Stop after this many unused names. 0 means no limit.
    #[arg(short = 'n', long, default_value_t = 0, value_name = "N")]
    limit: usize,

    /// Print only install-relative paths, one per line.
    #[arg(short = 'q', long)]
    quiet: bool,
}

#[derive(Serialize)]
pub struct Row<'a> {
    pub name: String,
    pub path: String,
    pub resref: &'a str,
    #[serde(rename = "type")]
    pub restype: String,
    pub size: u64,
    pub source: &'a str,
    pub container: &'a str,
    pub module: Option<&'a str>,
}

#[derive(Serialize)]
struct Summary {
    scanned: usize,
    catalog: usize,
    seeds: usize,
    reachable: usize,
    used: usize,
    candidates: usize,
    unused: usize,
    by_type: Vec<TypeCount>,
}

#[derive(Serialize)]
pub struct TypeCount {
    #[serde(rename = "type")]
    pub restype: String,
    pub count: usize,
    pub bytes: u64,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<i32> {
    let index = ctx.index()?;
    let graph = live::build(&index)?;
    let candidates = leftover_ids(&index, &graph, &args.filter, args.assets, args.all_copies)?;
    let candidate_count = {
        let mut all = args.filter.select(&index, "")?;
        if !args.all_copies {
            Filter::dedup_winners(&index, &mut all);
        }
        if args.filter.types.is_empty() && !args.assets {
            all.retain(|&i| !live::is_noise(index.resources[i as usize].restype));
        }
        all.len()
    };
    let by_type_rows = count_by_type(&index, &candidates);
    let unused_count = candidates.len();

    let stdout = std::io::stdout();
    let mut w = BufWriter::new(stdout.lock());

    if args.summary {
        let summary = Summary {
            scanned: graph.scanned,
            catalog: graph.catalog.len(),
            seeds: graph.seeds.len(),
            reachable: graph.reachable.len(),
            used: graph.used.len(),
            candidates: candidate_count,
            unused: unused_count,
            by_type: by_type_rows,
        };
        if ctx.out.json {
            ctx.out.json_value(&summary)?;
        } else {
            writeln!(
                w,
                "catalog {}  scanned {}  seeds {}  used {}  unused {}",
                summary.catalog, summary.scanned, summary.seeds, summary.used, summary.unused
            )?;
            writeln!(w)?;
            writeln!(w, "{:<8} {:>8} {:>12}", "type", "unused", "bytes")?;
            for row in &summary.by_type {
                writeln!(w, "{:<8} {:>8} {:>12}", row.restype, row.count, row.bytes)?;
            }
            writeln!(
                w,
                "\n{}",
                ctx.out.dim(
                    "Live-graph leftovers, not a runtime trace. Texture/model/audio omitted unless --assets."
                )
            )?;
        }
        w.flush()?;
        return Ok(if unused_count == 0 {
            exit::NO_MATCH
        } else {
            exit::OK
        });
    }

    write_resource_rows(ctx, &mut w, &index, &candidates, args.limit, args.quiet)?;
    if !ctx.out.json && !args.quiet && args.limit == 0 {
        writeln!(
            w,
            "\n{}",
            ctx.out.dim(&format!(
                "{} unused of {} candidates. Live-graph scan, not a runtime trace.",
                unused_count, candidate_count
            ))
        )?;
    }
    w.flush()?;
    Ok(if unused_count == 0 {
        exit::NO_MATCH
    } else {
        exit::OK
    })
}

pub fn leftover_ids(
    index: &kq_index::Index,
    graph: &LiveGraph,
    filter: &Filter,
    assets: bool,
    all_copies: bool,
) -> Result<Vec<u32>> {
    let mut candidates = filter.select(index, "")?;
    if !all_copies {
        Filter::dedup_winners(index, &mut candidates);
    }
    if filter.types.is_empty() && !assets {
        candidates.retain(|&i| !live::is_noise(index.resources[i as usize].restype));
    }
    candidates.retain(|&i| !graph.used_ids.contains(&i));
    Ok(candidates)
}

pub fn count_by_type(index: &kq_index::Index, ids: &[u32]) -> Vec<TypeCount> {
    let mut by_type: HashMap<String, (usize, u64)> = HashMap::new();
    for &i in ids {
        let r = &index.resources[i as usize];
        let ext = r.restype.extension().unwrap_or("unknown").to_string();
        let e = by_type.entry(ext).or_insert((0, 0));
        e.0 += 1;
        e.1 += r.size;
    }
    let mut rows: Vec<TypeCount> = by_type
        .into_iter()
        .map(|(restype, (count, bytes))| TypeCount {
            restype,
            count,
            bytes,
        })
        .collect();
    rows.sort_by(|a, b| b.count.cmp(&a.count).then(a.restype.cmp(&b.restype)));
    rows
}

pub fn write_resource_rows(
    ctx: &Ctx,
    w: &mut impl Write,
    index: &kq_index::Index,
    ids: &[u32],
    limit: usize,
    quiet: bool,
) -> Result<()> {
    let unused_count = ids.len();
    let printed = if limit > 0 && ids.len() > limit {
        &ids[..limit]
    } else {
        ids
    };
    for &i in printed {
        let r = &index.resources[i as usize];
        let source = index.source(r);
        if ctx.out.json {
            ctx.out.json_line(
                w,
                &Row {
                    name: r.filename(),
                    path: index.virt_path(r),
                    resref: &r.resref,
                    restype: r.restype.to_string(),
                    size: r.size,
                    source: source.kind.as_str(),
                    container: &source.label,
                    module: source.module_root.as_deref(),
                },
            )?;
        } else if quiet {
            writeln!(w, "{}", index.virt_path(r))?;
        } else {
            writeln!(
                w,
                "{}  {:>10}",
                ctx.out.accent(&index.virt_path(r)),
                r.size
            )?;
        }
    }
    if !ctx.out.json && !quiet && limit > 0 && unused_count > limit {
        writeln!(
            w,
            "{}",
            ctx.out.dim(&format!(
                "... {} more unused (use -n 0 or --summary)",
                unused_count - limit
            ))
        )?;
    }
    Ok(())
}
