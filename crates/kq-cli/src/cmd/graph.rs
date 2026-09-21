//! `kq graph` — used, unused, and overshadowed copies in this install.
//!
//! Default text is counts, then unused / overshadowed / unused talk lists.
//! JSON (`--json`) uses `status` of `used`, `unused`, or `overshadowed`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufWriter, Write};

use anyhow::Result;
use serde::Serialize;

use crate::filter::Filter;
use crate::live::{self, LiveGraph};
use crate::{exit, Ctx};

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum Format {
    Lists,
    Tree,
    Summary,
}

#[derive(clap::Args)]
pub struct Args {
    #[command(flatten)]
    filter: Filter,

    /// Exclude textures, models and audio from the report.
    #[arg(long)]
    no_assets: bool,

    /// `lists` also prints used paths; `tree` is the reachability tree then unused and overshadowed lists; `summary` is counts (by type unless `-t`).
    #[arg(long, value_enum)]
    format: Option<Format>,

    /// Tree depth for `--format tree`. 0 means unlimited.
    #[arg(long, default_value_t = 0, value_name = "N")]
    depth: usize,

    /// Print install-relative paths only, one per line.
    #[arg(short = 'q', long)]
    quiet: bool,

    /// Stop after this many rows. 0 means no limit.
    #[arg(short = 'n', long, default_value_t = 0, value_name = "N")]
    limit: usize,

    /// ResRef or module root to show (parent chain + outgoing existing/missing).
    #[arg(value_name = "NAME")]
    name: Option<String>,

    /// Include missing ResRef tokens (NAME mode).
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
struct Row<'a> {
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
    pub parent_path: Option<String>,
    pub mentions: Vec<String>,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden_by: Option<String>,
}

#[derive(Serialize)]
struct TypeCount {
    #[serde(rename = "type")]
    restype: String,
    count: usize,
    bytes: u64,
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
    resources: Vec<Row<'a>>,
    strings: Vec<TlkRecord>,
    #[serde(default)]
    tree: Vec<Node<'a>>,
    #[serde(default)]
    by_module: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    by_type: Vec<TypeCount>,
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
    unused: Section<'a>,
    overshadowed: Section<'a>,
    catalog: Vec<Row<'a>>,
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
    status: &'static str,
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
    let index = ctx.index()?;
    let graph = live::build(&index)?;

    if let Some(name) = args.name.as_deref() {
        return show_named(ctx, &index, &graph, name, args.missing);
    }

    let in_scope = candidate_ids(&index, &args.filter, args.no_assets)?;
    let in_scope_set: HashSet<u32> = in_scope.iter().copied().collect();
    let unused_ids = leftover_ids(&index, &graph, &args.filter, args.no_assets)?;
    let mut overshadowed_ids: Vec<u32> = in_scope
        .iter()
        .copied()
        .filter(|&id| live::is_overshadowed(&index, id))
        .collect();
    overshadowed_ids.sort_unstable();

    let mut used_ids: Vec<u32> = graph
        .used_ids
        .iter()
        .copied()
        .filter(|id| in_scope_set.contains(id) && !live::is_overshadowed(&index, *id))
        .collect();
    used_ids.sort_unstable();

    let used_strings = used_tlk_records(&graph);
    let unused_strings = unused_tlk_records(&graph);

    let used_paths = paths_for(&index, &used_ids);
    let unused_paths = paths_for(&index, &unused_ids);
    let overshadowed_pairs = hidden_pairs(&index, &overshadowed_ids);
    let talk_lines = talk_first_lines(&unused_strings);

    if ctx.out.json {
        return write_json(
            ctx,
            &index,
            &graph,
            &in_scope,
            &used_ids,
            &unused_ids,
            &overshadowed_ids,
            used_strings,
            unused_strings,
        );
    }

    let include_used_list = matches!(args.format, Some(Format::Lists));
    if args.quiet {
        return write_quiet(
            apply_limit_slice(&used_paths, args.limit),
            apply_limit_slice(&unused_paths, args.limit),
            apply_limit_slice(&overshadowed_pairs, args.limit),
            include_used_list,
        );
    }

    let stdout = std::io::stdout();
    let mut w = BufWriter::new(stdout.lock());
    let o = &ctx.out;

    match args.format {
        Some(Format::Summary) => {
            write!(
                w,
                "{}",
                format_counts(used_ids.len(), unused_ids.len(), overshadowed_ids.len())
            )?;
            if args.filter.types.is_empty() {
                writeln!(w)?;
                writeln!(w, "{:<8} {:>8} {:>12}", "type", "unused", "bytes")?;
                for row in count_by_type(&index, &unused_ids) {
                    writeln!(w, "{:<8} {:>8} {:>12}", row.restype, row.count, row.bytes)?;
                }
            }
        }
        Some(Format::Tree) => {
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
            let lists = format_list_sections(
                &[],
                &unused_paths,
                &overshadowed_pairs,
                &talk_lines,
                args.limit,
            );
            if !lists.is_empty() {
                writeln!(w)?;
                write!(w, "{lists}")?;
            }
        }
        Some(Format::Lists) | None => {
            write!(
                w,
                "{}",
                format_counts(used_ids.len(), unused_ids.len(), overshadowed_ids.len())
            )?;
            let used_list = if include_used_list { &used_paths } else { &[][..] };
            let lists = format_list_sections(
                used_list,
                &unused_paths,
                &overshadowed_pairs,
                &talk_lines,
                args.limit,
            );
            if !lists.is_empty() {
                writeln!(w)?;
                write!(w, "{lists}")?;
            }
        }
    }

    w.flush()?;
    Ok(exit::OK)
}

fn write_json(
    ctx: &Ctx,
    index: &kq_index::Index,
    graph: &LiveGraph,
    in_scope: &[u32],
    used_ids: &[u32],
    unused_ids: &[u32],
    overshadowed_ids: &[u32],
    used_strings: Vec<TlkRecord>,
    unused_strings: Vec<TlkRecord>,
) -> Result<i32> {
    let mut catalog: Vec<Row<'_>> = in_scope
        .iter()
        .map(|&id| resource_row(index, id, graph, copy_status(index, graph, id)))
        .collect();
    catalog.sort_by(|a, b| a.path.cmp(&b.path));

    let used_resources: Vec<_> = used_ids
        .iter()
        .map(|&id| resource_row(index, id, graph, "used"))
        .collect();
    let unused_resources: Vec<_> = unused_ids
        .iter()
        .map(|&id| resource_row(index, id, graph, "unused"))
        .collect();
    let overshadowed_resources: Vec<_> = overshadowed_ids
        .iter()
        .map(|&id| resource_row(index, id, graph, "overshadowed"))
        .collect();

    let report = Report {
        scanned: graph.scanned,
        catalog_resrefs: graph.catalog.len(),
        seeds: &graph.seeds,
        seed_ids: &graph.seed_ids,
        resources_in_scope: in_scope.len(),
        used: Section {
            count: used_resources.len(),
            resources: used_resources,
            strings: used_strings,
            tree: build_json_forest(index, graph, 0),
            by_module: group_by_module(index, used_ids),
            by_type: count_by_type(index, used_ids),
        },
        unused: Section {
            count: unused_resources.len(),
            resources: unused_resources,
            strings: unused_strings,
            tree: Vec::new(),
            by_module: group_by_module(index, unused_ids),
            by_type: count_by_type(index, unused_ids),
        },
        overshadowed: Section {
            count: overshadowed_resources.len(),
            resources: overshadowed_resources,
            strings: Vec::new(),
            tree: Vec::new(),
            by_module: group_by_module(index, overshadowed_ids),
            by_type: count_by_type(index, overshadowed_ids),
        },
        catalog,
        missing: Vec::new(),
    };
    ctx.out.json_value(&report)?;
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

fn unused_tlk_records(graph: &LiveGraph) -> Vec<TlkRecord> {
    graph
        .leftover_strings()
        .into_iter()
        .map(|row| TlkRecord {
            strref: row.strref,
            text: row.text.clone(),
            sound: row.sound.clone(),
            status: "unused",
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
    used: &[String],
    unused: &[String],
    overshadowed: &[(String, String)],
    include_used: bool,
) -> Result<i32> {
    let stdout = std::io::stdout();
    let mut w = BufWriter::new(stdout.lock());
    if include_used {
        for path in used {
            writeln!(w, "{path}")?;
        }
    }
    for path in unused {
        writeln!(w, "{path}")?;
    }
    for (path, _) in overshadowed {
        writeln!(w, "{path}")?;
    }
    w.flush()?;
    Ok(exit::OK)
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
            status: copy_status(index, graph, id),
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
        writeln!(w, "    status  {}", node.status)?;
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

fn format_counts(used: usize, unused: usize, overshadowed: usize) -> String {
    format!("used {used}\nunused {unused}\novershadowed {overshadowed}\n")
}

fn format_list_sections(
    used: &[String],
    unused: &[String],
    overshadowed: &[(String, String)],
    unused_talk: &[(i64, String)],
    limit: usize,
) -> String {
    let mut sections = Vec::new();
    if !used.is_empty() {
        let mut s = format!("Used ({})\n", used.len());
        for path in apply_limit_slice(used, limit) {
            s.push_str(&format!("  {path}\n"));
        }
        sections.push(s);
    }
    if !unused.is_empty() {
        let mut s = format!("Unused ({})\n", unused.len());
        for path in apply_limit_slice(unused, limit) {
            s.push_str(&format!("  {path}\n"));
        }
        sections.push(s);
    }
    if !overshadowed.is_empty() {
        let mut s = format!("Overshadowed ({})\n", overshadowed.len());
        for (path, hidden) in apply_limit_slice(overshadowed, limit) {
            s.push_str(&format!("  {path}  (hidden by {hidden})\n"));
        }
        sections.push(s);
    }
    if !unused_talk.is_empty() {
        let mut s = format!("Unused talk ({})\n", unused_talk.len());
        for (strref, line) in apply_limit_slice(unused_talk, limit) {
            s.push_str(&format!("  {strref}  {line}\n"));
        }
        sections.push(s);
    }
    sections.join("\n")
}

fn format_inventory_text(
    used: &[String],
    unused: &[String],
    overshadowed: &[(String, String)],
    unused_talk: &[(i64, String)],
    include_used_list: bool,
) -> String {
    let mut out = format_counts(used.len(), unused.len(), overshadowed.len());
    let used_list = if include_used_list { used } else { &[][..] };
    let lists = format_list_sections(used_list, unused, overshadowed, unused_talk, 0);
    if !lists.is_empty() {
        out.push('\n');
        out.push_str(&lists);
    }
    out
}

fn copy_status(index: &kq_index::Index, graph: &LiveGraph, id: u32) -> &'static str {
    if live::is_overshadowed(index, id) {
        "overshadowed"
    } else if graph.used_ids.contains(&id) {
        "used"
    } else {
        "unused"
    }
}

fn candidate_ids(
    index: &kq_index::Index,
    filter: &Filter,
    no_assets: bool,
) -> Result<Vec<u32>> {
    let mut candidates = filter.select(index, "")?;
    if no_assets && filter.types.is_empty() {
        candidates.retain(|&i| !live::is_asset(index.resources[i as usize].restype));
    }
    if filter.types.is_empty() {
        candidates.retain(|&i| !live::is_noise(index.resources[i as usize].restype));
    }
    Ok(candidates)
}

fn leftover_ids(
    index: &kq_index::Index,
    graph: &LiveGraph,
    filter: &Filter,
    no_assets: bool,
) -> Result<Vec<u32>> {
    let loaded = live::scoped_loaded_id_set(index);
    let mut candidates = candidate_ids(index, filter, no_assets)?;
    candidates.retain(|&i| {
        loaded.contains(&i) && !graph.used_ids.contains(&i) && !live::is_overshadowed(index, i)
    });
    Ok(candidates)
}

fn resource_row<'a>(
    index: &'a kq_index::Index,
    id: u32,
    graph: &LiveGraph,
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
    let hidden_by =
        live::overshadowed_by(index, id).map(|p| index.virt_path(&index.resources[p as usize]));
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
        parent_path,
        mentions,
        status,
        hidden_by,
    }
}

fn count_by_type(index: &kq_index::Index, ids: &[u32]) -> Vec<TypeCount> {
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
        .map(|(restype, (count, bytes))| TypeCount { restype, count, bytes })
        .collect();
    rows.sort_by(|a, b| b.count.cmp(&a.count).then(a.restype.cmp(&b.restype)));
    rows
}

fn paths_for(index: &kq_index::Index, ids: &[u32]) -> Vec<String> {
    ids.iter()
        .map(|&id| index.virt_path(&index.resources[id as usize]))
        .collect()
}

fn hidden_pairs(index: &kq_index::Index, ids: &[u32]) -> Vec<(String, String)> {
    ids.iter()
        .filter_map(|&id| {
            let path = index.virt_path(&index.resources[id as usize]);
            let hidden = live::overshadowed_by(index, id)
                .map(|p| index.virt_path(&index.resources[p as usize]))?;
            Some((path, hidden))
        })
        .collect()
}

fn talk_first_lines(rows: &[TlkRecord]) -> Vec<(i64, String)> {
    rows.iter()
        .map(|row| {
            let line = row.text.lines().next().unwrap_or("");
            (row.strref, one_line(line, 120))
        })
        .collect()
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
    use clap::Parser;
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

    fn graph_help() -> String {
        use clap::CommandFactory;
        let mut cmd = crate::Cli::command();
        let mut buf = Vec::new();
        cmd.find_subcommand_mut("graph")
            .unwrap()
            .write_long_help(&mut buf)
            .unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn unused_is_not_a_subcommand() {
        let parsed = crate::Cli::try_parse_from(["kq", "unused"]);
        assert!(parsed.is_err(), "kq unused must clap-error, not a subcommand");
    }

    #[test]
    fn leftovers_is_not_a_subcommand() {
        let parsed = crate::Cli::try_parse_from(["kq", "leftovers"]);
        assert!(
            parsed.is_err(),
            "kq leftovers must clap-error, not a subcommand"
        );
        let parsed = crate::Cli::try_parse_from(["kq", "leftover"]);
        assert!(
            parsed.is_err(),
            "kq leftover alias must clap-error, not a subcommand"
        );
    }

    #[test]
    fn export_is_not_a_subcommand() {
        let parsed = crate::Cli::try_parse_from(["kq", "export"]);
        assert!(parsed.is_err(), "kq export must clap-error, not a subcommand");
    }

    #[test]
    fn dropped_flags_are_rejected() {
        for flag in ["--what", "--shadowed", "--winners-only", "--summary"] {
            let parsed = crate::Cli::try_parse_from(["kq", "graph", flag]);
            assert!(
                parsed.is_err(),
                "kq graph {flag} must clap-error, not alias"
            );
        }
    }

    #[test]
    fn format_values_are_accepted() {
        for value in ["lists", "tree", "summary"] {
            crate::Cli::try_parse_from(["kq", "graph", "--format", value])
                .unwrap_or_else(|e| panic!("--format {value} should parse: {e}"));
        }
    }

    #[test]
    fn default_inventory_text_has_unused_heading() {
        let text = format_inventory_text(
            &["modules/danm13/danm13.rim/k_foo.ncs".into()],
            &["modules/danm13/danm13.rim/k_dead.ncs".into()],
            &[(
                "modules/danm13s.rim/k_ai_master.ncs".into(),
                "Override/k_ai_master.ncs".into(),
            )],
            &[(12345, "Some leftover line".into())],
            false,
        );
        assert!(
            text.contains("Unused (1)"),
            "default text must have Unused heading, got:\n{text}"
        );
        assert!(text.starts_with("used 1\nunused 1\novershadowed 1\n"), "{text}");
        assert!(
            text.contains("  modules/danm13/danm13.rim/k_dead.ncs\n"),
            "{text}"
        );
        assert!(
            text.contains(
                "  modules/danm13s.rim/k_ai_master.ncs  (hidden by Override/k_ai_master.ncs)\n"
            ),
            "{text}"
        );
        assert!(text.contains("Unused talk (1)"), "{text}");
        assert!(text.contains("  12345  Some leftover line\n"), "{text}");
        assert!(!text.contains("Used ("), "{text}");
    }

    #[test]
    fn json_status_is_unused_not_leftover() {
        let (index, graph) = fixture_named_script();
        let status = copy_status(&index, &graph, 2);
        assert_eq!(status, "used");

        let mut unused_graph = graph;
        unused_graph.used_ids.clear();
        let status = copy_status(&index, &unused_graph, 2);
        assert_eq!(status, "unused");

        let row = resource_row(&index, 2, &unused_graph, "unused");
        let v = serde_json::to_value(&row).unwrap();
        assert_eq!(v["status"], "unused");
        assert!(v.get("winner").is_none(), "{v}");
        assert!(v.get("shadowed_by").is_none(), "{v}");
        assert!(v.get("leftover").is_none(), "{v}");
    }

    fn fixture_module_copy_overshadowed_by_override() -> Index {
        let ncs = ResType::from_extension("ncs").unwrap().0;
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": [
                "/game/modules/tar_m03aa.mod",
                "/game/Override/shared.ncs"
            ],
            "sources": [
                {"kind":"module-mod","label":"tar_m03aa.mod","precedence":100,"module_root":"tar_m03aa"},
                {"kind":"override","label":"Override","precedence":0,"module_root":null}
            ],
            "resources": [
                {"resref":"shared","restype":ncs,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"shared","restype":ncs,"file":1,"offset":0,"size":1,"source":1}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        index
    }

    #[test]
    fn candidate_ids_include_overshadowed_copies() {
        let index = fixture_module_copy_overshadowed_by_override();
        let module_copy_id = 0u32;
        assert!(live::is_overshadowed(&index, module_copy_id));
        let ids = candidate_ids(&index, &Filter::default(), false).unwrap();
        assert!(
            ids.contains(&module_copy_id),
            "overshadowed copies must stay in the report"
        );
        assert!(ids.contains(&1));
    }

    #[test]
    fn leftover_ids_are_unused_not_overshadowed() {
        let index = fixture_module_copy_overshadowed_by_override();
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
        let unused = leftover_ids(&index, &graph, &Filter::default(), false).unwrap();
        assert!(
            !unused.contains(&0),
            "overshadowed module copy is not unused"
        );
        assert!(unused.contains(&1), "loaded override with no walk is unused");
    }

    #[test]
    fn resource_row_overshadowed_uses_hidden_by() {
        let index = fixture_module_copy_overshadowed_by_override();
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
        let row = resource_row(&index, 0, &graph, "overshadowed");
        let v = serde_json::to_value(&row).unwrap();
        assert_eq!(v["status"], "overshadowed");
        assert_eq!(v["hidden_by"], "Override/shared.ncs");
        assert!(v.get("shadowed_by").is_none(), "{v}");
        assert!(v.get("winner").is_none(), "{v}");
    }

    #[test]
    fn named_status_is_used_unused_or_overshadowed() {
        let (index, graph) = fixture_named_script();
        assert_eq!(copy_status(&index, &graph, 2), "used");
        let index = fixture_module_copy_overshadowed_by_override();
        assert_eq!(
            copy_status(
                &index,
                &LiveGraph {
                    catalog: HashSet::new(),
                    seeds: vec![],
                    seed_ids: vec![],
                    reachable: HashSet::new(),
                    used_ids: HashSet::from([1]),
                    parent: HashMap::new(),
                    edges: HashMap::new(),
                    missing: HashMap::new(),
                    module_entries: HashMap::new(),
                    used: HashSet::new(),
                    used_strrefs: HashSet::new(),
                    tlk: vec![],
                    scanned: 0,
                },
                0
            ),
            "overshadowed"
        );
    }

    #[test]
    fn lists_format_includes_used_heading() {
        let text = format_inventory_text(
            &["modules/a/foo.ncs".into()],
            &[],
            &[],
            &[],
            true,
        );
        assert!(text.contains("Used (1)"), "{text}");
        assert!(text.contains("  modules/a/foo.ncs\n"), "{text}");
        assert!(!text.contains("Unused ("), "{text}");
        assert!(!text.contains("Overshadowed ("), "{text}");
    }

    #[test]
    fn graph_help_has_format_not_dropped_flags() {
        let help = graph_help();
        let lowered = help.to_ascii_lowercase();
        assert!(help.contains("--format"), "{help}");
        assert!(!help.contains("--what"), "{help}");
        assert!(!help.contains("--shadowed"), "{help}");
        assert!(!help.contains("--winners-only"), "{help}");
        assert!(!help.contains("--summary"), "{help}");
        assert!(!lowered.contains("winner"), "{help}");
        assert!(!lowered.contains("loser"), "{help}");
    }
}
