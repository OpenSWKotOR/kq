//! LTR — Markov tables used to generate random character names.
//!
//! Layout (V1.0): `"LTR "`, `"V1.0"`, a letter count that must be 28, then
//! start/middle/end probability triples for singles, then the same for each
//! previous letter (doubles) and each previous pair (triples).

use std::path::Path;

use crate::error::Result;
use crate::reader::Reader;

/// KotOR's 28-letter name alphabet, in table order.
pub const LETTERS: &[u8; 28] = b"abcdefghijklmnopqrstuvwxyz'-";

#[derive(Clone, Debug)]
pub struct Chance {
    pub start: f32,
    pub middle: f32,
    pub end: f32,
}

#[derive(Clone, Debug)]
pub struct Ltr {
    pub letter_count: u8,
    /// Per-letter start/middle/end probabilities.
    pub singles: Vec<Chance>,
}

pub fn sniff(data: &[u8]) -> bool {
    data.starts_with(b"LTR ")
}

pub fn read(data: &[u8], path: &Path) -> Result<Ltr> {
    let mut r = Reader::new(data, path);
    r.expect_signature("LTR V1.0")?;
    r.seek(8)?;
    let letter_count = r.u8()?;
    if letter_count != 28 {
        return Err(r.malformed(format!(
            "LTR files that do not handle exactly 28 characters are not supported (got {letter_count})"
        )));
    }
    let n = letter_count as usize;

    let mut singles = Vec::with_capacity(n);
    let start = read_row(&mut r, n)?;
    let middle = read_row(&mut r, n)?;
    let end = read_row(&mut r, n)?;
    for i in 0..n {
        singles.push(Chance {
            start: start[i],
            middle: middle[i],
            end: end[i],
        });
    }

    // Consume doubles and triples so a truncated file still errors instead of
    // silently accepting a header-only stub. The values are not projected —
    // a 28³ table is not what `kq cat` should dump.
    for _ in 0..n {
        consume_block(&mut r, n)?;
    }
    for _ in 0..n {
        for _ in 0..n {
            consume_block(&mut r, n)?;
        }
    }

    Ok(Ltr {
        letter_count,
        singles,
    })
}

fn read_row(r: &mut Reader<'_>, n: usize) -> Result<Vec<f32>> {
    let mut row = Vec::with_capacity(n);
    for _ in 0..n {
        row.push(r.f32()?);
    }
    Ok(row)
}

fn consume_block(r: &mut Reader<'_>, n: usize) -> Result<()> {
    for _ in 0..(n * 3) {
        let _ = r.f32()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn rejects_wrong_letter_count() {
        let mut data = b"LTR V1.0".to_vec();
        data.push(26);
        assert!(read(&data, Path::new("nwn.ltr")).is_err());
    }
}
