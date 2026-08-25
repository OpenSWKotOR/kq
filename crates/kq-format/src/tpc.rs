//! TPC — KotOR texture, plus the TXI text that often rides in the same file.
//!
//! There is no magic number. The header is 14 bytes of size/alpha/dimensions/
//! format, image data begins at `0x80`, and any leftover bytes after the mip
//! chain are a TXI string. Pixels are not decoded: the greppable content is
//! the TXI and the dimensions.

use std::path::Path;

use crate::error::Result;
use crate::reader::Reader;

const IMAGE_START: usize = 0x80;
const MAX_DIM: u16 = 0x8000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TpcFormat {
    Dxt1,
    Dxt5,
    Greyscale,
    Rgb,
    Rgba,
    Bgra,
}

impl TpcFormat {
    fn name(self) -> &'static str {
        match self {
            Self::Dxt1 => "dxt1",
            Self::Dxt5 => "dxt5",
            Self::Greyscale => "greyscale",
            Self::Rgb => "rgb",
            Self::Rgba => "rgba",
            Self::Bgra => "bgra",
        }
    }

    fn size(self, w: u32, h: u32) -> usize {
        let (w, h) = (w.max(1), h.max(1));
        match self {
            Self::Dxt1 => {
                let blocks = w.div_ceil(4) * h.div_ceil(4);
                (blocks as usize * 8).max(8)
            }
            Self::Dxt5 => {
                let blocks = w.div_ceil(4) * h.div_ceil(4);
                (blocks as usize * 16).max(16)
            }
            Self::Greyscale => (w * h) as usize,
            Self::Rgb => (w * h * 3) as usize,
            Self::Rgba | Self::Bgra => (w * h * 4) as usize,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Tpc {
    pub width: u16,
    pub height: u16,
    pub format: TpcFormat,
    pub mipmaps: u8,
    pub alpha_test: f32,
    pub cube_map: bool,
    /// Trailing TXI text, empty when the file has none.
    pub txi: String,
}

pub fn sniff(data: &[u8]) -> bool {
    // No signature. Callers decide via restype; this only rejects obvious junk.
    data.len() >= IMAGE_START
}

pub fn read(data: &[u8], path: &Path) -> Result<Tpc> {
    let mut r = Reader::new(data, path);
    let data_size = r.u32()? as usize;
    let compressed = data_size != 0;
    let alpha_test = r.f32()?;
    let width = r.u16()?;
    let mut height = r.u16()?;
    let pixel_type = r.u8()?;
    let mipmaps = r.u8()?;

    if width.max(height) >= MAX_DIM {
        return Err(r.malformed(format!("unsupported TPC dimensions {width}x{height}")));
    }

    let format = match (compressed, pixel_type) {
        (true, 2) => TpcFormat::Dxt1,
        (true, 4) => TpcFormat::Dxt5,
        (false, 1) => TpcFormat::Greyscale,
        (false, 2) => TpcFormat::Rgb,
        (false, 4) => TpcFormat::Rgba,
        (false, 12) => TpcFormat::Bgra,
        _ => {
            return Err(r.malformed(format!(
                "unsupported TPC format (compressed={compressed}, pixel_type={pixel_type})"
            )));
        }
    };

    let mut cube_map = false;
    let mut layer_count: u32 = 1;
    let image_bytes = if compressed {
        data_size
    } else {
        format.size(width as u32, height as u32)
    };

    if compressed && height != 0 && width != 0 && height / width == 6 {
        cube_map = true;
        height /= 6;
        layer_count = 6;
    }

    let mut complete = image_bytes;
    for level in 1..mipmaps {
        let w = (width as u32 >> level).max(1);
        let h = (height as u32 >> level).max(1);
        complete += format.size(w, h);
    }
    complete *= layer_count as usize;

    let txi_at = IMAGE_START.saturating_add(complete);
    let txi = if txi_at < data.len() {
        String::from_utf8_lossy(&data[txi_at..])
            .trim_end_matches('\0')
            .to_string()
    } else {
        String::new()
    };

    Ok(Tpc {
        width,
        height,
        format,
        mipmaps,
        alpha_test,
        cube_map,
        txi,
    })
}

impl Tpc {
    pub fn format_name(&self) -> &'static str {
        self.format.name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn greyscale_1x1_with_txi() {
        let txi = b"clamp 1\r\n";
        let mut data = vec![0u8; 0x80 + 1 + txi.len()];
        // uncompressed, 1x1 greyscale, 1 mip
        data[8] = 1;
        data[9] = 0; // width
        data[10] = 1;
        data[11] = 0; // height
        data[12] = 1; // greyscale
        data[13] = 1; // mipmaps
        data[0x80] = 0x7F;
        data[0x81..].copy_from_slice(txi);
        let t = read(&data, Path::new("x.tpc")).unwrap();
        assert_eq!(t.width, 1);
        assert_eq!(t.height, 1);
        assert_eq!(t.format, TpcFormat::Greyscale);
        assert!(t.txi.contains("clamp"));
    }
}
