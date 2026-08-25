//! `kq graph` — live mention hierarchy and leftovers in one report.
//!
//! Shows what the engine can reach from hardcoded seeds as a tree, then
//! every resource (and optionally talk-table row) the walk never entered.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufWriter, Write};

use anyhow::Result;
use serde::Serialize;

use crate::cmd::unused;
use crate::filter::Filter;
use crate::live::{self, LiveGraph};
use crate::{exit, Ctx};

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
enum What {
    /// Reachable tree plus leftover resources (and a leftover-string count).
    #[default]
    Both,
    /// Only the reachable hierarchy.
    Used,
    /// Only leftover resources and strings.
    Leftovers,
}

#[derive(clap::Args)]
pub struct Args {
    #[command(flatten)]
    filter: Filter,

    /// What to print.
    #[arg(long, value_enum, default_value_t = What::Both)]
    what: What,

    /// Include textures, models and audio.
    #[arg(long)]
    assets: bool,

    /// Also list shadowed copies, not only winners.
    #[arg(long)]
    all_copies: bool,

    /// Counts only — no tree or path lists.
    #[arg(long)]
    summary: bool,

    /// Tree depth in text mode. 0 means unlimited.
    #[arg(long, default_value_t = 3, value_name = "N")]
    depth: usize,

    /// Print install-relative paths only, one per line (respects `--what`).
    #[arg(short = 'q', long)]
    quiet: bool,

    /// Stop after this many leftover rows. 0 means no limit.
    #[arg(short = 'n', long, default_value_t = 0, value_name = "N")]
    limit: usize,
}

#[derive(Serialize)]
struct Node<'a> {
    path: String,
    resref: &'a str,
    #[serde(rename = "type")]
    restype: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    module: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    mentions: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<Node<'a>>,
}

#[derive(Serialize)]
struct StringLeftover {
    strref: i64,
    text: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    sound: String,
}

#[derive(Serialize)]
struct Report<'a> {
    scanned: usize,
    catalog: usize,
    seeds: &'a [String],
    used_resources: usize,
    leftover_resources: usize,
    tlk_entries: usize,
    leftover_strings: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    tree: Option<Vec<Node<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    used_by_module: Option<BTreeMap<String, Vec<String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    leftovers: Option<Vec<unused::Row<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    leftover_strings_detail: Option<Vec<StringLeftover>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    by_type: Option<Vec<unused::TypeCount>>,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<i32> {
    let index = ctx.index()?;
    let graph = live::build(&index)?;

    let leftover_ids =
        unused::leftover_ids(&index, &graph, &args.filter, args.assets, args.all_copies)?;
    let leftover_strings: Vec<_> = graph.leftover_strings().into_iter().cloned().collect();

    let show_used = matches!(args.what, What::Both | What::Used);
    let show_leftovers = matches!(args.what, What::Both | What::Leftovers);

    if args.quiet {
        return write_quiet(ctx, &index, &graph, &leftover_ids, &leftover_strings, &args);
    }

    if args.summary {
        return write_summary(
            ctx,
            &graph,
            leftover_ids.len(),
            leftover_strings.len(),
            &args,
        );
    }

    let tree = if show_used && !ctx.out.json {
        Some(build_forest(&index, &graph, args.depth))
    } else {
        None
    };
    let json_tree = if show_used && ctx.out.json {
        Some(build_json_forest(&index, &graph, 0))
    } else {
        None
    };
    let used_by_module = if show_used && ctx.out.json {
        Some(group_used_by_module(&index, &graph))
    } else {
        None
    };

    if ctx.out.json {
        let mut leftover_rows = Vec::new();
        if show_leftovers {
            let ids = apply_limit(&leftover_ids, args.limit);
            for &i in ids {
                leftover_rows.push(unused_row(&index, i));
            }
        }
        let mut string_rows = Vec::new();
        if show_leftovers {
            let rows = apply_limit_slice(&leftover_strings, args.limit);
            for row in rows {
                string_rows.push(StringLeftover {
                    strref: row.strref,
                    text: row.text.clone(),
                    sound: row.sound.clone(),
                });
            }
        }
        let report = Report {
            scanned: graph.scanned,
            catalog: graph.catalog.len(),
            seeds: &graph.seeds,
            used_resources: graph.used_ids.len(),
            leftover_resources: leftover_ids.len(),
            tlk_entries: graph.tlk.len(),
            leftover_strings: leftover_strings.len(),
            tree: json_tree,
            used_by_module,
            leftovers: if show_leftovers {
                Some(leftover_rows)
            } else {
                None
            },
            leftover_strings_detail: if show_leftovers {
                Some(string_rows)
            } else {
                None
            },
            by_type: if show_leftovers {
                Some(unused::count_by_type(&index, &leftover_ids))
            } else {
                None
            },
        };
        ctx.out.json_value(&report)?;
        return Ok(if graph.used_ids.is_empty() && leftover_ids.is_empty() {
            exit::NO_MATCH
        } else {
            exit::OK
        });
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
                graph.used_ids.len(),
                graph.seeds.len()
            ))
        )?;
        writeln!(w, "{}", o.dim("seeds:"))?;
        for seed in &graph.seeds {
            writeln!(w, "  {}", seed)?;
        }
        writeln!(w)?;
        writeln!(w, "{}", o.dim("tree:"))?;
        if let Some(forest) = tree {
            for (i, root) in forest.iter().enumerate() {
                if i > 0 {
                    writeln!(w)?;
                }
                print_tree(&mut w, root, "", true, o)?;
            }
        }
        if args.depth > 0 {
            writeln!(
                w,
                "\n{}",
                o.dim(&format!(
                    "Tree truncated at depth {}. Use --depth 0 for the full hierarchy.",
                    args.depth
                ))
            )?;
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
        if !leftover_ids.is_empty() {
            writeln!(w, "{}", o.dim("resources:"))?;
            let ids = apply_limit(&leftover_ids, args.limit);
            for &i in ids {
                writeln!(w, "  {}", index.virt_path(&index.resources[i as usize]))?;
            }
            if args.limit > 0 && leftover_ids.len() > args.limit {
                writeln!(
                    w,
                    "  {}",
                    o.dim(&format!(
                        "... {} more (use -n 0)",
                        leftover_ids.len() - args.limit
                    ))
                )?;
            }
        }
        if !leftover_strings.is_empty() {
            writeln!(w, "{}", o.dim("strings:"))?;
            let rows = apply_limit_slice(&leftover_strings, args.limit);
            for row in rows {
                let text = one_line(&row.text, 72);
                if row.sound.is_empty() {
                    writeln!(w, "  {:>7}  {}", row.strref, text)?;
                } else {
                    writeln!(w, "  {:>7}  [{}] {}", row.strref, row.sound, text)?;
                }
            }
            if args.limit > 0 && leftover_strings.len() > args.limit {
                writeln!(
                    w,
                    "  {}",
                    o.dim(&format!(
                        "... {} more strings (use -n 0)",
                        leftover_strings.len() - args.limit
                    ))
                )?;
            }
        }
    }

    writeln!(
        w,
        "\n{}",
        o.dim(
            "Live-graph scan from engine seeds — not a runtime trace. \
             Isolated A↔B pairs stay in LEFTOVERS."
        )
    )?;
    w.flush()?;
    Ok(exit::OK)
}

struct TreeNode {
    path: String,
    mentions: Vec<String>,
    children: Vec<TreeNode>,
}

fn build_forest(index: &kq_index::Index, graph: &LiveGraph, max_depth: usize) -> Vec<TreeNode> {
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
        kids.sort_by_key(|&id| id);
    }
    map
}

fn build_subtree(
    index: &kq_index::Index,
    graph: &LiveGraph,
    children_map: &HashMap<u32, Vec<u32>>,
    id: u32,
    max_depth: usize,
    depth: usize,
) -> TreeNode {
    let r = &index.resources[id as usize];
    let mut mentions: Vec<String> = graph
        .edges
        .get(&id)
        .map(|set| {
            let mut v: Vec<_> = set.iter().cloned().collect();
            v.sort();
            v
        })
        .unwrap_or_default();
    mentions.truncate(8);
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
        path: index.virt_path(r),
        mentions,
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
    let mentions: Vec<String> = graph
        .edges
        .get(&id)
        .map(|set| {
            let mut v: Vec<_> = set.iter().cloned().collect();
            v.sort();
            v
        })
        .unwrap_or_default();
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
        path: index.virt_path(r),
        resref: &r.resref,
        restype: r.restype.to_string(),
        module: source.module_root.as_deref(),
        mentions,
        children,
    }
}

fn group_used_by_module(
    index: &kq_index::Index,
    graph: &LiveGraph,
) -> BTreeMap<String, Vec<String>> {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for &id in &graph.used_ids {
        let r = &index.resources[id as usize];
        let key = index
            .source(r)
            .module_root
            .clone()
            .unwrap_or_else(|| index.source(r).kind.as_str().to_string());
        map.entry(key)
            .or_default()
            .push(index.virt_path(r));
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
        let mention_prefix = format!("{prefix}   ");
        let joined = node.mentions.join(", ");
        let suffix = if graph_mentions_truncated(node) {
            " …"
        } else {
            ""
        };
        writeln!(w, "{}", o.dim(&format!("{mention_prefix}→ {joined}{suffix}")))?;
    }
    let child_prefix = format!("{prefix}{}   ", if is_last { " " } else { "│" });
    for (i, child) in node.children.iter().enumerate() {
        print_tree(
            w,
            child,
            &child_prefix,
            i + 1 == node.children.len(),
            o,
        )?;
    }
    Ok(())
}

fn graph_mentions_truncated(node: &TreeNode) -> bool {
    node.mentions.len() >= 8
}

fn write_quiet(
    ctx: &Ctx,
    index: &kq_index::Index,
    graph: &LiveGraph,
    leftover_ids: &[u32],
    leftover_strings: &[live::TlkRow],
    args: &Args,
) -> Result<i32> {
    let stdout = std::io::stdout();
    let mut w = BufWriter::new(stdout.lock());
    let show_used = matches!(args.what, What::Both | What::Used);
    let show_leftovers = matches!(args.what, What::Both | What::Leftovers);

    if show_used {
        let mut used: Vec<_> = graph.used_ids.iter().copied().collect();
        used.sort_by_key(|&id| index.virt_path(&index.resources[id as usize]));
        for id in used {
            writeln!(w, "{}", index.virt_path(&index.resources[id as usize]))?;
        }
    }
    if show_leftovers {
        let ids = apply_limit(leftover_ids, args.limit);
        for &i in ids {
            writeln!(w, "{}", index.virt_path(&index.resources[i as usize]))?;
        }
        if !ctx.out.json {
            let rows = apply_limit_slice(leftover_strings, args.limit);
            for row in rows {
                writeln!(w, "dialog.tlk#{}", row.strref)?;
            }
        }
    }
    w.flush()?;
    Ok(exit::OK)
}

fn write_summary(
    ctx: &Ctx,
    graph: &LiveGraph,
    leftover_resources: usize,
    leftover_strings: usize,
    args: &Args,
) -> Result<i32> {
    if ctx.out.json {
        let report = serde_json::json!({
            "scanned": graph.scanned,
            "catalog": graph.catalog.len(),
            "seeds": graph.seeds,
            "used_resources": graph.used_ids.len(),
            "leftover_resources": leftover_resources,
            "tlk_entries": graph.tlk.len(),
            "leftover_strings": leftover_strings,
        });
        ctx.out.json_value(&report)?;
    } else {
        let stdout = std::io::stdout();
        let mut w = BufWriter::new(stdout.lock());
        writeln!(
            w,
            "catalog {}  scanned {}  seeds {}  used {}  leftovers {} resources / {} strings",
            graph.catalog.len(),
            graph.scanned,
            graph.seeds.len(),
            graph.used_ids.len(),
            leftover_resources,
            leftover_strings
        )?;
        if matches!(args.what, What::Both | What::Leftovers) {
            writeln!(w, "  use `kq unused -q` or `kq graph --what leftovers -q` for leftover paths")?;
        }
        w.flush()?;
    }
    Ok(exit::OK)
}

fn unused_row<'a>(index: &'a kq_index::Index, i: u32) -> unused::Row<'a> {
    let r = &index.resources[i as usize];
    let source = index.source(r);
    unused::Row {
        name: r.filename(),
        path: index.virt_path(r),
        resref: &r.resref,
        restype: r.restype.to_string(),
        size: r.size,
        source: source.kind.as_str(),
        container: &source.label,
        module: source.module_root.as_deref(),
    }
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
