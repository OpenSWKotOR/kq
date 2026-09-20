//! `kq unused` — leftover *resources* the live graph never reaches.
//!
//! Install mode starts from engine-hardcoded names (2DAs, talk files, a
//! handful of modules and default scripts), then walks ResRef mentions.
//! Isolated A↔B pairs and everything sitting only in `rims/` stay unused
//! unless something live names them.
//!
//! `kq leftovers` is the same graph plus unused `dialog.tlk` rows.

use std::collections::{HashMap, HashSet};
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

    /// Deprecated: winners-only-per-scope is now the default.
    #[arg(long, hide = true)]
    winners_only: bool,

    /// Also list non-winner copies with status=shadowed.
    #[arg(long)]
    shadowed: bool,

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadowed_by: Option<String>,
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
    shadowed: usize,
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
    let _ = args.winners_only;
    let index = ctx.index()?;
    let graph = live::build(&index)?;
    let leftover = leftover_ids(&index, &graph, &args.filter, args.no_assets, false)?;
    let all_candidates = candidate_ids(&index, &args.filter, args.no_assets, args.shadowed)?;
    let candidate_count = all_candidates.len();
    let winners = winner_set(&index, &all_candidates);
    let by_type_rows = count_by_type(&index, &leftover);
    let unused_count = leftover.len();
    let shadowed_ids: Vec<u32> = if args.shadowed {
        all_candidates
            .iter()
            .copied()
            .filter(|i| !winners.contains(i))
            .collect()
    } else {
        Vec::new()
    };

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
            shadowed: shadowed_ids.len(),
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
        ctx, &mut w, &index, &graph, &winners, &leftover, args.limit, args.quiet, "leftover",
    )?;
    if args.shadowed {
        write_resource_rows(
            ctx,
            &mut w,
            &index,
            &graph,
            &winners,
            &shadowed_ids,
            0,
            args.quiet,
            "shadowed",
        )?;
    }
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
    include_shadowed: bool,
) -> Result<Vec<u32>> {
    let winners = live::scoped_winner_id_set(index);
    let mut candidates = filter.select(index, "")?;
    if !include_shadowed {
        candidates.retain(|&i| winners.contains(&i));
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
    include_shadowed: bool,
) -> Result<Vec<u32>> {
    let mut candidates = candidate_ids(index, filter, no_assets, include_shadowed)?;
    candidates.retain(|&i| !graph.used_ids.contains(&i));
    Ok(candidates)
}

pub fn winner_set(index: &kq_index::Index, _ids: &[u32]) -> HashSet<u32> {
    live::scoped_winner_id_set(index)
}

pub fn resource_row<'a>(
    index: &'a kq_index::Index,
    id: u32,
    graph: &LiveGraph,
    winners: &HashSet<u32>,
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
    let shadowed_by =
        live::shadowed_by(index, id).map(|p| index.virt_path(&index.resources[p as usize]));
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
        shadowed_by,
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
    winners: &HashSet<u32>,
    ids: &[u32],
    limit: usize,
    quiet: bool,
    status: &'static str,
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
                    .json_line(w, &resource_row(index, i, graph, winners, status))?;
            }
        } else if quiet {
            writeln!(w, "{}", index.virt_path(r))?;
        } else {
            writeln!(w, "{}  {:>10}", ctx.out.accent(&index.virt_path(r)), r.size)?;
        }
    }
    if status == "leftover" && !ctx.out.json && !quiet && limit > 0 && unused_count > limit {
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

#[cfg(test)]
mod tests {
    use super::*;
    use kq_format::ResType;
    use kq_index::Index;
    use serde_json::json;
    use std::collections::{HashMap, HashSet};

    fn fixture_mod_and_rim() -> Index {
        let ncs = ResType::from_extension("ncs").unwrap().0;
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": [
                "/game/modules/tar_m03aa.mod",
                "/game/modules/tar_m03aa_s.rim",
                "/game/modules/tar_m02aa.mod"
            ],
            "sources": [
                {"kind":"module-mod","label":"tar_m03aa.mod","precedence":100,"module_root":"tar_m03aa"},
                {"kind":"module-rim","label":"tar_m03aa_s.rim","precedence":200,"module_root":"tar_m03aa"},
                {"kind":"module-mod","label":"tar_m02aa.mod","precedence":100,"module_root":"tar_m02aa"}
            ],
            "resources": [
                {"resref":"k_ptar_rndtalk0","restype":ncs,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"k_ptar_rndtalk0","restype":ncs,"file":1,"offset":0,"size":1,"source":1},
                {"resref":"k_ptar_rndtalk0","restype":ncs,"file":2,"offset":0,"size":1,"source":2}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        index
    }

    #[test]
    fn candidate_ids_default_skips_rim_and_foreign_module_copies() {
        let index = fixture_mod_and_rim();
        let ids = candidate_ids(&index, &Filter::default(), false, false).unwrap();
        let winners = live::scoped_winner_id_set(&index);
        assert_eq!(ids.len(), 2);
        assert!(ids.iter().all(|i| winners.contains(i)));
    }

    #[test]
    fn candidate_ids_shadowed_includes_rim_non_winner() {
        let index = fixture_mod_and_rim();
        let all = candidate_ids(&index, &Filter::default(), false, true).unwrap();
        let only = candidate_ids(&index, &Filter::default(), false, false).unwrap();
        assert!(all.len() > only.len());
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn resource_row_shadowed_status_has_shadowed_by() {
        let index = fixture_mod_and_rim();
        let rim_id = 1u32;
        // Prefer adding empty `missing` / `module_entries` on LiveGraph in this task
        // so Task 8 only fills them.
        let graph = LiveGraph {
            catalog: HashSet::new(),
            seeds: vec![],
            seed_ids: vec![],
            reachable: HashSet::new(),
            used_ids: HashSet::new(),
            parent: HashMap::new(),
            edges: HashMap::new(),
            missing: HashMap::new(),
            module_entries: HashMap::new(),
            used: HashSet::new(),
            used_strrefs: HashSet::new(),
            tlk: vec![],
            scanned: 0,
        };
        let winners = live::scoped_winner_id_set(&index);
        let row = resource_row(&index, rim_id, &graph, &winners, "shadowed");
        assert_eq!(row.status, "shadowed");
        assert_eq!(
            row.shadowed_by.as_deref(),
            Some("modules/tar_m03aa.mod/k_ptar_rndtalk0.ncs")
        );
    }
}
