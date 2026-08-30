//! 2DA — the game's data tables: appearance, feats, items, spells.
//!
//! # Standard layout (V2.b)
//!
//! The on-disk format kq treats as authoritative:
//!
//! 1. Signature `2DA V2.b` and a newline.
//! 2. Column headers: one **tab-separated** line, terminated by NUL.
//! 3. `u32` row count.
//! 4. Row labels: each terminated by TAB (the last label's TAB may be absent).
//! 5. `u16` cell offsets (`row_count × column_count`), then `u16` data-block size.
//! 6. NUL-terminated cell strings in the data block.
//!
//! [`read`] implements this layout strictly. Any truncation or malformed field is
//! an error — the deserializer does not guess.
//!
//! # Salvage: duplicate 2DAs in `rims/` (non-standard)
//!
//! K1 Steam installs ship shadowed copies of several engine 2DAs inside
//! `rims/global.rim` and `rims/miniglobal.rim` (e.g. `appearance.2da`,
//! `baseitems.2da`). The game never loads them — Override and `data/2da.bif`
//! win — but indexing and export still see them.
//!
//! Those rim copies are **not** valid V2.b tab-header tables. They store column
//! names as **NUL-separated** strings (one name per column), often followed by
//! an extra padding NUL before the row count. Feeding them to [`read`] fails with
//! errors like “row-label table ends before all … labels were read”.
//!
//! [`read_salvage`] is a separate, heuristic parser for that non-standard layout.
//! [`read_or_salvage`] tries [`read`] first and falls back to [`read_salvage`].
//! Any salvage parse records explicit `_warnings` in JSON output (including a
//! banner that salvage ran). CLI decode paths use [`read_or_salvage`] by default.

use std::path::Path;

use crate::error::{FormatError, Result};
use crate::reader::{decode_cstr, Reader};
use crate::shared::{cp1252_display, format_error};

/// Emitted whenever [`read_salvage`] (or [`read_or_salvage`] after strict failure)
/// parses a table. Kept stable so agents can detect salvage output.
pub const SALVAGE_BANNER: &str = "2DA parsed with salvage (non-standard layout; \
see kq-format twoda docs — typical source: rims/global.rim or rims/miniglobal.rim)";

#[derive(Clone, Debug)]
pub struct TwoDa {
    pub columns: Vec<String>,
    /// Row labels as written, usually but not always the row index.
    pub labels: Vec<String>,
    /// `rows[r][c]` — always `columns.len()` wide.
    pub rows: Vec<Vec<String>>,
    /// Populated only by [`read_salvage`] / [`read_or_salvage`] fallback.
    pub warnings: Vec<String>,
}

impl TwoDa {
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns
            .iter()
            .position(|c| c.eq_ignore_ascii_case(name))
    }

    pub fn get(&self, row: usize, column: &str) -> Option<&str> {
        let c = self.column_index(column)?;
        self.rows.get(row)?.get(c).map(String::as_str)
    }
}

pub fn sniff(data: &[u8]) -> bool {
    data.starts_with(b"2DA ")
}

/// Strict V2.b deserializer — tab-separated column header block only.
///
/// The byte layout lives in the shared `kotor-formats` crate, so this reader
/// and OdyPatcher's writer cannot drift on offsets or field widths. The result
/// is narrowed here into kq's flat, query-shaped [`TwoDa`].
pub fn read(data: &[u8], path: &Path) -> Result<TwoDa> {
    reject_unterminated_header(data, path)?;

    let table = kotor_formats::twoda::TwoDaFile::parse(data, &path.to_string_lossy())
        .map_err(|err| format_error(err, path))?;

    let cell = |text: &str| cp1252_display(text);
    let columns: Vec<String> = (0..table.column_count())
        .map(|c| {
            table
                .column_label(c)
                .map(cell)
                .map_err(|err| format_error(err, path))
        })
        .collect::<Result<_>>()?;

    let mut labels = Vec::with_capacity(table.row_count());
    let mut rows = Vec::with_capacity(table.row_count());
    for row in 0..table.row_count() {
        labels.push(
            table
                .row_label(row)
                .map(cell)
                .map_err(|err| format_error(err, path))?,
        );
        let mut cells = Vec::with_capacity(columns.len());
        for column in 0..table.column_count() {
            cells.push(
                table
                    .cell(row, column)
                    .map(cell)
                    .map_err(|err| format_error(err, path))?,
            );
        }
        rows.push(cells);
    }

    Ok(TwoDa {
        columns,
        labels,
        rows,
        warnings: Vec::new(),
    })
}

/// Reject a column header whose last name is not TAB-terminated.
///
/// Every column name in a conformant V2.b table is followed by a TAB,
/// including the last one, and the shared reader emits a column only when it
/// sees that TAB. A header ending `…\tvalue\0` would therefore lose `value`
/// silently. Erroring here instead lets [`read_or_salvage`] treat the file as
/// non-standard rather than quietly handing back a table one column short.
fn reject_unterminated_header(data: &[u8], path: &Path) -> Result<()> {
    let mut pos = 8;
    while pos < data.len() && (data[pos] == b'\n' || data[pos] == b'\r') {
        pos += 1;
    }
    let start = pos;
    while pos < data.len() && data[pos] != 0 {
        pos += 1;
    }
    if pos > start && data[pos - 1] != b'\t' {
        let r = Reader::new(data, path);
        return Err(r.malformed(
            "column header block is not TAB-terminated; the last column name would be lost",
        ));
    }
    Ok(())
}

/// Heuristic parser for non-standard 2DA copies (see module docs — `rims/`).
///
/// Always prepends [`SALVAGE_BANNER`] to [`TwoDa::warnings`]. Further warnings
/// describe any truncated labels, offset tables, or bad cell pointers recovered.
pub fn read_salvage(data: &[u8], path: &Path) -> Result<TwoDa> {
    let start = header_start(data, path)?;
    let mut warnings = vec![SALVAGE_BANNER.to_string()];

    let columns = read_nul_separated_columns(data, start);
    if columns.len() < 2 {
        let r = Reader::new(data, path);
        return Err(r.malformed(format!(
            "salvage: expected NUL-separated column names, found {}",
            columns.len()
        )));
    }

    let mut pos = nul_separated_header_end(data, start);
    pos = skip_padding_nuls(data, pos);
    if pos + 4 > data.len() {
        let r = Reader::new(data, path);
        return Err(r.malformed("salvage: missing row count"));
    }

    let declared_rows = u32_le(data, pos) as usize;
    pos += 4;

    if !plausible_dimensions(data.len(), pos, declared_rows, columns.len()) {
        let r = Reader::new(data, path);
        return Err(r.malformed(format!(
            "salvage: implausible row count {declared_rows} for {} columns",
            columns.len()
        )));
    }

    let (labels, pos) = read_labels_salvage(data, pos, declared_rows, &mut warnings);
    let row_count = labels.len();
    if row_count == 0 {
        return Ok(TwoDa {
            columns,
            labels,
            rows: Vec::new(),
            warnings,
        });
    }

    let mut r = Reader::new(data, path);
    r.seek(pos.min(data.len()))?;
    let cell_count = row_count.saturating_mul(columns.len());
    let mut offsets = Vec::with_capacity(cell_count);
    for _ in 0..cell_count {
        match r.u16() {
            Ok(v) => offsets.push(v as usize),
            Err(_) => {
                warnings.push(format!(
                    "salvage: cell offset table ends after {}/{} entries",
                    offsets.len(),
                    cell_count
                ));
                break;
            }
        }
    }

    if r.u16().is_err() {
        warnings.push("salvage: missing cell data size".into());
    }
    let data_start = r.position();

    let mut rows = Vec::with_capacity(row_count);
    for row in 0..row_count {
        let mut cells = Vec::with_capacity(columns.len());
        for col in 0..columns.len() {
            let idx = row * columns.len() + col;
            let cell = if idx < offsets.len() {
                let off = offsets[idx];
                match data_start.checked_add(off) {
                    Some(cell_offset) if cell_offset < data.len() => {
                        decode_cstr(data, cell_offset)
                    }
                    _ => {
                        if col == 0 {
                            warnings.push(format!(
                                "salvage: bad cell offset at row {row}, column {col}"
                            ));
                        }
                        String::new()
                    }
                }
            } else {
                String::new()
            };
            cells.push(cell);
        }
        rows.push(cells);
    }

    Ok(TwoDa {
        columns,
        labels,
        rows,
        warnings,
    })
}

/// Strict [`read`], then [`read_salvage`] on failure. Used by CLI decode/export.
pub fn read_or_salvage(data: &[u8], path: &Path) -> Result<TwoDa> {
    match read(data, path) {
        Ok(t) => Ok(t),
        Err(_) => read_salvage(data, path),
    }
}

fn header_start(data: &[u8], path: &Path) -> Result<usize> {
    if data.len() < 8 || &data[..8] != b"2DA V2.b" {
        let sig = data.get(..8).unwrap_or(data);
        return Err(FormatError::BadSignature {
            path: path.to_path_buf(),
            expected: "2DA V2.b",
            found: String::from_utf8_lossy(sig).into_owned(),
        });
    }
    let mut pos = 8;
    while pos < data.len() && (data[pos] == b'\n' || data[pos] == b'\r') {
        pos += 1;
    }
    Ok(pos)
}

fn read_nul_separated_columns(data: &[u8], start: usize) -> Vec<String> {
    let mut cols = Vec::new();
    let mut p = start;
    while p < data.len() && data[p] != 0 {
        let s = p;
        while p < data.len() && data[p] != 0 {
            p += 1;
        }
        if s == p {
            break;
        }
        cols.push(String::from_utf8_lossy(&data[s..p]).into_owned());
        p += 1;
    }
    cols
}

fn nul_separated_header_end(data: &[u8], start: usize) -> usize {
    let mut p = start;
    while p < data.len() && data[p] != 0 {
        while p < data.len() && data[p] != 0 {
            p += 1;
        }
        if p < data.len() {
            p += 1;
        }
    }
    p
}

fn skip_padding_nuls(data: &[u8], mut pos: usize) -> usize {
    while pos < data.len() && data[pos] == 0 {
        pos += 1;
    }
    pos
}

fn u32_le(data: &[u8], pos: usize) -> u32 {
    u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
}

fn plausible_dimensions(data_len: usize, labels_start: usize, rows: usize, cols: usize) -> bool {
    if rows == 0 || cols == 0 || rows > 100_000 || cols > 512 {
        return false;
    }
    let tail = rows
        .saturating_add(rows.saturating_mul(cols).saturating_mul(2))
        .saturating_add(2);
    labels_start.saturating_add(tail) <= data_len
}

fn read_labels_salvage(
    data: &[u8],
    mut pos: usize,
    declared_rows: usize,
    warnings: &mut Vec<String>,
) -> (Vec<String>, usize) {
    let mut labels = Vec::with_capacity(declared_rows);
    for i in 0..declared_rows {
        if pos > data.len() {
            warnings.push(format!(
                "salvage: row-label table ends after {}/{} labels",
                labels.len(),
                declared_rows
            ));
            break;
        }
        let start = pos;
        while pos < data.len() && data[pos] != b'\t' {
            pos += 1;
        }
        labels.push(String::from_utf8_lossy(&data[start..pos]).into_owned());
        if pos < data.len() {
            pos += 1;
        } else if i + 1 < declared_rows {
            warnings.push(format!(
                "salvage: row-label table ends after {}/{} labels",
                labels.len(),
                declared_rows
            ));
            break;
        }
    }
    (labels, pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_tab_2da(columns: &[&str], labels: &[&str], cells: &[&[&str]]) -> Vec<u8> {
        let mut out = b"2DA V2.b\n".to_vec();
        for c in columns {
            out.extend_from_slice(c.as_bytes());
            // Every column name is TAB-terminated, the last one included.
            out.push(b'\t');
        }
        out.push(0);
        out.extend_from_slice(&(labels.len() as u32).to_le_bytes());
        for l in labels {
            out.extend_from_slice(l.as_bytes());
            out.push(b'\t');
        }
        let mut pool = String::new();
        let mut offsets = Vec::new();
        for row in cells {
            for cell in *row {
                let entry = format!("{cell}\0");
                if let Some(off) = pool.find(&entry) {
                    offsets.push(off as u16);
                } else {
                    offsets.push(pool.len() as u16);
                    pool.push_str(&entry);
                }
            }
        }
        for off in &offsets {
            out.extend_from_slice(&off.to_le_bytes());
        }
        out.extend_from_slice(&(pool.len() as u16).to_le_bytes());
        out.extend_from_slice(pool.as_bytes());
        out
    }

    /// Mimics the non-standard NUL-separated header block in `rims/global.rim`.
    fn build_rims_style_2da(columns: &[&str], labels: &[&str], cells: &[&[&str]]) -> Vec<u8> {
        let mut out = b"2DA V2.b\n".to_vec();
        for c in columns {
            out.extend_from_slice(c.as_bytes());
            out.push(0);
        }
        out.push(0); // padding NUL before row count
        out.extend_from_slice(&(labels.len() as u32).to_le_bytes());
        for l in labels {
            out.extend_from_slice(l.as_bytes());
            out.push(b'\t');
        }
        let mut pool = String::new();
        let mut offsets = Vec::new();
        for row in cells {
            for cell in *row {
                let entry = format!("{cell}\0");
                if let Some(off) = pool.find(&entry) {
                    offsets.push(off as u16);
                } else {
                    offsets.push(pool.len() as u16);
                    pool.push_str(&entry);
                }
            }
        }
        for off in &offsets {
            out.extend_from_slice(&off.to_le_bytes());
        }
        out.extend_from_slice(&(pool.len() as u16).to_le_bytes());
        out.extend_from_slice(pool.as_bytes());
        out
    }

    #[test]
    fn strict_read_tab_separated_headers() {
        let data = build_tab_2da(&["label", "value"], &["0", "1"], &[&["a", "1"], &["b", "2"]]);
        let t = read(&data, Path::new("test.2da")).unwrap();
        assert_eq!(t.columns, vec!["label", "value"]);
        assert_eq!(t.labels, vec!["0", "1"]);
        assert_eq!(t.rows[0][1], "1");
        assert!(t.warnings.is_empty());
    }

    #[test]
    fn strict_read_rejects_a_header_missing_its_final_tab() {
        // Dropping the trailing TAB used to cost the last column silently;
        // it is now reported so read_or_salvage can take over.
        let mut data = b"2DA V2.b\n".to_vec();
        data.extend_from_slice(b"label\tvalue\0");
        data.extend_from_slice(&0u32.to_le_bytes());
        assert!(read(&data, Path::new("test.2da")).is_err());
    }

    #[test]
    fn strict_read_rejects_rims_style_headers() {
        let data = build_rims_style_2da(
            &["label", "value"],
            &["0", "1"],
            &[&["a", "1"], &["b", "2"]],
        );
        assert!(read(&data, Path::new("rims/global.rim/appearance.2da")).is_err());
    }

    #[test]
    fn salvage_reads_rims_style_headers_with_warning() {
        let data = build_rims_style_2da(
            &["label", "value"],
            &["0", "1"],
            &[&["a", "1"], &["b", "2"]],
        );
        let t = read_salvage(&data, Path::new("rims/global.rim/appearance.2da")).unwrap();
        assert_eq!(t.columns, vec!["label", "value"]);
        assert_eq!(t.labels, vec!["0", "1"]);
        assert!(t.warnings.iter().any(|w| w.contains("salvage")));
        assert_eq!(t.warnings[0], SALVAGE_BANNER);
    }

    #[test]
    fn read_or_salvage_uses_strict_when_possible() {
        let data = build_tab_2da(&["label", "value"], &["0"], &[&["a", "1"]]);
        let t = read_or_salvage(&data, Path::new("data/2da.bif/appearance.2da")).unwrap();
        assert!(t.warnings.is_empty());
    }

    #[test]
    fn read_or_salvage_falls_back_for_rims_style() {
        let data = build_rims_style_2da(&["label", "value"], &["0"], &[&["a", "1"]]);
        let t = read_or_salvage(&data, Path::new("rims/miniglobal.rim/baseitems.2da")).unwrap();
        assert_eq!(t.warnings[0], SALVAGE_BANNER);
    }
}
