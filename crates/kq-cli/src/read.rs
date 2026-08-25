//! Pulling a resource's bytes off disk.
//!
//! Every indexed resource is a (file, offset, size) triple, whether it came
//! from a BIF, a capsule, or a loose file, so one reader covers all of them.

use std::io::{Read, Seek, SeekFrom};

use anyhow::{Context, Result};

use kq_index::{Index, Resource};

/// Read one resource's bytes.
pub fn read(index: &Index, r: &Resource) -> Result<Vec<u8>> {
    let path = index.file(r);
    let mut file =
        std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    if r.offset > 0 {
        file.seek(SeekFrom::Start(r.offset))
            .with_context(|| format!("cannot seek to {} in {}", r.offset, path.display()))?;
    }
    let mut buf = vec![0u8; r.size as usize];
    file.read_exact(&mut buf)
        .with_context(|| format!("{} is shorter than its index entry claims", path.display()))?;
    Ok(buf)
}
