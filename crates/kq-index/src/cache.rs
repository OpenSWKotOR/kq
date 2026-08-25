//! Persisting the index so the second run is instant.
//!
//! Indexing a retail install means reading ~370 container headers and
//! stat-ing every loose file — seconds of work that produces the same answer
//! until the install changes. The cache key is a fingerprint of what is on
//! disk, so an edited Override or a newly installed mod invalidates it
//! without the user having to know the cache exists.

use std::hash::Hasher;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::discover::Install;
use crate::index::{Index, SCHEMA_VERSION};

/// Directories whose contents decide whether a cached index is still valid.
fn fingerprint_inputs(install: &Install) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = [
        install.data.as_ref(),
        install.modules.as_ref(),
        install.override_dir.as_ref(),
        install.lips.as_ref(),
        install.texturepacks.as_ref(),
        install.rims.as_ref(),
    ]
    .into_iter()
    .flatten()
    .cloned()
    .collect();
    dirs.extend(install.streams.iter().cloned());
    dirs
}

/// Hash the install's shape: schema version, root, and every indexed file's
/// name, size and mtime.
///
/// Content is deliberately not hashed. Reading 1.3 GB of BIFs to decide
/// whether to skip reading them defeats the purpose, and name+size+mtime
/// catches every change a mod installer or a human actually makes.
pub fn fingerprint(install: &Install) -> u64 {
    let mut hasher = twox_hash::XxHash3_64::with_seed(0);
    hasher.write_u32(SCHEMA_VERSION);
    hasher.write(install.root.as_os_str().as_encoded_bytes());
    hasher.write(install.game.as_str().as_bytes());

    stamp(&mut hasher, &install.chitin);
    for path in &install.talk_tables {
        stamp(&mut hasher, path);
    }
    for dir in fingerprint_inputs(install) {
        let mut paths = Vec::new();
        collect(&dir, &mut paths);
        paths.sort();
        for p in paths {
            stamp(&mut hasher, &p);
        }
    }
    hasher.finish()
}

fn stamp(hasher: &mut impl Hasher, path: &Path) {
    hasher.write(path.as_os_str().as_encoded_bytes());
    if let Ok(meta) = std::fs::metadata(path) {
        hasher.write_u64(meta.len());
        if let Ok(mtime) = meta.modified() {
            if let Ok(d) = mtime.duration_since(std::time::UNIX_EPOCH) {
                hasher.write_u128(d.as_nanos());
            }
        }
    }
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for e in entries.flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(t) if t.is_file() => out.push(e.path()),
                _ => {}
            }
        }
    }
}

/// Where cached indexes live. Honors `KQ_CACHE_DIR`, then the platform cache
/// directory.
pub fn cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("KQ_CACHE_DIR") {
        return PathBuf::from(dir);
    }
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("kq")
}

fn cache_path(fingerprint: u64) -> PathBuf {
    cache_dir().join(format!("index-{SCHEMA_VERSION}-{fingerprint:016x}.mpk"))
}

/// Load a cached index if one matches the install's current state.
pub fn load(install: &Install) -> Option<Index> {
    let fp = fingerprint(install);
    let path = cache_path(fp);
    let bytes = std::fs::read(&path).ok()?;
    let mut index: Index = rmp_serde::from_slice(&bytes).ok()?;
    if index.schema != SCHEMA_VERSION || index.fingerprint != fp {
        return None;
    }
    index.reindex();
    Some(index)
}

/// Write an index to the cache.
///
/// Writes to a temporary file and renames, so a concurrent reader never sees
/// a half-written index and two concurrent writers cannot interleave.
pub fn store(index: &Index) -> Result<PathBuf> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("cannot create cache directory {}", dir.display()))?;
    let final_path = cache_path(index.fingerprint);
    let tmp = dir.join(format!(
        ".index-{}-{}.tmp",
        std::process::id(),
        index.fingerprint
    ));
    let bytes = rmp_serde::to_vec_named(index).context("serializing index")?;
    std::fs::write(&tmp, &bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &final_path)
        .with_context(|| format!("installing cache entry {}", final_path.display()))?;
    Ok(final_path)
}

/// Delete every cached index. Returns how many files were removed.
pub fn clear() -> Result<usize> {
    let dir = cache_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(0);
    };
    let mut n = 0;
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("index-") || name.starts_with(".index-") {
            std::fs::remove_file(e.path())?;
            n += 1;
        }
    }
    Ok(n)
}
