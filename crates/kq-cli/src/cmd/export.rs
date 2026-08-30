//! `kq export` — write every indexed resource as JSON under `<root>_json/`.
//!
//! Install layout is preserved: archives (`.bif`, `.rim`, `.mod`, `.erf`, `.sav`,
//! `.hak`) become folders named like the original file; each resource is
//! `<virt_path>.json` with the same structured envelope as `kq cat --json`.

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use serde::Serialize;

use crate::filter::Filter;
use crate::read;
use crate::render;
use crate::resource_json::{self, ResourceJson};
use crate::{exit, output, Ctx};

#[derive(clap::Args)]
pub struct Args {
    #[command(flatten)]
    filter: Filter,

    /// Output directory. Default: `<install-name>_json` next to the install root.
    #[arg(short = 'o', long, value_name = "DIR")]
    output: Option<PathBuf>,

    /// Overwrite an existing output directory.
    #[arg(long)]
    force: bool,
}

#[derive(Serialize)]
struct Manifest {
    schema: &'static str,
    source: String,
    resources: usize,
    exported: usize,
    failed: usize,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<i32> {
    let index = ctx.index()?;
    let out_dir = args
        .output
        .clone()
        .unwrap_or_else(|| default_output_dir(&index.root));
    let out_dir = out_dir.canonicalize().unwrap_or(out_dir);

    if out_dir.exists() {
        if !args.force {
            bail!(
                "output directory already exists: {} (pass --force to overwrite)",
                out_dir.display()
            );
        }
        fs::remove_dir_all(&out_dir)
            .with_context(|| format!("cannot remove {}", out_dir.display()))?;
    }
    fs::create_dir_all(&out_dir).with_context(|| format!("cannot create {}", out_dir.display()))?;

    let mut ids = args.filter.select(&index, "")?;
    ids.sort_unstable();
    let total = ids.len();
    output::warn(format!("exporting {total} resources to {}…", out_dir.display()));

    let exported = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);
    let errors: Mutex<Vec<String>> = Mutex::new(Vec::new());

    ids.par_iter().for_each(|&i| {
        let r = &index.resources[i as usize];
        match export_one(&index, r, &out_dir) {
            Ok(()) => {
                exported.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => {
                failed.fetch_add(1, Ordering::Relaxed);
                errors
                    .lock()
                    .expect("export errors lock")
                    .push(format!("{}: {e:#}", index.virt_path(r)));
            }
        }
    });

    let exported = exported.load(Ordering::Relaxed);
    let failed = failed.load(Ordering::Relaxed);
    let mut err_lines = errors.into_inner().expect("export errors lock");
    err_lines.sort();

    if !err_lines.is_empty() {
        let err_path = out_dir.join("_export_errors.txt");
        fs::write(&err_path, err_lines.join("\n")).with_context(|| err_path.display().to_string())?;
        output::warn(format!(
            "{failed} resources failed; see {}",
            err_path.display()
        ));
    }

    let manifest = Manifest {
        schema: "kq-export-1",
        source: index.root.display().to_string(),
        resources: total,
        exported,
        failed,
    };
    let manifest_path = out_dir.join("_manifest.json");
    let manifest_file = File::create(&manifest_path)
        .with_context(|| format!("cannot create {}", manifest_path.display()))?;
    serde_json::to_writer_pretty(BufWriter::new(manifest_file), &manifest)?;

    if ctx.out.json {
        ctx.out.json_value(&manifest)?;
    } else if ctx.out.text {
        println!(
            "exported {exported} resources to {} ({failed} failed)",
            out_dir.display()
        );
    }

    Ok(if exported == 0 {
        exit::NO_MATCH
    } else {
        exit::OK
    })
}

fn export_one(index: &kq_index::Index, r: &kq_index::Resource, out_dir: &Path) -> Result<()> {
    let rel = resource_json::export_relpath(index, r);
    let dest = out_dir.join(&rel);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).with_context(|| format!("cannot create {}", parent.display()))?;
    }

    let bytes = read::read(index, r)?;
    let decoded = render::decode_resource(index, r, &bytes)?;
    let doc: ResourceJson<'_> = resource_json::build_resource_json(index, r, &decoded);

    let file = File::create(&dest).with_context(|| format!("cannot create {}", dest.display()))?;
    serde_json::to_writer_pretty(BufWriter::new(file), &doc)?;
    Ok(())
}

/// `<parent>/<basename>_json`, e.g. `/game/swkotor` → `/game/swkotor_json`.
pub fn default_output_dir(root: &Path) -> PathBuf {
    let name = root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "install".into());
    root.parent()
        .unwrap_or(root)
        .join(format!("{name}_json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_output_dir_sibling_json_folder() {
        let root = PathBuf::from("/game/swkotor");
        assert_eq!(
            default_output_dir(&root),
            PathBuf::from("/game/swkotor_json")
        );
    }
}
