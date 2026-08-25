//! `chitin.key` — the index that maps ResRefs to (BIF, index) pairs.
//!
//! Layout (KEY V1, little-endian):
//! `"KEY V1  "`, u32 bif_count, u32 key_count, u32 file_table_offset,
//! u32 key_table_offset, u32 build_year, u32 build_day, 32 reserved bytes.
//! File table entries are 12 bytes; key table entries are 22.

use std::path::{Path, PathBuf};

use crate::container::Entry;
use crate::error::Result;
use crate::reader::{decode_cstr, Reader};
use crate::restype::ResType;

/// A BIF referenced by a KEY file.
#[derive(Clone, Debug)]
pub struct BifRef {
    /// Path as recorded in the KEY, with backslashes normalized.
    pub name: String,
    /// Resolved against the install root, case-corrected where needed.
    pub path: PathBuf,
    pub declared_size: u64,
}

#[derive(Clone, Debug)]
pub struct Key {
    pub path: PathBuf,
    pub bifs: Vec<BifRef>,
    /// One per key-table entry, in file order.
    pub keys: Vec<KeyEntry>,
    pub build_year: u32,
    pub build_day: u32,
}

#[derive(Clone, Debug)]
pub struct KeyEntry {
    pub resref: String,
    pub restype: ResType,
    /// Index into [`Key::bifs`].
    pub bif_index: usize,
    /// Index of the resource within that BIF's variable resource table.
    pub resource_index: usize,
}

impl Key {
    /// Parse a KEY file. `root` is the install directory the BIF paths are
    /// relative to.
    pub fn parse(data: &[u8], path: &Path, root: &Path) -> Result<Key> {
        let mut r = Reader::new(data, path);
        r.expect_signature("KEY V1  ")?;
        r.seek(8)?;

        let bif_count = r.u32()? as usize;
        let key_count = r.u32()? as usize;
        let file_table_offset = r.u32()? as usize;
        let key_table_offset = r.u32()? as usize;
        let build_year = r.u32()?;
        let build_day = r.u32()?;

        let mut bifs = Vec::with_capacity(bif_count);
        for i in 0..bif_count {
            r.seek(file_table_offset + i * 12)?;
            let declared_size = r.u32()? as u64;
            let name_offset = r.u32()? as usize;
            let name_len = r.u16()? as usize;
            let _drives = r.u16()?;

            // The recorded length includes the NUL on some builds and not on
            // others, so read to the NUL and use the length only as a bound.
            let raw = r.slice_at(name_offset, name_len.min(data.len() - name_offset))?;
            let name = decode_cstr(raw, 0);
            let name = if name.is_empty() {
                decode_cstr(data, name_offset)
            } else {
                name
            };
            let normalized = name.replace('\\', "/");
            bifs.push(BifRef {
                path: resolve_relative(root, &normalized),
                name: normalized,
                declared_size,
            });
        }

        let mut keys = Vec::with_capacity(key_count);
        for i in 0..key_count {
            r.seek(key_table_offset + i * 22)?;
            let resref = r.fixed_string(16)?;
            let restype = ResType(r.u16()?);
            let res_id = r.u32()?;
            let bif_index = (res_id >> 20) as usize;
            let resource_index = (res_id & 0x000F_FFFF) as usize;
            if bif_index >= bifs.len() {
                return Err(r.malformed(format!(
                    "key entry {i} ({resref}) points at BIF #{bif_index} but only {} are declared",
                    bifs.len()
                )));
            }
            keys.push(KeyEntry {
                resref,
                restype,
                bif_index,
                resource_index,
            });
        }

        Ok(Key {
            path: path.to_path_buf(),
            bifs,
            keys,
            build_year,
            build_day,
        })
    }
}

/// Resolve a KEY-relative path such as `data/2da.bif` against the install
/// root, falling back to a case-insensitive component walk.
///
/// KEY files were written on Windows, so their casing rarely matches what is
/// on a case-sensitive filesystem.
fn resolve_relative(root: &Path, rel: &str) -> PathBuf {
    let direct = root.join(rel);
    if direct.exists() {
        return direct;
    }
    let mut current = root.to_path_buf();
    for part in rel.split('/').filter(|p| !p.is_empty() && *p != ".") {
        let next = current.join(part);
        if next.exists() {
            current = next;
            continue;
        }
        let found = std::fs::read_dir(&current).ok().and_then(|entries| {
            entries
                .flatten()
                .find(|e| e.file_name().eq_ignore_ascii_case(part))
                .map(|e| e.path())
        });
        match found {
            Some(p) => current = p,
            None => return direct,
        }
    }
    current
}

/// Turn key-table entries into resolved container entries, given the BIF
/// resource tables that were read separately.
pub fn resolve_entries(key: &Key, bif_tables: &[Vec<crate::bif::BifResource>]) -> Vec<Entry> {
    let mut out = Vec::with_capacity(key.keys.len());
    for k in &key.keys {
        let Some(table) = bif_tables.get(k.bif_index) else {
            continue;
        };
        let Some(res) = table.get(k.resource_index) else {
            continue;
        };
        out.push(Entry {
            resref: k.resref.clone(),
            restype: k.restype,
            file: key.bifs[k.bif_index].path.clone(),
            offset: res.offset as u64,
            size: res.size as u64,
        });
    }
    out
}
