//! Bridge to the shared `kotor-formats` crate.
//!
//! The byte-layout knowledge for 2DA, TLK and SSF lives there, shared with
//! OdyPatcher so both sides agree on offsets and field widths. kq parses
//! through it and then narrows the result into its own query-shaped types.
//!
//! Two things have to be translated at the boundary: errors, which carry a
//! subsystem tag there and a path here, and text, which is stored losslessly
//! there and rendered for people here.

use std::path::Path;

use kotor_formats::error::PatchError;

use crate::error::FormatError;
use crate::gff::cp1252_char;

/// Turn a shared-crate error into one that names the file it came from.
///
/// The tagged form is kept — `"… (2DA-8)"` — because those codes are the
/// vocabulary the original Delphi tools used and they make a malformed file
/// easier to place.
pub(crate) fn format_error(err: PatchError, path: &Path) -> FormatError {
    FormatError::Malformed {
        path: path.to_path_buf(),
        message: err.tagged(),
    }
}

/// Re-render losslessly-decoded text as Windows-1252 for display.
///
/// The shared crate decodes bytes with a bijective Latin-1 map so that
/// writing a file back reproduces it exactly. That is the right choice for
/// storage but the wrong one to show a reader: byte `0x92` is a curly
/// apostrophe in the games' text, and the lossless map leaves it as U+0092.
///
/// This walks that back for the 0x80–0x9F range only, which is the only place
/// Windows-1252 and Latin-1 disagree. It is display-only and has no inverse;
/// nothing that writes bytes may use it.
pub(crate) fn cp1252_display(text: &str) -> String {
    if !text.chars().any(|c| ('\u{80}'..'\u{A0}').contains(&c)) {
        return text.to_string();
    }
    text.chars()
        .map(|c| {
            if ('\u{80}'..'\u{A0}').contains(&c) {
                cp1252_char(c as u8)
            } else {
                c
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_untouched() {
        assert_eq!(cp1252_display("plain text"), "plain text");
    }

    #[test]
    fn high_range_becomes_windows_1252() {
        // 0x92 is a right single quote in Windows-1252, not U+0092.
        let lossless = "don\u{92}t";
        assert_eq!(cp1252_display(lossless), "don\u{2019}t");
    }

    #[test]
    fn latin1_accents_pass_through_unchanged() {
        // 0xE9 is e-acute in both encodings, so it must not be remapped.
        assert_eq!(cp1252_display("caf\u{E9}"), "caf\u{E9}");
    }
}
