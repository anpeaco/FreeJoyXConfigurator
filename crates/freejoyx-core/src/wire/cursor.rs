//! Tiny little-endian reader over a `&[u8]`.
//!
//! Used by the per-field decoders so each field's read is one line:
//! `let firmware_version = cur.read_u16_le()?;`
//!
//! Encoding is symmetric: a parallel `Writer` writes the same fields
//! in the same order, and the paired `encode_<field>` /
//! `decode_<field>` tests catch any drift between the two.

use super::error::DecodeError;

/// Read cursor over a byte slice, tracking position.
///
/// All reads are little-endian (FreeJoyX wire format is LE on both
/// ends). Reads advance the cursor and return `DecodeError::BufferTooShort`
/// when the underlying buffer doesn't have enough bytes left.
pub struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    #[must_use]
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    #[must_use]
    pub fn position(&self) -> usize {
        self.pos
    }

    #[must_use]
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    /// Advance the cursor without reading. Used to skip reserved
    /// padding bytes in the struct.
    pub fn skip(&mut self, n: usize) -> Result<(), DecodeError> {
        self.ensure(n)?;
        self.pos += n;
        Ok(())
    }

    fn ensure(&self, n: usize) -> Result<(), DecodeError> {
        if self.remaining() < n {
            Err(DecodeError::BufferTooShort {
                needed: n,
                got: self.remaining(),
            })
        } else {
            Ok(())
        }
    }

    pub fn read_u8(&mut self) -> Result<u8, DecodeError> {
        self.ensure(1)?;
        let v = self.buf[self.pos];
        self.pos += 1;
        Ok(v)
    }

    pub fn read_i8(&mut self) -> Result<i8, DecodeError> {
        self.read_u8().map(|v| v as i8)
    }

    pub fn read_u16_le(&mut self) -> Result<u16, DecodeError> {
        self.ensure(2)?;
        let bytes = [self.buf[self.pos], self.buf[self.pos + 1]];
        self.pos += 2;
        Ok(u16::from_le_bytes(bytes))
    }

    pub fn read_i16_le(&mut self) -> Result<i16, DecodeError> {
        self.read_u16_le().map(|v| v as i16)
    }

    pub fn read_array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        self.ensure(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(&self.buf[self.pos..self.pos + N]);
        self.pos += N;
        Ok(out)
    }
}

/// Little-endian writer over a mutable byte slice, tracking position.
///
/// Panics if a write would exceed the buffer — encode paths control
/// the buffer length themselves and a panic indicates a codec bug.
pub struct Writer<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> Writer<'a> {
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    #[must_use]
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Advance the writer without writing. Caller is responsible for
    /// ensuring the skipped bytes are already zero or contain expected
    /// padding.
    pub fn skip(&mut self, n: usize) {
        self.pos += n;
    }

    pub fn write_u8(&mut self, v: u8) {
        self.buf[self.pos] = v;
        self.pos += 1;
    }

    pub fn write_i8(&mut self, v: i8) {
        self.write_u8(v as u8);
    }

    pub fn write_u16_le(&mut self, v: u16) {
        let bytes = v.to_le_bytes();
        self.buf[self.pos] = bytes[0];
        self.buf[self.pos + 1] = bytes[1];
        self.pos += 2;
    }

    pub fn write_i16_le(&mut self, v: i16) {
        self.write_u16_le(v as u16);
    }

    pub fn write_array<const N: usize>(&mut self, src: &[u8; N]) {
        self.buf[self.pos..self.pos + N].copy_from_slice(src);
        self.pos += N;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_u16_le() {
        let mut buf = [0u8; 2];
        Writer::new(&mut buf).write_u16_le(0x0010);
        assert_eq!(buf, [0x10, 0x00]);
        assert_eq!(Cursor::new(&buf).read_u16_le().unwrap(), 0x0010);
    }

    #[test]
    fn roundtrip_i16_le_signed() {
        let mut buf = [0u8; 2];
        Writer::new(&mut buf).write_i16_le(-1);
        assert_eq!(buf, [0xff, 0xff]);
        assert_eq!(Cursor::new(&buf).read_i16_le().unwrap(), -1);
    }

    #[test]
    fn short_buffer_errors() {
        let buf = [0x10];
        let err = Cursor::new(&buf).read_u16_le().unwrap_err();
        assert!(matches!(
            err,
            DecodeError::BufferTooShort { needed: 2, got: 1 }
        ));
    }

    #[test]
    fn skip_advances() {
        let buf = [0xaa, 0xbb, 0xcc];
        let mut c = Cursor::new(&buf);
        c.skip(2).unwrap();
        assert_eq!(c.read_u8().unwrap(), 0xcc);
    }
}
