//! RIM archives — the read-only module format.
//!
//! Layout: `"RIM V1.0"`, u32 reserved, u32 entry_count,
//! u32 offset_to_resource_list, 100 reserved bytes. Resource entries are
//! 32 bytes: char[16] resref, u32 restype, u32 res_id, u32 offset, u32 size.

use std::path::Path;

use crate::container::Entry;
use crate::error::Result;
use crate::reader::Reader;
use crate::restype::ResType;

pub fn sniff(data: &[u8]) -> bool {
    data.starts_with(b"RIM ")
}

pub fn read_entries(data: &[u8], path: &Path) -> Result<Vec<Entry>> {
    let mut r = Reader::new(data, path);
    r.expect_signature("RIM V1.0")?;
    r.seek(8)?;
    let _reserved = r.u32()?;
    let entry_count = r.u32()? as usize;
    let list_offset = r.u32()? as usize;

    let mut out = Vec::with_capacity(entry_count);
    for i in 0..entry_count {
        r.seek(list_offset + i * 32)?;
        let resref = r.fixed_string(16)?;
        let restype = ResType(r.u32()? as u16);
        let _res_id = r.u32()?;
        let offset = r.u32()? as u64;
        let size = r.u32()? as u64;
        out.push(Entry { resref, restype, file: path.to_path_buf(), offset, size });
    }
    Ok(out)
}
