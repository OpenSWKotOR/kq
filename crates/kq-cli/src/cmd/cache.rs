//! `kq cache` — see and control the index cache.

use anyhow::Result;
use serde::Serialize;

use crate::{exit, Ctx};

#[derive(clap::Args)]
pub struct Args {
    #[command(subcommand)]
    action: Action,
}

#[derive(clap::Subcommand)]
enum Action {
    /// Show where the cache lives and what is in it.
    Status,
    /// Delete every cached index.
    Clear,
}

#[derive(Serialize)]
struct Status {
    directory: String,
    entries: usize,
    bytes: u64,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<i32> {
    match args.action {
        Action::Status => {
            let dir = kq_index::cache::cache_dir();
            let mut entries = 0usize;
            let mut bytes = 0u64;
            if let Ok(read) = std::fs::read_dir(&dir) {
                for e in read.flatten() {
                    if e.file_name().to_string_lossy().starts_with("index-") {
                        entries += 1;
                        bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
                    }
                }
            }
            let status =
                Status { directory: dir.display().to_string(), entries, bytes };
            if ctx.out.json {
                ctx.out.json_value(&status)?;
            } else {
                println!("{:<12} {}", ctx.out.dim("directory"), status.directory);
                println!("{:<12} {}", ctx.out.dim("entries"), status.entries);
                println!("{:<12} {}", ctx.out.dim("bytes"), status.bytes);
            }
        }
        Action::Clear => {
            let n = kq_index::cache::clear()?;
            if ctx.out.json {
                ctx.out.json_value(&serde_json::json!({ "removed": n }))?;
            } else {
                println!("removed {n} cached index file(s)");
            }
        }
    }
    Ok(exit::OK)
}
