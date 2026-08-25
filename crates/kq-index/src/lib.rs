//! Discovering a KotOR installation and indexing everything in it.

pub mod cache;
pub mod discover;
pub mod game;
pub mod index;
pub mod source;

pub use discover::Install;
pub use game::Game;
pub use index::{Index, Resource};
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
pub fn open(root: &Path, read_cache: bool, write_cache: bool) -> Result<(Install, Index, Freshness)> {
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
