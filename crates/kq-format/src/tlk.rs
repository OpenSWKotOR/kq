//! TLK — `dialog.tlk`, the table every piece of displayed text points into.
//!
//! Layout (V3.0): signature, u32 language id, u32 string count,
//! u32 string-data offset, then a 40-byte entry per string.

use std::path::Path;

use kotor_formats::tlk::TlkFile;

use crate::error::Result;
use crate::shared::{cp1252_display, format_error};

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
    let file =
        TlkFile::parse(data, &path.to_string_lossy()).map_err(|err| format_error(err, path))?;

    let entries = file
        .entries()
        .iter()
        .map(|entry| Entry {
            text: cp1252_display(&entry.text),
            // Resource names are matched case-insensitively, so they are shown
            // folded, the way every other resref in kq's output is.
            sound: entry.sound_name().trim().to_ascii_lowercase(),
        })
        .collect();

    Ok(Tlk {
        language_id: file.language_id,
        entries,
    })
}
