//! Wrapper that pairs a [`ForUtil`] with an [`IndexInput`] for decoding.
//!
//! Port of `org.apache.lucene.codecs.lucene104.PostingIndexInput`. This exists
//! mostly to mirror the Lucene class structure; it simply forwards to
//! [`ForUtil::decode`].

use std::sync::{Arc, LazyLock};

use crate::error::Result;
use crate::internal::vectorization::{get_instance, PostingDecodingUtil, VectorizationProvider};
use crate::store::IndexInput;

use super::for_util::{ForUtil, BLOCK_SIZE};

/// `PostingIndexInput.VECTORIZATION_PROVIDER` (`PostingIndexInput.java:31-32`).
static VECTORIZATION_PROVIDER: LazyLock<Arc<dyn VectorizationProvider>> =
    LazyLock::new(|| get_instance());

/// Wrapper around an [`IndexInput`] and a [`ForUtil`] that optionally
/// optimizes decoding using vectorization.
///
/// Equivalent to `org.apache.lucene.codecs.lucene104.PostingIndexInput`. The
/// decoder is obtained once, from the selected provider, when the wrapper is
/// built, and owns the input (see [`PostingDecodingUtil`]).
pub struct PostingIndexInput {
    /// `public final ForUtil forUtil`.
    pub for_util: ForUtil,
    /// `private final PostingDecodingUtil postingDecodingUtil`.
    posting_decoding_util: PostingDecodingUtil,
}

impl PostingIndexInput {
    /// Equivalent to `PostingIndexInput(IndexInput, ForUtil)`.
    ///
    /// # Errors
    ///
    /// The errors of
    /// [`VectorizationProvider::new_posting_decoding_util`].
    pub fn new(input: Box<dyn IndexInput>, for_util: ForUtil) -> Result<Self> {
        Ok(Self {
            for_util,
            posting_decoding_util: VECTORIZATION_PROVIDER.new_posting_decoding_util(input)?,
        })
    }

    /// Decodes 256 integers stored on `bits_per_value` bits per value into
    /// `ints`.
    ///
    /// Equivalent to `PostingIndexInput.decode(int, int[])`.
    pub fn decode(&mut self, bits_per_value: i32, ints: &mut [i32; BLOCK_SIZE]) -> Result<()> {
        self.for_util
            .decode(bits_per_value, &mut self.posting_decoding_util, ints)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{MockIndexInput, VecDataOutput};

    #[test]
    fn posting_index_input_round_trip() {
        let mut for_util = ForUtil::new();
        let mut original = [0i32; BLOCK_SIZE];
        for (i, v) in original.iter_mut().enumerate() {
            *v = ((i * 0x9E37_79B9) & 0xFFFF) as i32;
        }

        let mut encoded = original;
        let mut out = VecDataOutput::new();
        for_util.encode(&mut encoded, 16, &mut out).unwrap();

        let data = out.into_inner();
        let input = MockIndexInput::new(data, "posting-index-input");
        let mut pii = PostingIndexInput::new(Box::new(input), ForUtil::new()).unwrap();
        let mut decoded = [0i32; BLOCK_SIZE];
        pii.decode(16, &mut decoded).unwrap();

        assert_eq!(&decoded[..], &original[..]);
    }
}
