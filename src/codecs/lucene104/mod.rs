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
//! * [`Lucene104PostingsFormat`] — the postings format wrapper.
//! * [`Lucene104PostingsWriter`] / [`Lucene104PostingsReader`] — low-level
//!   postings encode/decode skeletons.

pub mod codec;
pub mod for_util;
pub mod p_for_util;
pub mod posting_decoding_util;
pub mod posting_index_input;
pub mod postings_format;
pub mod postings_reader;
pub mod postings_util;
pub mod postings_writer;

pub use codec::{Lucene104Codec, Mode};
pub use for_util::{ForUtil, BLOCK_SIZE as FOR_BLOCK_SIZE, BLOCK_SIZE_LOG2 as FOR_BLOCK_SIZE_LOG2};
pub use p_for_util::PForUtil;
pub use posting_decoding_util::PostingDecodingUtil;
pub use posting_index_input::PostingIndexInput;
pub use postings_format::{
    Lucene104PostingsFormat, BLOCK_MASK, BLOCK_SIZE, BLOCK_SIZE_LOG2, DOC_CODEC, DOC_EXTENSION,
    LEVEL1_FACTOR, LEVEL1_MASK, LEVEL1_NUM_DOCS, MAX_BLOCK_SIZE, META_CODEC, META_EXTENSION,
    PAY_CODEC, PAY_EXTENSION, POS_CODEC, POS_EXTENSION, SKIP_INTERVAL, TERMS_CODEC,
    VERSION_CURRENT, VERSION_START,
};
pub use postings_reader::Lucene104PostingsReader;
pub use postings_util::{read_v_int_block, write_v_int_block};
pub use postings_writer::{write_impacts, Lucene104PostingsWriter};
