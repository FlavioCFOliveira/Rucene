//! Per-field delegation formats.
//!
//! Equivalent to `org.apache.lucene.codecs.perfield`.
//!
//! These formats allow different concrete formats to be used for different fields.
//! Each field is written with a segment suffix derived from the concrete format
//! name, and the mapping is recorded in per-field attributes so that readers can
//! reopen the correct concrete format for each field.

#![deny(unsafe_code)]
#![allow(clippy::type_complexity)]

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::{DocValuesType, IndexOptions};
use crate::internal::hppc::IntObjectHashMap;
use crate::util::java_collections::JavaHashMap;
use crate::util::IOUtils;

use super::doc_values::{doc_values_lookup, DocValuesConsumer, DocValuesFormat, DocValuesProducer};
use super::knn_vectors::{
    knn_vectors_lookup, ByteVectorValues, FieldVectorWriter, FloatVectorValues, KnnCollector,
    KnnVectorsFormat, KnnVectorsReader, KnnVectorsWriter,
};
use super::norms::NormsProducer;
use super::postings::{
    postings_lookup, Fields, FieldsConsumer, FieldsProducer, MergeState, PostingsFormat, Terms,
};
use super::state::{OwnedSegmentWriteState, SegmentReadState, SegmentWriteState};
use super::stub::FieldInfo;

// -----------------------------------------------------------------------------
// PerFieldPostingsFormat
// -----------------------------------------------------------------------------

/// Name of the per-field postings format.
pub const PER_FIELD_POSTINGS_NAME: &str = "PerField40";

/// Attribute key storing the concrete postings format name for a field.
pub const PER_FIELD_POSTINGS_FORMAT_KEY: &str = "PerFieldPostingsFormat.format";

/// Attribute key storing the segment suffix for a field's postings.
pub const PER_FIELD_POSTINGS_SUFFIX_KEY: &str = "PerFieldPostingsFormat.suffix";

/// Delegates postings encoding to a concrete format chosen per field.
///
/// Equivalent to `org.apache.lucene.codecs.perfield.PerFieldPostingsFormat`.
pub struct PerFieldPostingsFormat {
    resolver: Arc<dyn Fn(&str) -> Arc<dyn PostingsFormat> + Send + Sync>,
}

impl std::fmt::Debug for PerFieldPostingsFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PerFieldPostingsFormat")
            .field("name", &PER_FIELD_POSTINGS_NAME)
            .finish_non_exhaustive()
    }
}

impl PerFieldPostingsFormat {
    /// Creates a per-field postings format using the supplied resolver.
    pub fn new(resolver: impl Fn(&str) -> Arc<dyn PostingsFormat> + Send + Sync + 'static) -> Self {
        Self {
            resolver: Arc::new(resolver),
        }
    }

    /// Creates a per-field postings format from a static field-name to format
    /// map, falling back to `default_format` for unknown fields.
    pub fn from_map(
        map: HashMap<String, Arc<dyn PostingsFormat>>,
        default_format: Arc<dyn PostingsFormat>,
    ) -> Self {
        Self::new(move |field| {
            map.get(field)
                .cloned()
                .unwrap_or_else(|| default_format.clone())
        })
    }

    /// Returns the postings format that should be used for writing new segments
    /// of `field`.
    ///
    /// The field-to-format mapping is written to the index, so this is only
    /// consulted when writing, never when reading.
    ///
    /// Divergence: Lucene declares this method abstract and expects a subclass
    /// to supply the mapping (`PerFieldPostingsFormat.java:383-389`). Rust has no inheritance,
    /// so this port takes the mapping as a resolver closure given to
    /// [`new`](Self::new) or [`from_map`](Self::from_map), and this method is
    /// the call through it. The contract is unchanged: one format per field,
    /// decided by the caller.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.perfield.PerFieldPostingsFormat#getPostingsFormatForField`.
    pub fn get_postings_format_for_field(&self, field: &str) -> Arc<dyn PostingsFormat> {
        (self.resolver)(field)
    }
}

impl PostingsFormat for PerFieldPostingsFormat {
    /// This format answers with itself: Java's `instanceof PerFieldPostingsFormat` on it is
    /// `true`.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        PER_FIELD_POSTINGS_NAME
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        Ok(Box::new(FieldsWriter::new(state, self.resolver.clone())))
    }

    fn fields_producer<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn FieldsProducer>> {
        Ok(Box::new(FieldsReader::new(state)?))
    }
}

/// The identity of a format instance: Java keys the per-field writers' maps by
/// the format object (`Map<PostingsFormat, FieldsGroup>`,
/// `Map<DocValuesFormat, ConsumerAndSuffix>`), whose `equals` is identity, so
/// two instances of one format name get two suffixes
/// (`PerFieldPostingsFormat.java:207-245`, `PerFieldDocValuesFormat.java:186-230`).
/// The group holds a clone of the `Arc`, so the address cannot be reused while
/// the key is in use.
fn format_identity<T: ?Sized>(format: &Arc<T>) -> usize {
    Arc::as_ptr(format) as *const () as usize
}

fn get_suffix(format_name: &str, suffix: i32) -> String {
    format!("{}_{}", format_name, suffix)
}

fn get_full_segment_suffix(outer_segment_suffix: &str, segment_suffix: &str) -> String {
    if outer_segment_suffix.is_empty() {
        segment_suffix.to_string()
    } else {
        format!("{}_{}", outer_segment_suffix, segment_suffix)
    }
}

fn get_full_segment_suffix_postings(
    field_name: &str,
    outer_segment_suffix: &str,
    segment_suffix: &str,
) -> Result<String> {
    if outer_segment_suffix.is_empty() {
        Ok(segment_suffix.to_string())
    } else {
        Err(LuceneError::illegal_state(format!(
            "cannot embed PerFieldPostingsFormat inside itself (field \"{}\" returned PerFieldPostingsFormat)",
            field_name
        )))
    }
}

/// The fields one postings format writes, and the state its consumer is
/// created with.
///
/// Equivalent to `PerFieldPostingsFormat.FieldsGroup` (`PerFieldPostingsFormat.java:87-111`),
/// which also carries the format it maps from.
struct FieldsGroup<'a> {
    suffix: i32,
    fields: Vec<String>,
    format: Arc<dyn PostingsFormat>,
    state: SegmentWriteState<'a>,
}

struct FieldsWriter<'a> {
    write_state: SegmentWriteState<'a>,
    resolver: Arc<dyn Fn(&str) -> Arc<dyn PostingsFormat> + Send + Sync>,
    to_close: Vec<Box<dyn FieldsConsumer + 'a>>,
}

impl<'a> std::fmt::Debug for FieldsWriter<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FieldsWriter")
            .field("segment_suffix", &self.write_state.segment_suffix)
            .field("to_close", &self.to_close.len())
            .finish_non_exhaustive()
    }
}

impl<'a> FieldsWriter<'a> {
    fn new(
        write_state: &SegmentWriteState<'a>,
        resolver: Arc<dyn Fn(&str) -> Arc<dyn PostingsFormat> + Send + Sync>,
    ) -> Self {
        Self {
            write_state: write_state.clone(),
            resolver,
            to_close: Vec::new(),
        }
    }

    fn build_groups(
        &mut self,
        field_names: impl Iterator<Item = impl AsRef<str>>,
    ) -> Result<HashMap<usize, FieldsGroup<'a>>> {
        let mut suffixes: HashMap<String, i32> = HashMap::new();
        let mut groups: HashMap<usize, FieldsGroup<'a>> = HashMap::new();

        for field_ref in field_names {
            let field = field_ref.as_ref();
            let field_info = self
                .write_state
                .field_infos
                .field_info(field)
                .ok_or_else(|| {
                    LuceneError::illegal_argument(format!("field {} not found", field))
                })?;
            let format = (self.resolver)(field);
            let format_name = format.name().to_string();
            let identity = format_identity(&format);

            let group = match groups.entry(identity) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    // First time we are seeing this format; create a new instance
                    let suffix = suffixes.get(&format_name).copied().unwrap_or(0);
                    suffixes.insert(format_name.clone(), suffix + 1);
                    let segment_suffix = get_full_segment_suffix_postings(
                        field,
                        &self.write_state.segment_suffix,
                        &get_suffix(&format_name, suffix),
                    )?;
                    let state = self.write_state.with_new_suffix(segment_suffix);
                    entry.insert(FieldsGroup {
                        suffix,
                        fields: Vec::new(),
                        format: Arc::clone(&format),
                        state,
                    })
                }
            };
            group.fields.push(field.to_string());
            field_info.put_attribute(PER_FIELD_POSTINGS_FORMAT_KEY, &format_name);
            field_info.put_attribute(PER_FIELD_POSTINGS_SUFFIX_KEY, group.suffix.to_string());
        }

        for group in groups.values_mut() {
            group.fields.sort();
        }

        Ok(groups)
    }
}

impl<'a> FieldsWriter<'a> {
    /// Creates each group's consumer, adds it to the consumers to close before
    /// handing it its work, and on failure closes every consumer created so
    /// far while keeping the original error.
    ///
    /// Equivalent to the loops of `FieldsWriter.write` and `FieldsWriter.merge`
    /// (`PerFieldPostingsFormat.java:145-169, 185-201`): `toClose.add(consumer)`
    /// precedes the call, and the `finally` block runs
    /// `IOUtils.closeWhileHandlingException(toClose)` unless every call
    /// succeeded.
    fn run_groups(
        &mut self,
        groups: &HashMap<usize, FieldsGroup<'a>>,
        mut work: impl FnMut(&mut dyn FieldsConsumer, &FieldsGroup<'a>) -> Result<()>,
    ) -> Result<()> {
        let mut outcome = Ok(());
        for group in groups.values() {
            let consumer = match group.format.fields_consumer(&group.state) {
                Ok(consumer) => consumer,
                Err(error) => {
                    outcome = Err(error);
                    break;
                }
            };
            self.to_close.push(consumer);
            let consumer = self
                .to_close
                .last_mut()
                .expect("INVARIANT: the consumer was pushed one line above");
            if let Err(error) = work(consumer.as_mut(), group) {
                outcome = Err(error);
                break;
            }
        }
        if outcome.is_err() {
            IOUtils::close_while_handling_exception(
                self.to_close.iter_mut().map(|c| Some(move || c.close())),
            )?;
        }
        outcome
    }
}

impl<'a> FieldsConsumer for FieldsWriter<'a> {
    /// Equivalent to `PerFieldPostingsFormat.FieldsWriter.write(Fields,
    /// NormsProducer)` (`PerFieldPostingsFormat.java:140-170`): each format's
    /// consumer sees only its own fields.
    fn write(&mut self, fields: &dyn Fields, norms: Option<&dyn NormsProducer>) -> Result<()> {
        let groups = self.build_groups(fields.iterator())?;
        self.run_groups(&groups, |consumer, group| {
            // Exposes only the fields from this group.
            let masked = MaskedFields::new(fields, &group.fields);
            consumer.write(&masked, norms)
        })
    }

    /// Equivalent to `PerFieldPostingsFormat.FieldsWriter.merge(MergeState,
    /// NormsProducer)` (`PerFieldPostingsFormat.java:172-202`): the fields to
    /// merge are those the source producers list — their sorted names merged
    /// and deduplicated by a `MergedIterator` — and each format's consumer
    /// merges a state restricted to its own fields.
    fn merge(&mut self, merge_state: &MergeState, norms: Option<&dyn NormsProducer>) -> Result<()> {
        let indexed_field_names = crate::util::MergedIterator::with_remove_duplicates(
            true,
            merge_state
                .fields_producers
                .iter()
                .flatten()
                .map(|producer| producer.iterator())
                .collect::<Vec<_>>(),
        );
        let groups = self.build_groups(indexed_field_names)?;
        self.run_groups(&groups, |consumer, group| {
            consumer.merge(
                &PerFieldMergeState::restrict_fields(merge_state, &group.fields)?,
                norms,
            )
        })
    }

    /// Equivalent to `FieldsWriter.close()`, `IOUtils.close(toClose)`
    /// (`PerFieldPostingsFormat.java:264-267`): every consumer is closed, and
    /// the first error is raised once all of them have been.
    fn close(&mut self) -> Result<()> {
        IOUtils::close(self.to_close.iter_mut().map(|c| Some(move || c.close())))
    }
}

/// The `Fields` one format group of [`FieldsWriter::write`] sees: only the
/// fields of that group.
///
/// Equivalent to the anonymous `FilterFields` subclass
/// `PerFieldPostingsFormat.FieldsWriter.write` builds
/// (`PerFieldPostingsFormat.java:152-158`), which overrides `iterator()` alone.
/// `terms(String)` and `size()` are inherited from
/// [`FilterFields`](crate::index::filter_reader::FilterFields), so `size()`
/// reports the size of the **whole** `Fields`, not of the group, exactly as in
/// Java.
struct MaskedFields<'a> {
    base: crate::index::filter_reader::FilterFields<dyn Fields + 'a>,
    fields: &'a [String],
}

impl<'a> MaskedFields<'a> {
    fn new(inner: &'a dyn Fields, fields: &'a [String]) -> Self {
        Self {
            base: crate::index::filter_reader::FilterFields::new(Box::new(inner)),
            fields,
        }
    }
}

impl Fields for MaskedFields<'_> {
    fn size(&self) -> i32 {
        self.base.size()
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        self.base.terms(field)
    }

    /// Equivalent to the one method the anonymous subclass overrides:
    /// `group.fields.iterator()`.
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        let fields: Vec<String> = self.fields.to_vec();
        Box::new(fields.into_iter())
    }
}

/// A consumer that writes nothing, for the tests' stand-in formats.
#[cfg(test)]
#[derive(Debug, Default, Clone)]
struct NoOpFieldsConsumer;

#[cfg(test)]
impl FieldsConsumer for NoOpFieldsConsumer {
    fn write(&mut self, _fields: &dyn Fields, _norms: Option<&dyn NormsProducer>) -> Result<()> {
        Ok(())
    }

    fn merge(
        &mut self,
        _merge_state: &MergeState,
        _norms: Option<&dyn NormsProducer>,
    ) -> Result<()> {
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

struct FieldsReader {
    /// Java's `Map<String, FieldsProducer> formats = new HashMap<>()`
    /// (`PerFieldPostingsFormat.java:273`), keyed by segment suffix: its
    /// iteration order is the order the producers are closed in, which decides
    /// whose failure surfaces when several fail.
    formats: JavaHashMap<Box<dyn FieldsProducer>>,
    fields: BTreeMap<String, String>,
}

impl FieldsReader {
    fn new(read_state: &SegmentReadState) -> Result<Self> {
        let mut formats: JavaHashMap<Box<dyn FieldsProducer>> = JavaHashMap::new();
        let mut fields: BTreeMap<String, String> = BTreeMap::new();

        let result = (|| -> Result<()> {
            for fi in read_state.field_infos.iter() {
                if fi.index_options != IndexOptions::NONE {
                    // null formatName means the field is in fieldInfos, but has no postings!
                    // (`PerFieldPostingsFormat.java:305-307`)
                    let Some(format_name) = fi.get_attribute(PER_FIELD_POSTINGS_FORMAT_KEY) else {
                        continue;
                    };
                    let suffix =
                        fi.get_attribute(PER_FIELD_POSTINGS_SUFFIX_KEY)
                            .ok_or_else(|| {
                                LuceneError::illegal_state(format!(
                            "missing attribute: {PER_FIELD_POSTINGS_SUFFIX_KEY} for field: {}",
                            fi.name
                        ))
                            })?;
                    // `PostingsFormat.forName(formatName)`: the loader's
                    // `IllegalArgumentException` for an unknown name.
                    let format = postings_lookup(&format_name)?;
                    let suffix_num = suffix.parse::<i32>().map_err(|_| {
                        LuceneError::illegal_state(format!(
                            "invalid postings suffix for field \"{}\": {suffix}",
                            fi.name
                        ))
                    })?;
                    let segment_suffix = get_suffix(&format_name, suffix_num);
                    if !formats.contains_key(&segment_suffix) {
                        let state = read_state.with_new_suffix(segment_suffix.clone());
                        formats.insert(segment_suffix.clone(), format.fields_producer(&state)?);
                    }
                    fields.insert(fi.name.clone(), segment_suffix);
                }
            }
            Ok(())
        })();

        if let Err(e) = result {
            // `IOUtils.closeWhileHandlingException(formats.values())`
            // (`PerFieldPostingsFormat.java:325-329`).
            IOUtils::close_while_handling_exception(
                formats.iter_mut().map(|(_, p)| Some(move || p.close())),
            )?;
            return Err(e);
        }

        Ok(Self { formats, fields })
    }
}

impl std::fmt::Debug for FieldsReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PerFieldPostingsReader")
            .field("formats", &self.formats.len())
            .finish_non_exhaustive()
    }
}

impl Fields for FieldsReader {
    fn size(&self) -> i32 {
        self.fields.len() as i32
    }

    /// Equivalent to `PerFieldPostingsFormat.FieldsReader.terms(String)`
    /// (`PerFieldPostingsFormat.java:340-343`): `fieldsProducer == null ?
    /// null : fieldsProducer.terms(field)`, so a field this segment does not
    /// index has no terms. A merge asks every reader for every merged field
    /// (`MultiFields.terms`), including readers that lack it.
    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        let Some(suffix) = self.fields.get(field) else {
            return Ok(None);
        };
        self.formats
            .get(suffix)
            .ok_or_else(|| {
                LuceneError::illegal_state(format!("missing producer for suffix {suffix}"))
            })
            .and_then(|p| p.terms(field))
    }

    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        let names: Vec<String> = self.fields.keys().cloned().collect();
        Box::new(names.into_iter())
    }
}

impl FieldsProducer for FieldsReader {
    fn check_integrity(&self) -> Result<()> {
        for producer in self.formats.values() {
            producer.check_integrity()?;
        }
        Ok(())
    }

    /// Returns a reader whose every format is that format's merge instance.
    ///
    /// Equivalent to `PerFieldPostingsFormat.FieldsReader.getMergeInstance()`,
    /// which is `new FieldsReader(this)`: the copy constructor
    /// (`PerFieldPostingsFormat.java:276-294`) replaces each format's producer
    /// with its `getMergeInstance()` and points every field at the replacement
    /// of the producer it used. Fields are keyed by segment suffix here, so
    /// keeping the field-to-suffix map is that same remapping.
    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        // `formats.put(ent.getKey(), ...)` over `other.formats.entrySet()`,
        // into a new `HashMap` (`PerFieldPostingsFormat.java:279-284`).
        let mut formats: JavaHashMap<Box<dyn FieldsProducer>> = JavaHashMap::new();
        for (suffix, producer) in self.formats.iter() {
            formats.insert(suffix.clone(), producer.get_merge_instance()?);
        }
        // `assert producer != null` (PerFieldPostingsFormat.java:289): every field's producer has
        // its merge-instance replacement.
        debug_assert!(self
            .fields
            .values()
            .all(|suffix| formats.contains_key(suffix)));
        Ok(Box::new(FieldsReader {
            formats,
            fields: self.fields.clone(),
        }))
    }

    /// Equivalent to `PerFieldPostingsFormat.FieldsReader.close()`
    /// (`PerFieldPostingsFormat.java:350-353`): `IOUtils.close(formats.values())`.
    fn close(&self) -> Result<()> {
        IOUtils::close(self.formats.iter().map(|(_, p)| Some(move || p.close())))
    }
}

// -----------------------------------------------------------------------------
// PerFieldDocValuesFormat
// -----------------------------------------------------------------------------

/// Name of the per-field doc-values format.
pub const PER_FIELD_DOC_VALUES_NAME: &str = "PerFieldDV40";

/// Attribute key storing the concrete doc-values format name for a field.
pub const PER_FIELD_DOC_VALUES_FORMAT_KEY: &str = "PerFieldDocValuesFormat.format";

/// Attribute key storing the segment suffix for a field's doc values.
pub const PER_FIELD_DOC_VALUES_SUFFIX_KEY: &str = "PerFieldDocValuesFormat.suffix";

/// Delegates doc-values encoding to a concrete format chosen per field.
///
/// Equivalent to `org.apache.lucene.codecs.perfield.PerFieldDocValuesFormat`.
pub struct PerFieldDocValuesFormat {
    resolver: Arc<dyn Fn(&str) -> Arc<dyn DocValuesFormat> + Send + Sync>,
}

impl std::fmt::Debug for PerFieldDocValuesFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PerFieldDocValuesFormat")
            .field("name", &PER_FIELD_DOC_VALUES_NAME)
            .finish_non_exhaustive()
    }
}

impl PerFieldDocValuesFormat {
    /// Creates a per-field doc-values format using the supplied resolver.
    pub fn new(
        resolver: impl Fn(&str) -> Arc<dyn DocValuesFormat> + Send + Sync + 'static,
    ) -> Self {
        Self {
            resolver: Arc::new(resolver),
        }
    }

    /// Creates a per-field doc-values format from a static field-name to format
    /// map, falling back to `default_format` for unknown fields.
    pub fn from_map(
        map: HashMap<String, Arc<dyn DocValuesFormat>>,
        default_format: Arc<dyn DocValuesFormat>,
    ) -> Self {
        Self::new(move |field| {
            map.get(field)
                .cloned()
                .unwrap_or_else(|| default_format.clone())
        })
    }

    /// Returns the doc-values format that should be used for writing new
    /// segments of `field`.
    ///
    /// The field-to-format mapping is written to the index, so this is only
    /// consulted when writing, never when reading.
    ///
    /// Divergence: Lucene declares this method abstract and expects a subclass
    /// to supply the mapping (`PerFieldDocValuesFormat.java:382-389`). Rust has no inheritance,
    /// so this port takes the mapping as a resolver closure given to
    /// [`new`](Self::new) or [`from_map`](Self::from_map), and this method is
    /// the call through it. The contract is unchanged: one format per field,
    /// decided by the caller.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.perfield.PerFieldDocValuesFormat#getDocValuesFormatForField`.
    pub fn get_doc_values_format_for_field(&self, field: &str) -> Arc<dyn DocValuesFormat> {
        (self.resolver)(field)
    }
}

impl DocValuesFormat for PerFieldDocValuesFormat {
    /// This format answers with itself: Java's `instanceof PerFieldDocValuesFormat` on it is
    /// `true`.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        PER_FIELD_DOC_VALUES_NAME
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn DocValuesConsumer + 'a>> {
        Ok(Box::new(DvFieldsWriter::new(state, self.resolver.clone())))
    }

    fn fields_producer<'a>(
        &self,
        state: &SegmentReadState<'a>,
    ) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(DvFieldsReader::new(state)?))
    }
}

struct ConsumerAndSuffix<'a> {
    suffix: i32,
    consumer: Box<dyn DocValuesConsumer + 'a>,
    _phantom: std::marker::PhantomData<&'a ()>,
}

struct DvFieldsWriter<'a> {
    segment_write_state: SegmentWriteState<'a>,
    resolver: Arc<dyn Fn(&str) -> Arc<dyn DocValuesFormat> + Send + Sync>,
    /// Keyed by the format instance, as Java's `formats` map (see
    /// [`format_identity`]).
    formats: HashMap<usize, ConsumerAndSuffix<'a>>,
    /// The last suffix used per format name, as Java's `suffixes` map.
    suffixes: HashMap<String, i32>,
}

impl<'a> std::fmt::Debug for DvFieldsWriter<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DvFieldsWriter")
            .field("segment_suffix", &self.segment_write_state.segment_suffix)
            .field("formats", &self.formats.len())
            .finish_non_exhaustive()
    }
}

impl<'a> DvFieldsWriter<'a> {
    fn new(
        segment_write_state: &SegmentWriteState<'a>,
        resolver: Arc<dyn Fn(&str) -> Arc<dyn DocValuesFormat> + Send + Sync>,
    ) -> Self {
        Self {
            segment_write_state: segment_write_state.clone(),
            resolver,
            formats: HashMap::new(),
            suffixes: HashMap::new(),
        }
    }

    /// Records `key` on the `FieldInfo` the caller passed **and** on the one the
    /// segment's `FieldInfos` will serialise into the `.fnm`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java writes the attribute once, on
    /// the object the caller passed
    /// (`PerFieldDocValuesFormat.java:150,175`), because that object *is* the
    /// one `state.fieldInfos` holds: `IndexingChain` gives its `PerField` a
    /// reference to the `FieldInfo` the builder created, and
    /// `FieldInfos.Builder.finish()` hands those very objects to the
    /// `FieldInfos` it returns. This port copies the `FieldInfo` at both steps
    /// — `IndexingChain`'s `PerField` owns one, each doc-values writer owns
    /// another — so mutating the object the writer passes leaves the `.fnm`
    /// without the attribute, and the segment cannot be reopened. Writing to
    /// both objects reproduces what one shared object would have given.
    /// `PerFieldPostingsFormat.FieldsWriter` already reaches the segment's
    /// `FieldInfo` by name for the same reason (see `build_groups`).
    fn record_attribute(&self, field: &FieldInfo, key: &str, value: &str) {
        field.put_attribute(key, value);
        self.record_shared_attribute(&field.name, key, value);
    }

    /// Records `key` on the `FieldInfo` the segment's `FieldInfos` holds for
    /// `name`, when it holds one. See [`Self::record_attribute`].
    fn record_shared_attribute(&self, name: &str, key: &str, value: &str) {
        if let Some(shared) = self.segment_write_state.field_infos.field_info(name) {
            shared.put_attribute(key, value);
        }
    }

    fn get_instance(&mut self, field: &FieldInfo) -> Result<&mut Box<dyn DocValuesConsumer + 'a>> {
        self.get_instance_with_ignore(field, false)
    }

    fn get_instance_with_ignore(
        &mut self,
        field: &FieldInfo,
        ignore_current_format: bool,
    ) -> Result<&mut Box<dyn DocValuesConsumer + 'a>> {
        let format = if field.doc_values_gen != -1 {
            let format_name = if ignore_current_format {
                None
            } else {
                field.get_attribute(PER_FIELD_DOC_VALUES_FORMAT_KEY)
            };
            // `DocValuesFormat.forName(formatName)` throws for an unknown
            // name (`PerFieldDocValuesFormat.java:174-177`).
            match format_name {
                Some(name) => Some(doc_values_lookup(&name)?),
                None => Some((self.resolver)(&field.name)),
            }
        } else {
            Some((self.resolver)(&field.name))
        }
        .ok_or_else(|| {
            LuceneError::illegal_state(format!(
                "invalid null DocValuesFormat for field=\"{}\"",
                field.name
            ))
        })?;

        let format_name = format.name().to_string();
        self.record_attribute(field, PER_FIELD_DOC_VALUES_FORMAT_KEY, &format_name);
        let identity = format_identity(&format);

        if let std::collections::hash_map::Entry::Vacant(vacant) = self.formats.entry(identity) {
            // First time we are seeing this format; create a new instance
            let suffix = if field.doc_values_gen != -1 {
                let suffix_att = if ignore_current_format {
                    None
                } else {
                    field.get_attribute(PER_FIELD_DOC_VALUES_SUFFIX_KEY)
                };
                // even when dvGen is != -1, it can still be a new field, that never
                // existed in the segment, and therefore doesn't have the recorded
                // attributes yet.
                suffix_att.and_then(|s| s.parse::<i32>().ok())
            } else {
                None
            }
            // bump the suffix
            .unwrap_or_else(|| self.suffixes.get(&format_name).map_or(0, |s| s + 1));
            self.suffixes.insert(format_name.clone(), suffix);

            let segment_suffix = get_full_segment_suffix(
                &self.segment_write_state.segment_suffix,
                &get_suffix(&format_name, suffix),
            );
            let state = self.segment_write_state.with_new_suffix(segment_suffix);
            let consumer = format.fields_consumer(&state)?;
            vacant.insert(ConsumerAndSuffix {
                suffix,
                consumer,
                _phantom: std::marker::PhantomData,
            });
        }

        // `assert suffixes.containsKey(formatName)` (`PerFieldDocValuesFormat.java:229`),
        // checked on both branches: the new-instance branch has just inserted it.
        debug_assert!(self.suffixes.contains_key(&format_name));
        let entry = self
            .formats
            .get_mut(&identity)
            .expect("consumer just inserted");
        let suffix = entry.suffix.to_string();
        // `entry` borrows `self.formats` mutably, so the shared field info is
        // reached after the borrow ends, on the next statement.
        field.put_attribute(PER_FIELD_DOC_VALUES_SUFFIX_KEY, &suffix);
        self.record_shared_attribute(&field.name, PER_FIELD_DOC_VALUES_SUFFIX_KEY, &suffix);
        Ok(&mut self
            .formats
            .get_mut(&identity)
            .expect("consumer just inserted")
            .consumer)
    }

    /// The key of the consumer `get_instance_with_ignore` returned for
    /// `field` last: the identity of the format it resolved.
    fn instance_key(&mut self, field: &FieldInfo, ignore_current_format: bool) -> Result<usize> {
        let consumer: *const (dyn DocValuesConsumer + 'a) =
            &**self.get_instance_with_ignore(field, ignore_current_format)?;
        Ok(self
            .formats
            .iter()
            .find(|(_, entry)| std::ptr::addr_eq(&*entry.consumer, consumer))
            .map(|(key, _)| *key)
            .expect("INVARIANT: the consumer was just returned from `formats`"))
    }
}

impl<'a> DocValuesConsumer for DvFieldsWriter<'a> {
    fn add_numeric_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.get_instance(field)?.add_numeric_field(field, values)
    }

    fn add_binary_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.get_instance(field)?.add_binary_field(field, values)
    }

    fn add_sorted_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.get_instance(field)?.add_sorted_field(field, values)
    }

    fn add_sorted_numeric_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.get_instance(field)?
            .add_sorted_numeric_field(field, values)
    }

    fn add_sorted_set_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.get_instance(field)?
            .add_sorted_set_field(field, values)
    }

    fn merge(&mut self, merge_state: &MergeState) -> Result<()> {
        // `IdentityHashMap<DocValuesConsumer, Collection<String>>`: one
        // consumer per format instance.
        let mut consumers_to_fields: HashMap<usize, Vec<String>> = HashMap::new();

        // Group each consumer by the fields it handles
        for fi in merge_state.merge_field_infos.iter() {
            if fi.doc_values_type == DocValuesType::NONE {
                continue;
            }
            // merge should ignore current format for the fields being merged
            let consumer_key = self.instance_key(fi, true)?;
            consumers_to_fields
                .entry(consumer_key)
                .or_default()
                .push(fi.name.clone());
        }

        // Delegate the merge to the appropriate consumer
        for (consumer_key, fields) in consumers_to_fields {
            let restricted = PerFieldMergeState::restrict_fields(merge_state, &fields)?;
            self.formats
                .get_mut(&consumer_key)
                .expect("consumer present")
                .consumer
                .merge(&restricted)?;
        }

        Ok(())
    }

    /// Equivalent to `PerFieldDocValuesFormat.FieldsWriter.close()`
    /// (`PerFieldDocValuesFormat.java:239-243`): `IOUtils.close(formats.values())`.
    fn close(&mut self) -> Result<()> {
        IOUtils::close(
            self.formats
                .values_mut()
                .map(|entry| Some(move || entry.consumer.close())),
        )
    }
}

struct DvFieldsReader {
    /// Java's `Map<String, DocValuesProducer> formats = new HashMap<>()`
    /// (`PerFieldDocValuesFormat.java:261`), keyed by segment suffix: its
    /// iteration order is the order the producers are closed in, which decides
    /// whose failure surfaces when several fail.
    formats: JavaHashMap<Box<dyn DocValuesProducer>>,
    fields: HashMap<i32, String>,
}

impl DvFieldsReader {
    fn new(read_state: &SegmentReadState) -> Result<Self> {
        let mut formats: JavaHashMap<Box<dyn DocValuesProducer>> = JavaHashMap::new();
        let mut fields: HashMap<i32, String> = HashMap::new();

        let result = (|| -> Result<()> {
            for fi in read_state.field_infos.iter() {
                if fi.doc_values_type != DocValuesType::NONE {
                    let format_name = fi
                        .get_attribute(PER_FIELD_DOC_VALUES_FORMAT_KEY)
                        .ok_or_else(|| {
                            LuceneError::illegal_state(format!(
                            "missing attribute: {PER_FIELD_DOC_VALUES_FORMAT_KEY} for field: {}",
                            fi.name
                        ))
                        })?;
                    let suffix = fi
                        .get_attribute(PER_FIELD_DOC_VALUES_SUFFIX_KEY)
                        .ok_or_else(|| {
                            LuceneError::illegal_state(format!(
                            "missing attribute: {PER_FIELD_DOC_VALUES_SUFFIX_KEY} for field: {}",
                            fi.name
                        ))
                        })?;
                    // `DocValuesFormat.forName(formatName)`: the loader's
                    // `IllegalArgumentException` for an unknown name.
                    let format = doc_values_lookup(&format_name)?;
                    let suffix_num = suffix.parse::<i32>().map_err(|_| {
                        LuceneError::illegal_state(format!(
                            "invalid doc-values suffix for field \"{}\": {suffix}",
                            fi.name
                        ))
                    })?;
                    let segment_suffix = get_full_segment_suffix(
                        &read_state.segment_suffix,
                        &get_suffix(&format_name, suffix_num),
                    );
                    if !formats.contains_key(&segment_suffix) {
                        let state = read_state.with_new_suffix(segment_suffix.clone());
                        formats.insert(segment_suffix.clone(), format.fields_producer(&state)?);
                    }
                    fields.insert(fi.number, segment_suffix);
                }
            }
            Ok(())
        })();

        if let Err(e) = result {
            // `IOUtils.closeWhileHandlingException(formats.values())`
            // (`PerFieldDocValuesFormat.java:311-315`).
            IOUtils::close_while_handling_exception(
                formats.iter_mut().map(|(_, p)| Some(move || p.close())),
            )?;
            return Err(e);
        }

        Ok(Self { formats, fields })
    }

    fn producer_for(&self, field: &FieldInfo) -> Result<&(dyn DocValuesProducer + '_)> {
        let suffix = self.fields.get(&field.number).ok_or_else(|| {
            LuceneError::illegal_argument(format!(
                "field \"{}\" has no doc-values producer",
                field.name
            ))
        })?;
        self.formats.get(suffix).map(|b| b.as_ref()).ok_or_else(|| {
            LuceneError::illegal_state(format!("missing doc-values producer for suffix {suffix}"))
        })
    }
}

impl std::fmt::Debug for DvFieldsReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PerFieldDocValuesReader")
            .field("formats", &self.formats.len())
            .finish_non_exhaustive()
    }
}

impl DocValuesProducer for DvFieldsReader {
    fn get_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn super::doc_values::NumericDocValues>>> {
        // `producer == null ? null : producer.get_numeric(field)`
        // (`codecs/perfield/PerFieldDocValuesFormat.java:318-345`).
        if !self.fields.contains_key(&field.number) {
            return Ok(None);
        }
        self.producer_for(field)?.get_numeric(field)
    }

    fn get_binary(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn super::doc_values::BinaryDocValues>>> {
        // `producer == null ? null : producer.get_binary(field)`
        // (`codecs/perfield/PerFieldDocValuesFormat.java:318-345`).
        if !self.fields.contains_key(&field.number) {
            return Ok(None);
        }
        self.producer_for(field)?.get_binary(field)
    }

    fn get_sorted(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn super::doc_values::SortedDocValues>>> {
        // `producer == null ? null : producer.get_sorted(field)`
        // (`codecs/perfield/PerFieldDocValuesFormat.java:318-345`).
        if !self.fields.contains_key(&field.number) {
            return Ok(None);
        }
        self.producer_for(field)?.get_sorted(field)
    }

    fn get_sorted_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn super::doc_values::SortedNumericDocValues>>> {
        // `producer == null ? null : producer.get_sorted_numeric(field)`
        // (`codecs/perfield/PerFieldDocValuesFormat.java:318-345`).
        if !self.fields.contains_key(&field.number) {
            return Ok(None);
        }
        self.producer_for(field)?.get_sorted_numeric(field)
    }

    fn get_sorted_set(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn super::doc_values::SortedSetDocValues>>> {
        // `producer == null ? null : producer.get_sorted_set(field)`
        // (`codecs/perfield/PerFieldDocValuesFormat.java:318-345`).
        if !self.fields.contains_key(&field.number) {
            return Ok(None);
        }
        self.producer_for(field)?.get_sorted_set(field)
    }

    /// Equivalent to `PerFieldDocValuesFormat.FieldsReader.getSkipper(FieldInfo)`
    /// (`codecs/perfield/PerFieldDocValuesFormat.java:347-351`): `None` when no
    /// producer serves the field, otherwise the delegate's answer.
    fn get_skipper(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn super::doc_values::DocValuesSkipper>>> {
        if !self.fields.contains_key(&field.number) {
            return Ok(None);
        }
        self.producer_for(field)?.get_skipper(field)
    }

    fn check_integrity(&self) -> Result<()> {
        for producer in self.formats.values() {
            producer.check_integrity()?;
        }
        Ok(())
    }

    /// Returns a reader whose every format is that format's merge instance.
    ///
    /// Equivalent to `PerFieldDocValuesFormat.FieldsReader.getMergeInstance()`,
    /// which is `new FieldsReader(this)`: the copy constructor
    /// (`PerFieldDocValuesFormat.java:263-279`) replaces each format's producer
    /// with its `getMergeInstance()` and points every field at the replacement
    /// of the producer it used. Fields are keyed by segment suffix here, so
    /// keeping the field-to-suffix map is that same remapping.
    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        // `formats.put(ent.getKey(), ...)` over `other.formats.entrySet()`,
        // into a new `HashMap` (`PerFieldDocValuesFormat.java:265-271`).
        let mut formats: JavaHashMap<Box<dyn DocValuesProducer>> = JavaHashMap::new();
        for (suffix, producer) in self.formats.iter() {
            formats.insert(suffix.clone(), producer.get_merge_instance()?);
        }
        // `assert producer != null` (PerFieldDocValuesFormat.java:276): every field's producer has
        // its merge-instance replacement.
        debug_assert!(self
            .fields
            .values()
            .all(|suffix| formats.contains_key(suffix)));
        Ok(Box::new(DvFieldsReader {
            formats,
            fields: self.fields.clone(),
        }))
    }

    /// Equivalent to `PerFieldDocValuesFormat.FieldsReader.close()`
    /// (`PerFieldDocValuesFormat.java:354-357`): `IOUtils.close(formats.values())`.
    fn close(&self) -> Result<()> {
        IOUtils::close(self.formats.iter().map(|(_, p)| Some(move || p.close())))
    }
}

// -----------------------------------------------------------------------------
// PerFieldKnnVectorsFormat
// -----------------------------------------------------------------------------

/// Name of the per-field KNN-vectors format.
pub const PER_FIELD_KNN_VECTORS_NAME: &str = "PerFieldVectors90";

/// Attribute key storing the concrete KNN-vectors format name for a field.
pub const PER_FIELD_KNN_VECTORS_FORMAT_KEY: &str = "PerFieldKnnVectorsFormat.format";

/// Attribute key storing the segment suffix for a field's vectors.
pub const PER_FIELD_KNN_VECTORS_SUFFIX_KEY: &str = "PerFieldKnnVectorsFormat.suffix";

/// Delegates KNN-vectors encoding to a concrete format chosen per field.
///
/// Equivalent to `org.apache.lucene.codecs.perfield.PerFieldKnnVectorsFormat`.
pub struct PerFieldKnnVectorsFormat {
    resolver: Arc<dyn Fn(&str) -> Arc<dyn KnnVectorsFormat> + Send + Sync>,
}

impl std::fmt::Debug for PerFieldKnnVectorsFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PerFieldKnnVectorsFormat")
            .field("name", &PER_FIELD_KNN_VECTORS_NAME)
            .finish_non_exhaustive()
    }
}

impl PerFieldKnnVectorsFormat {
    /// Creates a per-field KNN-vectors format using the supplied resolver.
    pub fn new(
        resolver: impl Fn(&str) -> Arc<dyn KnnVectorsFormat> + Send + Sync + 'static,
    ) -> Self {
        Self {
            resolver: Arc::new(resolver),
        }
    }

    /// Creates a per-field KNN-vectors format from a static field-name to format
    /// map, falling back to `default_format` for unknown fields.
    pub fn from_map(
        map: HashMap<String, Arc<dyn KnnVectorsFormat>>,
        default_format: Arc<dyn KnnVectorsFormat>,
    ) -> Self {
        Self::new(move |field| {
            map.get(field)
                .cloned()
                .unwrap_or_else(|| default_format.clone())
        })
    }

    /// Returns the numeric-vector format that should be used for writing new
    /// segments of `field`.
    ///
    /// The field-to-format mapping is written to the index, so this is only
    /// consulted when writing, never when reading.
    ///
    /// Divergence: Lucene declares this method abstract and expects a subclass
    /// to supply the mapping (`PerFieldKnnVectorsFormat.java:93-100`). Rust has no inheritance,
    /// so this port takes the mapping as a resolver closure given to
    /// [`new`](Self::new) or [`from_map`](Self::from_map), and this method is
    /// the call through it. The contract is unchanged: one format per field,
    /// decided by the caller.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.perfield.PerFieldKnnVectorsFormat#getKnnVectorsFormatForField`.
    pub fn get_knn_vectors_format_for_field(&self, field: &str) -> Arc<dyn KnnVectorsFormat> {
        (self.resolver)(field)
    }
}

impl KnnVectorsFormat for PerFieldKnnVectorsFormat {
    fn name(&self) -> &str {
        PER_FIELD_KNN_VECTORS_NAME
    }

    fn fields_writer(&self, state: &OwnedSegmentWriteState) -> Result<Box<dyn KnnVectorsWriter>> {
        Ok(Box::new(KnnFieldsWriter::new(state, self.resolver.clone())))
    }

    fn fields_reader<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn KnnVectorsReader>> {
        Ok(Box::new(KnnFieldsReader::new(state)?))
    }

    fn get_max_dimensions(&self, field_name: &str) -> i32 {
        self.get_knn_vectors_format_for_field(field_name)
            .get_max_dimensions(field_name)
    }

    /// This format answers with itself: Java's `instanceof
    /// PerFieldKnnVectorsFormat` on it is `true`.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

struct WriterAndSuffix {
    suffix: i32,
    writer: Box<dyn KnnVectorsWriter>,
    /// The format the writer came from, held so that its address — the key
    /// of [`KnnFieldsWriter::formats`] — cannot be reused while it is a key.
    _format: Arc<dyn KnnVectorsFormat>,
}

/// Equivalent to `PerFieldKnnVectorsFormat.FieldsWriter`.
///
/// Java's writer stores the `SegmentWriteState` it was built from and derives a
/// suffixed copy of it for every field it dispatches
/// (`PerFieldKnnVectorsFormat.java:96-136`). This one does the same, over the
/// owned [`OwnedSegmentWriteState`]: unlike the postings and doc-values
/// per-field writers, which live and die inside one flush call, this writer is
/// created when the first vector field of the first document is indexed and
/// must survive until the segment flushes, so it cannot borrow the state.
struct KnnFieldsWriter {
    segment_write_state: OwnedSegmentWriteState,
    resolver: Arc<dyn Fn(&str) -> Arc<dyn KnnVectorsFormat> + Send + Sync>,
    /// Keyed by the format instance, as Java's `Map<KnnVectorsFormat,
    /// WriterAndSuffix> formats` (`PerFieldKnnVectorsFormat.java:103`), whose
    /// keys compare by identity (see [`format_identity`]): two instances of
    /// one format name get two writers and two suffixes (rmp #228).
    formats: HashMap<usize, WriterAndSuffix>,
    /// The last suffix used per format name, as Java's `suffixes` map.
    suffixes: HashMap<String, i32>,
}

impl std::fmt::Debug for KnnFieldsWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KnnFieldsWriter")
            .field("segment_suffix", &self.segment_write_state.segment_suffix)
            .field("formats", &self.formats.len())
            .finish_non_exhaustive()
    }
}

impl KnnFieldsWriter {
    fn new(
        segment_write_state: &OwnedSegmentWriteState,
        resolver: Arc<dyn Fn(&str) -> Arc<dyn KnnVectorsFormat> + Send + Sync>,
    ) -> Self {
        Self {
            segment_write_state: segment_write_state
                .with_new_suffix(segment_write_state.segment_suffix.clone()),
            resolver,
            formats: HashMap::new(),
            suffixes: HashMap::new(),
        }
    }

    fn get_instance(&mut self, field: &FieldInfo) -> Result<&mut Box<dyn KnnVectorsWriter>> {
        let format = (self.resolver)(&field.name);
        let format_name = format.name().to_string();

        field.put_attribute(PER_FIELD_KNN_VECTORS_FORMAT_KEY, &format_name);
        let identity = format_identity(&format);

        if !self.formats.contains_key(&identity) {
            // First time we are seeing this format; create a new instance
            let suffix = self.suffixes.get(&format_name).map_or(0, |s| s + 1);
            self.suffixes.insert(format_name.clone(), suffix);
            let segment_suffix = get_full_segment_suffix(
                &self.segment_write_state.segment_suffix,
                &get_suffix(&format_name, suffix),
            );
            let state = self.segment_write_state.with_new_suffix(segment_suffix);
            let writer = format.fields_writer(&state)?;
            self.formats.insert(
                identity,
                WriterAndSuffix {
                    suffix,
                    writer,
                    _format: Arc::clone(&format),
                },
            );
        }

        // we've already seen this format, so just grab its suffix
        debug_assert!(self.suffixes.contains_key(&format_name));
        let entry = self
            .formats
            .get_mut(&identity)
            .expect("writer just inserted");
        field.put_attribute(PER_FIELD_KNN_VECTORS_SUFFIX_KEY, entry.suffix.to_string());
        Ok(&mut entry.writer)
    }
}

impl KnnVectorsWriter for KnnFieldsWriter {
    fn add_field(&mut self, field_info: &FieldInfo) -> Result<FieldVectorWriter> {
        self.get_instance(field_info)?.add_field(field_info)
    }

    fn flush(
        &mut self,
        max_doc: i32,
        sort_map: Option<&super::knn_vectors::SorterDocMap>,
    ) -> Result<()> {
        for entry in self.formats.values_mut() {
            entry.writer.flush(max_doc, sort_map)?;
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        for entry in self.formats.values_mut() {
            entry.writer.finish()?;
        }
        Ok(())
    }

    fn merge_one_field<'m>(
        &mut self,
        field_info: &FieldInfo,
        merge_state: &'m MergeState,
    ) -> Result<Option<Box<dyn super::knn_vectors::IORunnable + 'm>>> {
        self.get_instance(field_info)?
            .merge_one_field(field_info, merge_state)
    }

    /// Equivalent to `PerFieldKnnVectorsFormat.FieldsWriter.close()`
    /// (`PerFieldKnnVectorsFormat.java:137-140`): `IOUtils.close(formats.values())`.
    fn close(&mut self) -> Result<()> {
        IOUtils::close(
            self.formats
                .values_mut()
                .map(|entry| Some(move || entry.writer.close())),
        )
    }

    /// Equivalent to `PerFieldKnnVectorsFormat.FieldsWriter.ramBytesUsed()`
    /// (`PerFieldKnnVectorsFormat.java:182-189`), which sums the delegates and
    /// counts nothing for itself.
    fn ram_bytes_used(&self) -> i64 {
        self.formats
            .values()
            .map(|entry| entry.writer.ram_bytes_used())
            .sum()
    }
}

/// The KNN-vectors reader that dispatches every field to the reader of the
/// format it was written with.
///
/// Equivalent to `org.apache.lucene.codecs.perfield.PerFieldKnnVectorsFormat.FieldsReader`,
/// which Java declares `public static` (`PerFieldKnnVectorsFormat.java:193`).
/// `Lucene104ScalarQuantizedVectorsWriter.getCentroid` tests for it with
/// `instanceof` (`Lucene104ScalarQuantizedVectorsWriter.java:389`), which
/// [`KnnVectorsReader::as_any`] stands for.
pub struct KnnFieldsReader {
    /// The readers, one per segment suffix, as Java's constructor collects
    /// them in a local `Map<String, KnnVectorsReader> formats = new
    /// HashMap<>()` (`PerFieldKnnVectorsFormat.java:209`), whose iteration
    /// order is the order a failed constructor closes them in.
    formats: JavaHashMap<Box<dyn KnnVectorsReader>>,
    /// Field number to the suffix of its reader: Java's
    /// `IntObjectHashMap<KnnVectorsReader> fields`
    /// (`PerFieldKnnVectorsFormat.java:195`), whose iteration order is the
    /// order `close()` closes the readers in.
    fields: IntObjectHashMap<String>,
    /// Field *name* to segment suffix, so that a field can be resolved without
    /// its `FieldInfo`. Java's `FieldsReader` keeps the whole `FieldInfos` and
    /// looks the number up through it (`PerFieldKnnVectorsFormat.java:269-274`);
    /// this map holds exactly the pairs that lookup would produce.
    field_names: HashMap<String, String>,
    /// Field number to field name, for every field with vectors. Java reads
    /// the same pairs from the `FieldInfos` it keeps; the merge copy needs
    /// them to give each field its own merge instance.
    names: HashMap<i32, String>,
}

impl KnnFieldsReader {
    /// Creates a reader over a segment, opening the reader of every
    /// KNN-vectors format its vector fields name.
    ///
    /// Equivalent to the public
    /// `PerFieldKnnVectorsFormat.FieldsReader(SegmentReadState)` constructor
    /// (`PerFieldKnnVectorsFormat.java:205-241`).
    ///
    /// ```
    /// // Public, as Java's constructor is.
    /// let _new = rucene::codecs::per_field::KnnFieldsReader::new;
    /// ```
    ///
    /// # Errors
    ///
    /// Propagates the error of a delegate reader, after closing the readers
    /// already opened, and returns [`LuceneErrorKind::IllegalState`](crate::error::LuceneErrorKind::IllegalState) when a vector
    /// field with a format attribute lacks its suffix attribute or names an
    /// unknown format. A vector field without a format attribute is skipped,
    /// as Java skips it.
    pub fn new(read_state: &SegmentReadState) -> Result<Self> {
        let mut formats: JavaHashMap<Box<dyn KnnVectorsReader>> = JavaHashMap::new();
        let mut fields: IntObjectHashMap<String> = IntObjectHashMap::new();
        let mut field_names: HashMap<String, String> = HashMap::new();
        let mut names: HashMap<i32, String> = HashMap::new();

        let result = (|| -> Result<()> {
            for fi in read_state.field_infos.iter() {
                if fi.has_vector_values() {
                    // null formatName means the field is in fieldInfos, but
                    // has no vectors! (`PerFieldKnnVectorsFormat.java:215-216`)
                    let Some(format_name) = fi.get_attribute(PER_FIELD_KNN_VECTORS_FORMAT_KEY)
                    else {
                        continue;
                    };
                    let suffix = fi
                        .get_attribute(PER_FIELD_KNN_VECTORS_SUFFIX_KEY)
                        .ok_or_else(|| {
                            LuceneError::illegal_state(format!(
                            "missing attribute: {PER_FIELD_KNN_VECTORS_SUFFIX_KEY} for field: {}",
                            fi.name
                        ))
                        })?;
                    // `KnnVectorsFormat.forName(formatName)`: the loader's
                    // `IllegalArgumentException` for an unknown name.
                    let format = knn_vectors_lookup(&format_name)?;
                    let suffix_num = suffix.parse::<i32>().map_err(|_| {
                        LuceneError::illegal_state(format!(
                            "invalid KNN-vectors suffix for field \"{}\": {suffix}",
                            fi.name
                        ))
                    })?;
                    let segment_suffix = get_full_segment_suffix(
                        &read_state.segment_suffix,
                        &get_suffix(&format_name, suffix_num),
                    );
                    if !formats.contains_key(&segment_suffix) {
                        let state = read_state.with_new_suffix(segment_suffix.clone());
                        formats.insert(segment_suffix.clone(), format.fields_reader(&state)?);
                    }
                    field_names.insert(fi.name.clone(), segment_suffix.clone());
                    names.insert(fi.number, fi.name.clone());
                    fields.put(fi.number, segment_suffix);
                }
            }
            Ok(())
        })();

        if let Err(e) = result {
            // `IOUtils.closeWhileHandlingException(formats.values())`
            // (`PerFieldKnnVectorsFormat.java:236-240`).
            IOUtils::close_while_handling_exception(
                formats.iter_mut().map(|(_, r)| Some(move || r.close())),
            )?;
            return Err(e);
        }

        Ok(Self {
            formats,
            fields,
            field_names,
            names,
        })
    }

    fn reader_for(&self, field: &FieldInfo) -> Result<&(dyn KnnVectorsReader + '_)> {
        let suffix = self.fields.get(field.number).ok_or_else(|| {
            LuceneError::illegal_argument(format!(
                "field \"{}\" has no KNN-vectors reader",
                field.name
            ))
        })?;
        self.formats.get(suffix).map(|b| b.as_ref()).ok_or_else(|| {
            LuceneError::illegal_state(format!("missing KNN-vectors reader for suffix {suffix}"))
        })
    }
}

#[cfg(test)]
impl KnnFieldsReader {
    /// A reader that dispatches every field of `fields` — `(number, name)`
    /// pairs — to `reader`, as a segment written with one per-field format
    /// does.
    pub(crate) fn with_one_format_for_tests(
        fields: &[(i32, &str)],
        reader: Box<dyn KnnVectorsReader>,
    ) -> Self {
        let suffix = "Test_0".to_string();
        let mut formats = JavaHashMap::new();
        formats.insert(suffix.clone(), reader);
        let mut numbers = IntObjectHashMap::new();
        for &(number, _) in fields {
            numbers.put(number, suffix.clone());
        }
        Self {
            formats,
            fields: numbers,
            field_names: fields
                .iter()
                .map(|&(_, name)| (name.to_string(), suffix.clone()))
                .collect(),
            names: fields
                .iter()
                .map(|&(number, name)| (number, name.to_string()))
                .collect(),
        }
    }
}

impl std::fmt::Debug for KnnFieldsReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PerFieldKnnVectorsReader")
            .field("formats", &self.formats.len())
            .finish_non_exhaustive()
    }
}

impl KnnVectorsReader for KnnFieldsReader {
    /// This reader answers with itself: it is the `true` arm of Java's
    /// `vectorsReader instanceof PerFieldKnnVectorsFormat.FieldsReader`
    /// (`Lucene104ScalarQuantizedVectorsWriter.java:389`).
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// Returns the underlying vectors reader `field` is dispatched to, or
    /// `None` when this reader does not know the field.
    ///
    /// Equivalent to `PerFieldKnnVectorsFormat.FieldsReader#getFieldReader(String)`,
    /// which looks the field's `FieldInfo` up and answers `null` when there is
    /// none. Java resolves the delegate through the field *number*; this reader
    /// keeps the same pairs keyed by name, so no `FieldInfos` lookup is needed.
    fn get_field_reader(&self, field: &str) -> Option<&dyn KnnVectorsReader> {
        let suffix = self.field_names.get(field)?;
        self.formats.get(suffix).map(|reader| reader.as_ref())
    }

    fn check_integrity(&self) -> Result<()> {
        for reader in self.formats.values() {
            reader.check_integrity()?;
        }
        Ok(())
    }

    /// Returns the field's vectors from the reader the field was written
    /// with, or `None` for a field this segment holds no vectors for.
    ///
    /// Equivalent to `PerFieldKnnVectorsFormat.FieldsReader.getFloatVectorValues(String)`
    /// (`PerFieldKnnVectorsFormat.java:285-293`), which answers `null` when the
    /// field is unknown or has no reader.
    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        match self.get_field_reader(field) {
            Some(reader) => reader.get_float_vector_values(field),
            None => Ok(None),
        }
    }

    /// Equivalent to `PerFieldKnnVectorsFormat.FieldsReader.getByteVectorValues(String)`
    /// (`PerFieldKnnVectorsFormat.java:295-303`); see
    /// [`get_float_vector_values`](Self::get_float_vector_values).
    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        match self.get_field_reader(field) {
            Some(reader) => reader.get_byte_vector_values(field),
            None => Ok(None),
        }
    }

    fn search(
        &self,
        field: &str,
        target: &[f32],
        knn_collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn crate::search::AcceptDocs,
    ) -> Result<()> {
        // `PerFieldKnnVectorsFormat.java:304-314`: a field this segment holds no
        // vectors for collects nothing.
        let Some(reader) = self.get_field_reader(field) else {
            return Ok(());
        };
        reader.search(field, target, knn_collector, accept_docs)
    }

    fn search_byte(
        &self,
        field: &str,
        target: &[u8],
        knn_collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn crate::search::AcceptDocs,
    ) -> Result<()> {
        // `PerFieldKnnVectorsFormat.java:316-326`.
        let Some(reader) = self.get_field_reader(field) else {
            return Ok(());
        };
        reader.search_byte(field, target, knn_collector, accept_docs)
    }

    /// Returns a reader that dispatches every field to a merge instance of the
    /// reader that field used.
    ///
    /// Equivalent to `PerFieldKnnVectorsFormat.FieldsReader.getMergeInstance()`,
    /// which is `new FieldsReader(this)`. That copy constructor
    /// (`PerFieldKnnVectorsFormat.java:243-250`) asks the delegate for a merge
    /// instance once *per field*, not once per format, so two fields of one
    /// format get two instances; each field is therefore its own entry here,
    /// keyed by the field's name.
    fn get_merge_instance(&self) -> Result<Box<dyn KnnVectorsReader>> {
        let mut formats: JavaHashMap<Box<dyn KnnVectorsReader>> = JavaHashMap::new();
        let mut fields: IntObjectHashMap<String> = IntObjectHashMap::new();
        let mut field_names: HashMap<String, String> = HashMap::with_capacity(self.names.len());
        for (&number, name) in &self.names {
            let Some(suffix) = self.fields.get(number) else {
                continue;
            };
            let reader = self.formats.get(suffix).ok_or_else(|| {
                LuceneError::illegal_state(format!(
                    "missing KNN-vectors reader for suffix {suffix}"
                ))
            })?;
            formats.insert(name.clone(), reader.get_merge_instance()?);
            fields.put(number, name.clone());
            field_names.insert(name.clone(), name.clone());
        }
        Ok(Box::new(KnnFieldsReader {
            formats,
            fields,
            field_names,
            names: self.names.clone(),
        }))
    }

    fn finish_merge(&mut self) -> Result<()> {
        for (_, reader) in self.formats.iter_mut() {
            reader.finish_merge()?;
        }
        Ok(())
    }

    fn get_off_heap_byte_size(&self, field_info: &FieldInfo) -> Result<HashMap<String, i64>> {
        match self.reader_for(field_info) {
            Ok(reader) => reader.get_off_heap_byte_size(field_info),
            Err(_) => Ok(HashMap::new()),
        }
    }

    /// Equivalent to `PerFieldKnnVectorsFormat.FieldsReader.close()`
    /// (`PerFieldKnnVectorsFormat.java:335-341`): `IOUtils.close(readers)`
    /// over `fields.values()`, one reader per field, so a reader several
    /// fields share is closed once for each of them.
    fn close(&self) -> Result<()> {
        // `for (ObjectCursor<KnnVectorsReader> cursor : fields.values())
        // readers.add(cursor.value);`, in the map's own iteration order.
        let suffixes: Vec<&String> = self
            .fields
            .values()
            .into_iter()
            .map(|cursor| cursor.value)
            .collect();
        let formats = &self.formats;
        IOUtils::close(suffixes.into_iter().map(|suffix| {
            Some(move || match formats.get(suffix) {
                Some(reader) => reader.close(),
                None => Ok(()),
            })
        }))
    }
}

// -----------------------------------------------------------------------------
// PerFieldMergeState
// -----------------------------------------------------------------------------

/// Restricts a [`MergeState`] to a subset of fields.
///
/// Equivalent to `org.apache.lucene.codecs.perfield.PerFieldMergeState`, a
/// utility class Java declares package-private and never instantiates.
pub struct PerFieldMergeState;

impl PerFieldMergeState {
    /// Returns a merge state that exposes only `fields`.
    ///
    /// Equivalent to `PerFieldMergeState.restrictFields(MergeState,
    /// Collection<String>)` (`PerFieldMergeState.java:43-74`). Only three
    /// things are restricted: every per-reader `FieldInfos` and the merged one
    /// become filtered views ([`crate::index::FieldInfos::filter`], Java's
    /// `FilterFieldInfos`), and every non-null postings producer is wrapped so
    /// that it lists only `fields` (`FilterFieldsProducer`). Everything else — the doc maps, the
    /// merged segment's `SegmentInfo`, every stored-fields, term-vectors, norms,
    /// doc-values, points and vectors reader, the live docs, the document
    /// counts, the info stream and the index-sort flag — is the original
    /// state's own, because Java hands the original's references to the
    /// `MergeState` constructor. The filtered views keep every field, so field
    /// numbers stay consistent.
    ///
    /// The wrapped producers are the original's own, not their merge
    /// instances: the merge state already holds merge instances
    /// (`MergeState.java:155-158`), and Java wraps `in.fieldsProducers[i]`
    /// as it is.
    ///
    /// `fields` keeps its order: a wrapped producer lists the fields in the
    /// order of the collection it was given, as Java's iterates the list it
    /// copied.
    pub fn restrict_fields(merge_state: &MergeState, fields: &[String]) -> Result<MergeState> {
        let field_infos: Vec<super::stub::FieldInfos> = merge_state
            .field_infos
            .iter()
            .map(|fi| fi.filter(fields.iter().cloned()))
            .collect();
        let fields_producers: Vec<Option<Arc<dyn FieldsProducer>>> = merge_state
            .fields_producers
            .iter()
            .map(|producer| {
                producer.as_ref().map(|producer| {
                    Arc::new(FilterFieldsProducer::new(Arc::clone(producer), fields))
                        as Arc<dyn FieldsProducer>
                })
            })
            .collect();
        let merge_field_infos = merge_state.merge_field_infos.filter(fields.iter().cloned());
        Ok(MergeState {
            field_infos,
            fields_producers,
            merge_field_infos,
            ..merge_state.clone()
        })
    }
}

/// A postings producer that lists only some of the fields of the producer it
/// wraps.
///
/// Equivalent to the private `PerFieldMergeState.FilterFieldsProducer`
/// (`PerFieldMergeState.java:230-271`).
///
/// **Divergence from Lucene 10.5.0.** Java's `close()` closes the wrapped
/// producer. The wrapped producer is shared — with the merge state this view
/// was derived from, exactly as in Java — and this port can only close a
/// producer it owns (`FieldsProducer::close` takes `&mut self`), so `close`
/// here releases nothing. No merge closes a restricted state's producers:
/// they belong to the readers being merged, which close them.
struct FilterFieldsProducer {
    inner: Arc<dyn FieldsProducer>,
    filtered: Vec<String>,
}

impl FilterFieldsProducer {
    /// Equivalent to `FilterFieldsProducer(FieldsProducer, Collection<String>)`.
    fn new(inner: Arc<dyn FieldsProducer>, filter_fields: &[String]) -> Self {
        Self {
            inner,
            filtered: filter_fields.to_vec(),
        }
    }
}

impl Fields for FilterFieldsProducer {
    fn size(&self) -> i32 {
        self.filtered.len() as i32
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        if !self.filtered.iter().any(|name| name == field) {
            return Err(LuceneError::illegal_argument(format!(
                "The field named '{field}' is not accessible in the current merge context, \
                 available ones are: [{}]",
                self.filtered.join(", ")
            )));
        }
        self.inner.terms(field)
    }

    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(self.filtered.iter().cloned())
    }
}

impl FieldsProducer for FilterFieldsProducer {
    fn check_integrity(&self) -> Result<()> {
        self.inner.check_integrity()
    }

    /// `FilterFieldsProducer` does not override `getMergeInstance()`, so Java
    /// inherits `FieldsProducer.getMergeInstance()`, which returns `this`; the
    /// owned equivalent is a view over the same producer and fields.
    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        Ok(Box::new(FilterFieldsProducer {
            inner: Arc::clone(&self.inner),
            filtered: self.filtered.clone(),
        }))
    }

    fn close(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::stub::FieldInfos;
    use crate::error::LuceneErrorKind;
    use crate::index::{IndexOptions, SegmentInfo};

    fn write_state() -> SegmentWriteState<'static> {
        use crate::codecs::stub::BufferedUpdates;
        use crate::store::DEFAULT_IO_CONTEXT;

        let dir: Arc<dyn crate::store::Directory> =
            Arc::new(crate::store::ByteBuffersDirectory::default());
        let info_stream: &'static dyn crate::util::InfoStream = &crate::util::NoOutputInfoStream;
        let segment_info: &'static SegmentInfo = Box::leak(Box::new(
            crate::codecs::tests::test_segment_info("test", 10),
        ));
        let field_infos: &'static FieldInfos = Box::leak(Box::new(FieldInfos::default()));
        let seg_updates: &'static BufferedUpdates = Box::leak(Box::new(BufferedUpdates::default()));
        let context: &'static dyn crate::store::IOContext = &*DEFAULT_IO_CONTEXT;

        SegmentWriteState::new(
            info_stream,
            dir,
            segment_info,
            field_infos,
            seg_updates,
            context,
        )
    }

    fn read_state() -> SegmentReadState<'static> {
        use crate::store::DEFAULT_IO_CONTEXT;

        let dir = crate::store::ByteBuffersDirectory::default();
        let dir_ref: &'static dyn crate::store::Directory = Box::leak(Box::new(dir));
        let segment_info: &'static SegmentInfo = Box::leak(Box::new(
            crate::codecs::tests::test_segment_info("test", 10),
        ));
        let field_infos: &'static FieldInfos = Box::leak(Box::new(FieldInfos::default()));
        let context: &'static dyn crate::store::IOContext = &*DEFAULT_IO_CONTEXT;

        SegmentReadState::new(dir_ref, segment_info, field_infos, context)
    }

    #[derive(Debug, Default, Clone)]
    struct NamedPostingsFormat(&'static str);

    impl PostingsFormat for NamedPostingsFormat {
        fn name(&self) -> &str {
            self.0
        }

        fn fields_consumer<'a>(
            &self,
            _state: &SegmentWriteState<'a>,
        ) -> Result<Box<dyn FieldsConsumer + 'a>> {
            Ok(Box::new(super::NoOpFieldsConsumer))
        }

        fn fields_producer<'a>(
            &self,
            _state: &SegmentReadState<'a>,
        ) -> Result<Box<dyn FieldsProducer>> {
            Ok(Box::new(NamedFieldsProducer(self.0)))
        }
    }

    #[derive(Debug, Default, Clone)]
    #[allow(dead_code)]
    struct NamedFieldsProducer(&'static str);

    impl Fields for NamedFieldsProducer {
        fn size(&self) -> i32 {
            0
        }

        fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
            Ok(None)
        }

        fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
            Box::new(std::iter::empty())
        }
    }

    impl FieldsProducer for NamedFieldsProducer {
        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }

        fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
            Ok(Box::new(self.clone()))
        }

        fn close(&self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn per_field_postings_name_and_factories() {
        let format = PerFieldPostingsFormat::new(|_| Arc::new(NamedPostingsFormat("Foo")));
        assert_eq!(format.name(), PER_FIELD_POSTINGS_NAME);

        let mut consumer = format.fields_consumer(&write_state()).unwrap();
        consumer.close().unwrap();

        let producer = format.fields_producer(&read_state()).unwrap();
        producer.close().unwrap();
    }

    /// A postings format whose consumer records being closed, and fails to
    /// write or to close when asked to.
    #[derive(Debug, Clone)]
    struct RecordingFormat {
        name: &'static str,
        fail_write: bool,
        fail_close: bool,
        closed: Arc<std::sync::atomic::AtomicBool>,
    }

    #[derive(Debug)]
    struct RecordingConsumer(RecordingFormat);

    impl FieldsConsumer for RecordingConsumer {
        fn write(
            &mut self,
            _fields: &dyn Fields,
            _norms: Option<&dyn NormsProducer>,
        ) -> Result<()> {
            if self.0.fail_write {
                return Err(LuceneError::illegal_state(format!("{} write", self.0.name)));
            }
            Ok(())
        }

        fn close(&mut self) -> Result<()> {
            self.0
                .closed
                .store(true, std::sync::atomic::Ordering::SeqCst);
            if self.0.fail_close {
                return Err(LuceneError::illegal_state(format!("{} close", self.0.name)));
            }
            Ok(())
        }
    }

    impl PostingsFormat for RecordingFormat {
        fn name(&self) -> &str {
            self.name
        }

        fn fields_consumer<'a>(
            &self,
            _state: &SegmentWriteState<'a>,
        ) -> Result<Box<dyn FieldsConsumer + 'a>> {
            Ok(Box::new(RecordingConsumer(self.clone())))
        }

        fn fields_producer<'a>(
            &self,
            _state: &SegmentReadState<'a>,
        ) -> Result<Box<dyn FieldsProducer>> {
            Ok(Box::new(NamedFieldsProducer(self.name)))
        }
    }

    fn recording_format(
        name: &'static str,
        fail_write: bool,
        fail_close: bool,
    ) -> (Arc<RecordingFormat>, Arc<std::sync::atomic::AtomicBool>) {
        let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let format = RecordingFormat {
            name,
            fail_write,
            fail_close,
            closed: Arc::clone(&closed),
        };
        (Arc::new(format), closed)
    }

    /// Fields `body` and `title`, indexed.
    #[derive(Debug, Default, Clone)]
    struct BodyAndTitle;

    impl Fields for BodyAndTitle {
        fn size(&self) -> i32 {
            2
        }
        fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
            Ok(None)
        }
        fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
            Box::new(["body".to_string(), "title".to_string()].into_iter())
        }
    }

    fn body_and_title_infos() -> FieldInfos {
        let mut body = FieldInfo::new("body", 0);
        body.index_options = IndexOptions::DOCS;
        let mut title = FieldInfo::new("title", 1);
        title.index_options = IndexOptions::DOCS;
        FieldInfos::new(vec![body, title]).unwrap()
    }

    /// Each group's consumer is added to the consumers to close before it
    /// writes, and a failed write closes it while keeping the write's error
    /// (`PerFieldPostingsFormat.java:160-168`).
    #[test]
    fn a_failed_write_closes_the_consumer_that_failed() {
        let (format, closed) = recording_format("Failing", true, false);
        let per_field =
            PerFieldPostingsFormat::new(move |_| Arc::clone(&format) as Arc<dyn PostingsFormat>);
        let mut state = write_state();
        state.field_infos = Box::leak(Box::new(body_and_title_infos()));
        let mut consumer = per_field.fields_consumer(&state).unwrap();
        let error = consumer.write(&BodyAndTitle, None).unwrap_err();
        assert!(format!("{error:?}").contains("Failing write"), "{error:?}");
        assert!(closed.load(std::sync::atomic::Ordering::SeqCst));
    }

    /// `close` closes every group's consumer and raises the first failure once
    /// all are closed: `IOUtils.close(toClose)` (`PerFieldPostingsFormat.java:264-267`).
    #[test]
    fn close_closes_every_consumer_and_raises_the_first_failure() {
        let (body_format, body_closed) = recording_format("BodyFormat", false, true);
        let (title_format, title_closed) = recording_format("TitleFormat", false, true);
        let per_field = PerFieldPostingsFormat::new(move |field| {
            if field == "body" {
                Arc::clone(&body_format) as Arc<dyn PostingsFormat>
            } else {
                Arc::clone(&title_format) as Arc<dyn PostingsFormat>
            }
        });
        let mut state = write_state();
        state.field_infos = Box::leak(Box::new(body_and_title_infos()));
        let mut consumer = per_field.fields_consumer(&state).unwrap();
        consumer.write(&BodyAndTitle, None).unwrap();
        let error = consumer.close().unwrap_err();
        assert!(format!("{error:?}").contains("Format close"), "{error:?}");
        assert!(body_closed.load(std::sync::atomic::Ordering::SeqCst));
        assert!(title_closed.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn per_field_postings_writer_records_two_formats() {
        let mut body = FieldInfo::new("body", 0);
        body.index_options = IndexOptions::DOCS;
        let mut title = FieldInfo::new("title", 1);
        title.index_options = IndexOptions::DOCS;
        let field_infos = FieldInfos::new(vec![body, title]).unwrap();

        let format_a = Arc::new(NamedPostingsFormat("FormatA"));
        let format_b = Arc::new(NamedPostingsFormat("FormatB"));
        let format = PerFieldPostingsFormat::new(move |field| {
            if field == "body" {
                format_a.clone()
            } else {
                format_b.clone()
            }
        });

        let mut state = write_state();
        state.field_infos = Box::leak(Box::new(field_infos));

        let mut consumer = format.fields_consumer(&state).unwrap();

        #[derive(Debug, Default, Clone)]
        struct TwoFields;
        impl Fields for TwoFields {
            fn size(&self) -> i32 {
                2
            }
            fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
                Ok(None)
            }
            fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
                Box::new(["body".to_string(), "title".to_string()].into_iter())
            }
        }

        consumer.write(&TwoFields, None).unwrap();
        consumer.close().unwrap();

        let body_info = state.field_infos.field_info("body").unwrap();
        let title_info = state.field_infos.field_info("title").unwrap();
        assert_eq!(
            body_info.get_attribute(PER_FIELD_POSTINGS_FORMAT_KEY),
            Some("FormatA".to_string())
        );
        assert_eq!(
            title_info.get_attribute(PER_FIELD_POSTINGS_FORMAT_KEY),
            Some("FormatB".to_string())
        );
        assert_eq!(
            body_info.get_attribute(PER_FIELD_POSTINGS_SUFFIX_KEY),
            Some("0".to_string())
        );
        assert_eq!(
            title_info.get_attribute(PER_FIELD_POSTINGS_SUFFIX_KEY),
            Some("0".to_string())
        );
    }

    /// `PerFieldMergeState.restrictFields` restricts the field infos and the
    /// postings producers, and nothing else: every other component of the
    /// restricted state is the original's own object (`PerFieldMergeState.java:56-73`).
    #[test]
    fn restrict_fields_shares_everything_but_the_fields() {
        use crate::codecs::norms::EmptyNormsProducer;
        use crate::codecs::points::EmptyPointsReader;
        use crate::codecs::{DocValuesProducer, NormsProducer, PointsReader};
        use crate::index::doc_values::EmptyDocValuesProducer;
        use crate::util::{Bits, FixedBitSet, InfoStream, NoOutputInfoStream};

        let mut body = FieldInfo::new("body", 0);
        body.index_options = IndexOptions::DOCS;
        let mut title = FieldInfo::new("title", 1);
        title.index_options = IndexOptions::DOCS;
        let mut vector = FieldInfo::new("vector", 2);
        vector.vector_dimension = 2;
        let infos = || FieldInfos::new(vec![body.clone(), title.clone(), vector.clone()]).unwrap();

        let doc_maps: Vec<crate::index::merge::DocMap> = vec![
            crate::index::merge::identity_doc_map(10, 0),
            crate::index::merge::identity_doc_map(5, 10),
        ];
        let mut deletions = FixedBitSet::new(5);
        deletions.set(1);
        let live_docs: Vec<Option<Arc<dyn Bits>>> = vec![None, Some(Arc::new(deletions))];
        let info_stream: Arc<dyn InfoStream> = Arc::new(NoOutputInfoStream);
        let norms: Arc<dyn NormsProducer> = Arc::new(EmptyNormsProducer);
        let doc_values: Arc<dyn DocValuesProducer> = Arc::new(EmptyDocValuesProducer);
        let points: Arc<dyn PointsReader> = Arc::new(EmptyPointsReader);
        let postings: Arc<dyn FieldsProducer> = Arc::new(NamedFieldsProducer("A"));
        let segment_info = Arc::new(crate::codecs::tests::test_segment_info("_m", 15));

        let mut original =
            MergeState::from_fields_producers(vec![Some(Arc::clone(&postings)), None], vec![10, 5])
                .with_field_infos(vec![infos(), infos()], infos());
        original.doc_maps = doc_maps.clone();
        original.live_docs = live_docs.clone();
        original.info_stream = Arc::clone(&info_stream);
        original.needs_index_sort = true;
        original.segment_info = Some(Arc::clone(&segment_info));
        original.norms_producers = vec![Some(Arc::clone(&norms)), None];
        original.doc_values_producers = vec![None, Some(Arc::clone(&doc_values))];
        original.points_readers = vec![Some(Arc::clone(&points)), None];

        let restricted =
            PerFieldMergeState::restrict_fields(&original, &["title".to_string()]).unwrap();

        // Shared, as Java passes the original's references.
        for (restricted_map, original_map) in restricted.doc_maps.iter().zip(&doc_maps) {
            assert!(Arc::ptr_eq(restricted_map, original_map), "doc maps");
        }
        assert!(restricted.live_docs[0].is_none());
        assert!(
            Arc::ptr_eq(
                restricted.live_docs[1].as_ref().unwrap(),
                live_docs[1].as_ref().unwrap()
            ),
            "live docs"
        );
        assert!(
            Arc::ptr_eq(&restricted.info_stream, &info_stream),
            "info stream"
        );
        assert!(restricted.needs_index_sort, "the index-sort flag");
        assert!(Arc::ptr_eq(
            restricted.segment_info.as_ref().unwrap(),
            &segment_info
        ));
        assert_eq!(restricted.max_docs, vec![10, 5]);
        assert!(Arc::ptr_eq(
            restricted.norms_producers[0].as_ref().unwrap(),
            &norms
        ));
        assert!(Arc::ptr_eq(
            restricted.doc_values_producers[1].as_ref().unwrap(),
            &doc_values
        ));
        assert!(Arc::ptr_eq(
            restricted.points_readers[0].as_ref().unwrap(),
            &points
        ));
        assert_eq!(restricted.stored_fields_readers.len(), 2);
        assert_eq!(restricted.term_vectors_readers.len(), 2);
        assert_eq!(restricted.knn_vectors_readers.len(), 2);

        // Restricted: the fields, and the postings producers that list them.
        let names: Vec<String> = restricted
            .merge_field_infos
            .iter()
            .map(|fi| fi.name.clone())
            .collect();
        assert_eq!(names, vec!["title".to_string()]);
        assert!(restricted.merge_field_infos.has_postings());
        assert!(
            restricted.merge_field_infos.has_vector_values(),
            "FilterFieldInfos keeps the unfiltered hasVectorValues()"
        );
        for field_infos in &restricted.field_infos {
            assert_eq!(field_infos.size(), 1);
            assert_eq!(field_infos.field_info_by_number(1).unwrap().name, "title");
        }
        let producer = restricted.fields_producers[0].as_ref().unwrap();
        assert_eq!(
            producer.iterator().collect::<Vec<_>>(),
            vec!["title".to_string()]
        );
        assert!(producer.terms("title").unwrap().is_none());
        assert!(matches!(
            producer.terms("body"),
            Err(LuceneError {
                kind: LuceneErrorKind::IllegalArgument { .. },
                ..
            })
        ));
        assert!(restricted.fields_producers[1].is_none());
    }

    #[test]
    fn per_field_merge_state_restricts_fields() {
        let body = FieldInfo::new("body", 0);
        let title = FieldInfo::new("title", 1);
        let merge_field_infos = FieldInfos::new(vec![body.clone(), title.clone()]).unwrap();
        let segment_field_infos = FieldInfos::new(vec![body, title]).unwrap();

        let producer_a = Arc::new(NamedFieldsProducer("A")) as Arc<dyn FieldsProducer>;
        let merge_state = MergeState::from_fields_producers(vec![Some(producer_a)], vec![10])
            .with_field_infos(vec![segment_field_infos], merge_field_infos);

        let restricted =
            PerFieldMergeState::restrict_fields(&merge_state, &["title".to_string()]).unwrap();
        assert_eq!(restricted.merge_field_infos.len(), 1);
        assert_eq!(
            restricted
                .merge_field_infos
                .field_info("title")
                .unwrap()
                .number,
            1
        );
        assert!(restricted.merge_field_infos.field_info("body").is_none());

        let producer = restricted.fields_producers[0].as_ref().unwrap();
        let names: Vec<String> = producer.iterator().collect();
        assert_eq!(names, vec!["title".to_string()]);
        assert_eq!(producer.size(), 1);
    }
}

/// The `IOUtils` close family at the per-field formats' call sites (#308).
#[cfg(test)]
mod close_family_tests {
    use super::*;
    use crate::codecs::postings::{Fields, Terms};
    use crate::error::LuceneErrorKind;
    use std::sync::Mutex;

    /// A sub-format reader or writer that records its `close()` and fails it
    /// when asked to.
    #[derive(Debug, Clone)]
    struct Closing {
        name: &'static str,
        fail: bool,
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    impl Closing {
        fn record(&self) -> Result<()> {
            self.log.lock().unwrap().push(self.name);
            if self.fail {
                return Err(LuceneError::from(LuceneErrorKind::Io(
                    std::io::Error::other(self.name),
                )));
            }
            Ok(())
        }
    }

    impl Fields for Closing {
        fn size(&self) -> i32 {
            0
        }
        fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
            Ok(None)
        }
        fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
            Box::new(std::iter::empty())
        }
    }

    impl FieldsProducer for Closing {
        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }
        fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
            Ok(Box::new(self.clone()))
        }
        fn close(&self) -> Result<()> {
            self.record()
        }
    }

    impl DocValuesProducer for Closing {
        fn get_numeric(
            &self,
            _field: &FieldInfo,
        ) -> Result<Option<Box<dyn crate::index::NumericDocValues>>> {
            Ok(None)
        }
        fn get_binary(
            &self,
            _field: &FieldInfo,
        ) -> Result<Option<Box<dyn crate::index::BinaryDocValues>>> {
            Ok(None)
        }
        fn get_sorted(
            &self,
            _field: &FieldInfo,
        ) -> Result<Option<Box<dyn crate::index::SortedDocValues>>> {
            Ok(None)
        }
        fn get_sorted_set(
            &self,
            _field: &FieldInfo,
        ) -> Result<Option<Box<dyn crate::index::SortedSetDocValues>>> {
            Ok(None)
        }
        fn get_sorted_numeric(
            &self,
            _field: &FieldInfo,
        ) -> Result<Option<Box<dyn crate::index::SortedNumericDocValues>>> {
            Ok(None)
        }
        fn get_skipper(
            &self,
            _field: &FieldInfo,
        ) -> Result<Option<Box<dyn crate::index::DocValuesSkipper>>> {
            Ok(None)
        }
        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }
        fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
            Ok(Box::new(self.clone()))
        }
        fn close(&self) -> Result<()> {
            self.record()
        }
    }

    impl DocValuesConsumer for Closing {
        fn add_numeric_field(&mut self, _: &FieldInfo, _: &dyn DocValuesProducer) -> Result<()> {
            Ok(())
        }
        fn add_binary_field(&mut self, _: &FieldInfo, _: &dyn DocValuesProducer) -> Result<()> {
            Ok(())
        }
        fn add_sorted_field(&mut self, _: &FieldInfo, _: &dyn DocValuesProducer) -> Result<()> {
            Ok(())
        }
        fn add_sorted_numeric_field(
            &mut self,
            _: &FieldInfo,
            _: &dyn DocValuesProducer,
        ) -> Result<()> {
            Ok(())
        }
        fn add_sorted_set_field(&mut self, _: &FieldInfo, _: &dyn DocValuesProducer) -> Result<()> {
            Ok(())
        }
        fn close(&mut self) -> Result<()> {
            self.record()
        }
    }

    impl KnnVectorsReader for Closing {
        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }
        fn get_float_vector_values(
            &self,
            _field: &str,
        ) -> Result<Option<Box<dyn crate::index::FloatVectorValues>>> {
            Ok(None)
        }
        fn get_byte_vector_values(
            &self,
            _field: &str,
        ) -> Result<Option<Box<dyn crate::index::ByteVectorValues>>> {
            Ok(None)
        }
        fn search(
            &self,
            _field: &str,
            _target: &[f32],
            _knn_collector: &mut dyn crate::search::KnnCollector,
            _accept_docs: &mut dyn crate::search::AcceptDocs,
        ) -> Result<()> {
            Ok(())
        }
        fn search_byte(
            &self,
            _field: &str,
            _target: &[u8],
            _knn_collector: &mut dyn crate::search::KnnCollector,
            _accept_docs: &mut dyn crate::search::AcceptDocs,
        ) -> Result<()> {
            Ok(())
        }
        fn get_merge_instance(&self) -> Result<Box<dyn KnnVectorsReader>> {
            Ok(Box::new(self.clone()))
        }
        fn close(&self) -> Result<()> {
            self.record()
        }
    }

    impl KnnVectorsWriter for Closing {
        fn add_field(
            &mut self,
            _field_info: &FieldInfo,
        ) -> Result<super::super::knn_vectors::FieldVectorWriter> {
            Err(LuceneError::unsupported_operation("test".to_string()))
        }
        fn flush(
            &mut self,
            _max_doc: i32,
            _sort_map: Option<&super::super::knn_vectors::SorterDocMap>,
        ) -> Result<()> {
            Ok(())
        }
        fn finish(&mut self) -> Result<()> {
            Ok(())
        }
        fn close(&mut self) -> Result<()> {
            self.record()
        }
        fn ram_bytes_used(&self) -> i64 {
            0
        }
    }

    fn failing_pair() -> (Closing, Closing, Arc<Mutex<Vec<&'static str>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let a = Closing {
            name: "a",
            fail: true,
            log: Arc::clone(&log),
        };
        let b = Closing {
            name: "b",
            fail: true,
            log: Arc::clone(&log),
        };
        (a, b, log)
    }

    /// A `JavaHashMap` holding `entries`, put in the order given.
    fn java_map<V, const N: usize>(entries: [(&str, V); N]) -> JavaHashMap<V> {
        let mut map = JavaHashMap::new();
        for (key, value) in entries {
            map.insert(key.to_string(), value);
        }
        map
    }

    /// An hppc `IntObjectHashMap` holding `entries`, put in the order given.
    fn int_map<const N: usize>(entries: [(i32, &str); N]) -> IntObjectHashMap<String> {
        let mut map = IntObjectHashMap::new();
        for (key, value) in entries {
            map.put(key, value.to_string());
        }
        map
    }

    /// The message of the `IOException` a [`Closing`] raised.
    fn surfaced(error: LuceneError) -> String {
        match error {
            LuceneError {
                kind: LuceneErrorKind::Io(e),
                ..
            } => e.to_string(),
            other => format!("{other:?}"),
        }
    }

    /// Eight failing closeables named after the segment suffixes `format_0` to
    /// `format_7`, in that order — the order Java's per-field readers put them
    /// into their `formats` map, one per new suffix in field order.
    fn eight_failing_suffixes(
        names: &'static [&'static str; 8],
    ) -> (Vec<(&'static str, Closing)>, Arc<Mutex<Vec<&'static str>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let closings = names
            .iter()
            .map(|&name| {
                (
                    name,
                    Closing {
                        name,
                        fail: true,
                        log: Arc::clone(&log),
                    },
                )
            })
            .collect();
        (closings, log)
    }

    const POSTINGS_SUFFIXES: [&str; 8] = [
        "Lucene104_0",
        "Lucene104_1",
        "Lucene104_2",
        "Lucene104_3",
        "Lucene104_4",
        "Lucene104_5",
        "Lucene104_6",
        "Lucene104_7",
    ];

    const DOC_VALUES_SUFFIXES: [&str; 8] = [
        "Lucene90_0",
        "Lucene90_1",
        "Lucene90_2",
        "Lucene90_3",
        "Lucene90_4",
        "Lucene90_5",
        "Lucene90_6",
        "Lucene90_7",
    ];

    /// rmp #308 (audit L2): `IOUtils.close(formats.values())`
    /// (`PerFieldPostingsFormat.java:350-353`) walks a `java.util.HashMap`
    /// keyed by segment suffix, so the producers close in that map's order and
    /// the first failure in it is the one that surfaces. Oracle
    /// `scratchpad/fixF/oracle/PerFieldOrderOracle.java`: `closed
    /// [Lucene104_3, Lucene104_2, Lucene104_1, Lucene104_0, Lucene104_7,
    /// Lucene104_6, Lucene104_5, Lucene104_4] surfaced Lucene104_3`.
    #[test]
    fn postings_reader_close_order_and_first_failure_are_javas() {
        let (closings, log) = eight_failing_suffixes(&POSTINGS_SUFFIXES);
        let mut formats = JavaHashMap::new();
        for (suffix, closing) in closings {
            formats.insert(
                suffix.to_string(),
                Box::new(closing) as Box<dyn FieldsProducer>,
            );
        }
        let reader = FieldsReader {
            formats,
            fields: BTreeMap::new(),
        };
        assert_eq!(surfaced(reader.close().unwrap_err()), "Lucene104_3");
        assert_eq!(
            *log.lock().unwrap(),
            [
                "Lucene104_3",
                "Lucene104_2",
                "Lucene104_1",
                "Lucene104_0",
                "Lucene104_7",
                "Lucene104_6",
                "Lucene104_5",
                "Lucene104_4"
            ]
        );
    }

    /// rmp #308 (audit L2): `IOUtils.close(formats.values())`
    /// (`PerFieldDocValuesFormat.java:354-357`) over the suffix-keyed
    /// `java.util.HashMap`. Oracle
    /// `scratchpad/fixF/oracle/PerFieldOrderOracle.java`: `closed [Lucene90_3,
    /// Lucene90_2, Lucene90_5, Lucene90_4, Lucene90_1, Lucene90_0, Lucene90_7,
    /// Lucene90_6] surfaced Lucene90_3`.
    #[test]
    fn doc_values_reader_close_order_and_first_failure_are_javas() {
        let (closings, log) = eight_failing_suffixes(&DOC_VALUES_SUFFIXES);
        let mut formats = JavaHashMap::new();
        for (suffix, closing) in closings {
            formats.insert(
                suffix.to_string(),
                Box::new(closing) as Box<dyn DocValuesProducer>,
            );
        }
        let reader = DvFieldsReader {
            formats,
            fields: HashMap::new(),
        };
        assert_eq!(surfaced(reader.close().unwrap_err()), "Lucene90_3");
        assert_eq!(
            *log.lock().unwrap(),
            [
                "Lucene90_3",
                "Lucene90_2",
                "Lucene90_5",
                "Lucene90_4",
                "Lucene90_1",
                "Lucene90_0",
                "Lucene90_7",
                "Lucene90_6"
            ]
        );
    }

    /// rmp #308 (audit L2): `PerFieldKnnVectorsFormat.FieldsReader.close()`
    /// collects `fields.values()` of an hppc `IntObjectHashMap`
    /// (`PerFieldKnnVectorsFormat.java:335-341`), which visits the occupied
    /// slots from a per-map seed and the empty key `0` last; with fields `0`
    /// and `1` that is field 1 then field 0 whatever the seed. Oracle
    /// `scratchpad/fixF/oracle/PerFieldOrderOracle.java`, over 64 fresh maps:
    /// `[field1, field0] surfaced field1`, the only order seen. Sixty-four
    /// readers here too, so an order that merely happens to match once cannot
    /// pass.
    #[test]
    fn knn_reader_close_order_and_first_failure_are_javas() {
        for _ in 0..64 {
            let log = Arc::new(Mutex::new(Vec::new()));
            let closing = |name| Closing {
                name,
                fail: true,
                log: Arc::clone(&log),
            };
            let reader = KnnFieldsReader {
                formats: java_map([
                    (
                        "field0",
                        Box::new(closing("field0")) as Box<dyn KnnVectorsReader>,
                    ),
                    (
                        "field1",
                        Box::new(closing("field1")) as Box<dyn KnnVectorsReader>,
                    ),
                ]),
                fields: int_map([(0, "field0"), (1, "field1")]),
                field_names: HashMap::new(),
                names: HashMap::new(),
            };
            assert_eq!(surfaced(reader.close().unwrap_err()), "field1");
            assert_eq!(*log.lock().unwrap(), ["field1", "field0"]);
        }
    }

    fn sorted(log: &Arc<Mutex<Vec<&'static str>>>) -> Vec<&'static str> {
        let mut closed = log.lock().unwrap().clone();
        closed.sort_unstable();
        closed
    }

    /// `PerFieldPostingsFormat.FieldsReader.close()` is
    /// `IOUtils.close(formats.values())` (`PerFieldPostingsFormat.java:350-353`):
    /// one failing producer does not keep the others open.
    #[test]
    fn postings_reader_close_closes_every_producer() {
        let (a, b, log) = failing_pair();
        let reader = FieldsReader {
            formats: java_map([
                ("a", Box::new(a) as Box<dyn FieldsProducer>),
                ("b", Box::new(b) as Box<dyn FieldsProducer>),
            ]),
            fields: BTreeMap::new(),
        };
        reader.close().unwrap_err();
        assert_eq!(sorted(&log), ["a", "b"]);
    }

    /// `PerFieldDocValuesFormat.FieldsReader.close()` is
    /// `IOUtils.close(formats.values())` (`PerFieldDocValuesFormat.java:354-357`).
    #[test]
    fn doc_values_reader_close_closes_every_producer() {
        let (a, b, log) = failing_pair();
        let reader = DvFieldsReader {
            formats: java_map([
                ("a", Box::new(a) as Box<dyn DocValuesProducer>),
                ("b", Box::new(b) as Box<dyn DocValuesProducer>),
            ]),
            fields: HashMap::new(),
        };
        reader.close().unwrap_err();
        assert_eq!(sorted(&log), ["a", "b"]);
    }

    /// `PerFieldDocValuesFormat.FieldsWriter.close()` is
    /// `IOUtils.close(formats.values())` (`PerFieldDocValuesFormat.java:239-243`).
    #[test]
    fn doc_values_writer_close_closes_every_consumer() {
        let (a, b, log) = failing_pair();
        let dir: Arc<dyn crate::store::Directory> =
            Arc::new(crate::store::ByteBuffersDirectory::default());
        let info = crate::codecs::tests::test_segment_info("_0", 1);
        let fis = crate::index::FieldInfos::default();
        let entry = |c: Closing| ConsumerAndSuffix {
            suffix: 0,
            consumer: Box::new(c),
            _phantom: std::marker::PhantomData,
        };
        let mut writer = DvFieldsWriter {
            segment_write_state: crate::codecs::tests::write_state(&dir, &info, &fis),
            resolver: Arc::new(|_: &str| -> Arc<dyn DocValuesFormat> { unreachable!() }),
            formats: HashMap::from([(1usize, entry(a)), (2usize, entry(b))]),
            suffixes: HashMap::new(),
        };
        writer.close().unwrap_err();
        assert_eq!(sorted(&log), ["a", "b"]);
    }

    /// A vector field without the per-field format attribute is skipped, not
    /// rejected: "null formatName means the field is in fieldInfos, but has
    /// no vectors!" (`PerFieldKnnVectorsFormat.java:212-233`). A segment
    /// whose vectors a non-per-field format wrote (a `FilterCodec` keeping
    /// the per-field codec's name) opens through the per-field reader.
    #[test]
    fn knn_reader_skips_a_vector_field_without_format_attribute() {
        use crate::store::DEFAULT_IO_CONTEXT;

        let dir = crate::store::ByteBuffersDirectory::default();
        let info = crate::codecs::tests::test_segment_info("_0", 1);
        let mut fi = crate::index::FieldInfo::new("v", 0);
        fi.vector_dimension = 3;
        fi.vector_encoding = crate::index::VectorEncoding::FLOAT32;
        assert!(fi.has_vector_values());
        let fis = crate::index::FieldInfos::new(vec![fi]).unwrap();
        let state = SegmentReadState::new(&dir, &info, &fis, &*DEFAULT_IO_CONTEXT);
        let reader = KnnFieldsReader::new(&state).unwrap();
        assert!(reader.formats.is_empty());
        assert!(!reader.fields.contains_key(0));
    }

    /// `PerFieldKnnVectorsFormat.FieldsWriter.close()` is
    /// `IOUtils.close(formats.values())` (`PerFieldKnnVectorsFormat.java:137-140`).
    #[test]
    fn knn_writer_close_closes_every_writer() {
        let (a, b, log) = failing_pair();
        let dir: Arc<dyn crate::store::Directory> =
            Arc::new(crate::store::ByteBuffersDirectory::default());
        let info = crate::codecs::tests::test_segment_info("_0", 1);
        let entry = |c: Closing| WriterAndSuffix {
            suffix: 0,
            writer: Box::new(c),
            _format: Arc::new(crate::codecs::knn_vectors::EmptyKnnVectorsFormat::new(
                "Empty",
            )),
        };
        let mut writer = KnnFieldsWriter {
            segment_write_state: crate::codecs::tests::owned_write_state(&dir, &info),
            resolver: Arc::new(|_: &str| -> Arc<dyn KnnVectorsFormat> { unreachable!() }),
            formats: HashMap::from([(1, entry(a)), (2, entry(b))]),
            suffixes: HashMap::new(),
        };
        writer.close().unwrap_err();
        assert_eq!(sorted(&log), ["a", "b"]);
    }

    /// `PerFieldKnnVectorsFormat.FieldsReader.close()` closes
    /// `fields.values()`, one reader per field
    /// (`PerFieldKnnVectorsFormat.java:335-341`): a reader two fields share is
    /// closed twice, and a failing reader does not keep the others open.
    #[test]
    fn knn_reader_close_closes_one_reader_per_field() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let shared = Closing {
            name: "shared",
            fail: false,
            log: Arc::clone(&log),
        };
        let reader = KnnFieldsReader {
            formats: java_map([("s", Box::new(shared) as Box<dyn KnnVectorsReader>)]),
            fields: int_map([(0, "s"), (1, "s")]),
            field_names: HashMap::new(),
            names: HashMap::new(),
        };
        reader.close().unwrap();
        assert_eq!(sorted(&log), ["shared", "shared"]);

        let (a, b, log) = failing_pair();
        let reader = KnnFieldsReader {
            formats: java_map([
                ("a", Box::new(a) as Box<dyn KnnVectorsReader>),
                ("b", Box::new(b) as Box<dyn KnnVectorsReader>),
            ]),
            fields: int_map([(0, "a"), (1, "b")]),
            field_names: HashMap::new(),
            names: HashMap::new(),
        };
        reader.close().unwrap_err();
        assert_eq!(sorted(&log), ["a", "b"]);
    }
}
