//! `kq which` — which copy of a resource the game loads, and what it overshadows.
//!
//! KotOR resolves a ResRef by walking locations in a fixed order, so the same
//! name can exist in Override, in a module, and in a BIF, with only one of
//! them ever loading. Getting that wrong is the most common way a mod appears
//! not to work, so the whole chain is shown, not just the copy the game loads.

use anyhow::Result;
use serde::Serialize;

use crate::{exit, Ctx};

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum OutputFormat {
    Json,
    Text,
}

#[derive(clap::Args)]
pub struct Args {
    /// ResRef to resolve, with or without an extension.
    #[arg(value_name = "RESREF")]
    resref: String,

    /// Restrict to one resource type when the name is ambiguous.
    #[arg(short = 't', long = "type", value_name = "EXT")]
    restype: Option<String>,

    /// Override the global output mode for this report.
    #[arg(long, value_enum, value_name = "FORMAT")]
    format: Option<OutputFormat>,
}

#[derive(Serialize)]
struct Hit {
    name: String,
    path: String,
    #[serde(rename = "type")]
    restype: String,
    source: &'static str,
    container: String,
    module: Option<String>,
    file: String,
    offset: u64,
    size: u64,
    /// True for the copy the engine loads.
    active: bool,
}

#[derive(Serialize)]
struct Report {
    resref: String,
    matches: usize,
    hits: Vec<Hit>,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<i32> {
    let index = ctx.index()?;
    let json_output = format_is_json(args.format, ctx.out.json);

    // `kq which nwscript.nss` should work as well as `kq which nwscript`.
    let (name, ext_from_name) = match args.resref.rsplit_once('.') {
        Some((base, ext)) if kq_format::ResType::from_extension(ext).is_some() => {
            (base.to_string(), Some(ext.to_string()))
        }
        _ => (args.resref.clone(), None),
    };
    let ext = args.restype.or(ext_from_name);
    let want = match ext.as_deref() {
        Some(e) => match kq_format::ResType::from_extension(e) {
            Some(t) => Some(t),
            None => anyhow::bail!("unknown resource type: {e}"),
        },
        None => None,
    };

    let ids = index.lookup(&name);
    let mut hits = Vec::new();
    let mut seen_active: std::collections::HashSet<kq_format::ResType> = Default::default();

    for &i in ids {
        let r = &index.resources[i as usize];
        if want.is_some_and(|t| r.restype != t) {
            continue;
        }
        let source = index.source(r);
        // `ids` is already precedence-ordered, so the first of each type wins.
        let active = seen_active.insert(r.restype);
        hits.push(Hit {
            name: r.filename(),
            path: index.virt_path(r),
            restype: r.restype.to_string(),
            source: source.kind.as_str(),
            container: source.label.clone(),
            module: source.module_root.clone(),
            file: index.rel_file(r),
            offset: r.offset,
            size: r.size,
            active,
        });
    }

    if json_output {
        ctx.out.json_value(&Report {
            resref: name,
            matches: hits.len(),
            hits,
        })?;
        return Ok(if ids.is_empty() {
            exit::NO_MATCH
        } else {
            exit::OK
        });
    }

    if hits.is_empty() {
        eprintln!("kq: no resource named {name}");
        return Ok(exit::NO_MATCH);
    }

    let o = &ctx.out;
    for h in &hits {
        println!("{}", text_hit_line(o, h));
    }
    if hits.len() > 1 {
        println!("\n{}", o.dim(text_footer()));
    }
    Ok(exit::OK)
}

fn text_hit_line(o: &crate::Out, h: &Hit) -> String {
    let marker = if h.active { o.bold("*") } else { o.dim(" ") };
    let mut line = format!(
        "{} {}  {}",
        marker,
        o.accent(&h.path),
        o.dim(&format!("{} bytes", h.size))
    );
    if !h.active {
        line.push_str(&format!("  {}", o.dim("(overshadowed)")));
    }
    line
}

fn text_footer() -> &'static str {
    "* is the copy the game loads; the rest are overshadowed."
}

fn format_is_json(format: Option<OutputFormat>, default_json: bool) -> bool {
    match format {
        Some(OutputFormat::Json) => true,
        Some(OutputFormat::Text) => false,
        None => default_json,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn explicit_format_overrides_default_without_changing_legacy_mode() {
        assert!(format_is_json(Some(OutputFormat::Json), false));
        assert!(!format_is_json(Some(OutputFormat::Text), true));
        assert!(format_is_json(None, true));
        assert!(!format_is_json(None, false));
    }

    #[test]
    fn parser_preserves_type_and_resref_with_explicit_json_format() {
        let cli = crate::Cli::try_parse_from([
            "kq",
            "which",
            "--install",
            "/game",
            "--type",
            "ncs",
            "k_pend_chest02",
            "--format",
            "json",
        ])
        .unwrap();
        let crate::Command::Which(args) = cli.command else {
            panic!("expected which command");
        };

        assert_eq!(args.restype.as_deref(), Some("ncs"));
        assert_eq!(args.resref, "k_pend_chest02");
        assert_eq!(args.format, Some(OutputFormat::Json));
    }

    fn plain_out() -> crate::Out {
        crate::Out {
            json: false,
            text: true,
            color: false,
        }
    }

    fn hit(path: &str, size: u64, active: bool) -> Hit {
        Hit {
            name: "foo.ncs".into(),
            path: path.into(),
            restype: "ncs".into(),
            source: "override",
            container: "Override".into(),
            module: None,
            file: path.into(),
            offset: 0,
            size,
            active,
        }
    }

    fn which_help() -> String {
        use clap::CommandFactory;
        let mut cmd = crate::Cli::command();
        let mut buf = Vec::new();
        cmd.find_subcommand_mut("which")
            .unwrap()
            .write_long_help(&mut buf)
            .unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn loaded_copy_is_star_marked_without_overshadowed_suffix() {
        let line = text_hit_line(&plain_out(), &hit("Override/foo.ncs", 12, true));
        assert!(line.contains('*'), "{line}");
        assert!(line.contains("Override/foo.ncs"), "{line}");
        assert!(!line.contains("(overshadowed)"), "{line}");
    }

    #[test]
    fn other_copies_carry_overshadowed_suffix_on_the_line() {
        let line = text_hit_line(
            &plain_out(),
            &hit("modules/end_m01aa.mod/foo.ncs", 12, false),
        );
        assert!(line.contains("(overshadowed)"), "{line}");
        assert!(!line.trim_start().starts_with('*'), "{line}");
    }

    #[test]
    fn footer_explains_star_is_the_copy_the_game_loads() {
        let footer = text_footer();
        assert!(footer.contains("the copy the game loads"), "{footer}");
        assert!(!footer.contains("the rest are shadowed."), "{footer}");
    }

    #[test]
    fn help_names_the_copy_the_game_loads() {
        let help = which_help();
        assert!(help.contains("the copy the game loads"), "{help}");
    }
}
