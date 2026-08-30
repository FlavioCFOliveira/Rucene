//! Port of `org.apache.lucene.util.fst.FST`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`FST`] | `FST<T>` |
//! | [`Arc`] | `FST.Arc<T>` |
//! | [`BitTable`] | `FST.Arc.BitTable` |
//! | [`BytesReader`] | `FST.BytesReader` |
//! | [`FSTMetadata`] | `FST.FSTMetadata<T>` |
//! | [`InputType`] | `FST.INPUT_TYPE` |
//!
//! The serialized layout is the one documented by Lucene 10.5.0 and is
//! reproduced byte for byte: a node is either a list of variable-length arcs,
//! or a fixed-length arc array preceded by a node header whose first byte is
//! [`ARCS_FOR_BINARY_SEARCH`], [`ARCS_FOR_DIRECT_ADDRESSING`] or
//! [`ARCS_FOR_CONTINUOUS`]. All node bytes are written in reverse, so a reader
//! walks them backwards; see [`BytesReader`].

use std::fmt;
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;

use crate::codecs::{check_header, write_header};
use crate::error::{LuceneError, Result};
use crate::store::{
    ByteBuffersDataOutput, DataInput, DataOutput, InputStreamDataInput, OutputStreamDataOutput,
};
use crate::util::Accountable;

use super::bit_table_util::BitTableUtil;
use super::fst_compiler::get_on_heap_reader_writer;
use super::fst_reader::FSTReader;
use super::on_heap_fst_store::OnHeapFSTStore;
use super::outputs::Outputs;

/// Set on an arc that accepts the input read so far.
///
/// Equivalent to `FST.BIT_FINAL_ARC`.
pub const BIT_FINAL_ARC: i32 = 1 << 0;

/// Set on the last arc of a node.
///
/// Equivalent to `FST.BIT_LAST_ARC`.
pub const BIT_LAST_ARC: i32 = 1 << 1;

/// Set when the arc's target node immediately follows the arc in the byte
/// stream, so that no target address is written.
///
/// Equivalent to `FST.BIT_TARGET_NEXT`.
pub const BIT_TARGET_NEXT: i32 = 1 << 2;

/// Set when the arc's target node has no outgoing arcs.
///
/// Equivalent to `FST.BIT_STOP_NODE`.
pub const BIT_STOP_NODE: i32 = 1 << 3;

/// Set when the arc carries an output.
///
/// Equivalent to `FST.BIT_ARC_HAS_OUTPUT`.
pub const BIT_ARC_HAS_OUTPUT: i32 = 1 << 4;

/// Set when the arc carries a final output for its target node.
///
/// Equivalent to `FST.BIT_ARC_HAS_FINAL_OUTPUT`.
pub const BIT_ARC_HAS_FINAL_OUTPUT: i32 = 1 << 5;

/// Node header byte declaring fixed-length (sparse) arcs designed for binary
/// search.
///
/// Equivalent to `FST.ARCS_FOR_BINARY_SEARCH`. Lucene reuses
/// [`BIT_ARC_HAS_FINAL_OUTPUT`] as the marker because that flag alone is
/// illegal on a real arc.
pub const ARCS_FOR_BINARY_SEARCH: u8 = BIT_ARC_HAS_FINAL_OUTPUT as u8;

/// Node header byte declaring fixed-length dense arcs plus a bit table designed
/// for direct addressing.
///
/// Equivalent to `FST.ARCS_FOR_DIRECT_ADDRESSING`.
pub const ARCS_FOR_DIRECT_ADDRESSING: u8 = 1 << 6;

/// Node header byte declaring continuous arcs, addressed directly by
/// `label - firstLabel` with no bit table.
///
/// Equivalent to `FST.ARCS_FOR_CONTINUOUS`.
pub const ARCS_FOR_CONTINUOUS: u8 = ARCS_FOR_DIRECT_ADDRESSING + ARCS_FOR_BINARY_SEARCH;

/// Codec name written in the FST metadata header.
///
/// Equivalent to `FST.FILE_FORMAT_NAME`.
pub const FILE_FORMAT_NAME: &str = "FST";

/// First supported version; the version released with Lucene 7.0.
///
/// Equivalent to `FST.VERSION_START`.
pub const VERSION_START: i32 = 6;

/// Version that switched the on-disk integers to little endian.
///
/// Equivalent to `FST.VERSION_LITTLE_ENDIAN`.
pub const VERSION_LITTLE_ENDIAN: i32 = 8;

/// Version that started storing continuous arcs.
///
/// Equivalent to `FST.VERSION_CONTINUOUS_ARCS`.
pub const VERSION_CONTINUOUS_ARCS: i32 = 9;

/// Current version written by this port.
///
/// Equivalent to `FST.VERSION_CURRENT`.
pub const VERSION_CURRENT: i32 = VERSION_CONTINUOUS_ARCS;

/// Version that was used when releasing Lucene 9.0.
///
/// Equivalent to `FST.VERSION_90`.
pub const VERSION_90: i32 = VERSION_LITTLE_ENDIAN;

/// Sentinel target of the virtual final node with no arcs. Never serialized.
///
/// Equivalent to `FST.FINAL_END_NODE`.
pub const FINAL_END_NODE: i64 = -1;

/// Sentinel target of the virtual non-final node with no arcs. Never
/// serialized.
///
/// Equivalent to `FST.NON_FINAL_END_NODE`.
pub const NON_FINAL_END_NODE: i64 = 0;

/// Label of the virtual arc that marks acceptance.
///
/// Equivalent to `FST.END_LABEL`.
pub const END_LABEL: i32 = -1;

/// Default block size, in bits, of the on-heap store used when loading an FST.
///
/// Equivalent to `FST.DEFAULT_MAX_BLOCK_BITS`, which is 30 on a 64-bit JVM and
/// 28 otherwise.
pub const DEFAULT_MAX_BLOCK_BITS: i32 = if cfg!(target_pointer_width = "64") {
    30
} else {
    28
};

/// Allowed range of each input label of an FST.
///
/// Equivalent to `FST.INPUT_TYPE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputType {
    /// One unsigned byte per label.
    Byte1,
    /// One unsigned little-endian short per label.
    Byte2,
    /// One variable-length int per label.
    Byte4,
}

/// Returns whether `bit` is set in `flags`.
///
/// Equivalent to the private static `FST.flag`.
pub fn flag_is_set(flags: i32, bit: i32) -> bool {
    (flags & bit) != 0
}

/// Number of bytes required to flag the presence of each arc in the given label
/// range, one bit per arc.
///
/// Equivalent to `FST.getNumPresenceBytes`.
pub fn get_num_presence_bytes(label_range: i32) -> i32 {
    debug_assert!(label_range >= 0);
    (label_range + 7) >> 3
}

/// Returns true if the node this arc points at has any outgoing arc.
///
/// Equivalent to the static `FST.targetHasArcs`.
pub fn target_has_arcs<T>(arc: &Arc<T>) -> bool {
    arc.target() > 0
}

/// Reads the bytes of an FST.
///
/// Equivalent to the abstract class `FST.BytesReader`, which extends
/// `DataInput` with an absolute position. Implementations must accept negative
/// counts in [`DataInput::skip_bytes`]: the FST walks its byte stream both ways.
///
/// # Java to Rust adaptations
///
/// * [`BytesReader::as_data_input`] performs, by hand, the `dyn BytesReader`
///   to `dyn DataInput` upcast that Java gets from subclassing. Rust only
///   supports trait-object upcasting from 1.86 and this crate's MSRV is 1.80,
///   so the upcast is spelled out as a trait method that every implementation
///   satisfies with `self`.
pub trait BytesReader: DataInput {
    /// Returns the current read position.
    ///
    /// Equivalent to `FST.BytesReader.getPosition`.
    fn position(&self) -> i64;

    /// Sets the current read position.
    ///
    /// Equivalent to `FST.BytesReader.setPosition`.
    fn set_position(&mut self, pos: i64);

    /// Returns this reader as a [`DataInput`].
    ///
    /// See the trait documentation: this replaces Java's implicit upcast from
    /// `FST.BytesReader` to `DataInput`.
    fn as_data_input(&mut self) -> &mut dyn DataInput;
}

/// Represents a single arc of an FST.
///
/// Equivalent to `FST.Arc<T>`. An `Arc` is a cursor: reading methods on
/// [`FST`] fill it in place, exactly as in Lucene.
#[derive(Debug, Clone, Default)]
pub struct Arc<T> {
    pub(crate) label: i32,
    pub(crate) output: T,
    pub(crate) target: i64,
    pub(crate) flags: u8,
    pub(crate) next_final_output: T,
    pub(crate) next_arc: i64,
    pub(crate) node_flags: u8,

    // Fields for arcs belonging to a node with fixed length arcs; only valid
    // when bytes_per_arc != 0.
    pub(crate) bytes_per_arc: i32,
    pub(crate) pos_arcs_start: i64,
    pub(crate) arc_idx: i32,
    pub(crate) num_arcs: i32,

    // Fields for a direct addressing node.
    pub(crate) bit_table_start: i64,
    pub(crate) first_label: i32,
    pub(crate) presence_index: i32,
}

impl<T: Clone> Arc<T> {
    /// Copies every field of `other` into this arc.
    ///
    /// Equivalent to `FST.Arc.copyFrom`.
    pub fn copy_from(&mut self, other: &Arc<T>) -> &mut Self {
        self.label = other.label;
        self.target = other.target;
        self.flags = other.flags;
        self.output = other.output.clone();
        self.next_final_output = other.next_final_output.clone();
        self.next_arc = other.next_arc;
        self.node_flags = other.node_flags;
        self.bytes_per_arc = other.bytes_per_arc;

        // Lucene copies these unconditionally too, even when bytes_per_arc == 0,
        // so that an arc always has a consistent state.
        self.pos_arcs_start = other.pos_arcs_start;
        self.arc_idx = other.arc_idx;
        self.num_arcs = other.num_arcs;
        self.bit_table_start = other.bit_table_start;
        self.first_label = other.first_label;
        self.presence_index = other.presence_index;

        self
    }
}

impl<T> Arc<T> {
    /// Returns whether `flag` is set on this arc.
    ///
    /// Equivalent to the package-private `FST.Arc.flag`.
    pub fn flag(&self, flag: i32) -> bool {
        flag_is_set(self.flags as i32, flag)
    }

    /// Returns whether this is the last arc of its node.
    ///
    /// Equivalent to `FST.Arc.isLast`.
    pub fn is_last(&self) -> bool {
        self.flag(BIT_LAST_ARC)
    }

    /// Returns whether this arc accepts the input read so far.
    ///
    /// Equivalent to `FST.Arc.isFinal`.
    pub fn is_final(&self) -> bool {
        self.flag(BIT_FINAL_ARC)
    }

    /// Returns the arc label.
    ///
    /// Equivalent to `FST.Arc.label`.
    pub fn label(&self) -> i32 {
        self.label
    }

    /// Returns the arc output.
    ///
    /// Equivalent to `FST.Arc.output`.
    pub fn output(&self) -> &T {
        &self.output
    }

    /// Returns the ord/address of the target node.
    ///
    /// Equivalent to `FST.Arc.target`.
    pub fn target(&self) -> i64 {
        self.target
    }

    /// Returns the raw arc flags.
    ///
    /// Equivalent to `FST.Arc.flags`.
    pub fn flags(&self) -> u8 {
        self.flags
    }

    /// Returns the final output of the target node.
    ///
    /// Equivalent to `FST.Arc.nextFinalOutput`.
    pub fn next_final_output(&self) -> &T {
        &self.next_final_output
    }

    /// Returns the address of the next arc of a variable-length arc list, or
    /// the ord/address of the next node when `label == END_LABEL`.
    ///
    /// Equivalent to the package-private `FST.Arc.nextArc`.
    pub fn next_arc(&self) -> i64 {
        self.next_arc
    }

    /// Returns the index of this arc inside a fixed-length arc array; only
    /// valid when [`Arc::bytes_per_arc`] is non-zero.
    ///
    /// Equivalent to `FST.Arc.arcIdx`.
    pub fn arc_idx(&self) -> i32 {
        self.arc_idx
    }

    /// Returns the node header flags.
    ///
    /// Equivalent to `FST.Arc.nodeFlags`. Only meaningful when it equals
    /// [`ARCS_FOR_BINARY_SEARCH`], [`ARCS_FOR_DIRECT_ADDRESSING`] or
    /// [`ARCS_FOR_CONTINUOUS`].
    pub fn node_flags(&self) -> u8 {
        self.node_flags
    }

    /// Returns where the first arc of a fixed-length arc array starts; only
    /// valid when [`Arc::bytes_per_arc`] is non-zero.
    ///
    /// Equivalent to `FST.Arc.posArcsStart`.
    pub fn pos_arcs_start(&self) -> i64 {
        self.pos_arcs_start
    }

    /// Returns the fixed number of bytes each arc of this node occupies, or `0`
    /// when the node uses variable-length arcs.
    ///
    /// Equivalent to `FST.Arc.bytesPerArc`.
    pub fn bytes_per_arc(&self) -> i32 {
        self.bytes_per_arc
    }

    /// Returns the number of arcs of a binary-search node, or the label range
    /// of a direct-addressing or continuous node.
    ///
    /// Equivalent to `FST.Arc.numArcs`.
    pub fn num_arcs(&self) -> i32 {
        self.num_arcs
    }

    /// Returns the first label of a direct-addressing or continuous node.
    ///
    /// Equivalent to the package-private `FST.Arc.firstLabel`.
    pub fn first_label(&self) -> i32 {
        self.first_label
    }

    /// Returns the index of the current label among the labels actually present
    /// in a direct-addressing node.
    ///
    /// Equivalent to the private field `FST.Arc.presenceIndex`.
    pub fn presence_index(&self) -> i32 {
        self.presence_index
    }

    /// Returns the start position of the bit table of a direct-addressing node.
    ///
    /// Equivalent to the private field `FST.Arc.bitTableStart`.
    pub fn bit_table_start(&self) -> i64 {
        self.bit_table_start
    }
}

impl<T: fmt::Debug> fmt::Display for Arc<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, " target={}", self.target())?;
        write!(f, " label=0x{:x}", self.label())?;
        if self.flag(BIT_FINAL_ARC) {
            write!(f, " final")?;
        }
        if self.flag(BIT_LAST_ARC) {
            write!(f, " last")?;
        }
        if self.flag(BIT_TARGET_NEXT) {
            write!(f, " targetNext")?;
        }
        if self.flag(BIT_STOP_NODE) {
            write!(f, " stop")?;
        }
        if self.flag(BIT_ARC_HAS_OUTPUT) {
            write!(f, " output={:?}", self.output())?;
        }
        if self.flag(BIT_ARC_HAS_FINAL_OUTPUT) {
            write!(f, " nextFinalOutput={:?}", self.next_final_output())?;
        }
        if self.bytes_per_arc() != 0 {
            let kind = if self.node_flags() == ARCS_FOR_DIRECT_ADDRESSING {
                "da"
            } else if self.node_flags() == ARCS_FOR_CONTINUOUS {
                "cs"
            } else {
                "bs"
            };
            write!(
                f,
                " arcArray(idx={} of {})({})",
                self.arc_idx(),
                self.num_arcs(),
                kind
            )?;
        }
        Ok(())
    }
}

/// Helper methods to read the bit table of a direct-addressing node.
///
/// Equivalent to the static nested class `FST.Arc.BitTable`. Every method
/// repositions the reader at the arc's bit-table start before delegating to
/// [`BitTableUtil`].
pub struct BitTable;

impl BitTable {
    /// Returns whether the bit at `bit_index` is set.
    ///
    /// Equivalent to `FST.Arc.BitTable.isBitSet`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn is_bit_set<T>(
        bit_index: i32,
        arc: &Arc<T>,
        input: &mut dyn BytesReader,
    ) -> Result<bool> {
        debug_assert_eq!(arc.node_flags(), ARCS_FOR_DIRECT_ADDRESSING);
        input.set_position(arc.bit_table_start);
        BitTableUtil::is_bit_set(bit_index, input)
    }

    /// Counts all bits set in the bit table, that is, the number of arcs of a
    /// direct-addressing node.
    ///
    /// Equivalent to `FST.Arc.BitTable.countBits`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn count_bits<T>(arc: &Arc<T>, input: &mut dyn BytesReader) -> Result<i32> {
        debug_assert_eq!(arc.node_flags(), ARCS_FOR_DIRECT_ADDRESSING);
        input.set_position(arc.bit_table_start);
        BitTableUtil::count_bits(get_num_presence_bytes(arc.num_arcs()), input)
    }

    /// Counts the bits set up to `bit_index`, exclusive.
    ///
    /// Equivalent to `FST.Arc.BitTable.countBitsUpTo`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn count_bits_up_to<T>(
        bit_index: i32,
        arc: &Arc<T>,
        input: &mut dyn BytesReader,
    ) -> Result<i32> {
        debug_assert_eq!(arc.node_flags(), ARCS_FOR_DIRECT_ADDRESSING);
        input.set_position(arc.bit_table_start);
        BitTableUtil::count_bits_up_to(bit_index, input)
    }

    /// Returns the index of the next bit set after `bit_index`, or `-1`.
    ///
    /// Equivalent to `FST.Arc.BitTable.nextBitSet`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn next_bit_set<T>(
        bit_index: i32,
        arc: &Arc<T>,
        input: &mut dyn BytesReader,
    ) -> Result<i32> {
        debug_assert_eq!(arc.node_flags(), ARCS_FOR_DIRECT_ADDRESSING);
        input.set_position(arc.bit_table_start);
        BitTableUtil::next_bit_set(bit_index, get_num_presence_bytes(arc.num_arcs()), input)
    }

    /// Returns the index of the previous bit set before `bit_index`, or `-1`.
    ///
    /// Equivalent to `FST.Arc.BitTable.previousBitSet`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn previous_bit_set<T>(
        bit_index: i32,
        arc: &Arc<T>,
        input: &mut dyn BytesReader,
    ) -> Result<i32> {
        debug_assert_eq!(arc.node_flags(), ARCS_FOR_DIRECT_ADDRESSING);
        input.set_position(arc.bit_table_start);
        BitTableUtil::previous_bit_set(bit_index, input)
    }
}

/// Metadata of a serialized FST.
///
/// Equivalent to `FST.FSTMetadata<T>`.
#[derive(Debug, Clone)]
pub struct FSTMetadata<O: Outputs> {
    pub(crate) input_type: InputType,
    pub(crate) outputs: O,
    pub(crate) version: i32,
    /// If present, this FST accepts the empty string and produces this output.
    pub(crate) empty_output: Option<O::Output>,
    pub(crate) start_node: i64,
    pub(crate) num_bytes: i64,
}

impl<O: Outputs> FSTMetadata<O> {
    /// Creates the metadata of an FST.
    ///
    /// Equivalent to the `FST.FSTMetadata` constructor.
    pub fn new(
        input_type: InputType,
        outputs: O,
        empty_output: Option<O::Output>,
        start_node: i64,
        version: i32,
        num_bytes: i64,
    ) -> Self {
        Self {
            input_type,
            outputs,
            version,
            empty_output,
            start_node,
            num_bytes,
        }
    }

    /// Returns the version constant of the binary format this FST was written
    /// in.
    ///
    /// Equivalent to `FST.FSTMetadata.getVersion`.
    pub fn version(&self) -> i32 {
        self.version
    }

    /// Returns the output produced for the empty string, if this FST accepts
    /// it.
    ///
    /// Equivalent to `FST.FSTMetadata.getEmptyOutput`.
    pub fn empty_output(&self) -> Option<&O::Output> {
        self.empty_output.as_ref()
    }

    /// Returns the number of FST bytes.
    ///
    /// Equivalent to `FST.FSTMetadata.getNumBytes`.
    pub fn num_bytes(&self) -> i64 {
        self.num_bytes
    }

    /// Returns the input type of this FST.
    ///
    /// Equivalent to the package-private field `FST.FSTMetadata.inputType`.
    pub fn input_type(&self) -> InputType {
        self.input_type
    }

    /// Returns the address of the start node.
    ///
    /// Equivalent to the package-private field `FST.FSTMetadata.startNode`.
    pub fn start_node(&self) -> i64 {
        self.start_node
    }

    /// Returns the outputs of this FST.
    ///
    /// Equivalent to the package-private field `FST.FSTMetadata.outputs`.
    pub fn outputs(&self) -> &O {
        &self.outputs
    }

    /// Writes the metadata to `meta_out`.
    ///
    /// Equivalent to `FST.FSTMetadata.save`. The empty-string output is written
    /// with its bytes reversed, because the FST byte stream is read backwards.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the underlying output.
    pub fn save(&self, meta_out: &mut dyn DataOutput) -> Result<()> {
        write_header(meta_out, FILE_FORMAT_NAME, VERSION_CURRENT)?;
        // TODO(Lucene): really this should be encoded as an arc arriving at the
        // root node instead of being special-cased here.
        if let Some(empty_output) = &self.empty_output {
            // Accepts the empty string.
            meta_out.write_byte(1)?;

            // Serialize the empty-string output.
            let mut ros = ByteBuffersDataOutput::new();
            self.outputs.write_final_output(empty_output, &mut ros)?;
            let mut empty_output_bytes = ros.to_array_copy();
            let empty_len = empty_output_bytes.len();

            empty_output_bytes.reverse();
            meta_out.write_v_int(i32::try_from(empty_len).map_err(|_| {
                LuceneError::IllegalState(format!("empty output too long: {empty_len}"))
            })?)?;
            meta_out.write_bytes(&empty_output_bytes, 0, empty_len)?;
        } else {
            meta_out.write_byte(0)?;
        }
        let t: u8 = match self.input_type {
            InputType::Byte1 => 0,
            InputType::Byte2 => 1,
            InputType::Byte4 => 2,
        };
        meta_out.write_byte(t)?;
        meta_out.write_v_long(self.start_node)?;
        meta_out.write_v_long(self.num_bytes)?;
        Ok(())
    }
}

/// A finite state transducer, using a compact byte format.
///
/// Equivalent to `org.apache.lucene.util.fst.FST<T>`. The format is similar to
/// the one used by [Morfologik](https://github.com/morfologik/morfologik-stemming).
///
/// # Java to Rust adaptations
///
/// * The type parameter is the [`Outputs`] implementation rather than the
///   output value type; see [`Outputs`] for the reasoning.
/// * The arc-reading methods return `Result<bool>` where Lucene returns the arc
///   or `null`: the arc is filled in place in both languages, so the boolean
///   only reports whether a matching arc was found.
/// * Methods that Lucene calls with the same `Arc` for both the followed arc
///   and the arc to fill have a dedicated `_in_place` variant, because Rust
///   cannot borrow one value both shared and mutably.
pub struct FST<O: Outputs> {
    pub(crate) metadata: FSTMetadata<O>,
    fst_reader: Box<dyn FSTReader>,
}

impl<O: Outputs> FST<O> {
    /// Loads a previously saved FST, reading its bytes into an
    /// [`OnHeapFSTStore`] with [`DEFAULT_MAX_BLOCK_BITS`].
    ///
    /// Equivalent to `FST(FSTMetadata<T>, DataInput)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading the FST bytes.
    pub fn new(metadata: FSTMetadata<O>, input: &mut dyn DataInput) -> Result<Self> {
        let num_bytes = metadata.num_bytes;
        let store = OnHeapFSTStore::new(DEFAULT_MAX_BLOCK_BITS, input, num_bytes)?;
        Ok(Self::from_reader(metadata, Box::new(store)))
    }

    /// Creates the FST from a metadata object and an [`FSTReader`].
    ///
    /// Equivalent to the package-private `FST(FSTMetadata<T>, FSTReader)`.
    pub fn from_reader(metadata: FSTMetadata<O>, fst_reader: Box<dyn FSTReader>) -> Self {
        Self {
            metadata,
            fst_reader,
        }
    }

    /// Creates an FST from an [`FSTReader`], or `None` when the metadata is
    /// absent because nothing is accepted by the FST.
    ///
    /// Equivalent to the static `FST.fromFSTReader`.
    pub fn from_fst_reader(
        fst_metadata: Option<FSTMetadata<O>>,
        fst_reader: Box<dyn FSTReader>,
    ) -> Option<Self> {
        fst_metadata.map(|metadata| Self::from_reader(metadata, fst_reader))
    }

    /// Reads the FST metadata from `meta_in`.
    ///
    /// Equivalent to the static `FST.readMetadata`. Only the formats from
    /// [`VERSION_START`] up to [`VERSION_CURRENT`] are read; FSTs are
    /// experimental and carry no back-compatibility promise.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::CorruptIndex`] when the input type byte is not
    /// one of `0`, `1` or `2`, and propagates header and I/O errors.
    pub fn read_metadata(meta_in: &mut dyn DataInput, outputs: O) -> Result<FSTMetadata<O>> {
        let version = check_header(meta_in, FILE_FORMAT_NAME, VERSION_START, VERSION_CURRENT)?;
        let empty_output = if meta_in.read_byte()? == 1 {
            // Accepts the empty string; 1 KB blocks.
            let mut empty_bytes = get_on_heap_reader_writer(10)?;
            let num_bytes = meta_in.read_v_int()?;
            if num_bytes < 0 {
                return Err(LuceneError::CorruptIndex(format!(
                    "invalid empty output length {num_bytes}"
                )));
            }
            empty_bytes.copy_bytes(meta_in, num_bytes as i64)?;
            empty_bytes.freeze();

            // De-serialize the empty-string output. NoOutputs writes zero bytes,
            // so the position is only set when something was written.
            let mut reader = empty_bytes.get_reverse_bytes_reader()?;
            if num_bytes > 0 {
                reader.set_position(num_bytes as i64 - 1);
            }
            Some(outputs.read_final_output(reader.as_data_input())?)
        } else {
            None
        };
        let t = meta_in.read_byte()?;
        let input_type = match t {
            0 => InputType::Byte1,
            1 => InputType::Byte2,
            2 => InputType::Byte4,
            _ => {
                return Err(LuceneError::CorruptIndex(format!("invalid input type {t}")));
            }
        };
        let start_node = meta_in.read_v_long()?;
        let num_bytes = meta_in.read_v_long()?;
        Ok(FSTMetadata::new(
            input_type,
            outputs,
            empty_output,
            start_node,
            version,
            num_bytes,
        ))
    }

    /// Returns the outputs of this FST.
    ///
    /// Equivalent to the public field `FST.outputs`.
    pub fn outputs(&self) -> &O {
        &self.metadata.outputs
    }

    /// Returns the number of FST bytes.
    ///
    /// Equivalent to `FST.numBytes`.
    pub fn num_bytes(&self) -> i64 {
        self.metadata.num_bytes
    }

    /// Returns the output produced for the empty string, if any.
    ///
    /// Equivalent to `FST.getEmptyOutput`.
    pub fn empty_output(&self) -> Option<&O::Output> {
        self.metadata.empty_output.as_ref()
    }

    /// Returns the metadata of this FST.
    ///
    /// Equivalent to `FST.getMetadata`.
    pub fn metadata(&self) -> &FSTMetadata<O> {
        &self.metadata
    }

    /// Saves the FST, writing its metadata to `meta_out` and its bytes to
    /// `out`.
    ///
    /// Equivalent to `FST.save(DataOutput, DataOutput)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the underlying outputs, including the
    /// [`LuceneError::UnsupportedOperation`] raised by stores that cannot write
    /// themselves back, such as `OffHeapFSTStore`.
    pub fn save(&self, meta_out: &mut dyn DataOutput, out: &mut dyn DataOutput) -> Result<()> {
        self.metadata.save(meta_out)?;
        self.fst_reader.write_to(out)
    }

    /// Writes this automaton to a file.
    ///
    /// Equivalent to `FST.save(Path)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors and the errors of [`FST::save`].
    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        let file = std::fs::File::create(path)?;
        let mut out = OutputStreamDataOutput::new(BufWriter::new(file));
        self.metadata.save(&mut out)?;
        self.fst_reader.write_to(&mut out)?;
        out.into_inner().flush()?;
        Ok(())
    }

    /// Reads an automaton from a file.
    ///
    /// Equivalent to the static `FST.read(Path, Outputs)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors and the errors of [`FST::read_metadata`].
    pub fn read_from_path(path: &Path, outputs: O) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        let mut input = InputStreamDataInput::new(BufReader::new(file));
        let metadata = Self::read_metadata(&mut input, outputs)?;
        Self::new(metadata, &mut input)
    }

    /// Reads one `BYTE1`/`BYTE2`/`BYTE4` label from `input`.
    ///
    /// Equivalent to `FST.readLabel`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn read_label(&self, input: &mut dyn DataInput) -> Result<i32> {
        let v = match self.metadata.input_type {
            // Unsigned byte.
            InputType::Byte1 => i32::from(input.read_byte()?),
            // Unsigned short. Before VERSION_LITTLE_ENDIAN labels were stored
            // big endian, so the bytes have to be swapped back.
            InputType::Byte2 => {
                let raw = input.read_short()?;
                let value = if self.metadata.version < VERSION_LITTLE_ENDIAN {
                    raw.swap_bytes()
                } else {
                    raw
                };
                i32::from(value as u16)
            }
            InputType::Byte4 => input.read_v_int()?,
        };
        Ok(v)
    }

    /// Returns a [`BytesReader`] for this FST, positioned at position `0`.
    ///
    /// Equivalent to `FST.getBytesReader`.
    ///
    /// # Errors
    ///
    /// Propagates the error raised while opening the underlying store; Lucene
    /// wraps the same failure in an unchecked `RuntimeException`.
    pub fn get_bytes_reader(&self) -> Result<Box<dyn BytesReader + '_>> {
        self.fst_reader.get_reverse_bytes_reader()
    }

    /// Reads the presence bits of a direct-addressing node.
    ///
    /// Equivalent to the private `FST.readPresenceBytes`: the bits themselves
    /// are not read, only their start position is kept and they are skipped.
    fn read_presence_bytes(
        &self,
        arc: &mut Arc<O::Output>,
        input: &mut dyn BytesReader,
    ) -> Result<()> {
        debug_assert!(arc.bytes_per_arc > 0);
        debug_assert_eq!(arc.node_flags, ARCS_FOR_DIRECT_ADDRESSING);
        arc.bit_table_start = input.position();
        input.skip_bytes(i64::from(get_num_presence_bytes(arc.num_arcs)))
    }

    /// Fills the virtual "start" arc, that is, an empty incoming arc to the
    /// FST's start node.
    ///
    /// Equivalent to `FST.getFirstArc`.
    pub fn get_first_arc(&self, arc: &mut Arc<O::Output>) {
        let no_output = self.metadata.outputs.no_output();

        if let Some(empty_output) = &self.metadata.empty_output {
            arc.flags = (BIT_FINAL_ARC | BIT_LAST_ARC) as u8;
            arc.next_final_output = empty_output.clone();
            if !self.metadata.outputs.equals(empty_output, &no_output) {
                arc.flags |= BIT_ARC_HAS_FINAL_OUTPUT as u8;
            }
        } else {
            arc.flags = BIT_LAST_ARC as u8;
            arc.next_final_output = no_output.clone();
        }
        arc.output = no_output;

        // If there are no nodes, ie the FST only accepts the empty string, then
        // start_node is 0.
        arc.target = self.metadata.start_node;
    }

    /// Follows `follow` and reads the last arc of its target into `arc`.
    ///
    /// Equivalent to the package-private `FST.readLastTargetArc`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn read_last_target_arc(
        &self,
        follow: &Arc<O::Output>,
        arc: &mut Arc<O::Output>,
        input: &mut dyn BytesReader,
    ) -> Result<()> {
        if !target_has_arcs(follow) {
            debug_assert!(follow.is_final());
            arc.label = END_LABEL;
            arc.target = FINAL_END_NODE;
            arc.output = follow.next_final_output.clone();
            arc.flags = BIT_LAST_ARC as u8;
            arc.node_flags = arc.flags;
            return Ok(());
        }

        input.set_position(follow.target);
        let flags = input.read_byte()?;
        arc.node_flags = flags;
        if flags == ARCS_FOR_BINARY_SEARCH
            || flags == ARCS_FOR_DIRECT_ADDRESSING
            || flags == ARCS_FOR_CONTINUOUS
        {
            // Special arc which is actually a node header for fixed length
            // arcs. Jump straight to the end to find the last arc.
            arc.num_arcs = input.read_v_int()?;
            arc.bytes_per_arc = input.read_v_int()?;
            if flags == ARCS_FOR_DIRECT_ADDRESSING {
                self.read_presence_bytes(arc, input)?;
                arc.first_label = self.read_label(input.as_data_input())?;
                arc.pos_arcs_start = input.position();
                self.read_last_arc_by_direct_addressing(arc, input)?;
            } else if flags == ARCS_FOR_BINARY_SEARCH {
                arc.arc_idx = arc.num_arcs - 2;
                arc.pos_arcs_start = input.position();
                self.read_next_real_arc(arc, input)?;
            } else {
                arc.first_label = self.read_label(input.as_data_input())?;
                arc.pos_arcs_start = input.position();
                self.read_last_arc_by_continuous(arc, input)?;
            }
        } else {
            arc.flags = flags;
            // Non-array: linear scan.
            arc.bytes_per_arc = 0;
            while !arc.is_last() {
                // Skip this arc.
                self.read_label(input.as_data_input())?;
                if arc.flag(BIT_ARC_HAS_OUTPUT) {
                    self.metadata.outputs.skip_output(input.as_data_input())?;
                }
                if arc.flag(BIT_ARC_HAS_FINAL_OUTPUT) {
                    self.metadata
                        .outputs
                        .skip_final_output(input.as_data_input())?;
                }
                if !arc.flag(BIT_STOP_NODE) && !arc.flag(BIT_TARGET_NEXT) {
                    read_unpacked_node_target(input)?;
                }
                arc.flags = input.read_byte()?;
            }
            // Undo the byte flags we read.
            input.skip_bytes(-1)?;
            arc.next_arc = input.position();
            self.read_next_real_arc(arc, input)?;
        }
        debug_assert!(arc.is_last());
        Ok(())
    }

    /// Follows `follow` and reads the first arc of its target into `arc`.
    ///
    /// Equivalent to `FST.readFirstTargetArc`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn read_first_target_arc(
        &self,
        follow: &Arc<O::Output>,
        arc: &mut Arc<O::Output>,
        input: &mut dyn BytesReader,
    ) -> Result<()> {
        self.read_first_target_arc_impl(
            follow.is_final(),
            follow.target,
            follow.next_final_output.clone(),
            arc,
            input,
        )
    }

    /// In-place variant of [`FST::read_first_target_arc`], for the call sites
    /// where Lucene passes the same `Arc` as both the followed arc and the
    /// destination.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn read_first_target_arc_in_place(
        &self,
        arc: &mut Arc<O::Output>,
        input: &mut dyn BytesReader,
    ) -> Result<()> {
        let is_final = arc.is_final();
        let target = arc.target;
        let next_final_output = arc.next_final_output.clone();
        self.read_first_target_arc_impl(is_final, target, next_final_output, arc, input)
    }

    fn read_first_target_arc_impl(
        &self,
        follow_is_final: bool,
        follow_target: i64,
        follow_next_final_output: O::Output,
        arc: &mut Arc<O::Output>,
        input: &mut dyn BytesReader,
    ) -> Result<()> {
        if follow_is_final {
            // Insert a "fake" final first arc.
            arc.label = END_LABEL;
            arc.output = follow_next_final_output;
            arc.flags = BIT_FINAL_ARC as u8;
            if follow_target <= 0 {
                arc.flags |= BIT_LAST_ARC as u8;
            } else {
                // NOTE: next_arc is a node (not an address) in this case.
                arc.next_arc = follow_target;
            }
            arc.target = FINAL_END_NODE;
            arc.node_flags = arc.flags;
            Ok(())
        } else {
            self.read_first_real_target_arc(follow_target, arc, input)
        }
    }

    /// Reads the node header at `node_address` into `arc`.
    ///
    /// Equivalent to the private `FST.readFirstArcInfo`.
    fn read_first_arc_info(
        &self,
        node_address: i64,
        arc: &mut Arc<O::Output>,
        input: &mut dyn BytesReader,
    ) -> Result<()> {
        input.set_position(node_address);

        let flags = input.read_byte()?;
        arc.node_flags = flags;
        if flags == ARCS_FOR_BINARY_SEARCH
            || flags == ARCS_FOR_DIRECT_ADDRESSING
            || flags == ARCS_FOR_CONTINUOUS
        {
            // Special arc which is actually a node header for fixed length arcs.
            arc.num_arcs = input.read_v_int()?;
            arc.bytes_per_arc = input.read_v_int()?;
            arc.arc_idx = -1;
            if flags == ARCS_FOR_DIRECT_ADDRESSING {
                self.read_presence_bytes(arc, input)?;
                arc.first_label = self.read_label(input.as_data_input())?;
                arc.presence_index = -1;
            } else if flags == ARCS_FOR_CONTINUOUS {
                arc.first_label = self.read_label(input.as_data_input())?;
            }
            arc.pos_arcs_start = input.position();
        } else {
            arc.next_arc = node_address;
            arc.bytes_per_arc = 0;
        }
        Ok(())
    }

    /// Reads the first real arc of the node at `node_address`.
    ///
    /// Equivalent to `FST.readFirstRealTargetArc`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn read_first_real_target_arc(
        &self,
        node_address: i64,
        arc: &mut Arc<O::Output>,
        input: &mut dyn BytesReader,
    ) -> Result<()> {
        self.read_first_arc_info(node_address, arc, input)?;
        self.read_next_real_arc(arc, input)
    }

    /// Returns whether the target of `follow` is stored in expanded format,
    /// that is, with fixed-length arcs.
    ///
    /// Equivalent to the package-private `FST.isExpandedTarget`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn is_expanded_target(
        &self,
        follow: &Arc<O::Output>,
        input: &mut dyn BytesReader,
    ) -> Result<bool> {
        if !target_has_arcs(follow) {
            Ok(false)
        } else {
            input.set_position(follow.target);
            let flags = input.read_byte()?;
            Ok(flags == ARCS_FOR_BINARY_SEARCH
                || flags == ARCS_FOR_DIRECT_ADDRESSING
                || flags == ARCS_FOR_CONTINUOUS)
        }
    }

    /// Reads the next arc into `arc`, in place.
    ///
    /// Equivalent to `FST.readNextArc`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when called on the last arc,
    /// and propagates reader errors.
    pub fn read_next_arc(
        &self,
        arc: &mut Arc<O::Output>,
        input: &mut dyn BytesReader,
    ) -> Result<()> {
        if arc.label == END_LABEL {
            // This was a fake inserted "final" arc.
            if arc.next_arc <= 0 {
                return Err(LuceneError::IllegalArgument(
                    "cannot readNextArc when arc.isLast()=true".to_string(),
                ));
            }
            let next = arc.next_arc;
            self.read_first_real_target_arc(next, arc, input)
        } else {
            self.read_next_real_arc(arc, input)
        }
    }

    /// Peeks at the next arc's label without altering `arc`.
    ///
    /// Equivalent to the package-private `FST.readNextArcLabel`. Must not be
    /// called when `arc.is_last()`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn read_next_arc_label(
        &self,
        arc: &Arc<O::Output>,
        input: &mut dyn BytesReader,
    ) -> Result<i32> {
        debug_assert!(!arc.is_last());

        if arc.label == END_LABEL {
            // The next arc is the first arc of a node; position to read the
            // first arc label.
            input.set_position(arc.next_arc);
            let flags = input.read_byte()?;
            if flags == ARCS_FOR_BINARY_SEARCH
                || flags == ARCS_FOR_DIRECT_ADDRESSING
                || flags == ARCS_FOR_CONTINUOUS
            {
                // Special arc which is actually a node header for fixed length
                // arcs.
                let num_arcs = input.read_v_int()?;
                input.read_v_int()?; // Skip bytesPerArc.
                if flags == ARCS_FOR_BINARY_SEARCH {
                    input.read_byte()?; // Skip arc flags.
                } else if flags == ARCS_FOR_DIRECT_ADDRESSING {
                    input.skip_bytes(i64::from(get_num_presence_bytes(num_arcs)))?;
                }
                // Nothing to do for ARCS_FOR_CONTINUOUS.
            }
        } else {
            match arc.node_flags {
                ARCS_FOR_BINARY_SEARCH => {
                    // Point to the next arc, -1 to skip arc flags.
                    input.set_position(
                        arc.pos_arcs_start
                            - i64::from(1 + arc.arc_idx) * i64::from(arc.bytes_per_arc)
                            - 1,
                    );
                }
                ARCS_FOR_DIRECT_ADDRESSING => {
                    // Direct addressing node: the label is not stored but
                    // inferred from the first label and the arc index.
                    let next_index = BitTable::next_bit_set(arc.arc_idx, arc, input)?;
                    debug_assert_ne!(next_index, -1);
                    return Ok(arc.first_label + next_index);
                }
                ARCS_FOR_CONTINUOUS => {
                    return Ok(arc.first_label + arc.arc_idx + 1);
                }
                _ => {
                    // Variable length arcs: linear search. Position to the next
                    // arc, -1 to skip flags.
                    debug_assert_eq!(arc.bytes_per_arc, 0);
                    input.set_position(arc.next_arc - 1);
                }
            }
        }
        self.read_label(input.as_data_input())
    }

    /// Reads the arc with the provided index of a binary-search node.
    ///
    /// Equivalent to `FST.readArcByIndex`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn read_arc_by_index(
        &self,
        arc: &mut Arc<O::Output>,
        input: &mut dyn BytesReader,
        idx: i32,
    ) -> Result<()> {
        debug_assert!(arc.bytes_per_arc > 0);
        debug_assert_eq!(arc.node_flags, ARCS_FOR_BINARY_SEARCH);
        debug_assert!(idx >= 0 && idx < arc.num_arcs);
        input.set_position(arc.pos_arcs_start - i64::from(idx) * i64::from(arc.bytes_per_arc));
        arc.arc_idx = idx;
        arc.flags = input.read_byte()?;
        self.read_arc(arc, input)
    }

    /// Reads the arc with the provided index in the label range of a continuous
    /// node.
    ///
    /// Equivalent to `FST.readArcByContinuous`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn read_arc_by_continuous(
        &self,
        arc: &mut Arc<O::Output>,
        input: &mut dyn BytesReader,
        range_index: i32,
    ) -> Result<()> {
        debug_assert!(range_index >= 0 && range_index < arc.num_arcs);
        input.set_position(
            arc.pos_arcs_start - i64::from(range_index) * i64::from(arc.bytes_per_arc),
        );
        arc.arc_idx = range_index;
        arc.flags = input.read_byte()?;
        self.read_arc(arc, input)
    }

    /// Reads a present direct-addressing arc with the provided index in the
    /// label range.
    ///
    /// Equivalent to `FST.readArcByDirectAddressing(Arc, BytesReader, int)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn read_arc_by_direct_addressing(
        &self,
        arc: &mut Arc<O::Output>,
        input: &mut dyn BytesReader,
        range_index: i32,
    ) -> Result<()> {
        debug_assert!(range_index >= 0 && range_index < arc.num_arcs);
        let presence_index = BitTable::count_bits_up_to(range_index, arc, input)?;
        self.read_arc_by_direct_addressing_with_presence(arc, input, range_index, presence_index)
    }

    /// Equivalent to the private
    /// `FST.readArcByDirectAddressing(Arc, BytesReader, int, int)`.
    fn read_arc_by_direct_addressing_with_presence(
        &self,
        arc: &mut Arc<O::Output>,
        input: &mut dyn BytesReader,
        range_index: i32,
        presence_index: i32,
    ) -> Result<()> {
        input.set_position(
            arc.pos_arcs_start - i64::from(presence_index) * i64::from(arc.bytes_per_arc),
        );
        arc.arc_idx = range_index;
        arc.presence_index = presence_index;
        arc.flags = input.read_byte()?;
        self.read_arc(arc, input)
    }

    /// Reads the last arc of a direct-addressing node.
    ///
    /// Equivalent to `FST.readLastArcByDirectAddressing`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn read_last_arc_by_direct_addressing(
        &self,
        arc: &mut Arc<O::Output>,
        input: &mut dyn BytesReader,
    ) -> Result<()> {
        let presence_index = BitTable::count_bits(arc, input)? - 1;
        let range_index = arc.num_arcs - 1;
        self.read_arc_by_direct_addressing_with_presence(arc, input, range_index, presence_index)
    }

    /// Reads the last arc of a continuous node.
    ///
    /// Equivalent to `FST.readLastArcByContinuous`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn read_last_arc_by_continuous(
        &self,
        arc: &mut Arc<O::Output>,
        input: &mut dyn BytesReader,
    ) -> Result<()> {
        let range_index = arc.num_arcs - 1;
        self.read_arc_by_continuous(arc, input, range_index)
    }

    /// Reads the next real arc; never call this when `arc.is_last()`.
    ///
    /// Equivalent to `FST.readNextRealArc`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn read_next_real_arc(
        &self,
        arc: &mut Arc<O::Output>,
        input: &mut dyn BytesReader,
    ) -> Result<()> {
        match arc.node_flags {
            ARCS_FOR_BINARY_SEARCH | ARCS_FOR_CONTINUOUS => {
                debug_assert!(arc.bytes_per_arc > 0);
                arc.arc_idx += 1;
                debug_assert!(arc.arc_idx >= 0 && arc.arc_idx < arc.num_arcs);
                input.set_position(
                    arc.pos_arcs_start - i64::from(arc.arc_idx) * i64::from(arc.bytes_per_arc),
                );
                arc.flags = input.read_byte()?;
            }
            ARCS_FOR_DIRECT_ADDRESSING => {
                let next_index = BitTable::next_bit_set(arc.arc_idx, arc, input)?;
                let presence_index = arc.presence_index + 1;
                return self.read_arc_by_direct_addressing_with_presence(
                    arc,
                    input,
                    next_index,
                    presence_index,
                );
            }
            _ => {
                // Variable length arcs: linear search.
                debug_assert_eq!(arc.bytes_per_arc, 0);
                input.set_position(arc.next_arc);
                arc.flags = input.read_byte()?;
            }
        }
        self.read_arc(arc, input)
    }

    /// Reads an arc whose flags byte has already been consumed.
    ///
    /// Equivalent to the private `FST.readArc`.
    fn read_arc(&self, arc: &mut Arc<O::Output>, input: &mut dyn BytesReader) -> Result<()> {
        if arc.node_flags == ARCS_FOR_DIRECT_ADDRESSING || arc.node_flags == ARCS_FOR_CONTINUOUS {
            arc.label = arc.first_label + arc.arc_idx;
        } else {
            arc.label = self.read_label(input.as_data_input())?;
        }

        arc.output = if arc.flag(BIT_ARC_HAS_OUTPUT) {
            self.metadata.outputs.read(input.as_data_input())?
        } else {
            self.metadata.outputs.no_output()
        };

        arc.next_final_output = if arc.flag(BIT_ARC_HAS_FINAL_OUTPUT) {
            self.metadata
                .outputs
                .read_final_output(input.as_data_input())?
        } else {
            self.metadata.outputs.no_output()
        };

        if arc.flag(BIT_STOP_NODE) {
            arc.target = if arc.flag(BIT_FINAL_ARC) {
                FINAL_END_NODE
            } else {
                NON_FINAL_END_NODE
            };
            arc.next_arc = input.position(); // Only useful for a list.
        } else if arc.flag(BIT_TARGET_NEXT) {
            arc.next_arc = input.position(); // Only useful for a list.
            if !arc.flag(BIT_LAST_ARC) {
                if arc.bytes_per_arc == 0 {
                    // Must scan.
                    self.seek_to_next_node(input)?;
                } else {
                    let num_arcs = if arc.node_flags == ARCS_FOR_DIRECT_ADDRESSING {
                        BitTable::count_bits(arc, input)?
                    } else {
                        arc.num_arcs
                    };
                    input.set_position(
                        arc.pos_arcs_start - i64::from(arc.bytes_per_arc) * i64::from(num_arcs),
                    );
                }
            }
            arc.target = input.position();
        } else {
            arc.target = read_unpacked_node_target(input)?;
            arc.next_arc = input.position(); // Only useful for a list.
        }
        Ok(())
    }

    /// Fills `arc` with the virtual end arc of `follow`, or returns `false`
    /// when `follow` is not final.
    ///
    /// Equivalent to the static package-private `FST.readEndArc`.
    pub fn read_end_arc(follow: &Arc<O::Output>, arc: &mut Arc<O::Output>) -> bool {
        if follow.is_final() {
            if follow.target <= 0 {
                arc.flags = BIT_LAST_ARC as u8;
            } else {
                arc.flags = 0;
                // NOTE: next_arc is a node (not an address) in this case.
                arc.next_arc = follow.target;
            }
            arc.output = follow.next_final_output.clone();
            arc.label = END_LABEL;
            true
        } else {
            false
        }
    }

    /// Finds the arc leaving `follow` whose label is `label_to_match`, filling
    /// `arc` in place. Returns `false` when there is no such arc.
    ///
    /// Equivalent to `FST.findTargetArc`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn find_target_arc(
        &self,
        label_to_match: i32,
        follow: &Arc<O::Output>,
        arc: &mut Arc<O::Output>,
        input: &mut dyn BytesReader,
    ) -> Result<bool> {
        self.find_target_arc_impl(
            label_to_match,
            follow.is_final(),
            follow.target,
            follow.next_final_output.clone(),
            arc,
            input,
        )
    }

    /// In-place variant of [`FST::find_target_arc`], for the call sites where
    /// Lucene passes the same `Arc` as both the followed arc and the
    /// destination.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn find_target_arc_in_place(
        &self,
        label_to_match: i32,
        arc: &mut Arc<O::Output>,
        input: &mut dyn BytesReader,
    ) -> Result<bool> {
        let is_final = arc.is_final();
        let target = arc.target;
        let next_final_output = arc.next_final_output.clone();
        self.find_target_arc_impl(
            label_to_match,
            is_final,
            target,
            next_final_output,
            arc,
            input,
        )
    }

    // Lucene returns `null` from two adjacent branches of the linear scan for
    // different reasons -- the labels are past the target, and the arc list is
    // exhausted -- so the branches are kept distinct to stay comparable with
    // the Java source.
    #[allow(clippy::if_same_then_else)]
    fn find_target_arc_impl(
        &self,
        label_to_match: i32,
        follow_is_final: bool,
        follow_target: i64,
        follow_next_final_output: O::Output,
        arc: &mut Arc<O::Output>,
        input: &mut dyn BytesReader,
    ) -> Result<bool> {
        if label_to_match == END_LABEL {
            if follow_is_final {
                if follow_target <= 0 {
                    arc.flags = BIT_LAST_ARC as u8;
                } else {
                    arc.flags = 0;
                    // NOTE: next_arc is a node (not an address) in this case.
                    arc.next_arc = follow_target;
                }
                arc.output = follow_next_final_output;
                arc.label = END_LABEL;
                arc.node_flags = arc.flags;
                return Ok(true);
            }
            return Ok(false);
        }

        if follow_target <= 0 {
            return Ok(false);
        }

        input.set_position(follow_target);

        let flags = input.read_byte()?;
        arc.node_flags = flags;
        if flags == ARCS_FOR_DIRECT_ADDRESSING {
            arc.num_arcs = input.read_v_int()?; // This is in fact the label range.
            arc.bytes_per_arc = input.read_v_int()?;
            self.read_presence_bytes(arc, input)?;
            arc.first_label = self.read_label(input.as_data_input())?;
            arc.pos_arcs_start = input.position();

            let arc_index = label_to_match - arc.first_label;
            if arc_index < 0 || arc_index >= arc.num_arcs {
                return Ok(false); // Before or after the label range.
            } else if !BitTable::is_bit_set(arc_index, arc, input)? {
                return Ok(false); // Arc missing in the range.
            }
            self.read_arc_by_direct_addressing(arc, input, arc_index)?;
            return Ok(true);
        } else if flags == ARCS_FOR_BINARY_SEARCH {
            arc.num_arcs = input.read_v_int()?;
            arc.bytes_per_arc = input.read_v_int()?;
            arc.pos_arcs_start = input.position();

            // The array is sparse; do a binary search.
            let mut low = 0i32;
            let mut high = arc.num_arcs - 1;
            while low <= high {
                let mid = ((low as u32 + high as u32) >> 1) as i32;
                // +1 to skip over flags.
                input.set_position(
                    arc.pos_arcs_start - (i64::from(arc.bytes_per_arc) * i64::from(mid) + 1),
                );
                let mid_label = self.read_label(input.as_data_input())?;
                let cmp = mid_label - label_to_match;
                if cmp < 0 {
                    low = mid + 1;
                } else if cmp > 0 {
                    high = mid - 1;
                } else {
                    arc.arc_idx = mid - 1;
                    self.read_next_real_arc(arc, input)?;
                    return Ok(true);
                }
            }
            return Ok(false);
        } else if flags == ARCS_FOR_CONTINUOUS {
            arc.num_arcs = input.read_v_int()?;
            arc.bytes_per_arc = input.read_v_int()?;
            arc.first_label = self.read_label(input.as_data_input())?;
            arc.pos_arcs_start = input.position();
            let arc_index = label_to_match - arc.first_label;
            if arc_index < 0 || arc_index >= arc.num_arcs {
                return Ok(false); // Before or after the label range.
            }
            arc.arc_idx = arc_index - 1;
            self.read_next_real_arc(arc, input)?;
            return Ok(true);
        }

        // Linear scan.
        self.read_first_arc_info(follow_target, arc, input)?;
        input.set_position(arc.next_arc);
        loop {
            debug_assert_eq!(arc.bytes_per_arc, 0);
            let flags = input.read_byte()?;
            arc.flags = flags;
            let pos = input.position();
            let label = self.read_label(input.as_data_input())?;
            if label == label_to_match {
                input.set_position(pos);
                self.read_arc(arc, input)?;
                return Ok(true);
            } else if label > label_to_match {
                return Ok(false);
            } else if arc.is_last() {
                return Ok(false);
            } else {
                if flag_is_set(i32::from(flags), BIT_ARC_HAS_OUTPUT) {
                    self.metadata.outputs.skip_output(input.as_data_input())?;
                }
                if flag_is_set(i32::from(flags), BIT_ARC_HAS_FINAL_OUTPUT) {
                    self.metadata
                        .outputs
                        .skip_final_output(input.as_data_input())?;
                }
                if !flag_is_set(i32::from(flags), BIT_STOP_NODE)
                    && !flag_is_set(i32::from(flags), BIT_TARGET_NEXT)
                {
                    read_unpacked_node_target(input)?;
                }
            }
        }
    }

    /// Scans forward over the remaining arcs of the current node.
    ///
    /// Equivalent to the private `FST.seekToNextNode`.
    fn seek_to_next_node(&self, input: &mut dyn BytesReader) -> Result<()> {
        loop {
            let flags = i32::from(input.read_byte()?);
            self.read_label(input.as_data_input())?;

            if flag_is_set(flags, BIT_ARC_HAS_OUTPUT) {
                self.metadata.outputs.skip_output(input.as_data_input())?;
            }

            if flag_is_set(flags, BIT_ARC_HAS_FINAL_OUTPUT) {
                self.metadata
                    .outputs
                    .skip_final_output(input.as_data_input())?;
            }

            if !flag_is_set(flags, BIT_STOP_NODE) && !flag_is_set(flags, BIT_TARGET_NEXT) {
                read_unpacked_node_target(input)?;
            }

            if flag_is_set(flags, BIT_LAST_ARC) {
                return Ok(());
            }
        }
    }
}

/// Reads a node target written as a variable-length long.
///
/// Equivalent to the private `FST.readUnpackedNodeTarget`.
fn read_unpacked_node_target(input: &mut dyn BytesReader) -> Result<i64> {
    input.read_v_long()
}

impl<O: Outputs> Accountable for FST<O> {
    fn ram_bytes_used(&self) -> i64 {
        BASE_RAM_BYTES_USED + self.fst_reader.ram_bytes_used()
    }
}

/// Shallow size of an [`FST`], mirroring
/// `RamUsageEstimator.shallowSizeOfInstance(FST.class)`.
const BASE_RAM_BYTES_USED: i64 = 32;

impl<O: Outputs> fmt::Display for FST<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "FST(input={:?},output={}",
            self.metadata.input_type, self.metadata.outputs
        )
    }
}

impl<O: Outputs> fmt::Debug for FST<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FST")
            .field("inputType", &self.metadata.input_type)
            .field("version", &self.metadata.version)
            .field("startNode", &self.metadata.start_node)
            .field("numBytes", &self.metadata.num_bytes)
            .finish()
    }
}
