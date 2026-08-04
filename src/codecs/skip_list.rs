//! Multi-level skip-list reader and writer.
//!
//! Equivalent to `org.apache.lucene.codecs.MultiLevelSkipListReader` and
//! `MultiLevelSkipListWriter`.
//!
//! Skip lists are organized into levels: level 0 contains a skip entry every
//! `skip_interval` documents; level *i+1* contains a skip entry every
//! `skip_interval * skip_multiplier^i` documents. Higher levels store child
//! pointers back to the matching entry in the level below, allowing logarithmic
//! skipping.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::store::{DataOutput, IndexInput, IndexOutput};

// -----------------------------------------------------------------------------
// Writer
// -----------------------------------------------------------------------------

/// Custom skip-data encoding used by a [`MultiLevelSkipListWriter`].
///
/// Concrete postings formats implement this trait to write the per-skip-entry
/// payload (typically doc-id deltas, frequencies, positions, etc.).
pub trait SkipDataWriter {
    /// Writes one skip entry for the given level.
    fn write_skip_data(&mut self, level: i32, skip_buffer: &mut dyn DataOutput) -> Result<()>;
}

/// In-memory buffer for a single skip level.
///
/// Wraps a `Vec<u8>` and implements [`DataOutput`]. It can be cheaply reset so
/// the same writer can be reused across terms.
#[derive(Debug, Default, Clone)]
struct SkipBuffer {
    bytes: Vec<u8>,
}

impl SkipBuffer {
    fn new() -> Self {
        Self::default()
    }

    fn clear(&mut self) {
        self.bytes.clear();
    }

    fn len(&self) -> usize {
        self.bytes.len()
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes
    }
}

impl DataOutput for SkipBuffer {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        self.bytes.push(b);
        Ok(())
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
        let end = offset
            .checked_add(len)
            .ok_or_else(|| LuceneError::IllegalArgument("offset + len overflowed".to_string()))?;
        if end > b.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "source buffer too small: offset={offset}, len={len}, buf.len()={}",
                b.len()
            )));
        }
        self.bytes.extend_from_slice(&b[offset..end]);
        Ok(())
    }
}

/// Writes multi-level skip lists.
///
/// Equivalent to `org.apache.lucene.codecs.MultiLevelSkipListWriter`.
pub struct MultiLevelSkipListWriter<W: SkipDataWriter> {
    skip_interval: i32,
    skip_multiplier: i32,
    window_length: i32,
    number_of_skip_levels: i32,
    skip_buffer: Vec<SkipBuffer>,
    skip_data_writer: W,
}

impl<W: SkipDataWriter + std::fmt::Debug> std::fmt::Debug for MultiLevelSkipListWriter<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiLevelSkipListWriter")
            .field("skip_interval", &self.skip_interval)
            .field("skip_multiplier", &self.skip_multiplier)
            .field("window_length", &self.window_length)
            .field("number_of_skip_levels", &self.number_of_skip_levels)
            .field("skip_buffer", &self.skip_buffer)
            .field("skip_data_writer", &self.skip_data_writer)
            .finish()
    }
}

impl<W: SkipDataWriter> MultiLevelSkipListWriter<W> {
    /// Creates a writer with a separate `skip_multiplier` for higher levels.
    pub fn new(
        skip_data_writer: W,
        skip_interval: i32,
        skip_multiplier: i32,
        max_skip_levels: i32,
        df: i32,
    ) -> Self {
        let number_of_skip_levels = if df > skip_interval {
            let levels = 1 + log(df / skip_interval, skip_multiplier);
            levels.min(max_skip_levels)
        } else {
            1
        };
        Self {
            skip_interval,
            skip_multiplier,
            window_length: skip_interval
                .checked_mul(skip_multiplier)
                .unwrap_or(i32::MAX),
            number_of_skip_levels,
            skip_buffer: Vec::new(),
            skip_data_writer,
        }
    }

    /// Creates a writer where the `skip_multiplier` equals the `skip_interval`.
    pub fn new_same_multiplier(
        skip_data_writer: W,
        skip_interval: i32,
        max_skip_levels: i32,
        df: i32,
    ) -> Self {
        Self::new(
            skip_data_writer,
            skip_interval,
            skip_interval,
            max_skip_levels,
            df,
        )
    }

    /// Returns the number of skip levels used by this writer.
    pub fn number_of_skip_levels(&self) -> i32 {
        self.number_of_skip_levels
    }

    /// Resets (or allocates) the internal skip buffers.
    pub fn reset_skip(&mut self) {
        if self.skip_buffer.is_empty() {
            self.skip_buffer = (0..self.number_of_skip_levels)
                .map(|_| SkipBuffer::new())
                .collect();
        } else {
            for buf in &mut self.skip_buffer {
                buf.clear();
            }
        }
    }

    /// Buffers a skip entry for the given document frequency.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `df` is not a multiple of
    /// `skip_interval`.
    pub fn buffer_skip(&mut self, df: i32) -> Result<()> {
        if df % self.skip_interval != 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "df ({df}) must be a multiple of skipInterval ({})",
                self.skip_interval
            )));
        }

        let mut num_levels = 1;
        if df % self.window_length == 0 {
            num_levels += 1;
            let mut remaining = df / self.window_length;
            while remaining % self.skip_multiplier == 0 && num_levels < self.number_of_skip_levels {
                num_levels += 1;
                remaining /= self.skip_multiplier;
            }
        }

        let mut child_pointer = 0i64;

        for level in 0..num_levels {
            self.skip_data_writer
                .write_skip_data(level, &mut self.skip_buffer[level as usize])?;

            let new_child_pointer = self.skip_buffer[level as usize].len() as i64;

            if level != 0 {
                Self::write_child_pointer(child_pointer, &mut self.skip_buffer[level as usize])?;
            }

            child_pointer = new_child_pointer;
        }

        Ok(())
    }

    /// Writes the buffered skip list to `output` and returns the file pointer
    /// where the skip list starts.
    pub fn write_skip(&mut self, output: &mut dyn IndexOutput) -> Result<i64> {
        let skip_pointer = output.file_pointer();
        if self.skip_buffer.is_empty() {
            return Ok(skip_pointer);
        }

        for level in (1..self.number_of_skip_levels).rev() {
            let length = self.skip_buffer[level as usize].len() as i64;
            if length > 0 {
                Self::write_level_length(length, output)?;
                output.write_bytes(
                    self.skip_buffer[level as usize].as_slice(),
                    0,
                    self.skip_buffer[level as usize].len(),
                )?;
            }
        }
        output.write_bytes(self.skip_buffer[0].as_slice(), 0, self.skip_buffer[0].len())?;

        Ok(skip_pointer)
    }

    /// Writes the length of a level to `output`.
    ///
    /// The default encoding is a variable-length long.
    fn write_level_length(level_length: i64, output: &mut dyn IndexOutput) -> Result<()> {
        output.write_v_long(level_length)
    }

    /// Writes a child pointer to `skip_buffer`.
    fn write_child_pointer(child_pointer: i64, skip_buffer: &mut dyn DataOutput) -> Result<()> {
        skip_buffer.write_v_long(child_pointer)
    }
}

// -----------------------------------------------------------------------------
// Reader
// -----------------------------------------------------------------------------

/// Custom skip-data decoding used by a [`MultiLevelSkipListReader`].
///
/// Concrete postings formats implement this trait to read the per-skip-entry
/// payload, and may override the default level-length and child-pointer
/// encodings.
pub trait SkipDataReader {
    /// Reads one skip entry for the given level.
    fn read_skip_data(&mut self, level: i32, skip_stream: &mut dyn IndexInput) -> Result<i32>;

    /// Reads the length of a higher skip level.
    ///
    /// The default encoding is a variable-length long.
    fn read_level_length(&self, skip_stream: &mut dyn IndexInput) -> Result<i64> {
        skip_stream.read_v_long()
    }

    /// Reads a child pointer.
    fn read_child_pointer(&self, skip_stream: &mut dyn IndexInput) -> Result<i64> {
        skip_stream.read_v_long()
    }
}

/// Reads multi-level skip lists.
///
/// Equivalent to `org.apache.lucene.codecs.MultiLevelSkipListReader`.
pub struct MultiLevelSkipListReader<R: SkipDataReader> {
    max_number_of_skip_levels: i32,
    number_of_skip_levels: i32,
    doc_count: i32,
    skip_stream: Vec<Option<Box<dyn IndexInput>>>,
    skip_pointer: Vec<i64>,
    skip_interval: Vec<i32>,
    num_skipped: Vec<i32>,
    skip_doc: Vec<i32>,
    child_pointer: Vec<i64>,
    last_doc: i32,
    last_child_pointer: i64,
    skip_multiplier: i32,
    skip_data_reader: R,
}

impl<R: SkipDataReader> MultiLevelSkipListReader<R> {
    /// Creates a reader with a separate `skip_multiplier` for higher levels.
    pub fn new(
        skip_stream: Box<dyn IndexInput>,
        skip_data_reader: R,
        max_skip_levels: i32,
        skip_interval: i32,
        skip_multiplier: i32,
    ) -> Self {
        let mut skip_intervals = vec![skip_interval; max_skip_levels as usize];
        for i in 1..max_skip_levels as usize {
            skip_intervals[i] = skip_intervals[i - 1]
                .checked_mul(skip_multiplier)
                .unwrap_or(i32::MAX);
        }

        let mut streams: Vec<Option<Box<dyn IndexInput>>> =
            (0..max_skip_levels).map(|_| None).collect();
        streams[0] = Some(skip_stream);

        Self {
            max_number_of_skip_levels: max_skip_levels,
            number_of_skip_levels: 1,
            doc_count: 0,
            skip_stream: streams,
            skip_pointer: vec![0; max_skip_levels as usize],
            skip_interval: skip_intervals,
            num_skipped: vec![0; max_skip_levels as usize],
            skip_doc: vec![0; max_skip_levels as usize],
            child_pointer: vec![0; max_skip_levels as usize],
            last_doc: 0,
            last_child_pointer: 0,
            skip_multiplier,
            skip_data_reader,
        }
    }

    /// Creates a reader where the `skip_multiplier` equals the `skip_interval`.
    pub fn new_same_multiplier(
        skip_stream: Box<dyn IndexInput>,
        skip_data_reader: R,
        max_skip_levels: i32,
        skip_interval: i32,
    ) -> Self {
        Self::new(
            skip_stream,
            skip_data_reader,
            max_skip_levels,
            skip_interval,
            skip_interval,
        )
    }

    /// Returns the document id of the last skip entry read by [`skip_to`].
    pub fn doc(&self) -> i32 {
        self.last_doc
    }

    /// Closes the higher-level skip streams.
    pub fn close(&mut self) -> Result<()> {
        for i in 1..self.skip_stream.len() {
            if let Some(stream) = self.skip_stream[i].as_mut() {
                stream.close()?;
            }
        }
        Ok(())
    }

    /// Initializes the reader for reuse on a new term.
    pub fn init(&mut self, skip_pointer: i64, df: i32) -> Result<()> {
        self.skip_pointer[0] = skip_pointer;
        self.doc_count = df;

        self.skip_doc.fill(0);
        self.num_skipped.fill(0);
        self.child_pointer.fill(0);

        for i in 1..self.number_of_skip_levels as usize {
            self.skip_stream[i] = None;
        }

        self.load_skip_levels()
    }

    fn load_skip_levels(&mut self) -> Result<()> {
        if self.doc_count <= self.skip_interval[0] {
            self.number_of_skip_levels = 1;
        } else {
            let levels = 1 + log(self.doc_count / self.skip_interval[0], self.skip_multiplier);
            self.number_of_skip_levels = levels.min(self.max_number_of_skip_levels);
        }

        let mut base = self.skip_stream[0]
            .take()
            .expect("INVARIANT: level 0 skip stream is set");
        base.seek(self.skip_pointer[0])?;

        for i in (1..self.number_of_skip_levels).rev() {
            let length = self.skip_data_reader.read_level_length(base.as_mut())?;
            self.skip_pointer[i as usize] = base.file_pointer();
            self.skip_stream[i as usize] = Some(base.clone_input()?);
            base.seek(base.file_pointer() + length)?;
        }

        self.skip_pointer[0] = base.file_pointer();
        self.skip_stream[0] = Some(base);
        Ok(())
    }

    /// Skips entries to the first beyond the current whose document number is
    /// greater than or equal to `target`. Returns the current doc count.
    pub fn skip_to(&mut self, target: i32) -> Result<i32> {
        let mut level = 0;
        while level < self.number_of_skip_levels - 1 && target > self.skip_doc[level as usize + 1] {
            level += 1;
        }

        while level >= 0 {
            if target > self.skip_doc[level as usize] {
                if !self.load_next_skip(level)? {
                    continue;
                }
            } else {
                if level > 0
                    && self.last_child_pointer
                        > self.skip_stream[level as usize - 1]
                            .as_ref()
                            .expect("INVARIANT: level stream is set")
                            .file_pointer()
                {
                    self.seek_child(level - 1)?;
                }
                level -= 1;
            }
        }

        Ok(self.num_skipped[0] - self.skip_interval[0] - 1)
    }

    fn load_next_skip(&mut self, level: i32) -> Result<bool> {
        self.set_last_skip_data(level);
        self.num_skipped[level as usize] += self.skip_interval[level as usize];

        if (self.num_skipped[level as usize] as u32) > (self.doc_count as u32) {
            self.skip_doc[level as usize] = i32::MAX;
            if self.number_of_skip_levels > level {
                self.number_of_skip_levels = level;
            }
            return Ok(false);
        }

        let stream = self.skip_stream[level as usize]
            .as_mut()
            .expect("INVARIANT: level stream is set");
        let delta = self
            .skip_data_reader
            .read_skip_data(level, stream.as_mut())?;
        self.skip_doc[level as usize] += delta;

        if level != 0 {
            self.child_pointer[level as usize] =
                self.skip_data_reader.read_child_pointer(stream.as_mut())?
                    + self.skip_pointer[level as usize - 1];
        }

        Ok(true)
    }

    /// Seeks the skip entry on the given level to `last_child_pointer`.
    fn seek_child(&mut self, level: i32) -> Result<()> {
        let stream = self.skip_stream[level as usize]
            .as_mut()
            .expect("INVARIANT: level stream is set");
        stream.seek(self.last_child_pointer)?;
        self.num_skipped[level as usize] =
            self.num_skipped[level as usize + 1] - self.skip_interval[level as usize + 1];
        self.skip_doc[level as usize] = self.last_doc;
        if level > 0 {
            self.child_pointer[level as usize] =
                self.skip_data_reader.read_child_pointer(stream.as_mut())?
                    + self.skip_pointer[level as usize - 1];
        }
        Ok(())
    }

    fn set_last_skip_data(&mut self, level: i32) {
        self.last_doc = self.skip_doc[level as usize];
        self.last_child_pointer = self.child_pointer[level as usize];
    }
}

impl<R: SkipDataReader> Drop for MultiLevelSkipListReader<R> {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

/// Computes `floor(log_base(x))` for positive `x` and `base > 1`.
fn log(x: i32, base: i32) -> i32 {
    if base <= 1 {
        return 0;
    }
    let mut ret = 0;
    let mut v = x as i64;
    let b = base as i64;
    while v >= b {
        v /= b;
        ret += 1;
    }
    ret
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{MockIndexInput, MockIndexOutput};

    /// Test skip codec that stores the per-level skip interval as the doc delta.
    #[derive(Debug, Clone, Copy)]
    struct IntervalSkipCodec {
        interval: i32,
        multiplier: i32,
    }

    impl IntervalSkipCodec {
        fn new(interval: i32, multiplier: i32) -> Self {
            Self {
                interval,
                multiplier,
            }
        }

        fn new_same(interval: i32) -> Self {
            Self::new(interval, interval)
        }

        fn delta(&self, level: i32) -> i32 {
            self.interval * self.multiplier.pow(level as u32)
        }
    }

    impl Default for IntervalSkipCodec {
        fn default() -> Self {
            Self::new_same(1)
        }
    }

    impl SkipDataWriter for IntervalSkipCodec {
        fn write_skip_data(&mut self, level: i32, skip_buffer: &mut dyn DataOutput) -> Result<()> {
            skip_buffer.write_v_int(self.delta(level))
        }
    }

    impl SkipDataReader for IntervalSkipCodec {
        fn read_skip_data(&mut self, _level: i32, skip_stream: &mut dyn IndexInput) -> Result<i32> {
            skip_stream.read_v_int()
        }
    }

    #[test]
    fn writer_round_trips_simple_skip_list() {
        let codec = IntervalSkipCodec::new_same(3);
        let mut writer = MultiLevelSkipListWriter::new_same_multiplier(codec, 3, 10, 27);
        writer.reset_skip();

        // df = 27, skipInterval = 3 -> 9 entries on level 0.
        for i in 1..=9 {
            writer.buffer_skip(i * 3).unwrap();
        }

        let mut output = MockIndexOutput::new("skip", "skip");
        let skip_pointer = writer.write_skip(&mut output).unwrap();
        assert_eq!(skip_pointer, 0);

        let bytes = output.into_inner();
        let input = MockIndexInput::new(bytes, "skip");
        let mut reader =
            MultiLevelSkipListReader::new_same_multiplier(Box::new(input), codec, 10, 3);
        reader.init(skip_pointer, 27).unwrap();

        // Skip to doc 20. Level-0 entries cover docs 3,6,...,27.
        // The last entry with doc <= 20 is doc 18; the caller would then scan
        // forward to doc 21.
        let count = reader.skip_to(20).unwrap();
        assert_eq!(reader.doc(), 18);
        assert_eq!(count, 17, "numSkipped[0] - skipInterval - 1");

        reader.close().unwrap();
    }

    #[test]
    fn writer_multi_level_round_trip() {
        let codec = IntervalSkipCodec::new_same(2);
        let mut writer = MultiLevelSkipListWriter::new_same_multiplier(codec, 2, 5, 64);
        writer.reset_skip();

        // skipInterval=2, windowLength=4. Every 4 docs we write level 1.
        // Every 8 docs we write level 2, etc.
        for df in (2..=64).step_by(2) {
            writer.buffer_skip(df).unwrap();
        }

        let mut output = MockIndexOutput::new("skip", "skip");
        let skip_pointer = writer.write_skip(&mut output).unwrap();
        let bytes = output.into_inner();
        let input = MockIndexInput::new(bytes, "skip");
        let mut reader =
            MultiLevelSkipListReader::new_same_multiplier(Box::new(input), codec, 5, 2);
        reader.init(skip_pointer, 64).unwrap();

        // Skip around and verify we land on multiples of the level-0 interval.
        // `getDoc()` returns the last skip entry whose doc is <= target, so the
        // caller scans forward from there.
        for target in [1, 5, 17, 33, 63] {
            let _ = reader.skip_to(target).unwrap();
            let doc = reader.doc();
            assert_eq!(
                doc % codec.interval,
                0,
                "skipped doc should be a multiple of the level-0 interval"
            );
            assert!(
                doc <= target || doc == 0,
                "skipped doc should be at or before target, or 0"
            );
        }

        reader.close().unwrap();
    }

    #[test]
    fn writer_rejects_non_multiple_df() {
        let codec = IntervalSkipCodec::new_same(3);
        let mut writer = MultiLevelSkipListWriter::new_same_multiplier(codec, 3, 10, 30);
        writer.reset_skip();
        let err = writer.buffer_skip(4).unwrap_err();
        assert!(matches!(err, LuceneError::IllegalArgument(_)));
    }

    #[test]
    fn reader_skip_to_end_exhausts_list() {
        let codec = IntervalSkipCodec::new_same(3);
        let mut writer = MultiLevelSkipListWriter::new_same_multiplier(codec, 3, 5, 12);
        writer.reset_skip();
        for df in [3, 6, 9, 12] {
            writer.buffer_skip(df).unwrap();
        }

        let mut output = MockIndexOutput::new("skip", "skip");
        let skip_pointer = writer.write_skip(&mut output).unwrap();
        let bytes = output.into_inner();
        let input = MockIndexInput::new(bytes, "skip");
        let mut reader =
            MultiLevelSkipListReader::new_same_multiplier(Box::new(input), codec, 5, 3);
        reader.init(skip_pointer, 12).unwrap();

        // Skip past the end; the list should be marked exhausted.
        let _ = reader.skip_to(100).unwrap();
        assert_eq!(reader.doc(), 12);

        reader.close().unwrap();
    }

    #[test]
    fn skip_levels_computation_matches_lucene() {
        // log(27/3, 3) = log(9, 3) = 2, so levels = 1 + 2 = 3.
        let codec = IntervalSkipCodec::new_same(3);
        let writer = MultiLevelSkipListWriter::new(codec, 3, 3, 10, 27);
        assert_eq!(writer.number_of_skip_levels(), 3);

        // df <= skipInterval -> one level.
        let writer = MultiLevelSkipListWriter::new_same_multiplier(codec, 8, 10, 5);
        assert_eq!(writer.number_of_skip_levels(), 1);
    }

    #[test]
    fn clone_input_is_used_for_higher_levels() {
        // Ensure the reader clones the base stream for higher levels.
        let codec = IntervalSkipCodec::new_same(2);
        let mut writer = MultiLevelSkipListWriter::new_same_multiplier(codec, 2, 5, 16);
        writer.reset_skip();
        for df in (2..=16).step_by(2) {
            writer.buffer_skip(df).unwrap();
        }

        let mut output = MockIndexOutput::new("skip", "skip");
        let skip_pointer = writer.write_skip(&mut output).unwrap();
        let bytes = output.into_inner();
        let input = MockIndexInput::new(bytes.clone(), "skip");
        let mut reader =
            MultiLevelSkipListReader::new_same_multiplier(Box::new(input), codec, 5, 2);
        reader.init(skip_pointer, 16).unwrap();

        // Number of levels should be > 1 for df=16.
        assert!(reader.number_of_skip_levels > 1);

        let _ = reader.skip_to(10).unwrap();
        reader.close().unwrap();
    }
}
