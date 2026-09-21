//! `kq graph` — complete live mention hierarchy and leftover map.
//!
//! JSON output (the default) includes every resource in scope with
//! `status`, `mentions`, `parent_path`, the full reachability tree, and
//! every leftover talk-table row — no truncation.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufWriter, Write};

use anyhow::Result;
use serde::Serialize;

use crate::cmd::unused;
use crate::filter::Filter;
use crate::live::{self, LiveGraph};
use crate::{exit, Ctx};

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
enum What {
    #[default]
    Both,
    Used,
    Leftovers,
}

#[derive(clap::Args)]
pub struct Args {
    #[command(flatten)]
    filter: Filter,

    #[arg(long, value_enum, default_value_t = What::Both)]
    what: What,

    /// Exclude textures, models and audio from the report.
    #[arg(long)]
    no_assets: bool,

    /// Deprecated: winners-only-per-scope is now the default.
    #[arg(long, hide = true)]
    winners_only: bool,

    /// Also list non-winner copies with status=shadowed.
    #[arg(long)]
    shadowed: bool,

    /// Counts only — no tree or path lists.
    #[arg(long)]
    summary: bool,

    /// Tree depth in `--text` mode. 0 means unlimited. JSON always emits the full tree.
    #[arg(long, default_value_t = 0, value_name = "N")]
    depth: usize,

    /// Print install-relative paths only, one per line (`--text` mode).
    #[arg(short = 'q', long)]
    quiet: bool,

    /// Stop after this many rows (`--text` mode only). 0 means no limit.
    #[arg(short = 'n', long, default_value_t = 0, value_name = "N")]
    limit: usize,

    /// ResRef or module root to show (parent chain + outgoing existing/missing).
    #[arg(value_name = "NAME")]
    name: Option<String>,

    /// Include missing ResRef tokens in output.
    #[arg(long)]
    missing: bool,
}

#[derive(Serialize)]
struct TlkRecord {
    strref: i64,
    text: String,
    sound: String,
    status: &'static str,
}

#[derive(Serialize)]
struct Node<'a> {
    id: u32,
    path: String,
    resref: &'a str,
    #[serde(rename = "type")]
    restype: String,
    module: Option<&'a str>,
    mentions: Vec<String>,
    children: Vec<Node<'a>>,
}

#[derive(Serialize)]
struct Section<'a> {
    count: usize,
    resources: Vec<unused::Row<'a>>,
    strings: Vec<TlkRecord>,
    #[serde(default)]
    tree: Vec<Node<'a>>,
    #[serde(default)]
    by_module: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    by_type: Vec<unused::TypeCount>,
}

#[derive(Serialize)]
struct MissingHit {
    path: String,
    token: String,
}

#[derive(Serialize)]
struct Report<'a> {
    scanned: usize,
    catalog_resrefs: usize,
    seeds: &'a [String],
    seed_ids: &'a [u32],
    resources_in_scope: usize,
    used: Section<'a>,
    leftovers: Section<'a>,
    catalog: Vec<unused::Row<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    missing: Vec<MissingHit>,
}

#[derive(Serialize)]
struct NamedNode<'a> {
    id: u32,
    path: String,
    resref: &'a str,
    #[serde(rename = "type")]
    restype: String,
    module: Option<&'a str>,
    parent_chain: Vec<String>,
    edges: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    missing: Vec<String>,
}

#[derive(Serialize)]
struct NamedReport<'a> {
    name: &'a str,
    nodes: Vec<NamedNode<'a>>,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<i32> {
    let _ = args.winners_only;
    let index = ctx.index()?;
    let graph = live::build(&index)?;

    if let Some(name) = args.name.as_deref() {
        return show_named(ctx, &index, &graph, name, true);
    }

    let in_scope = unused::candidate_ids(&index, &args.filter, args.no_assets, args.shadowed)?;
    let winners = unused::winner_set(&index, &in_scope);
    let leftover_ids = unused::leftover_ids(&index, &graph, &args.filter, args.no_assets, false)?;

    let mut used_ids: Vec<u32> = graph
        .used_ids
        .iter()
        .copied()
        .filter(|id| in_scope.contains(id))
        .collect();
    used_ids.sort_unstable();

    let leftover_set: HashSet<u32> = leftover_ids.iter().copied().collect();

    let used_strings = used_tlk_records(&graph);
    let leftover_strings = leftover_tlk_records(&graph);

    let show_used = matches!(args.what, What::Both | What::Used);
    let show_leftovers = matches!(args.what, What::Both | What::Leftovers);

    if args.quiet && !ctx.out.json {
        return write_quiet(
            &index,
            &used_ids,
            &leftover_ids,
            &leftover_strings,
            &args,
            show_used,
            show_leftovers,
        );
    }

    if args.summary {
        return write_summary(
            ctx,
            &graph,
            used_ids.len(),
            leftover_ids.len(),
            leftover_strings.len(),
        );
    }

    let mut catalog: Vec<unused::Row<'_>> = in_scope
        .iter()
        .map(|&id| {
            let status = if live::is_overshadowed(&index, id) {
                "shadowed"
            } else if leftover_set.contains(&id) {
                "leftover"
            } else {
                "used"
            };
            unused::resource_row(&index, id, &graph, &winners, status)
        })
        .collect();
    catalog.sort_by(|a, b| a.path.cmp(&b.path));

    let tree = build_json_forest(&index, &graph, 0);
    let used_by_module = group_by_module(&index, &used_ids);
    let leftover_by_module = group_by_module(&index, &leftover_ids);

    let used_resources: Vec<_> = used_ids
        .iter()
        .map(|&id| unused::resource_row(&index, id, &graph, &winners, "used"))
        .collect();
    let leftover_resources: Vec<_> = leftover_ids
        .iter()
        .map(|&id| unused::resource_row(&index, id, &graph, &winners, "leftover"))
        .collect();

    let missing_hits = if args.missing {
        missing_pairs(&index, &graph)
            .into_iter()
            .map(|(path, token)| MissingHit { path, token })
            .collect()
    } else {
        Vec::new()
    };

    if ctx.out.json {
        let report = Report {
            scanned: graph.scanned,
            catalog_resrefs: graph.catalog.len(),
            seeds: &graph.seeds,
            seed_ids: &graph.seed_ids,
            resources_in_scope: in_scope.len(),
            used: Section {
                count: if show_used { used_resources.len() } else { 0 },
                resources: if show_used {
                    used_resources
                } else {
                    Vec::new()
                },
                strings: if show_used { used_strings } else { Vec::new() },
                tree: if show_used { tree } else { Vec::new() },
                by_module: if show_used {
                    used_by_module
                } else {
                    BTreeMap::new()
                },
                by_type: Vec::new(),
            },
            leftovers: Section {
                count: if show_leftovers {
                    leftover_resources.len()
                } else {
                    0
                },
                resources: if show_leftovers {
                    leftover_resources
                } else {
                    Vec::new()
                },
                strings: if show_leftovers {
                    leftover_strings
                } else {
                    Vec::new()
                },
                tree: Vec::new(),
                by_module: if show_leftovers {
                    leftover_by_module
                } else {
                    BTreeMap::new()
                },
                by_type: if show_leftovers {
                    unused::count_by_type(&index, &leftover_ids)
                } else {
                    Vec::new()
                },
            },
            catalog,
            missing: missing_hits,
        };
        ctx.out.json_value(&report)?;
        return Ok(exit::OK);
    }

    let stdout = std::io::stdout();
    let mut w = BufWriter::new(stdout.lock());
    let o = &ctx.out;

    if show_used {
        writeln!(
            w,
            "{}",
            o.bold(&format!(
                "REACHABLE — {} resources from {} seeds",
                used_ids.len(),
                graph.seeds.len()
            ))
        )?;
        for seed in &graph.seeds {
            writeln!(w, "  seed  {seed}")?;
        }
        writeln!(w)?;
        let forest = build_text_forest(&index, &graph, args.depth);
        for (i, root) in forest.iter().enumerate() {
            if i > 0 {
                writeln!(w)?;
            }
            print_tree(&mut w, root, "", true, o)?;
        }
    }

    if show_leftovers {
        if show_used {
            writeln!(w)?;
        }
        writeln!(
            w,
            "{}",
            o.bold(&format!(
                "LEFTOVERS — {} resources, {} talk-table strings",
                leftover_ids.len(),
                leftover_strings.len()
            ))
        )?;
        let ids = apply_limit(&leftover_ids, args.limit);
        for &i in ids {
            writeln!(w, "  {}", index.virt_path(&index.resources[i as usize]))?;
        }
        for row in apply_limit_slice(&leftover_strings, args.limit) {
            writeln!(w, "  {:>7}  {}", row.strref, one_line(&row.text, 120))?;
        }
        if args.shadowed {
            let shadowed: Vec<u32> = in_scope
                .iter()
                .copied()
                .filter(|&i| live::is_overshadowed(&index, i))
                .collect();
            writeln!(w)?;
            writeln!(
                w,
                "{}",
                o.bold(&format!("SHADOWED — {} copies", shadowed.len()))
            )?;
            for &i in &shadowed {
                writeln!(w, "  {}", index.virt_path(&index.resources[i as usize]))?;
            }
        }
    }

    if args.missing && !missing_hits.is_empty() {
        writeln!(w)?;
        writeln!(
            w,
            "{}",
            o.bold(&format!("MISSING — {} unmatched ResRefs", missing_hits.len()))
        )?;
        for hit in &missing_hits {
            writeln!(w, "  {}  {}", hit.path, hit.token)?;
        }
    }

    w.flush()?;
    Ok(exit::OK)
}

fn used_tlk_records(graph: &LiveGraph) -> Vec<TlkRecord> {
    graph
        .tlk
        .iter()
        .filter(|row| graph.used_strrefs.contains(&row.strref))
        .map(|row| TlkRecord {
            strref: row.strref,
            text: row.text.clone(),
            sound: row.sound.clone(),
            status: "used",
        })
        .collect()
}

fn leftover_tlk_records(graph: &LiveGraph) -> Vec<TlkRecord> {
    graph
        .tlk
        .iter()
        .filter(|row| !graph.used_strrefs.contains(&row.strref))
        .map(|row| TlkRecord {
            strref: row.strref,
            text: row.text.clone(),
            sound: row.sound.clone(),
            status: "leftover",
        })
        .collect()
}

struct TreeNode {
    path: String,
    mentions: Vec<String>,
    children: Vec<TreeNode>,
}

fn build_text_forest(
    index: &kq_index::Index,
    graph: &LiveGraph,
    max_depth: usize,
) -> Vec<TreeNode> {
    let children_map = children_map(graph);
    graph
        .seed_ids
        .iter()
        .map(|&id| build_subtree(index, graph, &children_map, id, max_depth, 0))
        .collect()
}

fn build_json_forest<'a>(
    index: &'a kq_index::Index,
    graph: &'a LiveGraph,
    max_depth: usize,
) -> Vec<Node<'a>> {
    let children_map = children_map(graph);
    graph
        .seed_ids
        .iter()
        .map(|&id| json_subtree(index, graph, &children_map, id, max_depth, 0))
        .collect()
}

fn children_map(graph: &LiveGraph) -> HashMap<u32, Vec<u32>> {
    let mut map: HashMap<u32, Vec<u32>> = HashMap::new();
    for (&child, &parent) in &graph.parent {
        map.entry(parent).or_default().push(child);
    }
    for kids in map.values_mut() {
        kids.sort_unstable();
    }
    map
}

fn sorted_mentions(graph: &LiveGraph, id: u32) -> Vec<String> {
    graph
        .edges
        .get(&id)
        .map(|set| {
            let mut v: Vec<_> = set.iter().cloned().collect();
            v.sort();
            v
        })
        .unwrap_or_default()
}

fn build_subtree(
    index: &kq_index::Index,
    graph: &LiveGraph,
    children_map: &HashMap<u32, Vec<u32>>,
    id: u32,
    max_depth: usize,
    depth: usize,
) -> TreeNode {
    let children = if max_depth > 0 && depth + 1 >= max_depth {
        Vec::new()
    } else {
        children_map
            .get(&id)
            .map(|kids| {
                kids.iter()
                    .map(|&child| {
                        build_subtree(index, graph, children_map, child, max_depth, depth + 1)
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    TreeNode {
        path: index.virt_path(&index.resources[id as usize]),
        mentions: sorted_mentions(graph, id),
        children,
    }
}

fn json_subtree<'a>(
    index: &'a kq_index::Index,
    graph: &'a LiveGraph,
    children_map: &HashMap<u32, Vec<u32>>,
    id: u32,
    max_depth: usize,
    depth: usize,
) -> Node<'a> {
    let r = &index.resources[id as usize];
    let source = index.source(r);
    let children = if max_depth > 0 && depth + 1 >= max_depth {
        Vec::new()
    } else {
        children_map
            .get(&id)
            .map(|kids| {
                kids.iter()
                    .map(|&child| {
                        json_subtree(index, graph, children_map, child, max_depth, depth + 1)
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    Node {
        id,
        path: index.virt_path(r),
        resref: &r.resref,
        restype: r.restype.to_string(),
        module: source.module_root.as_deref(),
        mentions: sorted_mentions(graph, id),
        children,
    }
}

fn group_by_module(index: &kq_index::Index, ids: &[u32]) -> BTreeMap<String, Vec<String>> {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for &id in ids {
        let r = &index.resources[id as usize];
        let key = index
            .source(r)
            .module_root
            .clone()
            .unwrap_or_else(|| index.source(r).kind.as_str().to_string());
        map.entry(key).or_default().push(index.virt_path(r));
    }
    for paths in map.values_mut() {
        paths.sort();
    }
    map
}

fn print_tree(
    w: &mut impl Write,
    node: &TreeNode,
    prefix: &str,
    is_last: bool,
    o: &crate::output::Out,
) -> Result<()> {
    let branch = if is_last { "└─ " } else { "├─ " };
    writeln!(w, "{prefix}{branch}{}", o.accent(&node.path))?;
    if !node.mentions.is_empty() {
        writeln!(
            w,
            "{}",
            o.dim(&format!("{}   → {}", prefix, node.mentions.join(", ")))
        )?;
    }
    let child_prefix = format!("{prefix}{}   ", if is_last { " " } else { "│" });
    for (i, child) in node.children.iter().enumerate() {
        print_tree(w, child, &child_prefix, i + 1 == node.children.len(), o)?;
    }
    Ok(())
}

fn write_quiet(
    index: &kq_index::Index,
    used_ids: &[u32],
    leftover_ids: &[u32],
    leftover_strings: &[TlkRecord],
    args: &Args,
    show_used: bool,
    show_leftovers: bool,
) -> Result<i32> {
    let stdout = std::io::stdout();
    let mut w = BufWriter::new(stdout.lock());
    if show_used {
        for &id in used_ids {
            writeln!(w, "{}", index.virt_path(&index.resources[id as usize]))?;
        }
    }
    if show_leftovers {
        for &i in apply_limit(leftover_ids, args.limit) {
            writeln!(w, "{}", index.virt_path(&index.resources[i as usize]))?;
        }
        for row in apply_limit_slice(leftover_strings, args.limit) {
            writeln!(w, "dialog.tlk#{}/{}", row.strref, row.status)?;
        }
    }
    w.flush()?;
    Ok(exit::OK)
}

fn write_summary(
    ctx: &Ctx,
    graph: &LiveGraph,
    used: usize,
    leftover_resources: usize,
    leftover_strings: usize,
) -> Result<i32> {
    let payload = serde_json::json!({
        "scanned": graph.scanned,
        "catalog_resrefs": graph.catalog.len(),
        "seeds": graph.seeds,
        "used_resources": used,
        "leftover_resources": leftover_resources,
        "tlk_entries": graph.tlk.len(),
        "leftover_strings": leftover_strings,
    });
    if ctx.out.json {
        ctx.out.json_value(&payload)?;
    } else {
        let stdout = std::io::stdout();
        let mut w = BufWriter::new(stdout.lock());
        writeln!(
            w,
            "catalog {}  scanned {}  seeds {}  used {}  leftovers {} resources / {} strings",
            graph.catalog.len(),
            graph.scanned,
            graph.seeds.len(),
            used,
            leftover_resources,
            leftover_strings
        )?;
        w.flush()?;
    }
    Ok(exit::OK)
}

fn apply_limit(ids: &[u32], limit: usize) -> &[u32] {
    if limit > 0 && ids.len() > limit {
        &ids[..limit]
    } else {
        ids
    }
}

fn apply_limit_slice<T>(rows: &[T], limit: usize) -> &[T] {
    if limit > 0 && rows.len() > limit {
        &rows[..limit]
    } else {
        rows
    }
}

fn normalize_name(name: &str) -> String {
    let name = name.to_ascii_lowercase();
    match name.rsplit_once('.') {
        Some((base, ext)) if kq_format::ResType::from_extension(ext).is_some() => base.to_string(),
        _ => name,
    }
}

fn ids_for_name(index: &kq_index::Index, graph: &LiveGraph, name: &str) -> Vec<u32> {
    let name = normalize_name(name);
    let mut ids: Vec<u32> = live::scoped_loaded(index)
        .into_iter()
        .filter(|((_, rr, _), _)| *rr == name)
        .map(|(_, id)| id)
        .collect();
    if ids.is_empty() {
        if let Some(list) = graph.module_entries.get(&name) {
            ids.extend(list.iter().copied());
        }
    }
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn parent_chain(parent: &HashMap<u32, u32>, id: u32) -> Vec<u32> {
    let mut ids = vec![id];
    let mut cur = id;
    let mut seen = HashSet::from([id]);
    while let Some(&p) = parent.get(&cur) {
        if !seen.insert(p) {
            break;
        }
        ids.push(p);
        cur = p;
    }
    ids
}

fn missing_pairs(index: &kq_index::Index, graph: &LiveGraph) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for (&id, tokens) in &graph.missing {
        if (id as usize) >= index.resources.len() {
            continue;
        }
        let path = index.virt_path(&index.resources[id as usize]);
        let mut toks: Vec<_> = tokens.iter().cloned().collect();
        toks.sort();
        for tok in toks {
            pairs.push((path.clone(), tok));
        }
    }
    pairs.sort();
    pairs
}

fn sorted_set(set: Option<&HashSet<String>>) -> Vec<String> {
    set.map(|s| {
        let mut v: Vec<_> = s.iter().cloned().collect();
        v.sort();
        v
    })
    .unwrap_or_default()
}

fn show_named(
    ctx: &Ctx,
    index: &kq_index::Index,
    graph: &LiveGraph,
    name: &str,
    show_missing: bool,
) -> Result<i32> {
    let ids = ids_for_name(index, graph, name);
    if ids.is_empty() {
        if !ctx.out.json {
            eprintln!("kq: no resource or module named `{name}`");
        }
        return Ok(exit::NO_MATCH);
    }

    let mut nodes = Vec::new();
    for id in ids {
        let r = &index.resources[id as usize];
        let source = index.source(r);
        let chain_paths: Vec<String> = parent_chain(&graph.parent, id)
            .into_iter()
            .map(|i| index.virt_path(&index.resources[i as usize]))
            .collect();
        nodes.push(NamedNode {
            id,
            path: index.virt_path(r),
            resref: &r.resref,
            restype: r.restype.to_string(),
            module: source.module_root.as_deref(),
            parent_chain: chain_paths,
            edges: sorted_set(graph.edges.get(&id)),
            missing: if show_missing {
                sorted_set(graph.missing.get(&id))
            } else {
                Vec::new()
            },
        });
    }

    if ctx.out.json {
        ctx.out.json_value(&NamedReport { name, nodes })?;
        return Ok(exit::OK);
    }

    let stdout = std::io::stdout();
    let mut w = BufWriter::new(stdout.lock());
    let o = &ctx.out;
    writeln!(w, "{}", o.bold(name))?;
    for node in &nodes {
        writeln!(w, "  {}", o.accent(&node.path))?;
        if !node.parent_chain.is_empty() {
            writeln!(
                w,
                "{}",
                o.dim(&format!("    parent  {}", node.parent_chain.join(" ← ")))
            )?;
        }
        if !node.edges.is_empty() {
            writeln!(w, "    edges   {}", node.edges.join(", "))?;
        }
        if show_missing && !node.missing.is_empty() {
            writeln!(w, "    missing {}", node.missing.join(", "))?;
        }
    }
    w.flush()?;
    Ok(exit::OK)
}

fn one_line(s: &str, max: usize) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    if flat.chars().count() <= max {
        return flat;
    }
    let mut out = String::new();
    for (i, c) in flat.chars().enumerate() {
        if i + 1 >= max {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use kq_format::ResType;
    use kq_index::Index;
    use serde_json::json;

    fn fixture_named_script() -> (Index, LiveGraph) {
        let ncs = ResType::from_extension("ncs").unwrap().0;
        let ifo = ResType::from_extension("ifo").unwrap().0;
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": ["/game/modules/end_m01aa.mod"],
            "sources": [
                {"kind":"module-mod","label":"end_m01aa.mod","precedence":100,"module_root":"end_m01aa"}
            ],
            "resources": [
                {"resref":"module","restype":ifo,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"k_ai_master","restype":ncs,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"k_sup_gohawk","restype":ncs,"file":0,"offset":0,"size":1,"source":0}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();

        let mut parent = HashMap::new();
        parent.insert(2u32, 1u32);
        let mut edges = HashMap::new();
        edges.insert(2u32, HashSet::from(["k_ai_master".into()]));
        let mut missing = HashMap::new();
        missing.insert(2u32, HashSet::from(["k_rapidtransit".into()]));
        let mut module_entries = HashMap::new();
        module_entries.insert("end_m01aa".into(), vec![0u32]);

        let graph = LiveGraph {
            catalog: HashSet::from(["module".into(), "k_ai_master".into(), "k_sup_gohawk".into()]),
            seeds: vec!["end_m01aa".into()],
            seed_ids: vec![0],
            reachable: HashSet::new(),
            used_ids: HashSet::from([0, 1, 2]),
            parent,
            edges,
            missing,
            module_entries,
            used: HashSet::new(),
            used_strrefs: HashSet::new(),
            tlk: vec![],
            scanned: 3,
        };
        (index, graph)
    }

    #[test]
    fn named_node_lists_parent_and_missing() {
        let (index, graph) = fixture_named_script();
        let ids = ids_for_name(&index, &graph, "k_sup_gohawk");
        assert_eq!(ids, vec![2]);
        let chain = parent_chain(&graph.parent, 2);
        assert_eq!(chain, vec![2, 1]);
        assert!(graph.missing.get(&2).unwrap().contains("k_rapidtransit"));
        assert!(graph.edges.get(&2).unwrap().contains("k_ai_master"));
        let pairs = missing_pairs(&index, &graph);
        assert!(
            pairs
                .iter()
                .any(|(path, tok)| path.contains("k_sup_gohawk") && tok == "k_rapidtransit")
        );
    }

    #[test]
    fn named_module_root_uses_module_entries() {
        let (index, graph) = fixture_named_script();
        assert_eq!(ids_for_name(&index, &graph, "end_m01aa"), vec![0]);
        assert!(ids_for_name(&index, &graph, "no_such").is_empty());
    }
}
