//! Port of `org.apache.lucene.util.fst.FSTReader`.

use crate::error::Result;
use crate::store::DataOutput;
use crate::util::Accountable;

use super::fst::BytesReader;

/// Abstraction for reading the bytes that make up an FST.
///
/// Equivalent to `org.apache.lucene.util.fst.FSTReader`.
///
/// # Java to Rust adaptations
///
/// * [`FSTReader::get_reverse_bytes_reader`] returns a [`Result`]. Lucene
///   declares no checked exception and `OffHeapFSTStore` therefore wraps the
///   `IOException` raised while slicing its `IndexInput` in an unchecked
///   `RuntimeException`; this port reports the failure instead of panicking.
/// * The returned reader borrows the store, which is what Lucene's readers do
///   implicitly by holding a reference to the store's arrays.
pub trait FSTReader: Accountable {
    /// Returns the reverse [`BytesReader`] for this FST.
    ///
    /// Equivalent to `FSTReader.getReverseBytesReader`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while opening the underlying storage.
    fn get_reverse_bytes_reader(&self) -> Result<Box<dyn BytesReader + '_>>;

    /// Writes this FST to another [`DataOutput`].
    ///
    /// Equivalent to `FSTReader.writeTo`.
    ///
    /// # Errors
    ///
    /// Propagates write errors, and returns
    /// [`crate::error::LuceneError::UnsupportedOperation`] for stores that
    /// cannot reproduce their bytes.
    fn write_to(&self, out: &mut dyn DataOutput) -> Result<()>;
}
