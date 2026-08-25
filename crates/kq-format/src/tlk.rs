//! TLK — `dialog.tlk`, the table every piece of displayed text points into.
//!
//! Layout (V3.0): signature, u32 language id, u32 string count,
//! u32 string-data offset, then a 40-byte entry per string.

use std::path::Path;

use crate::error::Result;
use crate::gff::cp1252_char;
use crate::reader::Reader;

#[derive(Clone, Debug)]
pub struct Entry {
    pub text: String,
    /// Voice-over resource for this line, empty when there is none.
    pub sound: String,
}

#[derive(Clone, Debug)]
pub struct Tlk {
    pub language_id: u32,
    pub entries: Vec<Entry>,
}

impl Tlk {
    /// Look up a StrRef. Out-of-range and the sentinel `-1` yield `None`.
    pub fn get(&self, strref: i64) -> Option<&Entry> {
        if strref < 0 {
            return None;
        }
        self.entries.get(strref as usize)
    }
}

pub fn sniff(data: &[u8]) -> bool {
    data.starts_with(b"TLK ")
}

pub fn read(data: &[u8], path: &Path) -> Result<Tlk> {
    let mut r = Reader::new(data, path);
    r.expect_signature("TLK V3.0")?;
    r.seek(8)?;
    let language_id = r.u32()?;
    let count = r.u32()? as usize;
    let data_offset = r.u32()? as usize;

    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        r.seek(20 + i * 40)?;
        let _flags = r.u32()?;
        let sound = r.fixed_string(16)?;
        let _volume_variance = r.u32()?;
        let _pitch_variance = r.u32()?;
        let offset = r.u32()? as usize;
        let size = r.u32()? as usize;
        let _sound_length = r.f32()?;

        let text = match r.slice_at(data_offset + offset, size) {
            Ok(bytes) => decode(bytes),
            // A truncated string entry should not lose the other 49,999.
            Err(_) => String::new(),
        };
        entries.push(Entry { text, sound });
    }
    Ok(Tlk { language_id, entries })
}

fn decode(bytes: &[u8]) -> String {
    if bytes.is_ascii() {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        bytes.iter().map(|&b| cp1252_char(b)).collect()
    }
}
