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

use super::doc_values::{
    doc_values_for_name, DocValuesConsumer, DocValuesFormat, DocValuesProducer,
};
use super::knn_vectors::{
    knn_vectors_for_name, ByteVectorValues, FieldVectorWriter, FloatVectorValues, KnnCollector,
    KnnVectorsFormat, KnnVectorsReader, KnnVectorsWriter,
};
use super::norms::NormsProducer;
use super::postings::{
    postings_for_name, Fields, FieldsConsumer, FieldsProducer, MergeState, PostingsFormat, Terms,
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
        Err(LuceneError::IllegalState(format!(
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
    ) -> Result<HashMap<String, FieldsGroup<'a>>> {
        let mut suffixes: HashMap<String, i32> = HashMap::new();
        let mut groups: HashMap<String, FieldsGroup<'a>> = HashMap::new();

        for field_ref in field_names {
            let field = field_ref.as_ref();
            let field_info = self
                .write_state
                .field_infos
                .field_info(field)
                .ok_or_else(|| {
                    LuceneError::IllegalArgument(format!("field {} not found", field))
                })?;
            let format = (self.resolver)(field);
            let format_name = format.name().to_string();

            if !groups.contains_key(&format_name) {
                let suffix = suffixes.get(&format_name).copied().unwrap_or(0);
                suffixes.insert(format_name.clone(), suffix + 1);
                let segment_suffix = get_full_segment_suffix_postings(
                    field,
                    &self.write_state.segment_suffix,
                    &get_suffix(&format_name, suffix),
                )?;
                let state = self.write_state.with_new_suffix(segment_suffix);
                groups.insert(
                    format_name.clone(),
                    FieldsGroup {
                        suffix,
                        fields: Vec::new(),
                        format: Arc::clone(&format),
                        state,
                    },
                );
            }

            let group = groups.get_mut(&format_name).expect("group just inserted");
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
        groups: &HashMap<String, FieldsGroup<'a>>,
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
            for consumer in self.to_close.iter_mut() {
                let _ = consumer.close();
            }
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
                .map(|producer| {
                    let names: Vec<String> = producer.iterator().collect();
                    Box::new(names.into_iter()) as Box<dyn Iterator<Item = String>>
                })
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
        let mut first_error = None;
        for consumer in self.to_close.iter_mut() {
            if let Err(error) = consumer.close() {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
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
    formats: HashMap<String, Box<dyn FieldsProducer>>,
    fields: BTreeMap<String, String>,
}

impl FieldsReader {
    fn new(read_state: &SegmentReadState) -> Result<Self> {
        let mut formats: HashMap<String, Box<dyn FieldsProducer>> = HashMap::new();
        let mut fields: BTreeMap<String, String> = BTreeMap::new();

        let result = (|| -> Result<()> {
            for fi in read_state.field_infos.iter() {
                if fi.index_options != IndexOptions::NONE {
                    let format_name =
                        fi.get_attribute(PER_FIELD_POSTINGS_FORMAT_KEY)
                            .ok_or_else(|| {
                                LuceneError::IllegalState(format!(
                            "missing attribute: {PER_FIELD_POSTINGS_FORMAT_KEY} for field: {}",
                            fi.name
                        ))
                            })?;
                    let suffix =
                        fi.get_attribute(PER_FIELD_POSTINGS_SUFFIX_KEY)
                            .ok_or_else(|| {
                                LuceneError::IllegalState(format!(
                            "missing attribute: {PER_FIELD_POSTINGS_SUFFIX_KEY} for field: {}",
                            fi.name
                        ))
                            })?;
                    let format = postings_for_name(&format_name).ok_or_else(|| {
                        LuceneError::IllegalState(format!("unknown postings format: {format_name}"))
                    })?;
                    let suffix_num = suffix.parse::<i32>().map_err(|_| {
                        LuceneError::IllegalState(format!(
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
            for producer in formats.values_mut() {
                let _ = producer.close();
            }
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

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        let suffix = self
            .fields
            .get(field)
            .ok_or_else(|| LuceneError::IllegalArgument(format!("field not found: {field}")))?;
        self.formats
            .get(suffix)
            .ok_or_else(|| {
                LuceneError::IllegalState(format!("missing producer for suffix {suffix}"))
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
        let mut formats: HashMap<String, Box<dyn FieldsProducer>> =
            HashMap::with_capacity(self.formats.len());
        for (suffix, producer) in &self.formats {
            formats.insert(suffix.clone(), producer.get_merge_instance()?);
        }
        Ok(Box::new(FieldsReader {
            formats,
            fields: self.fields.clone(),
        }))
    }

    fn close(&mut self) -> Result<()> {
        for producer in self.formats.values_mut() {
            producer.close()?;
        }
        Ok(())
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
    formats: HashMap<String, ConsumerAndSuffix<'a>>,
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
            format_name
                .and_then(|name| doc_values_for_name(&name))
                .or_else(|| Some((self.resolver)(&field.name)))
        } else {
            Some((self.resolver)(&field.name))
        }
        .ok_or_else(|| {
            LuceneError::IllegalState(format!(
                "invalid null DocValuesFormat for field=\"{}\"",
                field.name
            ))
        })?;

        let format_name = format.name().to_string();
        self.record_attribute(field, PER_FIELD_DOC_VALUES_FORMAT_KEY, &format_name);

        if !self.formats.contains_key(&format_name) {
            let suffix = if field.doc_values_gen != -1 {
                let suffix_att = if ignore_current_format {
                    None
                } else {
                    field.get_attribute(PER_FIELD_DOC_VALUES_SUFFIX_KEY)
                };
                suffix_att.and_then(|s| s.parse::<i32>().ok())
            } else {
                None
            }
            .unwrap_or_else(|| {
                let next = self.suffixes.get(&format_name).copied().unwrap_or(0);
                self.suffixes.insert(format_name.clone(), next + 1);
                next
            });

            let segment_suffix = get_full_segment_suffix(
                &self.segment_write_state.segment_suffix,
                &get_suffix(&format_name, suffix),
            );
            let state = self.segment_write_state.with_new_suffix(segment_suffix);
            let consumer = format.fields_consumer(&state)?;
            self.formats.insert(
                format_name.clone(),
                ConsumerAndSuffix {
                    suffix,
                    consumer,
                    _phantom: std::marker::PhantomData,
                },
            );
        }

        let entry = self
            .formats
            .get_mut(&format_name)
            .expect("consumer just inserted");
        let suffix = entry.suffix.to_string();
        // `entry` borrows `self.formats` mutably, so the shared field info is
        // reached after the borrow ends, on the next statement.
        field.put_attribute(PER_FIELD_DOC_VALUES_SUFFIX_KEY, &suffix);
        self.record_shared_attribute(&field.name, PER_FIELD_DOC_VALUES_SUFFIX_KEY, &suffix);
        Ok(&mut self
            .formats
            .get_mut(&format_name)
            .expect("consumer just inserted")
            .consumer)
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
        let mut consumers_to_fields: HashMap<String, Vec<String>> = HashMap::new();

        for fi in merge_state.merge_field_infos.iter() {
            if fi.doc_values_type == DocValuesType::NONE {
                continue;
            }
            let consumer_key = {
                // Ensure the concrete consumer for this field exists and record
                // the chosen format name on the field info.
                let _ = self.get_instance_with_ignore(fi, true)?;
                let format_name = fi
                    .get_attribute(PER_FIELD_DOC_VALUES_FORMAT_KEY)
                    .ok_or_else(|| {
                        LuceneError::IllegalState(format!(
                            "missing attribute: {PER_FIELD_DOC_VALUES_FORMAT_KEY} for field: {}",
                            fi.name
                        ))
                    })?;
                if !self.formats.contains_key(&format_name) {
                    return Err(LuceneError::IllegalState(format!(
                        "consumer missing for format {format_name}"
                    )));
                }
                format_name
            };
            consumers_to_fields
                .entry(consumer_key)
                .or_default()
                .push(fi.name.clone());
        }

        for (format_name, fields) in consumers_to_fields {
            let restricted = PerFieldMergeState::restrict_fields(merge_state, &fields)?;
            self.formats
                .get_mut(&format_name)
                .expect("consumer present")
                .consumer
                .merge(&restricted)?;
        }

        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        for entry in self.formats.values_mut() {
            entry.consumer.close()?;
        }
        Ok(())
    }
}

struct DvFieldsReader {
    formats: HashMap<String, Box<dyn DocValuesProducer>>,
    fields: HashMap<i32, String>,
}

impl DvFieldsReader {
    fn new(read_state: &SegmentReadState) -> Result<Self> {
        let mut formats: HashMap<String, Box<dyn DocValuesProducer>> = HashMap::new();
        let mut fields: HashMap<i32, String> = HashMap::new();

        let result = (|| -> Result<()> {
            for fi in read_state.field_infos.iter() {
                if fi.doc_values_type != DocValuesType::NONE {
                    let format_name = fi
                        .get_attribute(PER_FIELD_DOC_VALUES_FORMAT_KEY)
                        .ok_or_else(|| {
                            LuceneError::IllegalState(format!(
                            "missing attribute: {PER_FIELD_DOC_VALUES_FORMAT_KEY} for field: {}",
                            fi.name
                        ))
                        })?;
                    let suffix = fi
                        .get_attribute(PER_FIELD_DOC_VALUES_SUFFIX_KEY)
                        .ok_or_else(|| {
                            LuceneError::IllegalState(format!(
                            "missing attribute: {PER_FIELD_DOC_VALUES_SUFFIX_KEY} for field: {}",
                            fi.name
                        ))
                        })?;
                    let format = doc_values_for_name(&format_name).ok_or_else(|| {
                        LuceneError::IllegalState(format!(
                            "unknown doc-values format: {format_name}"
                        ))
                    })?;
                    let suffix_num = suffix.parse::<i32>().map_err(|_| {
                        LuceneError::IllegalState(format!(
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
            for producer in formats.values_mut() {
                let _ = producer.close();
            }
            return Err(e);
        }

        Ok(Self { formats, fields })
    }

    fn producer_for(&self, field: &FieldInfo) -> Result<&(dyn DocValuesProducer + '_)> {
        let suffix = self.fields.get(&field.number).ok_or_else(|| {
            LuceneError::IllegalArgument(format!(
                "field \"{}\" has no doc-values producer",
                field.name
            ))
        })?;
        self.formats.get(suffix).map(|b| b.as_ref()).ok_or_else(|| {
            LuceneError::IllegalState(format!("missing doc-values producer for suffix {suffix}"))
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
        let mut formats: HashMap<String, Box<dyn DocValuesProducer>> =
            HashMap::with_capacity(self.formats.len());
        for (suffix, producer) in &self.formats {
            formats.insert(suffix.clone(), producer.get_merge_instance()?);
        }
        Ok(Box::new(DvFieldsReader {
            formats,
            fields: self.fields.clone(),
        }))
    }

    fn close(&mut self) -> Result<()> {
        for producer in self.formats.values_mut() {
            producer.close()?;
        }
        Ok(())
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
}

struct WriterAndSuffix {
    suffix: i32,
    writer: Box<dyn KnnVectorsWriter>,
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
    formats: HashMap<String, WriterAndSuffix>,
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

        if !self.formats.contains_key(&format_name) {
            let suffix = self.suffixes.get(&format_name).copied().unwrap_or(0);
            self.suffixes.insert(format_name.clone(), suffix + 1);
            let segment_suffix = get_full_segment_suffix(
                &self.segment_write_state.segment_suffix,
                &get_suffix(&format_name, suffix),
            );
            let state = self.segment_write_state.with_new_suffix(segment_suffix);
            let writer = format.fields_writer(&state)?;
            self.formats
                .insert(format_name.clone(), WriterAndSuffix { suffix, writer });
        }

        let entry = self
            .formats
            .get_mut(&format_name)
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

    fn close(&mut self) -> Result<()> {
        for entry in self.formats.values_mut() {
            entry.writer.close()?;
        }
        Ok(())
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

struct KnnFieldsReader {
    formats: HashMap<String, Box<dyn KnnVectorsReader>>,
    fields: HashMap<i32, String>,
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
    fn new(read_state: &SegmentReadState) -> Result<Self> {
        let mut formats: HashMap<String, Box<dyn KnnVectorsReader>> = HashMap::new();
        let mut fields: HashMap<i32, String> = HashMap::new();
        let mut field_names: HashMap<String, String> = HashMap::new();
        let mut names: HashMap<i32, String> = HashMap::new();

        let result = (|| -> Result<()> {
            for fi in read_state.field_infos.iter() {
                if fi.has_vector_values() {
                    let format_name = fi
                        .get_attribute(PER_FIELD_KNN_VECTORS_FORMAT_KEY)
                        .ok_or_else(|| {
                            LuceneError::IllegalState(format!(
                            "missing attribute: {PER_FIELD_KNN_VECTORS_FORMAT_KEY} for field: {}",
                            fi.name
                        ))
                        })?;
                    let suffix = fi
                        .get_attribute(PER_FIELD_KNN_VECTORS_SUFFIX_KEY)
                        .ok_or_else(|| {
                            LuceneError::IllegalState(format!(
                            "missing attribute: {PER_FIELD_KNN_VECTORS_SUFFIX_KEY} for field: {}",
                            fi.name
                        ))
                        })?;
                    let format = knn_vectors_for_name(&format_name).ok_or_else(|| {
                        LuceneError::IllegalState(format!(
                            "unknown KNN-vectors format: {format_name}"
                        ))
                    })?;
                    let suffix_num = suffix.parse::<i32>().map_err(|_| {
                        LuceneError::IllegalState(format!(
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
                    fields.insert(fi.number, segment_suffix);
                }
            }
            Ok(())
        })();

        if let Err(e) = result {
            for reader in formats.values_mut() {
                let _ = reader.close();
            }
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
        let suffix = self.fields.get(&field.number).ok_or_else(|| {
            LuceneError::IllegalArgument(format!(
                "field \"{}\" has no KNN-vectors reader",
                field.name
            ))
        })?;
        self.formats.get(suffix).map(|b| b.as_ref()).ok_or_else(|| {
            LuceneError::IllegalState(format!("missing KNN-vectors reader for suffix {suffix}"))
        })
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
        let mut formats: HashMap<String, Box<dyn KnnVectorsReader>> =
            HashMap::with_capacity(self.fields.len());
        let mut fields: HashMap<i32, String> = HashMap::with_capacity(self.fields.len());
        let mut field_names: HashMap<String, String> = HashMap::with_capacity(self.fields.len());
        for (&number, name) in &self.names {
            let Some(suffix) = self.fields.get(&number) else {
                continue;
            };
            let reader = self.formats.get(suffix).ok_or_else(|| {
                LuceneError::IllegalState(format!("missing KNN-vectors reader for suffix {suffix}"))
            })?;
            formats.insert(name.clone(), reader.get_merge_instance()?);
            fields.insert(number, name.clone());
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
        for reader in self.formats.values_mut() {
            reader.finish_merge()?;
        }
        Ok(())
    }

    fn get_off_heap_byte_size(&self, field_info: &FieldInfo) -> HashMap<String, i64> {
        self.reader_for(field_info)
            .map(|r| r.get_off_heap_byte_size(field_info))
            .unwrap_or_default()
    }

    fn close(&mut self) -> Result<()> {
        for reader in self.formats.values_mut() {
            reader.close()?;
        }
        Ok(())
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
            return Err(LuceneError::IllegalArgument(format!(
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

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::stub::FieldInfos;
    use crate::index::{IndexOptions, SegmentInfo};

    fn write_state() -> SegmentWriteState<'static> {
        use crate::codecs::stub::BufferedUpdates;
        use crate::store::DEFAULT_IO_CONTEXT;
        use crate::util::default_info_stream;

        let dir: Arc<dyn crate::store::Directory> = Arc::new(crate::store::RamDirectory::default());
        let info_stream: &'static dyn crate::util::InfoStream = default_info_stream();
        let segment_info: &'static SegmentInfo = Box::leak(Box::new(
            crate::codecs::tests::test_segment_info("test", 10),
        ));
        let field_infos: &'static FieldInfos = Box::leak(Box::new(FieldInfos::default()));
        let seg_updates: &'static BufferedUpdates = Box::leak(Box::new(BufferedUpdates));
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

        let dir = crate::store::RamDirectory::default();
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

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn per_field_postings_name_and_factories() {
        let format = PerFieldPostingsFormat::new(|_| Arc::new(NamedPostingsFormat("Foo")));
        assert_eq!(format.name(), PER_FIELD_POSTINGS_NAME);

        let mut consumer = format.fields_consumer(&write_state()).unwrap();
        consumer.close().unwrap();

        let mut producer = format.fields_producer(&read_state()).unwrap();
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
                return Err(LuceneError::IllegalState(format!("{} write", self.0.name)));
            }
            Ok(())
        }

        fn close(&mut self) -> Result<()> {
            self.0
                .closed
                .store(true, std::sync::atomic::Ordering::SeqCst);
            if self.0.fail_close {
                return Err(LuceneError::IllegalState(format!("{} close", self.0.name)));
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
            Err(LuceneError::IllegalArgument(_))
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
