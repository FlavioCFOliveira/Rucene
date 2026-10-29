//! Index sorting ported from `org.apache.lucene.index`.
//!
//! Covers `IndexSorter` and its per-type implementations, plus `Sorter` and its
//! `DocMap`: the machinery that reorders documents at flush and merge time.

use std::cmp::Ordering;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::leaf_reader::LeafReader;
use crate::index::{BinaryDocValues, NumericDocValues, SortedDocValues};
use crate::search::SortField;
use crate::search::NO_MORE_DOCS;
use crate::util::packed::{
    MonotonicLongValues, PackedInts, PackedLongValues, PackedLongValuesBuilderOps,
};
use crate::util::selector::PivotOps;
use crate::util::sorter::{ordering_to_int, Sorter as UtilSorter, TimSorter, TimSorterState};
use crate::util::spi::{NamedSPI, NamedSPILoader};
use crate::util::{BytesRef, FixedBitSet};

/// Maps a document to a `long` that orders it against the other documents of a
/// merge.
///
/// Equivalent to `IndexSorter.ComparableProvider`.
pub type ComparableProvider = Box<dyn Fn(i32) -> Result<i64>>;

/// Orders two documents of the same segment.
///
/// Equivalent to `IndexSorter.DocComparator`.
pub type DocComparator = Box<dyn Fn(i32, i32) -> Ordering + Send + Sync>;

/// Caches the sort value of the current document of every segment being
/// merged, and compares them.
///
/// Equivalent to `IndexSorter.ComparableValues` (`IndexSorter.java:65-112`),
/// the merge-sort counterpart of [`DocComparator`]. Most sorts reduce a
/// document to one sortable `long` and implement
/// [`IndexSorter::get_comparable_providers`], which
/// [`from_comparable_providers`] adapts to this interface; a sort whose value
/// cannot be a `long` — [`BinarySorter`] — implements it directly.
pub trait ComparableValues {
    /// Reads the value of document `doc_id` of segment `reader_index`, so that
    /// [`compare`](Self::compare) can later compare it.
    ///
    /// Equivalent to `ComparableValues.setTopValue(int, int)`. The merge sort
    /// visits each segment's documents in increasing order, so `doc_id` never
    /// decreases for a given `reader_index`.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the value raises.
    fn set_top_value(&mut self, reader_index: usize, doc_id: i32) -> Result<()>;

    /// Compares the values last read for two segments, without applying the
    /// sort's `reverse` flag, which the caller applies.
    ///
    /// Equivalent to `ComparableValues.compare(int, int)`.
    fn compare(&self, reader_index_a: usize, reader_index_b: usize) -> Ordering;
}

/// Adapts one [`ComparableProvider`] per segment to [`ComparableValues`].
///
/// Equivalent to the static
/// `IndexSorter.ComparableValues.fromComparableProviders(ComparableProvider[])`.
pub fn from_comparable_providers(providers: Vec<ComparableProvider>) -> Box<dyn ComparableValues> {
    let values = vec![0i64; providers.len()];
    Box::new(ProvidersComparableValues { providers, values })
}

/// The anonymous `ComparableValues`
/// `ComparableValues.fromComparableProviders` returns: one `long` per segment,
/// compared with `Long.compare`.
struct ProvidersComparableValues {
    providers: Vec<ComparableProvider>,
    values: Vec<i64>,
}

impl ComparableValues for ProvidersComparableValues {
    fn set_top_value(&mut self, reader_index: usize, doc_id: i32) -> Result<()> {
        self.values[reader_index] = (self.providers[reader_index])(doc_id)?;
        Ok(())
    }

    fn compare(&self, reader_index_a: usize, reader_index_b: usize) -> Ordering {
        self.values[reader_index_a].cmp(&self.values[reader_index_b])
    }
}

/// Sorts the documents of an index by one field.
///
/// Equivalent to `org.apache.lucene.index.IndexSorter`.
pub trait IndexSorter: Send + Sync {
    /// Returns one comparable provider per reader, for merge-time ordering.
    ///
    /// Equivalent to `IndexSorter.getComparableProviders(List<LeafReader>)`. As
    /// in Java, a sorter whose value is a `long` implements it, and the default
    /// [`get_comparable_values`](Self::get_comparable_values) adapts it; a
    /// sorter that implements [`get_comparable_values`](Self::get_comparable_values)
    /// itself need not, and inherits this default, which fails.
    ///
    /// # Errors
    ///
    /// The default returns [`LuceneError::UnsupportedOperation`] with Java's
    /// message, `"<class> does not implement getComparableProviders"`.
    /// **Divergence from Lucene 10.5.0.** Java names the class with
    /// `getClass().getName()`; Rust has no runtime class name, so the default
    /// names the implementing type with [`std::any::type_name`]. The one
    /// Lucene class that inherits the default, `BinarySorter`, renders the
    /// Java name; see [`BinarySorter`].
    fn get_comparable_providers(
        &self,
        readers: &[Arc<dyn LeafReader>],
    ) -> Result<Vec<ComparableProvider>> {
        let _ = readers;
        Err(LuceneError::UnsupportedOperation(format!(
            "{} does not implement getComparableProviders",
            std::any::type_name::<Self>()
        )))
    }

    /// Returns the within-segment comparator, for flush-time ordering.
    ///
    /// Equivalent to `IndexSorter.getDocComparator(LeafReader, int)`.
    fn get_doc_comparator(&self, reader: &dyn LeafReader, max_doc: i32) -> Result<DocComparator>;

    /// Returns the SPI name of the [`SortFieldProvider`] that serialises this
    /// sorter's `SortField`.
    ///
    /// Equivalent to `IndexSorter.getProviderName()`.
    fn get_provider_name(&self) -> &str;

    /// Builds the merge-time value holder.
    ///
    /// Equivalent to `IndexSorter.getComparableValues(List<LeafReader>)`, whose
    /// default adapts [`get_comparable_providers`](Self::get_comparable_providers)
    /// through [`from_comparable_providers`].
    ///
    /// # Errors
    ///
    /// Propagates whatever building the providers raises.
    fn get_comparable_values(
        &self,
        readers: &[Arc<dyn LeafReader>],
    ) -> Result<Box<dyn ComparableValues>> {
        Ok(from_comparable_providers(
            self.get_comparable_providers(readers)?,
        ))
    }
}

/// Supplies the numeric doc values a sorter reads.
///
/// Equivalent to `IndexSorter.NumericDocValuesProvider`.
pub type NumericDocValuesProvider =
    Arc<dyn Fn(&dyn LeafReader) -> Result<Option<Box<dyn NumericDocValues>>> + Send + Sync>;

/// Supplies the sorted doc values a sorter reads.
///
/// Equivalent to `IndexSorter.SortedDocValuesProvider`.
pub type SortedDocValuesProvider =
    Arc<dyn Fn(&dyn LeafReader) -> Result<Option<Box<dyn SortedDocValues>>> + Send + Sync>;

/// Supplies the binary doc values a sorter reads.
///
/// Equivalent to `IndexSorter.BinaryDocValuesProvider`
/// (`IndexSorter.java:184-189`). As in Java — whose only provider is
/// `BinarySortField::getSortKeyDocValues`, built on `DocValues.getBinary` — a
/// reader without the field yields empty values rather than none.
pub type BinaryDocValuesProvider =
    Arc<dyn Fn(&dyn LeafReader) -> Result<Box<dyn BinaryDocValues>> + Send + Sync>;

/// Compares two binary sort keys, either of which may be missing (`None`).
///
/// Equivalent to the `Comparator<BytesRef>` `IndexSorter.BinarySorter` takes,
/// which Java documents as accepting `null` for a missing value and not
/// applying the `reverse` flag. A key is handed over as the byte slice a
/// `BytesRef` would view.
pub type BinaryComparator = Arc<dyn Fn(Option<&[u8]>, Option<&[u8]>) -> Ordering + Send + Sync>;

/// Reads every value of a numeric doc-values field into a dense array, filling
/// the documents that have no value with `missing_value`.
///
/// This is the shared body of the four numeric sorters: Java writes it out once
/// per type because the array is typed, but the comparison is always over the
/// raw `long` the doc values carry.
fn dense_numeric_values(
    reader: &dyn LeafReader,
    provider: &NumericDocValuesProvider,
    max_doc: i32,
    missing_value: Option<i64>,
) -> Result<Vec<i64>> {
    let mut values = vec![missing_value.unwrap_or(0); max_doc.max(0) as usize];
    if let Some(mut dvs) = provider(reader)? {
        loop {
            let doc_id = dvs.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            if doc_id >= 0 && (doc_id as usize) < values.len() {
                values[doc_id as usize] = dvs.long_value()?;
            }
        }
    }
    Ok(values)
}

/// Builds one comparable provider per reader over a numeric doc-values field.
fn numeric_comparable_providers(
    readers: &[Arc<dyn LeafReader>],
    provider: &NumericDocValuesProvider,
    missing_value: i64,
) -> Result<Vec<ComparableProvider>> {
    let mut providers: Vec<ComparableProvider> = Vec::with_capacity(readers.len());
    for reader in readers {
        let values = std::cell::RefCell::new(provider(reader.as_ref())?);
        providers.push(Box::new(move |doc_id: i32| -> Result<i64> {
            let mut guard = values.borrow_mut();
            let Some(dvs) = guard.as_mut() else {
                return Ok(missing_value);
            };
            if dvs.advance_exact(doc_id)? {
                dvs.long_value()
            } else {
                Ok(missing_value)
            }
        }));
    }
    Ok(providers)
}

/// The four numeric sort kinds, which differ only in how the raw `long` read
/// from doc values is interpreted before comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericSortKind {
    /// The value is a signed 32-bit integer, as `IndexSorter.IntSorter`.
    Int,
    /// The value is a signed 64-bit integer, as `IndexSorter.LongSorter`.
    Long,
    /// The value is a `float` in its sortable-int encoding, as
    /// `IndexSorter.FloatSorter`.
    Float,
    /// The value is a `double` in its sortable-long encoding, as
    /// `IndexSorter.DoubleSorter`.
    Double,
}

impl NumericSortKind {
    /// Orders two raw doc-values longs according to the kind.
    fn compare(self, a: i64, b: i64) -> Ordering {
        match self {
            NumericSortKind::Int => (a as i32).cmp(&(b as i32)),
            NumericSortKind::Long => a.cmp(&b),
            NumericSortKind::Float => {
                let a = f32::from_bits(a as i32 as u32);
                let b = f32::from_bits(b as i32 as u32);
                a.total_cmp(&b)
            }
            NumericSortKind::Double => {
                let a = f64::from_bits(a as u64);
                let b = f64::from_bits(b as u64);
                a.total_cmp(&b)
            }
        }
    }
}

/// An [`IndexSorter`] over a numeric doc-values field.
///
/// Equivalent to `IndexSorter.IntSorter`, `LongSorter`, `FloatSorter` and
/// `DoubleSorter`, which differ only in the numeric type they compare.
///
/// **Divergence from Lucene 10.5.0.** Java writes four near-identical final
/// classes because each holds a typed `int[]`/`long[]`/`float[]`/`double[]`
/// array and a typed boxed `missingValue`. This port keeps the values in the
/// raw `long` form doc values store them in and selects the comparison with
/// [`NumericSortKind`], which is the only thing the four classes actually differ
/// by. The ordering produced is the same.
pub struct NumericSorter {
    provider_name: String,
    kind: NumericSortKind,
    missing_value: Option<i64>,
    reverse_mul: i32,
    values_provider: NumericDocValuesProvider,
}

impl NumericSorter {
    /// Creates a numeric sorter.
    pub fn new(
        provider_name: impl Into<String>,
        kind: NumericSortKind,
        missing_value: Option<i64>,
        reverse: bool,
        values_provider: NumericDocValuesProvider,
    ) -> Self {
        Self {
            provider_name: provider_name.into(),
            kind,
            missing_value,
            reverse_mul: if reverse { -1 } else { 1 },
            values_provider,
        }
    }
}

impl IndexSorter for NumericSorter {
    fn get_comparable_providers(
        &self,
        readers: &[Arc<dyn LeafReader>],
    ) -> Result<Vec<ComparableProvider>> {
        numeric_comparable_providers(
            readers,
            &self.values_provider,
            self.missing_value.unwrap_or(0),
        )
    }

    fn get_doc_comparator(&self, reader: &dyn LeafReader, max_doc: i32) -> Result<DocComparator> {
        let values =
            dense_numeric_values(reader, &self.values_provider, max_doc, self.missing_value)?;
        let kind = self.kind;
        let reverse_mul = self.reverse_mul;
        Ok(Box::new(move |a: i32, b: i32| {
            let ordering = kind.compare(values[a as usize], values[b as usize]);
            if reverse_mul < 0 {
                ordering.reverse()
            } else {
                ordering
            }
        }))
    }

    fn get_provider_name(&self) -> &str {
        &self.provider_name
    }
}

/// An [`IndexSorter`] over a sorted doc-values field, comparing by ordinal.
///
/// Equivalent to `IndexSorter.StringSorter`.
pub struct StringSorter {
    provider_name: String,
    missing_value_is_last: bool,
    reverse_mul: i32,
    values_provider: SortedDocValuesProvider,
}

impl StringSorter {
    /// Creates a string sorter. `missing_value_is_last` selects Lucene's
    /// `SortField.STRING_LAST` behaviour over `STRING_FIRST`.
    pub fn new(
        provider_name: impl Into<String>,
        missing_value_is_last: bool,
        reverse: bool,
        values_provider: SortedDocValuesProvider,
    ) -> Self {
        Self {
            provider_name: provider_name.into(),
            missing_value_is_last,
            reverse_mul: if reverse { -1 } else { 1 },
            values_provider,
        }
    }

    /// The ordinal a document with no value takes: `-1` sorts it first,
    /// `i32::MAX` sorts it last.
    fn missing_ord(&self) -> i32 {
        if self.missing_value_is_last {
            i32::MAX
        } else {
            -1
        }
    }
}

impl IndexSorter for StringSorter {
    /// Equivalent to `IndexSorter.StringSorter.getComparableProviders(List)`
    /// (`IndexSorter.java:478-509`): an `OrdinalMap` over every reader's sorted
    /// values makes the ordinals comparable across segments, and each
    /// document compares by its global ordinal.
    ///
    /// Java reads the ordinals map's terms and the per-document ordinals from
    /// the same `SortedDocValues` objects. This port's `OrdinalMap::build_sorted`
    /// consumes the values it walks, so each reader's values are asked for
    /// twice: once for the map, once for the documents. Java's provider never
    /// answers `null` (`DocValues.getSorted` hands back an empty instance), so
    /// a reader without the field contributes empty values here too.
    fn get_comparable_providers(
        &self,
        readers: &[Arc<dyn LeafReader>],
    ) -> Result<Vec<ComparableProvider>> {
        let sorted_values = |reader: &Arc<dyn LeafReader>| -> Result<Box<dyn SortedDocValues>> {
            Ok((self.values_provider)(reader.as_ref())?.unwrap_or_else(|| {
                Box::new(crate::index::doc_values::DocValues::empty_sorted())
                    as Box<dyn SortedDocValues>
            }))
        };
        let mut map_values = Vec::with_capacity(readers.len());
        for reader in readers {
            map_values.push(sorted_values(reader)?);
        }
        let ordinal_map = Arc::new(crate::index::doc_values::OrdinalMap::build_sorted(
            map_values,
            crate::util::packed::PackedInts::DEFAULT,
        )?);
        // `Integer.MAX_VALUE` for `SortField.STRING_LAST`, `Integer.MIN_VALUE`
        // otherwise.
        let missing_ord = if self.missing_value_is_last {
            i64::from(i32::MAX)
        } else {
            i64::from(i32::MIN)
        };
        let mut providers: Vec<ComparableProvider> = Vec::with_capacity(readers.len());
        for (reader_index, reader) in readers.iter().enumerate() {
            let reader_values = std::cell::RefCell::new(sorted_values(reader)?);
            let ordinal_map = Arc::clone(&ordinal_map);
            providers.push(Box::new(move |doc_id: i32| {
                let mut values = reader_values.borrow_mut();
                if values.advance_exact(doc_id)? {
                    // Translate the segment's ord to the global ord space.
                    let ord = values.ord_value()?;
                    Ok(ordinal_map
                        .get_global_ords(reader_index)
                        .get(i64::from(ord)))
                } else {
                    Ok(missing_ord)
                }
            }));
        }
        Ok(providers)
    }

    fn get_doc_comparator(&self, reader: &dyn LeafReader, max_doc: i32) -> Result<DocComparator> {
        let missing_ord = self.missing_ord();
        let mut ords = vec![missing_ord; max_doc.max(0) as usize];
        if let Some(mut sorted) = (self.values_provider)(reader)? {
            loop {
                let doc_id = sorted.next_doc()?;
                if doc_id == NO_MORE_DOCS {
                    break;
                }
                if doc_id >= 0 && (doc_id as usize) < ords.len() {
                    ords[doc_id as usize] = sorted.ord_value()?;
                }
            }
        }
        let reverse_mul = self.reverse_mul;
        Ok(Box::new(move |a: i32, b: i32| {
            let ordering = ords[a as usize].cmp(&ords[b as usize]);
            if reverse_mul < 0 {
                ordering.reverse()
            } else {
                ordering
            }
        }))
    }

    fn get_provider_name(&self) -> &str {
        &self.provider_name
    }
}

/// An [`IndexSorter`] over a binary doc-values field, comparing the raw bytes.
///
/// Equivalent to `org.apache.lucene.index.IndexSorter.BinarySorter`
/// (`IndexSorter.java:537-654`), which `BinarySortField.getIndexSorter()`
/// builds. Its values cannot be reduced to a `long`, so it implements
/// [`get_comparable_values`](IndexSorter::get_comparable_values) directly and
/// leaves [`get_comparable_providers`](IndexSorter::get_comparable_providers)
/// to the failing default.
pub struct BinarySorter {
    provider_name: String,
    reverse_mul: i32,
    /// Compares two values, either of which may be missing; the reverse flag
    /// is not applied by this comparator.
    comparator: BinaryComparator,
    values_provider: BinaryDocValuesProvider,
}

impl BinarySorter {
    /// Creates a binary sorter.
    ///
    /// Equivalent to `new BinarySorter(String, boolean, Comparator<BytesRef>,
    /// BinaryDocValuesProvider)`.
    pub fn new(
        provider_name: impl Into<String>,
        reverse: bool,
        comparator: BinaryComparator,
        values_provider: BinaryDocValuesProvider,
    ) -> Self {
        Self {
            provider_name: provider_name.into(),
            reverse_mul: if reverse { -1 } else { 1 },
            comparator,
            values_provider,
        }
    }
}

/// The anonymous `ComparableValues` `BinarySorter.getComparableValues`
/// returns: the sort key of each segment's current document, or `None` when
/// that document has no value.
///
/// Java keeps each head as the `BytesRef` the doc values returned, without
/// copying, since it stays valid until that segment's cursor moves; this port's
/// [`BinaryDocValues::binary_value`] returns an owned [`BytesRef`], which is
/// kept instead.
struct BinaryComparableValues {
    values: Vec<Box<dyn BinaryDocValues>>,
    heads: Vec<Option<BytesRef>>,
    last_doc: Vec<i32>,
    comparator: BinaryComparator,
}

impl ComparableValues for BinaryComparableValues {
    /// Equivalent to the anonymous class's `setTopValue(int, int)`: it only
    /// ever scans forward with `nextDoc()`, never seeks, so reads stay
    /// sequential even for block-compressed formats.
    fn set_top_value(&mut self, reader_index: usize, doc_id: i32) -> Result<()> {
        debug_assert!(
            doc_id >= self.last_doc[reader_index],
            "out of order access: docID={doc_id} < lastDoc={}",
            self.last_doc[reader_index]
        );
        self.last_doc[reader_index] = doc_id;
        let values = &mut self.values[reader_index];
        let mut doc = values.doc_id();
        while doc < doc_id {
            doc = values.next_doc()?;
        }
        self.heads[reader_index] = if doc == doc_id {
            Some(values.binary_value()?)
        } else {
            None
        };
        Ok(())
    }

    fn compare(&self, reader_index_a: usize, reader_index_b: usize) -> Ordering {
        (self.comparator)(
            self.heads[reader_index_a].as_ref().map(BytesRef::slice),
            self.heads[reader_index_b].as_ref().map(BytesRef::slice),
        )
    }
}

impl IndexSorter for BinarySorter {
    /// Equivalent to the `IndexSorter.getComparableProviders` default that
    /// `BinarySorter` inherits, which throws
    /// `UnsupportedOperationException(getClass().getName() + " does not
    /// implement getComparableProviders")`. Java's class name for this type is
    /// rendered here, which the trait default cannot do.
    fn get_comparable_providers(
        &self,
        readers: &[Arc<dyn LeafReader>],
    ) -> Result<Vec<ComparableProvider>> {
        let _ = readers;
        Err(LuceneError::UnsupportedOperation(
            "org.apache.lucene.index.IndexSorter$BinarySorter does not implement \
             getComparableProviders"
                .to_string(),
        ))
    }

    /// Equivalent to `BinarySorter.getComparableValues(List)`.
    fn get_comparable_values(
        &self,
        readers: &[Arc<dyn LeafReader>],
    ) -> Result<Box<dyn ComparableValues>> {
        let mut values = Vec::with_capacity(readers.len());
        for reader in readers {
            values.push((self.values_provider)(reader.as_ref())?);
        }
        Ok(Box::new(BinaryComparableValues {
            values,
            heads: vec![None; readers.len()],
            last_doc: vec![-1; readers.len()],
            comparator: Arc::clone(&self.comparator),
        }))
    }

    /// Equivalent to `BinarySorter.getDocComparator(LeafReader, int)`: the
    /// values are materialised once, concatenated into one buffer with a
    /// per-document offset array and a presence bitset, and compared in place.
    fn get_doc_comparator(&self, reader: &dyn LeafReader, max_doc: i32) -> Result<DocComparator> {
        let mut values = (self.values_provider)(reader)?;
        let max_doc = max_doc.max(0) as usize;
        let mut blob: Vec<u8> = Vec::with_capacity(max_doc.max(16));
        let mut offsets = vec![0usize; max_doc + 1];
        let mut present = FixedBitSet::new(max_doc);
        let mut doc = values.next_doc()?;
        for (i, offset) in offsets.iter_mut().enumerate().take(max_doc) {
            *offset = blob.len();
            if doc == i as i32 {
                let value = values.binary_value()?;
                blob.extend_from_slice(value.slice());
                present.set(i);
                doc = values.next_doc()?;
            }
        }
        offsets[max_doc] = blob.len();
        let comparator = Arc::clone(&self.comparator);
        let reverse_mul = self.reverse_mul;
        Ok(Box::new(move |doc_id1: i32, doc_id2: i32| {
            let value = |doc_id: i32| -> Option<&[u8]> {
                let doc_id = doc_id as usize;
                if present.get(doc_id) {
                    Some(&blob[offsets[doc_id]..offsets[doc_id + 1]])
                } else {
                    None
                }
            };
            let ordering = comparator(value(doc_id1), value(doc_id2));
            if reverse_mul < 0 {
                ordering.reverse()
            } else {
                ordering
            }
        }))
    }

    fn get_provider_name(&self) -> &str {
        &self.provider_name
    }
}

// -----------------------------------------------------------------------------
// Sorter
// -----------------------------------------------------------------------------

/// The `oldToNew` half of a [`SorterDocMap`], in either of the two forms
/// `Sorter.PackableDocMap` keeps it in.
///
/// Equivalent to the pair of fields `PackableDocMap.oldToNewUnpacked` (an
/// `int[]`) and `PackableDocMap.oldToNewPacked` (a monotonic
/// `PackedLongValues`), exactly one of which is non-null (`Sorter.java:268-283`).
#[derive(Debug, Clone)]
enum OldToNew {
    /// `oldToNewUnpacked`: the `int[]` the sort built.
    Unpacked(Arc<Vec<i32>>),
    /// `oldToNewPacked`: the same mapping compressed by
    /// [`PackableDocMap.pack()`](SorterDocMap::pack).
    Packed(Arc<MonotonicLongValues>),
}

/// Maps document numbers between the sorted and unsorted views of a segment.
///
/// Equivalent to `org.apache.lucene.index.Sorter.DocMap` as implemented by
/// `Sorter.PackableDocMap` (`Sorter.java:260-310`), the only implementation
/// `Sorter` produces: `newToOld` is a monotonic `PackedLongValues`, and
/// `oldToNew` is either the `int[]` the sort built or, once
/// [`pack`](Self::pack)ed, a monotonic `PackedLongValues` too.
///
/// Java also lets other classes subclass `Sorter.DocMap` (for example the
/// anonymous maps `SortingCodecReader` tests build); in this port every map is
/// a `SorterDocMap`, built by the sort or from explicit arrays with
/// [`from_old_to_new`](Self::from_old_to_new).
#[derive(Debug, Clone)]
pub struct SorterDocMap {
    new_to_old: Arc<MonotonicLongValues>,
    max_doc: i32,
    old_to_new: OldToNew,
}

impl SorterDocMap {
    /// Builds the unpacked form from the two arrays.
    ///
    /// Equivalent to the private constructor
    /// `PackableDocMap(int[] oldToNewUnpacked, PackedLongValues newToOld, int maxDoc)`.
    fn unpacked(old_to_new: Vec<i32>, new_to_old: MonotonicLongValues, max_doc: i32) -> Self {
        Self {
            new_to_old: Arc::new(new_to_old),
            max_doc,
            old_to_new: OldToNew::Unpacked(Arc::new(old_to_new)),
        }
    }

    /// Returns the new document number of `doc_id`.
    ///
    /// Equivalent to `PackableDocMap.oldToNew(int)`.
    pub fn old_to_new(&self, doc_id: i32) -> i32 {
        match &self.old_to_new {
            OldToNew::Unpacked(old_to_new) => old_to_new[doc_id as usize],
            OldToNew::Packed(old_to_new) => old_to_new.get(i64::from(doc_id)) as i32,
        }
    }

    /// Returns the old document number of `doc_id`.
    ///
    /// Equivalent to `PackableDocMap.newToOld(int)`.
    pub fn new_to_old(&self, doc_id: i32) -> i32 {
        self.new_to_old.get(i64::from(doc_id)) as i32
    }

    /// Returns how many documents the map covers.
    ///
    /// Equivalent to `PackableDocMap.size()`.
    pub fn size(&self) -> i32 {
        self.max_doc
    }

    /// Returns whether `oldToNew` is held packed.
    pub fn is_packed(&self) -> bool {
        matches!(self.old_to_new, OldToNew::Packed(_))
    }

    /// Packs `oldToNew` into a monotonic `PackedLongValues`; a map that is
    /// already packed is returned as is.
    ///
    /// Equivalent to `PackableDocMap.pack()` (`Sorter.java:295-305`).
    ///
    /// # Errors
    ///
    /// Propagates the error building the packed values raises.
    pub fn pack(self) -> Result<Self> {
        match &self.old_to_new {
            OldToNew::Packed(_) => Ok(self),
            OldToNew::Unpacked(old_to_new) => {
                let packed = build_monotonic(
                    old_to_new
                        .iter()
                        .take(self.max_doc as usize)
                        .map(|&d| i64::from(d)),
                )?;
                Ok(Self {
                    new_to_old: self.new_to_old,
                    max_doc: self.max_doc,
                    old_to_new: OldToNew::Packed(Arc::new(packed)),
                })
            }
        }
    }

    /// Checks that the two directions invert each other.
    ///
    /// Equivalent to `Sorter.isConsistent(DocMap)`, which Java only calls from
    /// assertions.
    pub fn is_consistent(&self) -> bool {
        (0..self.size()).all(|i| {
            let new_id = self.old_to_new(i);
            new_id >= 0 && new_id < self.size() && self.new_to_old(new_id) == i
        })
    }
}

/// Fills `PackedLongValues.monotonicBuilder(PackedInts.COMPACT)` with `values`
/// and builds it.
fn build_monotonic(values: impl Iterator<Item = i64>) -> Result<MonotonicLongValues> {
    let mut builder = PackedLongValues::monotonic_builder(
        PackedLongValues::DEFAULT_PAGE_SIZE as usize,
        PackedInts::COMPACT,
    )?;
    for value in values {
        builder.add(value)?;
    }
    builder.build()
}

/// The `TimSorter` `Sorter.sort(int, DocComparator)` sorts the document ids
/// with.
///
/// Equivalent to the private `Sorter.DocValueSorter extends TimSorter`
/// (`Sorter.java:79-125`): `maxTempSlots = docs.length / 64` and a `tmp` array
/// of the same length.
struct DocValueSorter<'a> {
    docs: &'a mut [i32],
    comparator: &'a DocComparator,
    tmp: Vec<i32>,
    pivot_index: usize,
    state: TimSorterState,
}

impl<'a> DocValueSorter<'a> {
    /// Equivalent to `DocValueSorter(int[], DocComparator)`.
    fn new(docs: &'a mut [i32], comparator: &'a DocComparator) -> Self {
        let max_temp_slots = docs.len() / 64;
        Self {
            docs,
            comparator,
            tmp: vec![0i32; max_temp_slots],
            pivot_index: 0,
            state: TimSorterState::new(max_temp_slots),
        }
    }
}

impl PivotOps for DocValueSorter<'_> {
    fn swap(&mut self, i: usize, j: usize) {
        self.docs.swap(i, j);
    }

    /// `Sorter.setPivot(int)`, which `DocValueSorter` inherits.
    fn set_pivot(&mut self, i: usize) {
        self.pivot_index = i;
    }

    /// `Sorter.comparePivot(int)`, which `DocValueSorter` inherits.
    fn compare_pivot(&mut self, j: usize) -> i32 {
        self.compare(self.pivot_index, j)
    }

    fn compare(&mut self, i: usize, j: usize) -> i32 {
        ordering_to_int((self.comparator)(self.docs[i], self.docs[j]))
    }
}

impl UtilSorter for DocValueSorter<'_> {
    fn sort(&mut self, from: usize, to: usize) {
        self.tim_sort(from, to);
    }

    fn do_rotate(&mut self, lo: usize, mid: usize, hi: usize) {
        self.tim_do_rotate(lo, mid, hi);
    }
}

impl TimSorter for DocValueSorter<'_> {
    fn tim_state(&mut self) -> &mut TimSorterState {
        &mut self.state
    }

    fn copy(&mut self, src: usize, dest: usize) {
        self.docs[dest] = self.docs[src];
    }

    fn save(&mut self, i: usize, len: usize) {
        self.tmp[..len].copy_from_slice(&self.docs[i..i + len]);
    }

    fn restore(&mut self, i: usize, j: usize) {
        self.docs[j] = self.tmp[i];
    }

    fn compare_saved(&mut self, i: usize, j: usize) -> i32 {
        ordering_to_int((self.comparator)(self.tmp[i], self.docs[j]))
    }
}

/// Sorts the documents of a segment.
///
/// Equivalent to `org.apache.lucene.index.Sorter`.
pub struct Sorter {
    id: String,
}

impl Sorter {
    /// Creates a sorter identified by `id`, which is the string form of the sort
    /// it applies.
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }

    /// Returns the identifier of the sort this sorter applies.
    ///
    /// Equivalent to `Sorter.getID()`.
    pub fn get_id(&self) -> &str {
        &self.id
    }

    /// Sorts `max_doc` documents by `comparator`, returning `None` when they are
    /// already in order. The map is returned unpacked.
    ///
    /// Equivalent to the private `Sorter.sort(int, DocComparator)`
    /// (`Sorter.java:128-169`): the document ids are sorted with
    /// `DocValueSorter`, a `TimSorter` with `maxDoc / 64` temporary slots;
    /// `newToOld` is stored in a monotonic `PackedLongValues` and inverted into
    /// the `int[]` `oldToNew`.
    ///
    /// # Panics
    ///
    /// Only when building the monotonic values fails, which cannot happen with
    /// the default page size; Java's builder throws nothing there either.
    pub fn sort(max_doc: i32, comparator: &DocComparator) -> Option<SorterDocMap> {
        // check if the index is sorted
        let mut sorted = true;
        for i in 1..max_doc {
            if comparator(i - 1, i) == Ordering::Greater {
                sorted = false;
                break;
            }
        }
        if sorted {
            return None;
        }

        // sort doc IDs
        let mut docs: Vec<i32> = (0..max_doc).collect();
        {
            let mut sorter = DocValueSorter::new(&mut docs, comparator);
            // It can be common to sort a reader, add docs, sort it again, ...
            // and in that case timSort can save a lot of time
            UtilSorter::sort(&mut sorter, 0, max_doc as usize); // docs is now the newToOld mapping
        }

        // The reason why we use MonotonicAppendingLongBuffer here is that it
        // wastes very little memory if the index is in random order but can
        // save a lot of memory if the index is already "almost" sorted
        let new_to_old = build_monotonic(docs.iter().map(|&d| i64::from(d)))
            .expect("INVARIANT: the default page size is a valid monotonic page size");

        // invert the docs mapping:
        for i in 0..max_doc {
            docs[new_to_old.get(i64::from(i)) as usize] = i;
        } // docs is now the oldToNew mapping

        Some(SorterDocMap::unpacked(docs, new_to_old, max_doc))
    }

    /// Sorts `max_doc` documents by several comparators applied in order,
    /// returning `None` when they are already in order. The map is returned
    /// packed.
    ///
    /// Equivalent to `Sorter.sort(int, DocComparator[])` (`Sorter.java:213-216`).
    ///
    /// # Errors
    ///
    /// Propagates the error packing the map raises.
    pub fn sort_multi(
        max_doc: i32,
        comparators: Vec<DocComparator>,
    ) -> Result<Option<SorterDocMap>> {
        Self::sort_and_leave_unpacked(max_doc, comparators)
            .map(SorterDocMap::pack)
            .transpose()
    }

    /// Sorts `max_doc` documents by several comparators applied in order, then
    /// by document id, returning `None` when they are already in order. The
    /// map is returned unpacked.
    ///
    /// Equivalent to `Sorter.sortAndLeaveUnpacked(int, DocComparator[])`
    /// (`Sorter.java:218-231`), which `IndexingChain.maybeSortSegment` calls so
    /// that the flush can read `oldToNew` from the `int[]`.
    pub fn sort_and_leave_unpacked(
        max_doc: i32,
        comparators: Vec<DocComparator>,
    ) -> Option<SorterDocMap> {
        let combined: DocComparator = Box::new(move |doc_id1: i32, doc_id2: i32| {
            for comparator in &comparators {
                let comp = comparator(doc_id1, doc_id2);
                if comp != Ordering::Equal {
                    return comp;
                }
            }
            doc_id1.cmp(&doc_id2) // docid order tiebreak
        });
        Sorter::sort(max_doc, &combined)
    }
}

// -----------------------------------------------------------------------------
// MultiSorter
// -----------------------------------------------------------------------------

/// One reader's cursor while several sorted readers are merged.
struct LeafAndDocId {
    reader_index: usize,
    doc_id: i32,
    max_doc: i32,
    live_docs: Option<Box<dyn crate::util::Bits>>,
}

/// The `lessThan` of the anonymous `PriorityQueue<LeafAndDocID>` of
/// `MultiSorter.sort` (`MultiSorter.java:87-104`): the sort fields' cached
/// values in order, each reversed by its `reverseMul`, then the reader index,
/// then the document id.
struct LeafAndDocIdLess<'a> {
    comparables: &'a std::cell::RefCell<Vec<Box<dyn ComparableValues>>>,
    reverse_muls: &'a [i32],
}

impl crate::util::extra::PriorityQueueComparator<LeafAndDocId> for LeafAndDocIdLess<'_> {
    fn less_than(&self, a: &LeafAndDocId, b: &LeafAndDocId) -> bool {
        let comparables = self.comparables.borrow();
        for (i, comparable) in comparables.iter().enumerate() {
            let cmp = comparable.compare(a.reader_index, b.reader_index);
            if cmp != Ordering::Equal {
                return self.reverse_muls[i] * ordering_to_int(cmp) < 0;
            }
        }
        // tie-break by docID natural order:
        if a.reader_index != b.reader_index {
            a.reader_index < b.reader_index
        } else {
            a.doc_id < b.doc_id
        }
    }
}

/// Interleaves several already-sorted segments into one sorted order.
///
/// Equivalent to `org.apache.lucene.index.MultiSorter`.
pub struct MultiSorter;

impl MultiSorter {
    /// Caches, for every sort field, the value of document `doc_id` of segment
    /// `reader_index`, so that the priority queue can compare it. When the
    /// segment uses document blocks, the value of the parent (last) document of
    /// `doc_id`'s block is used instead.
    ///
    /// Equivalent to the private static
    /// `MultiSorter.setComparableValues(IndexSorter.ComparableValues[], BitSet, int, int)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading a document's sort value raised.
    fn set_comparable_values(
        comparables: &mut [Box<dyn ComparableValues>],
        parents: Option<&dyn crate::util::bit_set::BitSet>,
        reader_index: usize,
        doc_id: i32,
    ) -> Result<()> {
        let effective_doc_id = match parents {
            None => doc_id,
            Some(parents) => parents.next_set_bit(doc_id),
        };
        for comparable in comparables.iter_mut() {
            comparable.set_top_value(reader_index, effective_doc_id)?;
        }
        Ok(())
    }

    /// Returns one doc map per reader, or `None` when concatenating the readers
    /// already produces the sorted order.
    ///
    /// Equivalent to `MultiSorter.sort(Sort, List<CodecReader>)`.
    ///
    /// A segment that uses document blocks sorts each block by the value of
    /// its parent (last) document, found through the parent field
    /// (`MultiSorter.java:47-66`).
    ///
    /// The remapped document numbers of each reader are kept in a monotonic
    /// `PackedLongValues`, filled in the merged order (`MultiSorter.java:107-152`).
    ///
    /// **Divergence from Lucene 10.5.0.** Java queues every reader, including
    /// one with `maxDoc == 0`, positioned on document 0
    /// (`MultiSorter.java:107-115`). When that reader reaches the top of the
    /// queue it adds one entry to its builder and, having no live docs,
    /// increments `mappedDocID` (`MultiSorter.java:128-131`), so every document
    /// that surfaces after it is mapped one past its slot and the last one to
    /// the merged segment's `maxDoc`. The case is reachable: `addIndexes(
    /// CodecReader...)` does not filter empty readers
    /// (`IndexWriter.java:3264-3296`), and `MergeState.buildDocMaps` hands
    /// them to `MultiSorter.sort` (`MergeState.java:214-224`). Observed against
    /// `lucene-core-10.5.0.jar` with readers `[{1, 3}, empty, {2}]` sorted by a
    /// long field: Java maps the three documents to `1, 3` and `2`, leaving
    /// slot 0 to the empty reader and mapping a document to 3 in a three-
    /// document segment. Reproducing that would write document numbers out of
    /// the segment's range, so this port does not queue an empty reader and
    /// maps the documents to `0, 2` and `1`. Readers that are not empty are
    /// merged exactly as Java merges them.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::CorruptIndex`] when a segment created by Lucene
    /// 10 or later has blocks but the index declares no parent field, and
    /// propagates whatever reading the sort values raises.
    pub fn sort(
        sorters: &[Arc<dyn IndexSorter>],
        reverse_muls: &[i32],
        readers: &[Arc<dyn LeafReader>],
    ) -> Result<Option<Vec<crate::index::merge::DocMap>>> {
        if sorters.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "cannot sort an index with no sort fields".to_string(),
            ));
        }

        let leaf_count = readers.len();

        // Per-reader parent bitset (used when index sorting with document
        // blocks). Shared across all sort fields; `None` when the reader does
        // not use blocks. A document's sort value is the value of the parent
        // (last) document of its block.
        let mut parents: Vec<Option<crate::util::FixedBitSet>> = Vec::with_capacity(leaf_count);
        for reader in readers {
            let field_infos = reader.get_field_infos();
            let meta_data = reader.get_meta_data();
            let mut reader_parents = None;
            if meta_data.has_blocks() {
                match field_infos.get_parent_field() {
                    Some(parent_field) => {
                        let mut parent_docs = reader
                            .get_numeric_doc_values(parent_field)?
                            .ok_or_else(|| {
                                LuceneError::illegal_state(format!(
                                    "parent field: {parent_field} must be present if index \
                                     sorting is used with blocks"
                                ))
                            })?;
                        // `BitSet.of(parentDocs, codecReader.maxDoc())`.
                        let mut bits = crate::util::FixedBitSet::new(reader.max_doc() as usize);
                        loop {
                            let doc = parent_docs.next_doc()?;
                            if doc == NO_MORE_DOCS {
                                break;
                            }
                            bits.set(doc as usize);
                        }
                        reader_parents = Some(bits);
                    }
                    None if meta_data.created_version_major() >= 10 => {
                        return Err(LuceneError::corrupt_index(
                            format!(
                                "parent field is not set but the index has blocks and uses \
                                 index sorting. indexCreatedVersionMajor: {}",
                                meta_data.created_version_major()
                            ),
                            "IndexingChain",
                        ));
                    }
                    None => {}
                }
            }
            parents.push(reader_parents);
        }

        let mut comparables: Vec<Box<dyn ComparableValues>> = Vec::with_capacity(sorters.len());
        for sorter in sorters {
            comparables.push(sorter.get_comparable_values(readers)?);
        }

        let comparables = std::cell::RefCell::new(comparables);
        let mut queue = crate::util::extra::PriorityQueue::new(
            leaf_count,
            LeafAndDocIdLess {
                comparables: &comparables,
                reverse_muls,
            },
        )?;

        // `builders[i] = PackedLongValues.monotonicBuilder(PackedInts.COMPACT)`
        // (`MultiSorter.java:107-114`).
        let mut builders = Vec::with_capacity(leaf_count);
        for (reader_index, reader) in readers.iter().enumerate() {
            let max_doc = reader.max_doc();
            // See the divergence note above: Java queues an empty reader too.
            if max_doc != 0 {
                // `setComparableValues(comparables, parents[i], i, leaf.docID)`,
                // with `leaf.docID` still 0, then `queue.add(leaf)`.
                Self::set_comparable_values(
                    &mut comparables.borrow_mut(),
                    parents[reader_index]
                        .as_ref()
                        .map(|bits| bits as &dyn crate::util::bit_set::BitSet),
                    reader_index,
                    0,
                )?;
                queue.add(LeafAndDocId {
                    reader_index,
                    doc_id: 0,
                    max_doc,
                    live_docs: reader.get_live_docs(),
                });
            }
            builders.push(PackedLongValues::monotonic_builder(
                PackedLongValues::DEFAULT_PAGE_SIZE as usize,
                PackedInts::COMPACT,
            )?);
        }

        // merge sort:
        let mut mapped_doc_id = 0i32;
        let mut last_reader_index = 0usize;
        let mut is_sorted = true;
        while queue.size() != 0 {
            let top = queue.top_mut().expect("INVARIANT: the queue is not empty");
            if last_reader_index > top.reader_index {
                // merge sort is needed
                is_sorted = false;
            }
            last_reader_index = top.reader_index;
            builders[top.reader_index].add(i64::from(mapped_doc_id))?;
            let alive = match &top.live_docs {
                Some(live_docs) => live_docs.get(top.doc_id as usize),
                None => true,
            };
            if alive {
                mapped_doc_id += 1;
            }
            top.doc_id += 1;
            if top.doc_id < top.max_doc {
                let (reader_index, next) = (top.reader_index, top.doc_id);
                Self::set_comparable_values(
                    &mut comparables.borrow_mut(),
                    parents[reader_index]
                        .as_ref()
                        .map(|bits| bits as &dyn crate::util::bit_set::BitSet),
                    reader_index,
                    next,
                )?;
                queue.update_top();
            } else {
                queue.pop();
            }
        }

        if is_sorted {
            return Ok(None);
        }

        let mut doc_maps: Vec<crate::index::merge::DocMap> = Vec::with_capacity(leaf_count);
        for (builder, reader) in builders.into_iter().zip(readers.iter()) {
            let remapped = builder.build()?;
            let live_docs = reader.get_live_docs();
            doc_maps.push(Arc::new(move |doc_id: i32| {
                let alive = match &live_docs {
                    Some(bits) => bits.get(doc_id as usize),
                    None => true,
                };
                if alive {
                    remapped.get(i64::from(doc_id)) as i32
                } else {
                    -1
                }
            }));
        }
        Ok(Some(doc_maps))
    }
}

// -----------------------------------------------------------------------------
// SortFieldProvider
// -----------------------------------------------------------------------------

/// Serialises and deserialises a `SortField` so an index sort survives a commit.
///
/// Equivalent to `org.apache.lucene.index.SortFieldProvider`
/// (`index/SortFieldProvider.java:28-116`), an abstract class implementing
/// `NamedSPILoader.NamedSPI`: [`NamedSPI::get_name`] is the name the provider
/// is registered and serialised under.
///
/// Lucene core ships four providers, listed in its
/// `META-INF/services/org.apache.lucene.index.SortFieldProvider`:
/// [`SortField`'s](crate::search::sort::Provider),
/// [`SortedNumericSortField`'s](crate::search::sorted_numeric_sort_field::Provider),
/// [`SortedSetSortField`'s](crate::search::sorted_set_sort_field::Provider) and
/// [`BinarySortField`'s](crate::search::binary_sort_field::Provider).
pub trait SortFieldProvider: NamedSPI + Send + Sync {
    /// Reads a sort field from `input`.
    ///
    /// Equivalent to `SortFieldProvider.readSortField(DataInput)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading `input` raises.
    fn read_sort_field(&self, input: &mut dyn crate::store::DataInput) -> Result<SortField>;

    /// Writes `sort_field` to `output`, without the provider name, which the
    /// caller writes first.
    ///
    /// Equivalent to `SortFieldProvider.writeSortField(SortField, DataOutput)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `sort_field` is not of the
    /// provider's kind (Java's `assert sf instanceof ...`), and propagates
    /// whatever writing raises.
    fn write_sort_field(
        &self,
        sort_field: &SortField,
        output: &mut dyn crate::store::DataOutput,
    ) -> Result<()>;
}

/// The class name `NamedSPILoader` reports in its lookup error.
const SORT_FIELD_PROVIDER_CLASS: &str = "org.apache.lucene.index.SortFieldProvider";

/// The loader behind [`SortFieldProvider`]'s static entry points.
///
/// Equivalent to `SortFieldProvider.Holder.LOADER`, which Java hides in a
/// holder class so the providers are only discovered when first needed. It is
/// seeded with the four providers `lucene-core` lists in
/// `META-INF/services/org.apache.lucene.index.SortFieldProvider`, in the order
/// of that file, which is what Java's service loader registers. Rust has no
/// run-time service discovery, so a provider defined outside the crate is added
/// with [`reload_sort_field_providers`].
static SORT_FIELD_PROVIDERS: std::sync::LazyLock<
    std::sync::RwLock<NamedSPILoader<dyn SortFieldProvider>>,
> = std::sync::LazyLock::new(|| {
    let providers: [Arc<dyn SortFieldProvider>; 4] = [
        Arc::new(crate::search::sort::Provider::new()),
        Arc::new(crate::search::sorted_numeric_sort_field::Provider::new()),
        Arc::new(crate::search::sorted_set_sort_field::Provider::new()),
        Arc::new(crate::search::binary_sort_field::Provider::new()),
    ];
    std::sync::RwLock::new(
        NamedSPILoader::with_providers(SORT_FIELD_PROVIDER_CLASS, providers)
            .expect("INVARIANT: the built-in provider names are legal service names"),
    )
});

/// Looks a sort-field provider up by name.
///
/// Equivalent to `SortFieldProvider.forName(String)`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when no provider is registered
/// under `name`, with `NamedSPILoader.lookup`'s message.
pub fn sort_field_provider_for_name(name: &str) -> Result<Arc<dyn SortFieldProvider>> {
    SORT_FIELD_PROVIDERS
        .read()
        .expect("INVARIANT: the sort-field provider lock is never poisoned")
        .lookup(name)
}

/// Returns the names of every registered sort-field provider, in registration
/// order.
///
/// Equivalent to `SortFieldProvider.availableSortFieldProviders()`.
pub fn available_sort_field_providers() -> Vec<String> {
    SORT_FIELD_PROVIDERS
        .read()
        .expect("INVARIANT: the sort-field provider lock is never poisoned")
        .available_services()
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// Adds `providers` to the global loader; a name already registered keeps its
/// first provider.
///
/// Equivalent to `SortFieldProvider.reloadSortFieldProviders(ClassLoader)`,
/// which rescans a classloader for providers it has not seen. Rust has no
/// run-time service discovery, so the caller supplies the providers.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when a provider's name is not a
/// legal service name.
pub fn reload_sort_field_providers(
    providers: impl IntoIterator<Item = Arc<dyn SortFieldProvider>>,
) -> Result<()> {
    SORT_FIELD_PROVIDERS
        .write()
        .expect("INVARIANT: the sort-field provider lock is never poisoned")
        .reload(providers)
}

/// Writes `sort_field` with the provider its index sorter names.
///
/// Equivalent to the static `SortFieldProvider.write(SortField, DataOutput)`
/// (`SortFieldProvider.java:79-86`), which does not write the provider name:
/// `Lucene99SegmentInfoFormat.write` writes it first.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the sort field has no index
/// sorter ("Cannot serialize sort field ...") or names an unregistered
/// provider, and propagates whatever the provider raises.
pub fn write_sort_field(
    sort_field: &SortField,
    output: &mut dyn crate::store::DataOutput,
) -> Result<()> {
    let Some(sorter) = sort_field.get_index_sorter() else {
        return Err(LuceneError::IllegalArgument(format!(
            "Cannot serialize sort field {sort_field}"
        )));
    };
    let provider = sort_field_provider_for_name(sorter.get_provider_name())?;
    provider.write_sort_field(sort_field, output)
}

impl std::fmt::Display for Sorter {
    /// Port of `Sorter.toString()`, which is `getID()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.get_id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A comparator over a value per document, so ties are easy to set up.
    fn by_value(values: Vec<i64>) -> DocComparator {
        Box::new(move |a: i32, b: i32| values[a as usize].cmp(&values[b as usize]))
    }

    #[test]
    fn sorted_documents_need_no_map() {
        assert!(Sorter::sort_and_leave_unpacked(5, vec![by_value(vec![1, 1, 2, 3, 3])]).is_none());
        assert!(Sorter::sort_multi(0, vec![by_value(vec![])])
            .unwrap()
            .is_none());
    }

    #[test]
    fn ties_keep_document_order_and_both_forms_agree() {
        // Many ties over a long, partly ordered run, so the `TimSorter` merges
        // runs with its temporary slots (`maxDoc / 64` of them).
        let max_doc = 1_000;
        let values: Vec<i64> = (0..max_doc).map(|doc| ((doc * 7919) % 13) as i64).collect();
        let unpacked =
            Sorter::sort_and_leave_unpacked(max_doc, vec![by_value(values.clone())]).unwrap();
        assert!(!unpacked.is_packed());
        assert!(unpacked.is_consistent());
        let mut expected: Vec<i32> = (0..max_doc).collect();
        expected.sort_by_key(|&doc| (values[doc as usize], doc));
        for (new_doc, &old_doc) in expected.iter().enumerate() {
            assert_eq!(unpacked.new_to_old(new_doc as i32), old_doc);
            assert_eq!(unpacked.old_to_new(old_doc), new_doc as i32);
        }

        let packed = Sorter::sort_multi(max_doc, vec![by_value(values)])
            .unwrap()
            .unwrap();
        assert!(packed.is_packed());
        for doc in 0..max_doc {
            assert_eq!(packed.old_to_new(doc), unpacked.old_to_new(doc));
            assert_eq!(packed.new_to_old(doc), unpacked.new_to_old(doc));
        }
        assert_eq!(packed.size(), max_doc);
    }

    #[test]
    fn the_sort_field_providers_load_through_the_named_spi_loader() {
        // `META-INF/services/org.apache.lucene.index.SortFieldProvider` of
        // `lucene-core`, in file order.
        let available = available_sort_field_providers();
        assert_eq!(
            &available[..4],
            &[
                "SortField",
                "SortedNumericSortField",
                "SortedSetSortField",
                "BinarySortField"
            ]
        );
        assert_eq!(
            sort_field_provider_for_name("SortedSetSortField")
                .unwrap()
                .get_name(),
            "SortedSetSortField"
        );
        let error = sort_field_provider_for_name("NoSuchProvider")
            .err()
            .expect("an unknown name is refused");
        assert!(
            error.to_string().contains(
                "An SPI class of type org.apache.lucene.index.SortFieldProvider with name 'NoSuchProvider' does not exist"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn a_sort_field_is_written_by_the_provider_its_sorter_names() {
        use crate::search::sorted_numeric_selector::SortedNumericSelectorType;
        use crate::search::sorted_numeric_sort_field::SortedNumericSortField;
        use crate::search::{read_sort, write_sort, Sort, SortFieldType};
        use crate::store::{ByteArrayDataInput, VecDataOutput};

        let field = SortedNumericSortField::with_selector(
            "multi",
            SortFieldType::Long,
            true,
            SortedNumericSelectorType::MAX,
        )
        .unwrap()
        .into_sort_field();
        assert_eq!(
            field.get_index_sorter().unwrap().get_provider_name(),
            "SortedNumericSortField"
        );
        let sort = Sort::new_fields(vec![field.clone()]).unwrap();
        let mut out = VecDataOutput::new();
        write_sort(&mut out, &sort).unwrap();
        let bytes = out.into_inner();
        let mut input = ByteArrayDataInput::new(&bytes);
        let read = read_sort(&mut input).unwrap().unwrap();
        assert_eq!(read.fields(), &[field]);
    }
}
