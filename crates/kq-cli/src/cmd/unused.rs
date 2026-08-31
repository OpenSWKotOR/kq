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

    /// Exclude textures, models and audio from leftover reports.
    #[arg(long)]
    no_assets: bool,

    /// Only the winning copy of each name (shadowed copies omitted).
    #[arg(long)]
    winners_only: bool,

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
    pub id: u32,
    pub name: String,
    pub path: String,
    pub resref: &'a str,
    #[serde(rename = "type")]
    pub restype: String,
    pub size: u64,
    pub source: &'a str,
    pub container: &'a str,
    pub module: Option<&'a str>,
    pub file: String,
    pub offset: u64,
    pub winner: bool,
    pub parent_path: Option<String>,
    pub mentions: Vec<String>,
    pub status: &'static str,
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
    let candidates = leftover_ids(
        &index,
        &graph,
        &args.filter,
        args.no_assets,
        args.winners_only,
    )?;
    let all_candidates = candidate_ids(&index, &args.filter, args.no_assets, args.winners_only)?;
    let candidate_count = all_candidates.len();
    let winners = winner_set(&index, &all_candidates);
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
                    "Live-graph leftovers, not a runtime trace. Pass --no-assets to drop textures/models/audio."
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

    write_resource_rows(
        ctx,
        &mut w,
        &index,
        &graph,
        &winners,
        &candidates,
        args.limit,
        args.quiet,
    )?;
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

pub fn candidate_ids(
    index: &kq_index::Index,
    filter: &Filter,
    no_assets: bool,
    winners_only: bool,
) -> Result<Vec<u32>> {
    let mut candidates = filter.select(index, "")?;
    if winners_only {
        Filter::dedup_winners(index, &mut candidates);
    }
    if no_assets && filter.types.is_empty() {
        candidates.retain(|&i| !live::is_asset(index.resources[i as usize].restype));
    }
    if filter.types.is_empty() {
        candidates.retain(|&i| !live::is_noise(index.resources[i as usize].restype));
    }
    Ok(candidates)
}

pub fn leftover_ids(
    index: &kq_index::Index,
    graph: &LiveGraph,
    filter: &Filter,
    no_assets: bool,
    winners_only: bool,
) -> Result<Vec<u32>> {
    let mut candidates = candidate_ids(index, filter, no_assets, winners_only)?;
    candidates.retain(|&i| !graph.used_ids.contains(&i));
    Ok(candidates)
}

pub fn winner_set(index: &kq_index::Index, ids: &[u32]) -> std::collections::HashSet<u32> {
    let mut winners = ids.to_vec();
    Filter::dedup_winners(index, &mut winners);
    winners.into_iter().collect()
}

pub fn resource_row<'a>(
    index: &'a kq_index::Index,
    id: u32,
    graph: &LiveGraph,
    winners: &std::collections::HashSet<u32>,
    status: &'static str,
) -> Row<'a> {
    let r = &index.resources[id as usize];
    let source = index.source(r);
    let parent_path = graph
        .parent
        .get(&id)
        .map(|&p| index.virt_path(&index.resources[p as usize]));
    let mentions = graph
        .edges
        .get(&id)
        .map(|set| {
            let mut v: Vec<_> = set.iter().cloned().collect();
            v.sort();
            v
        })
        .unwrap_or_default();
    Row {
        id,
        name: r.filename(),
        path: index.virt_path(r),
        resref: &r.resref,
        restype: r.restype.to_string(),
        size: r.size,
        source: source.kind.as_str(),
        container: &source.label,
        module: source.module_root.as_deref(),
        file: index.rel_file(r),
        offset: r.offset,
        winner: winners.contains(&id),
        parent_path,
        mentions,
        status,
    }
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

#[allow(clippy::too_many_arguments)]
pub fn write_resource_rows(
    ctx: &Ctx,
    w: &mut impl Write,
    index: &kq_index::Index,
    graph: &LiveGraph,
    winners: &std::collections::HashSet<u32>,
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
        if ctx.out.json {
            if quiet {
                ctx.out
                    .json_line(w, &serde_json::json!({ "path": index.virt_path(r) }))?;
            } else {
                ctx.out
                    .json_line(w, &resource_row(index, i, graph, winners, "leftover"))?;
            }
        } else if quiet {
            writeln!(w, "{}", index.virt_path(r))?;
        } else {
            writeln!(w, "{}  {:>10}", ctx.out.accent(&index.virt_path(r)), r.size)?;
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
