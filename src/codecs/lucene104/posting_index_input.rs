//! Wrapper that pairs a [`ForUtil`] with an [`IndexInput`] for decoding.
//!
//! Port of `org.apache.lucene.codecs.lucene104.PostingIndexInput`. This exists
//! mostly to mirror the Lucene class structure; it simply forwards to
//! [`ForUtil::decode`].

use crate::error::Result;
use crate::store::IndexInput;

use super::for_util::{ForUtil, BLOCK_SIZE};
use super::posting_decoding_util::PostingDecodingUtil;

/// Wrapper around an [`IndexInput`] and a [`ForUtil`].
pub struct PostingIndexInput {
    input: Box<dyn IndexInput>,
    for_util: ForUtil,
}

impl PostingIndexInput {
    /// Creates a new wrapper over `input`.
    pub fn new(input: Box<dyn IndexInput>) -> Self {
        Self {
            input,
            for_util: ForUtil::new(),
        }
    }

    /// Decodes 256 integers stored on `bits_per_value` bits per value into
    /// `ints`.
    pub fn decode(&mut self, bits_per_value: i32, ints: &mut [i32; BLOCK_SIZE]) -> Result<()> {
        let mut pdu = PostingDecodingUtil::new(&mut *self.input);
        self.for_util.decode(bits_per_value, &mut pdu, ints)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ByteArrayDataOutput, MockIndexInput};

    #[test]
    fn posting_index_input_round_trip() {
        let mut for_util = ForUtil::new();
        let mut original = [0i32; BLOCK_SIZE];
        for (i, v) in original.iter_mut().enumerate() {
            *v = ((i * 0x9E37_79B9) & 0xFFFF) as i32;
        }

        let mut encoded = original;
        let mut out = ByteArrayDataOutput::new();
        for_util.encode(&mut encoded, 16, &mut out).unwrap();

        let data = out.into_inner();
        let input = MockIndexInput::new(data, "posting-index-input");
        let mut pii = PostingIndexInput::new(Box::new(input));
        let mut decoded = [0i32; BLOCK_SIZE];
        pii.decode(16, &mut decoded).unwrap();

        assert_eq!(&decoded[..], &original[..]);
    }
}
