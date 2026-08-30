//! Finite state transducers, ported from `org.apache.lucene.util.fst`.
//!
//! An FST maps a sorted sequence of input labels to an arbitrary output, in a
//! compact byte format that is written once and then only read. It is the
//! structure behind Lucene's terms dictionary, so the serialized layout is
//! reproduced byte for byte from Apache Lucene Core 10.5.0.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`BitTableUtil`] | `BitTableUtil` |
//! | [`ByteBlockPoolReverseBytesReader`] | `ByteBlockPoolReverseBytesReader` |
//! | [`ByteSequenceOutputs`] | `ByteSequenceOutputs` |
//! | [`BytesRefFSTEnum`] | `BytesRefFSTEnum` |
//! | [`CharSequenceOutputs`] | `CharSequenceOutputs` |
//! | [`FST`] | `FST` |
//! | [`FSTCompiler`] | `FSTCompiler` |
//! | [`FSTEnum`] | `FSTEnum` |
//! | [`FSTReader`] | `FSTReader` |
//! | [`ForwardBytesReader`] | `ForwardBytesReader` |
//! | [`GrowableByteArrayDataOutput`] | `GrowableByteArrayDataOutput` |
//! | [`IntSequenceOutputs`] | `IntSequenceOutputs` |
//! | [`IntsRefFSTEnum`] | `IntsRefFSTEnum` |
//! | [`NoOutputs`] | `NoOutputs` |
//! | [`NodeHash`] | `NodeHash` |
//! | [`OffHeapFSTStore`] | `OffHeapFSTStore` |
//! | [`OnHeapFSTStore`] | `OnHeapFSTStore` |
//! | [`Outputs`] | `Outputs` |
//! | [`PairOutputs`] | `PairOutputs` |
//! | [`PositiveIntOutputs`] | `PositiveIntOutputs` |
//! | [`ReadWriteDataOutput`] | `ReadWriteDataOutput` |
//! | [`ReverseBytesReader`] | `ReverseBytesReader` |
//! | [`ReverseRandomAccessReader`] | `ReverseRandomAccessReader` |
//! | [`Util`] | `Util` |
//!
//! # Building and reading an FST
//!
//! Terms must be added in sorted order. The compiler streams the bytes to a
//! [`FstOutputSink`]; the on-heap sink returned by
//! [`get_on_heap_reader_writer`] makes the FST readable straight away.
//!
//! ```text
//! let mut compiler = Builder::new(InputType::Byte1, PositiveIntOutputs::get_singleton()).build()?;
//! compiler.add(&term, output)?;
//! let metadata = compiler.compile()?;
//! let fst = FST::from_fst_reader(metadata, compiler.into_fst_reader()?);
//! ```
//!
//! # Port-wide adaptations
//!
//! These apply across the whole module; the individual items document the rest.
//!
//! * **The type parameter is the outputs implementation.** Lucene writes
//!   `FST<T>` where `T` is the output value type and the `Outputs<T>` instance
//!   is a constructor argument. This port writes `FST<O>` where `O: Outputs`
//!   and the value type is `O::Output`, which removes every trait object from
//!   the hot path. `FST<PositiveIntOutputs>` is this port's spelling of
//!   Lucene's `FST<Long>`.
//! * **Reading methods return `Result<bool>` instead of an arc or `null`.**
//!   Both languages fill the arc in place, so the boolean only reports whether
//!   a matching arc was found. Where Lucene passes the same `Arc` as both the
//!   followed arc and the destination, this port offers a dedicated
//!   `_in_place` method, because Rust cannot borrow one value both shared and
//!   mutably.
//! * **`NO_OUTPUT` is compared by value, not by reference.** Lucene requires
//!   every `Outputs` implementation to return one singleton for "no output" and
//!   then tests `arc.output != NO_OUTPUT` by identity when it chooses the arc
//!   flags. `FSTCompiler.validOutput` asserts the invariant that makes the two
//!   forms interchangeable -- an output either *is* the singleton or is not
//!   equal to it -- so the same flags, and therefore the same bytes, are
//!   written.
//! * **Java assertions are not reproduced when they perform I/O.** Lucene's
//!   `assert` statements are disabled in production and several of them, such
//!   as `FST.Arc.BitTable.assertIsValid`, read from and reposition the byte
//!   reader. Cheap assertions become `debug_assert!`; the ones that would
//!   change reader state are omitted, which is what a production JVM does.
//! * **`FSTReader::get_reverse_bytes_reader` returns a `Result`.** Lucene
//!   declares no checked exception there, so `OffHeapFSTStore` wraps the
//!   `IOException` it cannot avoid in an unchecked `RuntimeException`; this
//!   port reports the failure instead of panicking.

#[allow(clippy::module_inception)]
mod fst;

mod bit_table_util;
mod byte_block_pool_reverse_bytes_reader;
mod byte_sequence_outputs;
mod bytes_ref_fst_enum;
mod char_sequence_outputs;
mod forward_bytes_reader;
mod fst_compiler;
mod fst_enum;
mod fst_reader;
mod growable_byte_array_data_output;
mod int_sequence_outputs;
mod ints_ref_fst_enum;
mod no_outputs;
mod node_hash;
mod off_heap_fst_store;
mod on_heap_fst_store;
mod outputs;
mod pair_outputs;
mod positive_int_outputs;
mod read_write_data_output;
mod reverse_bytes_reader;
mod reverse_random_access_reader;
mod util;

pub use bit_table_util::BitTableUtil;
pub use byte_block_pool_reverse_bytes_reader::ByteBlockPoolReverseBytesReader;
pub use byte_sequence_outputs::ByteSequenceOutputs;
pub use char_sequence_outputs::CharSequenceOutputs;
pub use forward_bytes_reader::ForwardBytesReader;
pub use fst_enum::{FSTEnum, FSTEnumTarget};
pub use fst_reader::FSTReader;
pub use growable_byte_array_data_output::GrowableByteArrayDataOutput;
pub use int_sequence_outputs::IntSequenceOutputs;
pub use no_outputs::NoOutputs;
pub use node_hash::NodeHash;
pub use off_heap_fst_store::OffHeapFSTStore;
pub use on_heap_fst_store::OnHeapFSTStore;
pub use outputs::Outputs;
pub use pair_outputs::{Pair, PairOutputs};
pub use positive_int_outputs::PositiveIntOutputs;
pub use read_write_data_output::ReadWriteDataOutput;
pub use reverse_bytes_reader::ReverseBytesReader;
pub use reverse_random_access_reader::ReverseRandomAccessReader;

pub use fst::{
    flag_is_set, get_num_presence_bytes, target_has_arcs, Arc, BitTable, BytesReader, FSTMetadata,
    InputType, ARCS_FOR_BINARY_SEARCH, ARCS_FOR_CONTINUOUS, ARCS_FOR_DIRECT_ADDRESSING,
    BIT_ARC_HAS_FINAL_OUTPUT, BIT_ARC_HAS_OUTPUT, BIT_FINAL_ARC, BIT_LAST_ARC, BIT_STOP_NODE,
    BIT_TARGET_NEXT, DEFAULT_MAX_BLOCK_BITS, END_LABEL, FILE_FORMAT_NAME, FINAL_END_NODE, FST,
    NON_FINAL_END_NODE, VERSION_90, VERSION_CONTINUOUS_ARCS, VERSION_CURRENT,
    VERSION_LITTLE_ENDIAN, VERSION_START,
};

pub use fst_compiler::{
    get_on_heap_reader_writer, Builder, CompiledNode, CompilerArc, FSTCompiler,
    FixedLengthArcsBuffer, FstOutputSink, NullFSTReader, UnCompiledNode,
    DIRECT_ADDRESSING_MAX_OVERSIZING_FACTOR, FIXED_LENGTH_ARC_DEEP_NUM_ARCS,
    FIXED_LENGTH_ARC_SHALLOW_DEPTH, FIXED_LENGTH_ARC_SHALLOW_NUM_ARCS,
};

pub use util::{
    DefaultTopNSearcherHooks, FSTPath, OutputComparator, PathComparator, PathResult, TopNSearcher,
    TopNSearcherHooks, TopResults, Util,
};

// Lucene nests `InputOutput` inside each enum class; Rust has no nested types,
// so the two are re-exported under distinct names.
pub use bytes_ref_fst_enum::{BytesRefFSTEnum, InputOutput as BytesRefInputOutput};
pub use ints_ref_fst_enum::{InputOutput as IntsRefInputOutput, IntsRefFSTEnum};
