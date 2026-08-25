//! BIF archives — the bulk data files a KEY indexes.
//!
//! Layout (BIFF V1): `"BIFFV1  "`, u32 variable_count, u32 fixed_count,
//! u32 variable_table_offset. Variable entries are 16 bytes:
//! u32 id, u32 offset, u32 size, u32 restype.

use std::path::Path;

use crate::error::Result;
use crate::reader::Reader;
use crate::restype::ResType;

/// One row of a BIF's variable resource table.
///
/// The BIF's own restype is advisory — the KEY is authoritative for naming,
/// and the two disagree on a handful of retail entries.
#[derive(Clone, Copy, Debug)]
pub struct BifResource {
    pub id: u32,
    pub offset: u32,
    pub size: u32,
    pub restype: ResType,
}

/// Read only the header and resource table. The payload is left on disk so
/// indexing a 400 MB BIF costs a few kilobytes of reads.
pub fn read_table(data: &[u8], path: &Path) -> Result<Vec<BifResource>> {
    let mut r = Reader::new(data, path);
    r.expect_signature("BIFF")?;
    r.seek(8)?;
    let variable_count = r.u32()? as usize;
    let _fixed_count = r.u32()?;
    let table_offset = r.u32()? as usize;

    let mut out = Vec::with_capacity(variable_count);
    for i in 0..variable_count {
        r.seek(table_offset + i * 16)?;
        let id = r.u32()?;
        let offset = r.u32()?;
        let size = r.u32()?;
        let restype = ResType(r.u32()? as u16);
        out.push(BifResource { id, offset, size, restype });
    }
    Ok(out)
}
