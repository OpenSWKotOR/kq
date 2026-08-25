//! `kq leftovers` — the inverse of the live graph.
//!
//! Walk every file, catalog every ResRef (and every `dialog.tlk` row), build
//! the same mention graph `kq unused` uses, then emit what that graph never
//! reaches. Talk-table leftovers are unused strings that might be worth
//! restoring.

use std::io::{BufWriter, Write};

use anyhow::Result;
use serde::Serialize;

use crate::cmd::unused;
use crate::filter::Filter;
use crate::live;
use crate::{exit, Ctx};

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
enum What {
    /// Leftover strings, plus a leftover-resource summary.
    #[default]
    Both,
    /// Only unused `dialog.tlk` rows.
    Strings,
    /// Only unused resources (same list as `kq unused`).
    Resources,
}

#[derive(clap::Args)]
pub struct Args {
    #[command(flatten)]
    filter: Filter,

    /// What to print: leftover strings, leftover resources, or both.
    #[arg(long, value_enum, default_value_t = What::Both)]
    what: What,

    /// Include the complete ResRef catalog (JSON array of `resref.ext`).
    #[arg(long)]
    catalog: bool,

    /// Include textures, models and audio in leftover *resources*.
    #[arg(long)]
    assets: bool,

    /// Also consider shadowed copies, not only the winner of each name.
    #[arg(long)]
    all_copies: bool,

    /// Print counts instead of every leftover string / name.
    #[arg(long)]
    summary: bool,

    /// Stop after this many leftover rows. 0 means no limit.
    #[arg(short = 'n', long, default_value_t = 0, value_name = "N")]
    limit: usize,
}

#[derive(Serialize)]
struct StringRow<'a> {
    strref: i64,
    text: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    sound: &'a str,
}

#[derive(Serialize)]
struct Report {
    scanned: usize,
    catalog: usize,
    seeds: Vec<String>,
    reachable_resrefs: usize,
    used_resrefs: usize,
    leftover_resources: usize,
    tlk_entries: usize,
    used_strings: usize,
    leftover_strings: usize,
    leftover_empty_strings: usize,
    by_type: Vec<unused::TypeCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    catalog_names: Option<Vec<String>>,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<i32> {
    let index = ctx.index()?;
    let graph = live::build(&index)?;

    let resource_ids =
        unused::leftover_ids(&index, &graph, &args.filter, args.assets, args.all_copies)?;
    let leftover_resources = resource_ids.len();
    let by_type = unused::count_by_type(&index, &resource_ids);

    let leftover_tlk = graph.leftover_strings();
    let leftover_empty = leftover_tlk.iter().filter(|r| r.text.is_empty()).count();

    let catalog_names = if args.catalog {
        let mut names: Vec<String> = index.resources.iter().map(|r| r.filename()).collect();
        names.sort();
        names.dedup();
        Some(names)
    } else {
        None
    };

    let report = Report {
        scanned: graph.scanned,
        catalog: graph.catalog.len(),
        seeds: graph.seeds.clone(),
        reachable_resrefs: graph.reachable.len(),
        used_resrefs: graph.used.len(),
        leftover_resources,
        tlk_entries: graph.tlk.len(),
        used_strings: graph.used_strrefs.len(),
        leftover_strings: leftover_tlk.len(),
        leftover_empty_strings: leftover_empty,
        by_type,
        catalog_names,
    };

    let show_strings = matches!(args.what, What::Both | What::Strings);
    let show_resources = matches!(args.what, What::Both | What::Resources);
    let interesting =
        (show_strings && !leftover_tlk.is_empty()) || (show_resources && leftover_resources > 0);

    let stdout = std::io::stdout();
    let mut w = BufWriter::new(stdout.lock());

    if args.summary {
        if ctx.out.json {
            ctx.out.json_value(&report)?;
        } else {
            writeln!(
                w,
                "catalog {}  scanned {}  seeds {}  used-resrefs {}  leftover-resources {}",
                report.catalog,
                report.scanned,
                report.seeds.len(),
                report.used_resrefs,
                report.leftover_resources
            )?;
            writeln!(
                w,
                "tlk {}  used-strings {}  leftover-strings {}  leftover-empty {}",
                report.tlk_entries,
                report.used_strings,
                report.leftover_strings,
                report.leftover_empty_strings
            )?;
            if show_resources {
                writeln!(w)?;
                writeln!(w, "{:<8} {:>8} {:>12}", "type", "unused", "bytes")?;
                for row in &report.by_type {
                    writeln!(w, "{:<8} {:>8} {:>12}", row.restype, row.count, row.bytes)?;
                }
            }
            writeln!(
                w,
                "\n{}",
                ctx.out.dim(
                    "Live-graph leftovers, not a runtime trace. Isolated A↔B pairs stay unused. rims/ is not a seed."
                )
            )?;
        }
        w.flush()?;
        return Ok(if interesting {
            exit::OK
        } else {
            exit::NO_MATCH
        });
    }

    if show_resources && !show_strings {
        unused::write_resource_rows(ctx, &mut w, &index, &resource_ids, args.limit, false)?;
        w.flush()?;
        return Ok(if leftover_resources == 0 {
            exit::NO_MATCH
        } else {
            exit::OK
        });
    }

    let mut printed = leftover_tlk;
    if args.limit > 0 {
        printed.truncate(args.limit);
    }
    for row in &printed {
        if ctx.out.json {
            ctx.out.json_line(
                &mut w,
                &StringRow {
                    strref: row.strref,
                    text: &row.text,
                    sound: &row.sound,
                },
            )?;
        } else {
            let text = one_line(&row.text, 80);
            if row.sound.is_empty() {
                writeln!(w, "{:>7}  {}", row.strref, text)?;
            } else {
                writeln!(
                    w,
                    "{:>7}  [{}] {}",
                    row.strref,
                    ctx.out.dim(&row.sound),
                    text
                )?;
            }
        }
    }
    if !ctx.out.json && args.limit > 0 && report.leftover_strings > args.limit {
        writeln!(
            w,
            "{}",
            ctx.out.dim(&format!(
                "... {} more leftover strings (use -n 0 or --summary)",
                report.leftover_strings - args.limit
            ))
        )?;
    }
    if !ctx.out.json && show_resources {
        writeln!(
            w,
            "\n{}",
            ctx.out.dim(&format!(
                "{} leftover strings, {} leftover resources. kq unused lists the resources.",
                report.leftover_strings, leftover_resources
            ))
        )?;
    } else if !ctx.out.json && args.limit == 0 {
        writeln!(
            w,
            "\n{}",
            ctx.out.dim(&format!(
                "{} leftover of {} talk-table rows. Live-graph scan, not a runtime trace.",
                report.leftover_strings,
                graph.tlk.len()
            ))
        )?;
    }

    w.flush()?;
    Ok(if interesting {
        exit::OK
    } else {
        exit::NO_MATCH
    })
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
