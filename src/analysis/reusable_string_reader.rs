//! A reusable character reader backed by a string, ported from
//! `org.apache.lucene.analysis.ReusableStringReader`.

#![deny(unsafe_code)]

use crate::analysis::CharReader;
use crate::error::Result;

/// Internal reader that can be reset with a new string.
///
/// Equivalent to `org.apache.lucene.analysis.ReusableStringReader`.
#[derive(Debug, Default)]
pub struct ReusableStringReader {
    chars: Vec<u16>,
    pos: usize,
    size: usize,
}

impl ReusableStringReader {
    /// Creates an empty reader.
    pub fn new() -> Self {
        Self::default()
    }

    /// Resets the reader to read from `value`.
    pub fn set_value(&mut self, value: impl Into<String>) {
        self.chars = value.into().encode_utf16().collect();
        self.size = self.chars.len();
        self.pos = 0;
    }
}

impl CharReader for ReusableStringReader {
    fn read(&mut self, buf: &mut [u16]) -> Result<usize> {
        if self.pos >= self.size {
            self.chars.clear();
            return Ok(0);
        }
        let n = (self.size - self.pos).min(buf.len());
        buf[..n].copy_from_slice(&self.chars[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }

    fn close(&mut self) -> Result<()> {
        self.pos = self.size;
        self.chars.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_and_read() {
        let mut reader = ReusableStringReader::new();
        reader.set_value("hello world");

        let mut buf = [0u16; 5];
        assert_eq!(reader.read(&mut buf).unwrap(), 5);
        assert_eq!(String::from_utf16_lossy(&buf), "hello");

        reader.set_value("foo");
        assert_eq!(reader.read(&mut buf).unwrap(), 3);
        assert_eq!(String::from_utf16_lossy(&buf[..3]), "foo");
    }

    #[test]
    fn read_returns_zero_at_end() {
        let mut reader = ReusableStringReader::new();
        reader.set_value("hi");
        let mut buf = [0u16; 10];
        assert_eq!(reader.read(&mut buf).unwrap(), 2);
        assert_eq!(reader.read(&mut buf).unwrap(), 0);
    }

    #[test]
    fn close_exhausts_reader() {
        let mut reader = ReusableStringReader::new();
        reader.set_value("test");
        reader.close().unwrap();
        let mut buf = [0u16; 10];
        assert_eq!(reader.read(&mut buf).unwrap(), 0);
    }
}
