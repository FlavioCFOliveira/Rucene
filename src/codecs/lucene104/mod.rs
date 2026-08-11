//! Low-level helpers and formats for the `Lucene104` codec.
//!
//! This module currently provides the scalar frame-of-reference and
//! group-varint primitives used by the Lucene 10.4 postings format:
//!
//! * [`ForUtil`] — bit-packs 256 integers per block.
//! * [`PForUtil`] — patched frame-of-reference encoder/decoder.
//! * [`PostingIndexInput`] — `IndexInput` + `ForUtil` decoding wrapper.
//! * [`PostingDecodingUtil`] — scalar `split_ints` primitive.
//! * [`PostingsUtil`] — variable-length postings block helpers.

pub mod for_util;
pub mod p_for_util;
pub mod posting_decoding_util;
pub mod posting_index_input;
pub mod postings_util;

pub use for_util::{ForUtil, BLOCK_SIZE, BLOCK_SIZE_LOG2};
pub use p_for_util::PForUtil;
pub use posting_decoding_util::PostingDecodingUtil;
pub use posting_index_input::PostingIndexInput;
pub use postings_util::{read_v_int_block, write_v_int_block};
