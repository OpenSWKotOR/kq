//! ERF-family archives: `.erf`, `.mod`, `.sav`, `.hak`.
//!
//! Layout (V1.0): 8-byte signature, u32 language_count,
//! u32 localized_string_size, u32 entry_count, u32 offset_to_localized_string,
//! u32 offset_to_key_list, u32 offset_to_resource_list, u32 build_year,
//! u32 build_day, u32 description_strref, 116 reserved bytes.
//! Key entries are 24 bytes; resource entries are 8.

use std::path::Path;

use crate::container::Entry;
use crate::error::{FormatError, Result};
use crate::reader::Reader;
use crate::restype::ResType;

pub const SIGNATURES: [&str; 4] = ["ERF ", "MOD ", "SAV ", "HAK "];

/// True when the first bytes look like any ERF-family archive.
pub fn sniff(data: &[u8]) -> bool {
    data.len() >= 8 && SIGNATURES.iter().any(|s| data.starts_with(s.as_bytes()))
}

pub fn read_entries(data: &[u8], path: &Path) -> Result<Vec<Entry>> {
    let mut r = Reader::new(data, path);
    let sig = r.slice_at(0, 4)?;
    if !SIGNATURES.iter().any(|s| sig == s.as_bytes()) {
        return Err(FormatError::BadSignature {
            path: path.to_path_buf(),
            expected: "ERF/MOD/SAV/HAK",
            found: String::from_utf8_lossy(sig).into_owned(),
        });
    }
    let version = r.slice_at(4, 4)?;
    if version != b"V1.0" {
        return Err(FormatError::BadVersion {
            path: path.to_path_buf(),
            format: "ERF",
            version: String::from_utf8_lossy(version).into_owned(),
        });
    }

    r.seek(8)?;
    let _language_count = r.u32()?;
    let _localized_string_size = r.u32()?;
    let entry_count = r.u32()? as usize;
    let _offset_to_localized_string = r.u32()?;
    let key_list_offset = r.u32()? as usize;
    let resource_list_offset = r.u32()? as usize;

    let mut out = Vec::with_capacity(entry_count);
    for i in 0..entry_count {
        r.seek(key_list_offset + i * 24)?;
        let resref = r.fixed_string(16)?;
        let _res_id = r.u32()?;
        let restype = ResType(r.u16()?);

        r.seek(resource_list_offset + i * 8)?;
        let offset = r.u32()? as u64;
        let size = r.u32()? as u64;

        out.push(Entry {
            resref,
            restype,
            file: path.to_path_buf(),
            offset,
            size,
        });
    }
    Ok(out)
}
