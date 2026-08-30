//! `kq delta` — structural compare of two decoded resources.

use std::path::PathBuf;

use anyhow::{Result, bail};
use serde::Serialize;

use kq_format::delta::{self, Op};

use crate::side::{self, Loaded};
use crate::{exit, Ctx};

#[derive(clap::Args)]
pub struct Args {
    /// Left side: ResRef, resource file, or JSON (`-` = stdin).
    #[arg(value_name = "LEFT")]
    left: String,

    /// Right side. Omit with `--shadow`.
    #[arg(value_name = "RIGHT")]
    right: Option<String>,

    /// Left copy from this container label (`kq which` / `cat --from`).
    #[arg(long, value_name = "NAME")]
    from: Option<String>,

    /// Right copy from this container label.
    #[arg(long = "against", value_name = "NAME")]
    against: Option<String>,

    /// Compare the winner to the next shadowed copy. `RIGHT` is omitted.
    #[arg(long)]
    shadow: bool,

    /// Look up `RIGHT` in this other install (or compare the same ResRef across installs).
    #[arg(long, value_name = "PATH")]
    other: Option<PathBuf>,

    /// Include the `cat` envelope (path, offset, size), not just `content`.
    #[arg(long)]
    envelope: bool,

    /// Write the delta document to this file instead of stdout.
    #[arg(short = 'o', long, value_name = "FILE")]
    output: Option<PathBuf>,
}

#[derive(Serialize)]
struct Report<'a> {
    schema: &'static str,
    left: &'a str,
    right: &'a str,
    equal: bool,
    changes: usize,
    ops: &'a [Op],
}

pub fn run(ctx: &Ctx, args: Args) -> Result<i32> {
    let content_only = !args.envelope;
    let (left, right) = load_pair(ctx, &args, content_only)?;
    let ops = delta::diff(&left.value, &right.value);
    let report = Report {
        schema: "kq-delta-1",
        left: &left.label,
        right: &right.label,
        equal: ops.is_empty(),
        changes: ops.len(),
        ops: &ops,
    };

    if let Some(path) = &args.output {
        let file = std::fs::File::create(path)?;
        serde_json::to_writer_pretty(std::io::BufWriter::new(file), &report)?;
    } else if ctx.out.text {
        print_text(ctx, &left, &right, &ops);
    } else {
        ctx.out.json_value(&report)?;
    }

    Ok(if ops.is_empty() {
        exit::OK
    } else {
        exit::DIFFER
    })
}

fn load_pair(ctx: &Ctx, args: &Args, content_only: bool) -> Result<(Loaded, Loaded)> {
    if args.shadow {
        if args.right.is_some() {
            bail!("--shadow takes only LEFT (the ResRef)");
        }
        return side::load_shadow_pair(ctx, &args.left, content_only);
    }
    let left = side::load(ctx, &args.left, args.from.as_deref(), content_only, None)?;
    let right_spec = args.right.as_deref().unwrap_or(&args.left);
    if args.right.is_none() && args.other.is_none() && args.against.is_none() {
        bail!("RIGHT is required unless --shadow, --against, or --other is set");
    }
    let other = args.other.as_deref().filter(|_| !looks_like_file(right_spec));
    let right = side::load(
        ctx,
        right_spec,
        args.against.as_deref(),
        content_only,
        other,
    )?;
    Ok((left, right))
}

fn looks_like_file(spec: &str) -> bool {
    spec.contains('/') || spec.contains('\\') || std::path::Path::new(spec).exists()
}

fn print_text(ctx: &Ctx, left: &Loaded, right: &Loaded, ops: &[Op]) {
    println!("--- {}", left.label);
    println!("+++ {}", right.label);
    if ops.is_empty() {
        println!("(no changes)");
        return;
    }
    for op in ops {
        let path = delta::pointer_to_gron(op.path());
        let path = if path.is_empty() { "<root>" } else { path.as_str() };
        match op {
            Op::Remove { old, .. } => {
                println!(
                    "{}",
                    ctx.out.minus(&format!("- {path} = {}", fmt_old(old)))
                );
            }
            Op::Add { value, .. } => {
                println!(
                    "{}",
                    ctx.out.plus(&format!("+ {path} = {}", compact(value)))
                );
            }
            Op::Replace { old, value, .. } => {
                println!(
                    "{}",
                    ctx.out.minus(&format!("- {path} = {}", fmt_old(old)))
                );
                println!(
                    "{}",
                    ctx.out.plus(&format!("+ {path} = {}", compact(value)))
                );
            }
        }
    }
}

fn fmt_old(old: &Option<serde_json::Value>) -> String {
    match old {
        Some(v) => compact(v),
        None => "…".into(),
    }
}

fn compact(v: &serde_json::Value) -> String {
    if v.is_object() || v.is_array() {
        let s = serde_json::to_string(v).unwrap_or_else(|_| v.to_string());
        if s.len() > 80 {
            return format!("{}… ({} bytes)", &s[..77], s.len());
        }
        return s;
    }
    match v {
        serde_json::Value::String(s) => serde_json::Value::String(s.clone()).to_string(),
        other => other.to_string(),
    }
}
