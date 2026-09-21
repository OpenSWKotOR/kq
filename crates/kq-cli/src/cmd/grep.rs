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

    /// Print N lines of context before and after each match.
    #[arg(short = 'C', long, default_value_t = 0, value_name = "N")]
    context: usize,

    /// Include only the copy the game loads.
    #[arg(long)]
    loaded: bool,

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
    path: String,
    source: &'a str,
    container: &'a str,
    module: Option<&'a str>,
    line: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    matched: Option<bool>,
}

struct ResourceMatches {
    index: u32,
    lines: Vec<RenderedLine>,
}

struct RenderedLine {
    text: String,
    matched: bool,
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
    if args.loaded {
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
            let decoded = render::decode_resource(&index, r, &bytes).ok()?;
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
            let lines = matching_lines(&projected, &re, args.context);
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
            writeln!(w, "{}", index.virt_path(r))?;
            continue;
        }
        for line in &m.lines {
            if ctx.out.json {
                let hit = Hit {
                    resource: r.filename(),
                    path: index.virt_path(r),
                    source: source.kind.as_str(),
                    container: &source.label,
                    module: source.module_root.as_deref(),
                    line: line.text.clone(),
                    matched: (args.context > 0).then_some(line.matched),
                };
                ctx.out.json_line(&mut w, &hit)?;
            } else {
                let separator = if line.matched { ' ' } else { '-' };
                writeln!(
                    w,
                    "{}{separator}{}",
                    ctx.out.accent(&index.virt_path(r)),
                    line.text
                )?;
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

fn matching_lines(projected: &str, re: &regex::Regex, context: usize) -> Vec<RenderedLine> {
    let lines: Vec<&str> = projected.lines().collect();
    let matches: Vec<bool> = lines.iter().map(|line| re.is_match(line)).collect();
    let mut included = vec![false; lines.len()];

    for (index, &matched) in matches.iter().enumerate() {
        if matched {
            let start = index.saturating_sub(context);
            let end = index
                .saturating_add(context)
                .min(lines.len().saturating_sub(1));
            included[start..=end].fill(true);
        }
    }

    lines
        .into_iter()
        .zip(matches)
        .zip(included)
        .filter_map(|((text, matched), included)| {
            included.then(|| RenderedLine {
                text: text.to_string(),
                matched,
            })
        })
        .collect()
}

/// Whether `kq grep` reads this resource by default.
///
/// GFF, 2DA and TLK all decode; plain-text formats need no decoding. Every
/// other type — textures, models, audio, scripts pending a decompiler — is
/// skipped so a plain `kq grep` does not read gigabytes of pixels.
fn is_searchable(t: kq_format::ResType) -> bool {
    t.is_gff()
        || t.is_plain_text()
        || matches!(
            t.extension(),
            Some(
                "2da"
                    | "tlk"
                    | "ssf"
                    | "lip"
                    | "ncs"
                    | "ltr"
                    | "tpc"
                    | "mdl"
                    | "wok"
                    | "dwk"
                    | "pwk"
                    | "wav"
                    | "bmu"
            )
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use serde_json::json;

    #[test]
    fn context_lines_are_a_source_ordered_union() {
        let re = regex::Regex::new("hit").unwrap();
        let lines = matching_lines("zero\nhit one\ntwo\nhit three\nfour", &re, 1);

        assert_eq!(
            lines
                .iter()
                .map(|line| (line.text.as_str(), line.matched))
                .collect::<Vec<_>>(),
            vec![
                ("zero", false),
                ("hit one", true),
                ("two", false),
                ("hit three", true),
                ("four", false),
            ]
        );
    }

    #[test]
    fn zero_context_keeps_only_matches_and_legacy_json_shape() {
        let re = regex::Regex::new("hit").unwrap();
        let lines = matching_lines("before\nhit\nafter", &re, 0);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "hit");
        assert!(lines[0].matched);

        let hit = Hit {
            resource: "test.utc".into(),
            path: "Override/test.utc".into(),
            source: "override",
            container: "Override",
            module: None,
            line: "hit".into(),
            matched: None,
        };
        assert_eq!(
            serde_json::to_value(hit).unwrap(),
            json!({
                "resource": "test.utc",
                "path": "Override/test.utc",
                "source": "override",
                "container": "Override",
                "module": null,
                "line": "hit"
            })
        );
    }

    #[test]
    fn context_json_marks_each_line_deterministically() {
        let hit = Hit {
            resource: "test.utc".into(),
            path: "Override/test.utc".into(),
            source: "override",
            container: "Override",
            module: None,
            line: "before".into(),
            matched: Some(false),
        };
        assert_eq!(serde_json::to_value(hit).unwrap()["matched"], false);
    }

    fn grep_help() -> String {
        use clap::CommandFactory;
        let mut cmd = crate::Cli::command();
        let mut buf = Vec::new();
        cmd.find_subcommand_mut("grep")
            .unwrap()
            .write_long_help(&mut buf)
            .unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn loaded_flag_is_accepted() {
        let cli = crate::Cli::try_parse_from(["kq", "grep", "ActionUseSkill", "--loaded"]).unwrap();
        let crate::Command::Grep(args) = cli.command else {
            panic!("expected grep");
        };
        assert!(args.loaded);
    }

    #[test]
    fn winners_flag_is_rejected() {
        let parsed = crate::Cli::try_parse_from(["kq", "grep", "ActionUseSkill", "--winners"]);
        assert!(
            parsed.is_err(),
            "kq grep --winners must clap-error, not alias"
        );
    }

    #[test]
    fn help_uses_loaded_not_winner() {
        let help = grep_help();
        let lowered = help.to_ascii_lowercase();
        assert!(help.contains("--loaded"), "{help}");
        assert!(!help.contains("--winners"), "{help}");
        assert!(!lowered.contains("winner"), "{help}");
        assert!(!lowered.contains("loser"), "{help}");
        assert!(help.contains("the copy the game loads"), "{help}");
    }
}
