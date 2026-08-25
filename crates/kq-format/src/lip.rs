//! LIP — lip-sync keyframes: a mouth shape at each timestamp, played against
//! the matching WAV.
//!
//! Layout (V1.0): `"LIP "`, `"V1.0"`, f32 total length (seconds), u32
//! keyframe count, then that many (f32 time, u8 shape) pairs.

use std::path::Path;

use crate::error::Result;
use crate::reader::Reader;

/// The mouth-shape id PyKotor's `LIPShape` enum uses. Kept as a raw byte
/// rather than a closed enum — retail data only uses a handful of values but
/// nothing guarantees a mod author's LIP file does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape(pub u8);

#[derive(Clone, Debug)]
pub struct Keyframe {
    pub time: f32,
    pub shape: Shape,
}

#[derive(Clone, Debug)]
pub struct Lip {
    /// Total duration in seconds, matching the paired WAV.
    pub length: f32,
    pub keyframes: Vec<Keyframe>,
}

pub fn sniff(data: &[u8]) -> bool {
    data.starts_with(b"LIP ")
}

pub fn read(data: &[u8], path: &Path) -> Result<Lip> {
    let mut r = Reader::new(data, path);
    r.expect_signature("LIP V1.0")?;
    r.seek(8)?;
    let length = r.f32()?;
    let count = r.u32()? as usize;

    let mut keyframes = Vec::with_capacity(count);
    for _ in 0..count {
        let time = r.f32()?;
        let shape = Shape(r.u8()?);
        keyframes.push(Keyframe { time, shape });
    }
    Ok(Lip { length, keyframes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn reads_keyframes() {
        let mut data = b"LIP V1.0".to_vec();
        data.extend_from_slice(&1.5f32.to_le_bytes());
        data.extend_from_slice(&2u32.to_le_bytes());
        data.extend_from_slice(&0.0f32.to_le_bytes());
        data.push(3);
        data.extend_from_slice(&0.5f32.to_le_bytes());
        data.push(7);
        let l = read(&data, Path::new("line.lip")).unwrap();
        assert_eq!(l.length, 1.5);
        assert_eq!(l.keyframes.len(), 2);
        assert_eq!(l.keyframes[1].shape.0, 7);
    }
}
