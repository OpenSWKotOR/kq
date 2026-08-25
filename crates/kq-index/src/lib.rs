//! Discovering a KotOR installation and indexing everything in it.

pub mod cache;
pub mod discover;
pub mod game;
pub mod index;
pub mod source;

pub use discover::Install;
pub use game::Game;
pub use index::{Index, Resource, RootKind};
pub use source::{Source, SourceKind};

use std::path::Path;

use anyhow::Result;

/// How an index was obtained. Reported so `--json` consumers can tell a warm
/// run from a cold one without timing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    Cached,
    Built,
}

/// Open an installation and get its index.
///
/// `read_cache` and `write_cache` are independent: `--refresh` skips the read
/// so it recomputes, but still writes, so the rebuilt index actually replaces
/// the stale one instead of leaving it for the next plain run to load.
pub fn open(
    root: &Path,
    read_cache: bool,
    write_cache: bool,
) -> Result<(Install, Index, Freshness)> {
    let install = discover::open(root)?;
    if read_cache {
        if let Some(index) = cache::load(&install) {
            return Ok((install, index, Freshness::Cached));
        }
    }
    let index = index::build(&install)?;
    if write_cache {
        // A cache that cannot be written is a slow run, not a failed one.
        let _ = cache::store(&index);
    }
    Ok((install, index, Freshness::Built))
}

/// Open a standalone target: a single capsule, a directory of loose files, or
/// one resource file, with no installation around it.
///
/// Never cached — each of these is already a single fast parse (one archive
/// header, one directory walk, one `stat`), so a cache would only add a
/// staleness risk for no measurable speed gain.
pub fn open_standalone(path: &Path) -> Result<Index> {
    if path.is_dir() {
        return index::build_folder(path);
    }
    if !path.is_file() {
        anyhow::bail!("{} does not exist", path.display());
    }
    let data = std::fs::read(path)?;
    match kq_format::sniff(&data) {
        Some(kq_format::ContainerKind::Erf | kq_format::ContainerKind::Rim) => {
            index::build_capsule(path)
        }
        _ => index::build_single_file(path),
    }
}
