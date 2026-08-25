//! WAV / BMU — audio metadata. Samples are not decoded.
//!
//! KotOR ships three flavours: a normal RIFF/WAVE, a 470-byte obfuscated SFX
//! header in front of a WAVE, and a BMU-wrapped MP3.

use std::path::Path;

use crate::error::Result;
use crate::reader::Reader;

const SFX_HEADER: usize = 470;

#[derive(Clone, Debug)]
pub struct Wav {
    pub kind: &'static str,
    pub channels: Option<u16>,
    pub sample_rate: Option<u32>,
    pub bits_per_sample: Option<u16>,
    pub encoding: Option<u16>,
    pub data_bytes: Option<u32>,
}

pub fn sniff(data: &[u8]) -> bool {
    looks_like_riff(data) || data.starts_with(b"BMU ") || has_sfx_header(data)
}

fn looks_like_riff(data: &[u8]) -> bool {
    data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WAVE"
}

fn has_sfx_header(data: &[u8]) -> bool {
    data.len() > SFX_HEADER + 12 && looks_like_riff(&data[SFX_HEADER..])
}

pub fn read(data: &[u8], path: &Path) -> Result<Wav> {
    let r = Reader::new(data, path);
    if data.starts_with(b"BMU ") {
        return Ok(Wav {
            kind: "bmu",
            channels: None,
            sample_rate: None,
            bits_per_sample: None,
            encoding: None,
            data_bytes: Some(data.len().saturating_sub(8) as u32),
        });
    }
    let (body, kind) = if has_sfx_header(data) {
        (&data[SFX_HEADER..], "sfx")
    } else if looks_like_riff(data) {
        (data, "wav")
    } else {
        return Err(r.malformed("not a RIFF/WAVE, BMU, or obfuscated SFX"));
    };
    parse_riff(body, kind, path)
}

fn parse_riff(data: &[u8], kind: &'static str, path: &Path) -> Result<Wav> {
    let mut r = Reader::new(data, path);
    r.expect_signature("RIFF")?;
    r.seek(8)?;
    let wave = r.take(4)?;
    if wave != b"WAVE" {
        return Err(r.malformed("RIFF file is not WAVE"));
    }

    let mut wav = Wav {
        kind,
        channels: None,
        sample_rate: None,
        bits_per_sample: None,
        encoding: None,
        data_bytes: None,
    };

    while r.position() + 8 <= r.len() {
        let id = r.take(4)?;
        let size = r.u32()? as usize;
        let payload_at = r.position();
        if id == b"fmt " && size >= 16 {
            wav.encoding = Some(r.u16()?);
            wav.channels = Some(r.u16()?);
            wav.sample_rate = Some(r.u32()?);
            let _byte_rate = r.u32()?;
            let _align = r.u16()?;
            wav.bits_per_sample = Some(r.u16()?);
        } else if id == b"data" {
            wav.data_bytes = Some(size as u32);
        }
        // Chunks are word-aligned.
        let next = payload_at.saturating_add(size + (size & 1));
        if next <= r.len() {
            r.seek(next)?;
        } else {
            break;
        }
    }
    Ok(wav)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn reads_minimal_pcm() {
        let mut data = Vec::new();
        data.extend_from_slice(b"RIFF");
        data.extend_from_slice(&36u32.to_le_bytes());
        data.extend_from_slice(b"WAVE");
        data.extend_from_slice(b"fmt ");
        data.extend_from_slice(&16u32.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes()); // PCM
        data.extend_from_slice(&1u16.to_le_bytes()); // mono
        data.extend_from_slice(&22050u32.to_le_bytes());
        data.extend_from_slice(&44100u32.to_le_bytes());
        data.extend_from_slice(&2u16.to_le_bytes());
        data.extend_from_slice(&16u16.to_le_bytes());
        data.extend_from_slice(b"data");
        data.extend_from_slice(&0u32.to_le_bytes());
        let w = read(&data, Path::new("x.wav")).unwrap();
        assert_eq!(w.kind, "wav");
        assert_eq!(w.channels, Some(1));
        assert_eq!(w.sample_rate, Some(22050));
        assert_eq!(w.bits_per_sample, Some(16));
    }
}
