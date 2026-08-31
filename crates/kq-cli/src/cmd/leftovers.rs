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

    /// Exclude textures, models and audio from leftover *resources*.
    #[arg(long)]
    no_assets: bool,

    /// Only the winning copy of each name (shadowed copies omitted).
    #[arg(long)]
    winners_only: bool,

    /// Print counts instead of every leftover string / name.
    #[arg(long)]
    summary: bool,

    /// Stop after this many leftover rows. 0 means no limit.
    #[arg(short = 'n', long, default_value_t = 0, value_name = "N")]
    limit: usize,
}

#[derive(Serialize)]
struct StringRow {
    strref: i64,
    text: String,
    sound: String,
    status: &'static str,
}

#[derive(Serialize)]
struct Report<'a> {
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
    leftover_string_rows: Vec<StringRow>,
    leftover_resource_rows: Vec<unused::Row<'a>>,
    catalog_names: Vec<String>,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<i32> {
    let index = ctx.index()?;
    let graph = live::build(&index)?;

    let in_scope = unused::candidate_ids(&index, &args.filter, args.no_assets, args.winners_only)?;
    let winners = unused::winner_set(&index, &in_scope);
    let resource_ids = unused::leftover_ids(
        &index,
        &graph,
        &args.filter,
        args.no_assets,
        args.winners_only,
    )?;
    let leftover_resources = resource_ids.len();
    let by_type = unused::count_by_type(&index, &resource_ids);

    let leftover_tlk = graph.leftover_strings();
    let leftover_empty = leftover_tlk.iter().filter(|r| r.text.is_empty()).count();

    let mut catalog_names: Vec<String> = index.resources.iter().map(|r| r.filename()).collect();
    catalog_names.sort();
    catalog_names.dedup();

    let leftover_string_rows: Vec<StringRow> = leftover_tlk
        .iter()
        .map(|row| StringRow {
            strref: row.strref,
            text: row.text.clone(),
            sound: row.sound.clone(),
            status: "leftover",
        })
        .collect();
    let leftover_resource_rows: Vec<_> = resource_ids
        .iter()
        .map(|&id| unused::resource_row(&index, id, &graph, &winners, "leftover"))
        .collect();

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
        leftover_string_rows,
        leftover_resource_rows,
        catalog_names,
    };

    let show_strings = matches!(args.what, What::Both | What::Strings);
    let show_resources = matches!(args.what, What::Both | What::Resources);
    let interesting =
        (show_strings && !leftover_tlk.is_empty()) || (show_resources && leftover_resources > 0);

    let stdout = std::io::stdout();
    let mut w = BufWriter::new(stdout.lock());

    if ctx.out.json {
        ctx.out.json_value(&report)?;
        w.flush()?;
        return Ok(if interesting {
            exit::OK
        } else {
            exit::NO_MATCH
        });
    }

    if args.summary {
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
        w.flush()?;
        return Ok(if interesting {
            exit::OK
        } else {
            exit::NO_MATCH
        });
    }

    if show_resources && !show_strings {
        unused::write_resource_rows(
            ctx,
            &mut w,
            &index,
            &graph,
            &winners,
            &resource_ids,
            args.limit,
            false,
        )?;
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
                    text: row.text.clone(),
                    sound: row.sound.clone(),
                    status: "leftover",
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
