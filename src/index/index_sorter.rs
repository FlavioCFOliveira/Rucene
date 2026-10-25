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

/// Maps document numbers between the sorted and unsorted views of a segment.
///
/// Equivalent to `org.apache.lucene.index.Sorter.DocMap`.
///
/// **Divergence from Lucene 10.5.0.** Java holds `oldToNew` and `newToOld` in
/// `PackedLongValues`, which compresses an almost-sorted permutation to a
/// fraction of its size. `PackedLongValues` is not ported yet, so this port
/// keeps both directions in plain `Vec<i32>`, as `norms_writer.rs` already does
/// for the same reason. The mapping is identical; only the memory footprint
/// differs.
#[derive(Debug, Clone)]
pub struct SorterDocMap {
    old_to_new: Vec<i32>,
    new_to_old: Vec<i32>,
}

impl SorterDocMap {
    /// Returns the new document number of `doc_id`.
    ///
    /// Equivalent to `Sorter.DocMap.oldToNew(int)`.
    pub fn old_to_new(&self, doc_id: i32) -> i32 {
        self.old_to_new[doc_id as usize]
    }

    /// Returns the old document number of `doc_id`.
    ///
    /// Equivalent to `Sorter.DocMap.newToOld(int)`.
    pub fn new_to_old(&self, doc_id: i32) -> i32 {
        self.new_to_old[doc_id as usize]
    }

    /// Returns how many documents the map covers.
    pub fn size(&self) -> i32 {
        self.old_to_new.len() as i32
    }

    /// Checks that the two directions invert each other.
    ///
    /// Equivalent to `Sorter.isConsistent(DocMap)`, which Java only calls from
    /// assertions.
    pub fn is_consistent(&self) -> bool {
        (0..self.size()).all(|doc| {
            self.new_to_old(self.old_to_new(doc)) == doc
                && self.old_to_new(self.new_to_old(doc)) == doc
        })
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
    /// already in order.
    ///
    /// Equivalent to the private `Sorter.sort(int, DocComparator)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java sorts with `TimSorter`, which is
    /// not ported. This port uses the standard library's stable sort, which is
    /// also an adaptive stable merge sort and so produces the same permutation
    /// for the same comparator.
    pub fn sort(max_doc: i32, comparator: &DocComparator) -> Option<SorterDocMap> {
        let n = max_doc.max(0) as usize;
        let already_sorted =
            (1..n).all(|i| comparator(i as i32 - 1, i as i32) != Ordering::Greater);
        if already_sorted {
            return None;
        }

        let mut docs: Vec<i32> = (0..n as i32).collect();
        docs.sort_by(|a, b| comparator(*a, *b));

        // `docs` is now the newToOld mapping; invert it for oldToNew.
        let new_to_old = docs;
        let mut old_to_new = vec![0i32; n];
        for (new_doc, &old_doc) in new_to_old.iter().enumerate() {
            old_to_new[old_doc as usize] = new_doc as i32;
        }

        Some(SorterDocMap {
            old_to_new,
            new_to_old,
        })
    }

    /// Sorts `max_doc` documents by several comparators applied in order,
    /// returning `None` when they are already in order.
    ///
    /// Equivalent to `Sorter.sort(int, DocComparator[])` and to the
    /// `Sorter.sortAndLeaveUnpacked(int, DocComparator[])` it delegates to.
    /// Java splits the two because `sort` packs the map afterwards, through
    /// `PackableDocMap.pack()`; this port's [`SorterDocMap`] is the unpacked
    /// form and is never packed, so the two collapse into one function.
    pub fn sort_multi(max_doc: i32, comparators: Vec<DocComparator>) -> Option<SorterDocMap> {
        let combined: DocComparator = Box::new(move |a: i32, b: i32| {
            for comparator in &comparators {
                let ordering = comparator(a, b);
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            Ordering::Equal
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
    /// **Divergence from Lucene 10.5.0.** Java keeps the remapped document
    /// numbers in `PackedLongValues`, which is not ported, so the remapping is
    /// a plain `Vec<i32>`; the numbers are the same.
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
                                LuceneError::IllegalState(format!(
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

        // One cursor per reader, each positioned on its first document.
        let mut cursors: Vec<LeafAndDocId> = Vec::with_capacity(leaf_count);
        for (reader_index, reader) in readers.iter().enumerate() {
            let max_doc = reader.max_doc();
            if max_doc == 0 {
                continue;
            }
            // Java: `setComparableValues(comparables, parents[i], i, leaf.docID)`
            // (`MultiSorter.java:112`), with `leaf.docID` still 0.
            Self::set_comparable_values(
                &mut comparables,
                parents[reader_index]
                    .as_ref()
                    .map(|bits| bits as &dyn crate::util::bit_set::BitSet),
                reader_index,
                0,
            )?;
            cursors.push(LeafAndDocId {
                reader_index,
                doc_id: 0,
                max_doc,
                live_docs: reader.get_live_docs(),
            });
        }

        let mut remapped: Vec<Vec<i32>> = readers
            .iter()
            .map(|reader| vec![-1i32; reader.max_doc().max(0) as usize])
            .collect();

        let mut mapped_doc_id = 0i32;
        let mut last_reader_index = 0usize;
        let mut is_sorted = true;

        while !cursors.is_empty() {
            // Pick the smallest cursor under the comparators, breaking ties by
            // reader index then document id, as Java's priority queue does.
            let mut top = 0usize;
            for candidate in 1..cursors.len() {
                let mut less = false;
                let mut decided = false;
                for (i, comparable) in comparables.iter().enumerate() {
                    let cmp = comparable
                        .compare(cursors[candidate].reader_index, cursors[top].reader_index);
                    if cmp != Ordering::Equal {
                        let signed = if reverse_muls[i] < 0 {
                            cmp.reverse()
                        } else {
                            cmp
                        };
                        less = signed == Ordering::Less;
                        decided = true;
                        break;
                    }
                }
                if !decided {
                    less = if cursors[candidate].reader_index != cursors[top].reader_index {
                        cursors[candidate].reader_index < cursors[top].reader_index
                    } else {
                        cursors[candidate].doc_id < cursors[top].doc_id
                    };
                }
                if less {
                    top = candidate;
                }
            }

            let reader_index = cursors[top].reader_index;
            if last_reader_index > reader_index {
                // The readers interleave, so a real merge sort is needed.
                is_sorted = false;
            }
            last_reader_index = reader_index;

            let doc_id = cursors[top].doc_id;
            remapped[reader_index][doc_id as usize] = mapped_doc_id;
            let alive = match &cursors[top].live_docs {
                Some(live_docs) => live_docs.get(doc_id as usize),
                None => true,
            };
            if alive {
                mapped_doc_id += 1;
            }

            cursors[top].doc_id += 1;
            if cursors[top].doc_id < cursors[top].max_doc {
                let next = cursors[top].doc_id;
                // Java: `setComparableValues(comparables, parents[top.readerIndex],
                // top.readerIndex, top.docID)` (`MultiSorter.java:134`).
                Self::set_comparable_values(
                    &mut comparables,
                    parents[reader_index]
                        .as_ref()
                        .map(|bits| bits as &dyn crate::util::bit_set::BitSet),
                    reader_index,
                    next,
                )?;
            } else {
                cursors.remove(top);
            }
        }

        if is_sorted {
            return Ok(None);
        }

        let mut doc_maps: Vec<crate::index::merge::DocMap> = Vec::with_capacity(leaf_count);
        for (index, reader) in readers.iter().enumerate() {
            let mapping = std::mem::take(&mut remapped[index]);
            let live_docs = reader.get_live_docs();
            doc_maps.push(Arc::new(move |doc_id: i32| {
                let alive = match &live_docs {
                    Some(bits) => bits.get(doc_id as usize),
                    None => true,
                };
                if alive {
                    mapping.get(doc_id as usize).copied().unwrap_or(-1)
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
/// Equivalent to `org.apache.lucene.index.SortFieldProvider`.
pub trait SortFieldProvider: Send + Sync {
    /// The SPI name this provider is registered under, which is what the segment
    /// info file records.
    ///
    /// Equivalent to `NamedSPILoader.NamedSPI.getName()`.
    fn name(&self) -> &str;

    /// Reads a sort field from `input`.
    ///
    /// Equivalent to `SortFieldProvider.readSortField(DataInput)`.
    fn read_sort_field(&self, input: &mut dyn crate::store::DataInput) -> Result<SortField>;

    /// Writes `sort_field` to `output`, without the provider name, which the
    /// registry writes first.
    ///
    /// Equivalent to `SortFieldProvider.writeSortField(SortField, DataOutput)`.
    fn write_sort_field(
        &self,
        sort_field: &SortField,
        output: &mut dyn crate::store::DataOutput,
    ) -> Result<()>;
}

/// The registry of [`SortFieldProvider`] implementations, by name.
///
/// **Divergence from Lucene 10.5.0.** Java discovers providers through
/// `NamedSPILoader`, which reads `META-INF/services` off the classpath. Rust has
/// no equivalent at run time, so providers are registered explicitly into this
/// registry, exactly as the crate already does for doc-values and postings
/// formats.
#[derive(Default)]
pub struct SortFieldProviders {
    providers: std::collections::HashMap<String, Arc<dyn SortFieldProvider>>,
}

impl SortFieldProviders {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `provider` under its own name.
    pub fn register(&mut self, provider: Arc<dyn SortFieldProvider>) {
        self.providers.insert(provider.name().to_string(), provider);
    }

    /// Looks a provider up by name.
    ///
    /// Equivalent to `SortFieldProvider.forName(String)`.
    pub fn for_name(&self, name: &str) -> Result<Arc<dyn SortFieldProvider>> {
        self.providers.get(name).cloned().ok_or_else(|| {
            LuceneError::IllegalArgument(format!("no SortFieldProvider registered as '{name}'"))
        })
    }

    /// Returns every registered provider name.
    ///
    /// Equivalent to `SortFieldProvider.availableSortFieldProviders()`.
    pub fn available_sort_field_providers(&self) -> Vec<String> {
        self.available()
    }

    /// Returns every registered provider name.
    ///
    /// Equivalent to `SortFieldProvider.availableSortFieldProviders()`.
    pub fn available(&self) -> Vec<String> {
        let mut names: Vec<String> = self.providers.keys().cloned().collect();
        names.sort();
        names
    }

    /// Writes `sort_field` preceded by its provider name.
    ///
    /// Equivalent to `SortFieldProvider.write(SortField, DataOutput)`.
    pub fn write(
        &self,
        provider_name: &str,
        sort_field: &SortField,
        output: &mut dyn crate::store::DataOutput,
    ) -> Result<()> {
        let provider = self.for_name(provider_name)?;
        output.write_string(provider_name)?;
        provider.write_sort_field(sort_field, output)
    }
}

// ---------------------------------------------------------------------------
// The global registry behind SortFieldProvider's static entry points
// ---------------------------------------------------------------------------

/// The registry behind [`SortFieldProvider`]'s static entry points.
///
/// Equivalent to `SortFieldProvider.Holder.LOADER`, which Java hides in a
/// holder class so the providers are only discovered when first needed.
static SORT_FIELD_PROVIDERS: std::sync::LazyLock<std::sync::RwLock<SortFieldProviders>> =
    std::sync::LazyLock::new(|| std::sync::RwLock::new(SortFieldProviders::new()));

/// Looks a sort-field provider up by name.
///
/// Equivalent to `SortFieldProvider.forName(String)`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when no provider is registered
/// under `name`.
pub fn sort_field_provider_for_name(name: &str) -> Result<Arc<dyn SortFieldProvider>> {
    SORT_FIELD_PROVIDERS
        .read()
        .expect("INVARIANT: the sort-field provider lock is never poisoned")
        .for_name(name)
}

/// Returns the names of every registered sort-field provider.
///
/// Equivalent to `SortFieldProvider.availableSortFieldProviders()`.
pub fn available_sort_field_providers() -> Vec<String> {
    SORT_FIELD_PROVIDERS
        .read()
        .expect("INVARIANT: the sort-field provider lock is never poisoned")
        .available()
}

/// Adds `provider` to the global registry under its own name.
///
/// Equivalent to `SortFieldProvider.reloadSortFieldProviders(ClassLoader)`,
/// which rescans a classloader for providers it has not seen. Rust has no
/// run-time service discovery, so the caller supplies the provider.
pub fn reload_sort_field_providers(provider: Arc<dyn SortFieldProvider>) {
    SORT_FIELD_PROVIDERS
        .write()
        .expect("INVARIANT: the sort-field provider lock is never poisoned")
        .register(provider);
}

impl std::fmt::Display for Sorter {
    /// Port of `Sorter.toString()`, which is `getID()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.get_id())
    }
}
