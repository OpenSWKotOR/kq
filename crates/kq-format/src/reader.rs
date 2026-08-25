use std::path::{Path, PathBuf};

use crate::error::{FormatError, Result};

/// A bounds-checked cursor over an in-memory container.
///
/// Every read reports the owning path so a malformed archive names itself in
/// the error rather than surfacing as an anonymous slice panic.
pub struct Reader<'a> {
    data: &'a [u8],
    path: PathBuf,
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8], path: impl Into<PathBuf>) -> Self {
        Self {
            data,
            path: path.into(),
            pos: 0,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn seek(&mut self, offset: usize) -> Result<()> {
        if offset > self.data.len() {
            return Err(self.truncated(offset, 0));
        }
        self.pos = offset;
        Ok(())
    }

    fn truncated(&self, offset: usize, needed: usize) -> FormatError {
        FormatError::Truncated {
            path: self.path.clone(),
            offset,
            needed,
            len: self.data.len(),
        }
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| self.truncated(self.pos, n))?;
        if end > self.data.len() {
            return Err(self.truncated(self.pos, n));
        }
        let out = &self.data[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    pub fn slice_at(&self, offset: usize, n: usize) -> Result<&'a [u8]> {
        let end = offset
            .checked_add(n)
            .ok_or_else(|| self.truncated(offset, n))?;
        if end > self.data.len() {
            return Err(self.truncated(offset, n));
        }
        Ok(&self.data[offset..end])
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }

    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }

    pub fn u16_be(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    pub fn i16_be(&mut self) -> Result<i16> {
        Ok(self.u16_be()? as i16)
    }

    pub fn u32_be(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn i32_be(&mut self) -> Result<i32> {
        Ok(self.u32_be()? as i32)
    }

    pub fn f32_be(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32_be()?))
    }

    /// Read a fixed-width, NUL-padded field such as a 16-byte ResRef.
    ///
    /// The engines are inconsistent about padding, so anything from the first
    /// NUL onward is dropped and the result is lowercased — ResRefs are
    /// case-insensitive everywhere they are compared.
    pub fn fixed_string(&mut self, width: usize) -> Result<String> {
        let raw = self.take(width)?;
        Ok(decode_fixed(raw))
    }

    /// Like [`Reader::fixed_string`] but preserving case.
    ///
    /// GFF field labels are compared case-insensitively but displayed as
    /// written, and lowercasing them would make output unreadable.
    pub fn fixed_string_cased(&mut self, width: usize) -> Result<String> {
        let raw = self.take(width)?;
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        Ok(String::from_utf8_lossy(&raw[..end]).trim_end().to_string())
    }

    pub fn malformed(&self, message: impl Into<String>) -> FormatError {
        FormatError::Malformed {
            path: self.path.clone(),
            message: message.into(),
        }
    }

    pub fn expect_signature(&mut self, expected: &'static str) -> Result<()> {
        let raw = self.slice_at(0, expected.len())?;
        if raw != expected.as_bytes() {
            return Err(FormatError::BadSignature {
                path: self.path.clone(),
                expected,
                found: String::from_utf8_lossy(raw).into_owned(),
            });
        }
        Ok(())
    }
}

/// Trim a NUL-padded fixed-width field and lowercase it.
pub fn decode_fixed(raw: &[u8]) -> String {
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end])
        .trim()
        .to_ascii_lowercase()
}

/// Decode a NUL-terminated string starting at `offset`, preserving case.
pub fn decode_cstr(data: &[u8], offset: usize) -> String {
    if offset >= data.len() {
        return String::new();
    }
    let rest = &data[offset..];
    let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
    String::from_utf8_lossy(&rest[..end]).into_owned()
}
