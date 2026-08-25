//! 2DA — the game's data tables: appearance, feats, items, spells.
//!
//! Layout (V2.b): the signature line, tab-separated column headers ending in
//! NUL, a u32 row count, tab-terminated row labels, a u16 cell offset per
//! cell, a u16 data-block size, then NUL-terminated cell strings.
//!
//! Cells are stored deduplicated — a column of mostly `****` costs one
//! string — which is why offsets, not lengths, are what the file records.

use std::path::Path;

use crate::error::{FormatError, Result};
use crate::reader::{decode_cstr, Reader};

#[derive(Clone, Debug)]
pub struct TwoDa {
    pub columns: Vec<String>,
    /// Row labels as written, usually but not always the row index.
    pub labels: Vec<String>,
    /// `rows[r][c]` — always `columns.len()` wide.
    pub rows: Vec<Vec<String>>,
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

pub fn read(data: &[u8], path: &Path) -> Result<TwoDa> {
    let mut r = Reader::new(data, path);
    let sig = r.slice_at(0, 8)?;
    if sig != b"2DA V2.b" {
        return Err(FormatError::BadSignature {
            path: path.to_path_buf(),
            expected: "2DA V2.b",
            found: String::from_utf8_lossy(sig).into_owned(),
        });
    }
    // A newline follows the signature; some writers emit \r\n.
    let mut pos = 8;
    while pos < data.len() && (data[pos] == b'\n' || data[pos] == b'\r') {
        pos += 1;
    }

    // Column headers: tab-separated, terminated by NUL.
    let start = pos;
    while pos < data.len() && data[pos] != 0 {
        pos += 1;
    }
    let header = String::from_utf8_lossy(&data[start..pos]);
    let columns: Vec<String> = header
        .split('\t')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    pos += 1;

    r.seek(pos)?;
    let row_count = r.u32()? as usize;

    // Row labels: each terminated by a tab. A label's tab can legitimately
    // be the file's last byte, so `p` may sit at `data.len()` between rows —
    // that is not truncation, only a missing final label is.
    let mut labels = Vec::with_capacity(row_count);
    let mut p = r.position();
    for _ in 0..row_count {
        if p > data.len() {
            return Err(r.malformed(format!(
                "row-label table ends before all {row_count} labels were read"
            )));
        }
        let start = p;
        while p < data.len() && data[p] != b'\t' {
            p += 1;
        }
        labels.push(String::from_utf8_lossy(&data[start..p]).to_string());
        p += 1;
    }

    let cell_count = row_count.saturating_mul(columns.len());
    r.seek(p.min(data.len()))?;
    let mut offsets = Vec::with_capacity(cell_count);
    for _ in 0..cell_count {
        offsets.push(r.u16()? as usize);
    }
    let _data_size = r.u16()?;
    let data_start = r.position();

    let mut rows = Vec::with_capacity(row_count);
    for row in 0..row_count {
        let mut cells = Vec::with_capacity(columns.len());
        for col in 0..columns.len() {
            let off = offsets[row * columns.len() + col];
            let cell_offset = data_start.checked_add(off).ok_or_else(|| {
                r.malformed(format!("cell offset overflow at row {row}, column {col}"))
            })?;
            cells.push(decode_cstr(data, cell_offset));
        }
        rows.push(cells);
    }

    Ok(TwoDa {
        columns,
        labels,
        rows,
    })
}
