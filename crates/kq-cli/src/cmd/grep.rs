//! `kq grep` — search resource contents as text.
//!
//! Every resource is decoded to its gron projection (see `render`) and the
//! pattern is matched line by line, so a hit's address is the field path,
//! not a meaningless byte offset into a binary blob.

use std::io::{BufWriter, Write};

use anyhow::Result;
use rayon::prelude::*;
use regex::RegexBuilder;
use serde::Serialize;

use crate::filter::Filter;
use crate::render::{self, Format};
use crate::{exit, read, Ctx};

#[derive(clap::Args)]
pub struct Args {
    /// Pattern to search for. Regex, unless --fixed-strings.
    pattern: String,

    /// Only resources whose name matches this glob.
    #[arg(value_name = "NAME_GLOB")]
    name: Option<String>,

    #[command(flatten)]
    filter: Filter,

    /// Case-insensitive match.
    ///
    /// No `-i` short form: that's already `--install`, global on every
    /// command, and a per-command override would only be silently shadowed.
    #[arg(long)]
    ignore_case: bool,

    /// Treat the pattern as a literal string, not a regex.
    #[arg(short = 'F', long)]
    fixed_strings: bool,

    /// List matching resource names only, one per line, no content.
    #[arg(short = 'l', long)]
    files_with_matches: bool,

    /// Include only the copy the game would actually load.
    #[arg(long)]
    winners: bool,

    /// Also search resource types with no known decoder, as raw bytes.
    ///
    /// Off by default: it would otherwise mean reading every texture, model
    /// and sound in the install to search bytes that were never text.
    #[arg(long)]
    include_binary: bool,

    /// Stop after this many matching resources. 0 means no limit.
    #[arg(short = 'n', long, default_value_t = 0, value_name = "N")]
    limit: usize,
}

#[derive(Serialize)]
struct Hit<'a> {
    resource: String,
    source: &'a str,
    container: &'a str,
    module: Option<&'a str>,
    line: String,
}

struct ResourceMatches {
    index: u32,
    lines: Vec<String>,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<i32> {
    let index = ctx.index()?;

    let pattern = if args.fixed_strings {
        regex::escape(&args.pattern)
    } else {
        args.pattern.clone()
    };
    let re = RegexBuilder::new(&pattern)
        .case_insensitive(args.ignore_case)
        .build()
        .map_err(|e| anyhow::anyhow!("bad pattern: {e}"))?;

    let mut selected = args
        .filter
        .select(&index, args.name.as_deref().unwrap_or(""))?;
    if args.winners {
        Filter::dedup_winners(&index, &mut selected);
    }
    if !args.include_binary {
        selected.retain(|&i| is_searchable(index.resources[i as usize].restype));
    }

    let mut results: Vec<ResourceMatches> = selected
        .par_iter()
        .filter_map(|&i| {
            let r = &index.resources[i as usize];
            let bytes = read::read(&index, r).ok()?;
            let decoded = render::decode(&bytes, Some(r.restype), &r.filename()).ok()?;
            // A type with no decoder renders as one placeholder line that can
            // never match a pattern. --include-binary means "search the
            // actual bytes", so force them to text instead of rendering that
            // placeholder.
            let projected = match &decoded {
                render::Decoded::Opaque { .. } if args.include_binary => {
                    let text = render::Decoded::Text(render::raw_as_text(&bytes));
                    render::render(&text, Format::Gron, &r.filename()).ok()?
                }
                _ => render::render(&decoded, Format::Gron, &r.filename()).ok()?,
            };
            let lines: Vec<String> = projected
                .lines()
                .filter(|l| re.is_match(l))
                .map(str::to_string)
                .collect();
            if lines.is_empty() {
                None
            } else {
                Some(ResourceMatches { index: i, lines })
            }
        })
        .collect();

    // `selected` was already in deterministic order; par_iter scrambled it.
    let order: std::collections::HashMap<u32, usize> = selected
        .iter()
        .enumerate()
        .map(|(pos, &i)| (i, pos))
        .collect();
    results.sort_by_key(|m| order[&m.index]);

    let total_resources = results.len();
    let total_lines: usize = results.iter().map(|m| m.lines.len()).sum();
    if args.limit > 0 {
        results.truncate(args.limit);
    }

    let stdout = std::io::stdout();
    let mut w = BufWriter::new(stdout.lock());

    for m in &results {
        let r = &index.resources[m.index as usize];
        let source = index.source(r);
        if args.files_with_matches {
            writeln!(w, "{}", r.filename())?;
            continue;
        }
        for line in &m.lines {
            if ctx.out.json {
                let hit = Hit {
                    resource: r.filename(),
                    source: source.kind.as_str(),
                    container: &source.label,
                    module: source.module_root.as_deref(),
                    line: line.clone(),
                };
                ctx.out.json_line(&mut w, &hit)?;
            } else {
                writeln!(w, "{} {}", ctx.out.accent(&r.filename()), line)?;
            }
        }
    }

    if !ctx.out.json && args.limit > 0 && total_resources > results.len() {
        writeln!(
            w,
            "{}",
            ctx.out.dim(&format!(
                "... {} more matching resource(s) (use -n 0 for all)",
                total_resources - results.len()
            ))
        )?;
    }
    w.flush()?;

    if total_lines == 0 {
        Ok(exit::NO_MATCH)
    } else {
        Ok(exit::OK)
    }
}

/// Whether `kq grep` reads this resource by default.
///
/// GFF, 2DA and TLK all decode; plain-text formats need no decoding. Every
/// other type — textures, models, audio, scripts pending a decompiler — is
/// skipped so a plain `kq grep` does not read gigabytes of pixels.
fn is_searchable(t: kq_format::ResType) -> bool {
    t.is_gff() || t.is_plain_text() || matches!(t.extension(), Some("2da" | "tlk"))
}
