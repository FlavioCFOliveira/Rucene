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

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::{DocValuesType, IndexOptions};

use super::doc_values::{
    doc_values_for_name, DocValuesConsumer, DocValuesFormat, DocValuesProducer,
    EmptyDocValuesProducer,
};
use super::knn_vectors::{
    knn_vectors_for_name, ByteVectorValues, EmptyKnnVectorsReader, FloatVectorValues, KnnCollector,
    KnnFieldVectorsWriter, KnnVectorsFormat, KnnVectorsReader, KnnVectorsWriter,
};
use super::postings::{
    postings_for_name, Fields, FieldsConsumer, FieldsProducer, MergeState, NormsProducer,
    PostingsFormat, Terms,
};
use super::state::{SegmentReadState, SegmentWriteState};
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

    fn fields_producer<'a>(
        &self,
        state: &SegmentReadState<'a>,
    ) -> Result<Box<dyn FieldsProducer + 'a>> {
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

struct FieldsGroup<'a> {
    suffix: i32,
    fields: Vec<String>,
    consumer: Box<dyn FieldsConsumer + 'a>,
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
                let consumer = format.fields_consumer(&state)?;
                groups.insert(
                    format_name.clone(),
                    FieldsGroup {
                        suffix,
                        fields: Vec::new(),
                        consumer,
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

impl<'a> FieldsConsumer for FieldsWriter<'a> {
    fn write(&mut self, fields: &dyn Fields, norms: &dyn NormsProducer) -> Result<()> {
        let mut groups = self.build_groups(fields.iterator())?;
        for group in groups.values_mut() {
            let masked = MaskedFields::new(fields, &group.fields);
            group.consumer.write(&masked, norms)?;
            self.to_close.push(std::mem::replace(
                &mut group.consumer,
                Box::new(NoOpFieldsConsumer) as Box<dyn FieldsConsumer + 'a>,
            ));
        }
        Ok(())
    }

    fn merge(&mut self, merge_state: &MergeState, norms: &dyn NormsProducer) -> Result<()> {
        let field_names = merge_state
            .merge_field_infos
            .iter()
            .filter(|fi| fi.index_options != IndexOptions::NONE)
            .map(|fi| fi.name.clone());
        let mut groups = self.build_groups(field_names)?;
        for group in groups.values_mut() {
            let restricted = PerFieldMergeState::restrict_fields(merge_state, &group.fields)?;
            group.consumer.merge(&restricted, norms)?;
            self.to_close.push(std::mem::replace(
                &mut group.consumer,
                Box::new(NoOpFieldsConsumer) as Box<dyn FieldsConsumer + 'a>,
            ));
        }
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        for consumer in self.to_close.iter_mut() {
            consumer.close()?;
        }
        Ok(())
    }
}

struct MaskedFields<'a> {
    inner: &'a dyn Fields,
    fields: &'a [String],
}

impl<'a> MaskedFields<'a> {
    fn new(inner: &'a dyn Fields, fields: &'a [String]) -> Self {
        Self { inner, fields }
    }
}

impl Fields for MaskedFields<'_> {
    fn size(&self) -> i32 {
        self.fields.len() as i32
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        self.inner.terms(field)
    }

    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        let fields: Vec<String> = self.fields.to_vec();
        Box::new(fields.into_iter())
    }
}

#[derive(Debug, Default, Clone)]
struct NoOpFieldsConsumer;

impl FieldsConsumer for NoOpFieldsConsumer {
    fn write(&mut self, _fields: &dyn Fields, _norms: &dyn NormsProducer) -> Result<()> {
        Ok(())
    }

    fn merge(&mut self, _merge_state: &MergeState, _norms: &dyn NormsProducer) -> Result<()> {
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Default, Clone)]
struct NoOpFieldsProducer;

impl Fields for NoOpFieldsProducer {
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

impl FieldsProducer for NoOpFieldsProducer {
    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        Ok(Box::new(NoOpFieldsProducer))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

struct FieldsReader<'a> {
    formats: HashMap<String, Box<dyn FieldsProducer + 'a>>,
    fields: BTreeMap<String, String>,
}

impl<'a> FieldsReader<'a> {
    fn new(read_state: &SegmentReadState<'a>) -> Result<Self> {
        let mut formats: HashMap<String, Box<dyn FieldsProducer + 'a>> = HashMap::new();
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

impl<'a> std::fmt::Debug for FieldsReader<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PerFieldPostingsReader")
            .field("formats", &self.formats.len())
            .finish_non_exhaustive()
    }
}

impl<'a> Fields for FieldsReader<'a> {
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

impl<'a> FieldsProducer for FieldsReader<'a> {
    fn check_integrity(&self) -> Result<()> {
        for producer in self.formats.values() {
            producer.check_integrity()?;
        }
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        // TODO: return a real merge-optimized clone that shares the underlying
        // producers once concrete producers can provide one. For now the stub
        // merge path only needs a valid object.
        Ok(Box::new(NoOpFieldsProducer))
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
    ) -> Result<Box<dyn DocValuesProducer + 'a>> {
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
        field.put_attribute(PER_FIELD_DOC_VALUES_FORMAT_KEY, &format_name);

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
        field.put_attribute(PER_FIELD_DOC_VALUES_SUFFIX_KEY, entry.suffix.to_string());
        Ok(&mut entry.consumer)
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

struct DvFieldsReader<'a> {
    formats: HashMap<String, Box<dyn DocValuesProducer + 'a>>,
    fields: HashMap<i32, String>,
}

impl<'a> DvFieldsReader<'a> {
    fn new(read_state: &SegmentReadState<'a>) -> Result<Self> {
        let mut formats: HashMap<String, Box<dyn DocValuesProducer + 'a>> = HashMap::new();
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

    fn producer_for(&self, field: &FieldInfo) -> Result<&(dyn DocValuesProducer + 'a)> {
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

impl<'a> std::fmt::Debug for DvFieldsReader<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PerFieldDocValuesReader")
            .field("formats", &self.formats.len())
            .finish_non_exhaustive()
    }
}

impl<'a> DocValuesProducer for DvFieldsReader<'a> {
    fn get_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Box<dyn super::doc_values::NumericDocValues>> {
        self.producer_for(field)?.get_numeric(field)
    }

    fn get_binary(&self, field: &FieldInfo) -> Result<Box<dyn super::doc_values::BinaryDocValues>> {
        self.producer_for(field)?.get_binary(field)
    }

    fn get_sorted(&self, field: &FieldInfo) -> Result<Box<dyn super::doc_values::SortedDocValues>> {
        self.producer_for(field)?.get_sorted(field)
    }

    fn get_sorted_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Box<dyn super::doc_values::SortedNumericDocValues>> {
        self.producer_for(field)?.get_sorted_numeric(field)
    }

    fn get_sorted_set(
        &self,
        field: &FieldInfo,
    ) -> Result<Box<dyn super::doc_values::SortedSetDocValues>> {
        self.producer_for(field)?.get_sorted_set(field)
    }

    fn get_skipper(
        &self,
        field: &FieldInfo,
    ) -> Result<Box<dyn super::doc_values::DocValuesSkipper>> {
        self.producer_for(field)?.get_skipper(field)
    }

    fn check_integrity(&self) -> Result<()> {
        for producer in self.formats.values() {
            producer.check_integrity()?;
        }
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        // TODO: return a real merge-optimized clone once concrete producers can
        // provide one. For now the per-field merge path only needs a valid object.
        Ok(Box::new(EmptyDocValuesProducer))
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
}

impl KnnVectorsFormat for PerFieldKnnVectorsFormat {
    fn name(&self) -> &str {
        PER_FIELD_KNN_VECTORS_NAME
    }

    fn fields_writer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn KnnVectorsWriter + 'a>> {
        Ok(Box::new(KnnFieldsWriter::new(state, self.resolver.clone())))
    }

    fn fields_reader<'a>(
        &self,
        state: &SegmentReadState<'a>,
    ) -> Result<Box<dyn KnnVectorsReader + 'a>> {
        Ok(Box::new(KnnFieldsReader::new(state)?))
    }

    fn get_max_dimensions(&self, field_name: &str) -> i32 {
        (self.resolver)(field_name).get_max_dimensions(field_name)
    }
}

struct WriterAndSuffix<'a> {
    suffix: i32,
    writer: Box<dyn KnnVectorsWriter + 'a>,
    _phantom: std::marker::PhantomData<&'a ()>,
}

struct KnnFieldsWriter<'a> {
    segment_write_state: SegmentWriteState<'a>,
    resolver: Arc<dyn Fn(&str) -> Arc<dyn KnnVectorsFormat> + Send + Sync>,
    formats: HashMap<String, WriterAndSuffix<'a>>,
    suffixes: HashMap<String, i32>,
}

impl<'a> std::fmt::Debug for KnnFieldsWriter<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KnnFieldsWriter")
            .field("segment_suffix", &self.segment_write_state.segment_suffix)
            .field("formats", &self.formats.len())
            .finish_non_exhaustive()
    }
}

impl<'a> KnnFieldsWriter<'a> {
    fn new(
        segment_write_state: &SegmentWriteState<'a>,
        resolver: Arc<dyn Fn(&str) -> Arc<dyn KnnVectorsFormat> + Send + Sync>,
    ) -> Self {
        Self {
            segment_write_state: segment_write_state.clone(),
            resolver,
            formats: HashMap::new(),
            suffixes: HashMap::new(),
        }
    }

    fn get_instance(&mut self, field: &FieldInfo) -> Result<&mut Box<dyn KnnVectorsWriter + 'a>> {
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
            self.formats.insert(
                format_name.clone(),
                WriterAndSuffix {
                    suffix,
                    writer,
                    _phantom: std::marker::PhantomData,
                },
            );
        }

        let entry = self
            .formats
            .get_mut(&format_name)
            .expect("writer just inserted");
        field.put_attribute(PER_FIELD_KNN_VECTORS_SUFFIX_KEY, entry.suffix.to_string());
        Ok(&mut entry.writer)
    }
}

impl<'a> KnnVectorsWriter for KnnFieldsWriter<'a> {
    fn add_field(
        &mut self,
        field_info: &FieldInfo,
    ) -> Result<Box<dyn KnnFieldVectorsWriter<Vec<f32>>>> {
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

    fn merge_one_field(
        &mut self,
        field_info: &FieldInfo,
        merge_state: &MergeState,
    ) -> Result<Option<Box<dyn super::knn_vectors::IORunnable>>> {
        self.get_instance(field_info)?
            .merge_one_field(field_info, merge_state)
    }

    fn close(&mut self) -> Result<()> {
        for entry in self.formats.values_mut() {
            entry.writer.close()?;
        }
        Ok(())
    }
}

struct KnnFieldsReader<'a> {
    formats: HashMap<String, Box<dyn KnnVectorsReader + 'a>>,
    fields: HashMap<i32, String>,
}

impl<'a> KnnFieldsReader<'a> {
    fn new(read_state: &SegmentReadState<'a>) -> Result<Self> {
        let mut formats: HashMap<String, Box<dyn KnnVectorsReader + 'a>> = HashMap::new();
        let mut fields: HashMap<i32, String> = HashMap::new();

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

        Ok(Self { formats, fields })
    }

    fn reader_for(&self, field: &FieldInfo) -> Result<&(dyn KnnVectorsReader + 'a)> {
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

    fn reader_for_name(&self, field: &str) -> Result<(&(dyn KnnVectorsReader + 'a), FieldInfo)> {
        let info = FieldInfo::new(field, 0);
        let reader = self.reader_for(&info)?;
        Ok((reader, info))
    }
}

impl<'a> std::fmt::Debug for KnnFieldsReader<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PerFieldKnnVectorsReader")
            .field("formats", &self.formats.len())
            .finish_non_exhaustive()
    }
}

impl<'a> KnnVectorsReader for KnnFieldsReader<'a> {
    fn check_integrity(&self) -> Result<()> {
        for reader in self.formats.values() {
            reader.check_integrity()?;
        }
        Ok(())
    }

    fn get_float_vector_values(&self, field: &str) -> Result<Box<dyn FloatVectorValues>> {
        let (reader, _info) = self.reader_for_name(field)?;
        reader.get_float_vector_values(field)
    }

    fn get_byte_vector_values(&self, field: &str) -> Result<Box<dyn ByteVectorValues>> {
        let (reader, _info) = self.reader_for_name(field)?;
        reader.get_byte_vector_values(field)
    }

    fn search(
        &mut self,
        field: &str,
        target: &[f32],
        knn_collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn crate::search::AcceptDocs,
    ) -> Result<()> {
        let suffix = {
            let info = FieldInfo::new(field, 0);
            self.fields
                .get(&info.number)
                .ok_or_else(|| {
                    LuceneError::IllegalArgument(format!(
                        "field \"{field}\" has no KNN-vectors reader"
                    ))
                })?
                .clone()
        };
        let reader = self.formats.get_mut(&suffix).ok_or_else(|| {
            LuceneError::IllegalState(format!("missing KNN-vectors reader for suffix {suffix}"))
        })?;
        reader.search(field, target, knn_collector, accept_docs)
    }

    fn search_byte(
        &mut self,
        field: &str,
        target: &[u8],
        knn_collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn crate::search::AcceptDocs,
    ) -> Result<()> {
        let suffix = {
            let info = FieldInfo::new(field, 0);
            self.fields
                .get(&info.number)
                .ok_or_else(|| {
                    LuceneError::IllegalArgument(format!(
                        "field \"{field}\" has no KNN-vectors reader"
                    ))
                })?
                .clone()
        };
        let reader = self.formats.get_mut(&suffix).ok_or_else(|| {
            LuceneError::IllegalState(format!("missing KNN-vectors reader for suffix {suffix}"))
        })?;
        reader.search_byte(field, target, knn_collector, accept_docs)
    }

    fn get_merge_instance(&self) -> Result<Box<dyn KnnVectorsReader>> {
        // TODO: return a real merge-optimized clone once concrete readers can
        // provide one. For now the per-field merge path only needs a valid object.
        Ok(Box::new(EmptyKnnVectorsReader))
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
/// Equivalent to `org.apache.lucene.codecs.perfield.PerFieldMergeState`.
pub struct PerFieldMergeState;

impl PerFieldMergeState {
    /// Returns a new merge state that exposes only the named fields.
    ///
    /// The new state keeps all original field numbers so that any code that
    /// looks up fields by number remains consistent; only iteration and
    /// field-name lookup are filtered. The per-field fields producers are
    /// cloned through their merge-optimized instance and wrapped so that only
    /// the requested fields are visible.
    pub fn restrict_fields(merge_state: &MergeState, fields: &[String]) -> Result<MergeState> {
        let names: HashSet<String> = fields.iter().cloned().collect();

        let field_infos: Vec<super::stub::FieldInfos> = merge_state
            .field_infos
            .iter()
            .map(|fi| fi.filter(names.iter().cloned()))
            .collect();
        let merge_field_infos = merge_state.merge_field_infos.filter(names.iter().cloned());

        let fields_producers: Vec<Option<Box<dyn FieldsProducer>>> = merge_state
            .fields_producers
            .iter()
            .map(|producer| {
                producer
                    .as_ref()
                    .map(|p| {
                        let merge_instance = p.get_merge_instance()?;
                        Ok(Box::new(FilterFieldsProducer::new(merge_instance, &names))
                            as Box<dyn FieldsProducer>)
                    })
                    .transpose()
            })
            .collect::<Result<Vec<_>>>()?;

        let mut restricted = MergeState::new(fields_producers, merge_state.max_docs.clone())
            .with_field_infos(field_infos, merge_field_infos);
        // The remaining reader arrays are not needed by the per-field postings
        // merge path; concrete doc-values/points/vectors consumers that do need
        // them are responsible for sharing the original merge state directly.
        restricted.stored_fields_readers = Vec::new();
        restricted.term_vectors_readers = Vec::new();
        restricted.norms_producers = Vec::new();
        restricted.doc_values_producers = Vec::new();
        restricted.points_readers = Vec::new();
        restricted.knn_vectors_readers = Vec::new();

        Ok(restricted)
    }
}

struct FilterFieldsProducer {
    inner: Box<dyn FieldsProducer>,
    filtered: HashSet<String>,
}

impl FilterFieldsProducer {
    fn new(inner: Box<dyn FieldsProducer>, filtered: &HashSet<String>) -> Self {
        Self {
            inner,
            filtered: filtered.clone(),
        }
    }
}

impl Fields for FilterFieldsProducer {
    fn size(&self) -> i32 {
        self.filtered.len() as i32
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        if !self.filtered.contains(field) {
            return Err(LuceneError::IllegalArgument(format!(
                "field \"{field}\" is not accessible in the current merge context"
            )));
        }
        self.inner.terms(field)
    }

    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        let mut names: Vec<String> = self.filtered.iter().cloned().collect();
        names.sort();
        Box::new(names.into_iter())
    }
}

impl FieldsProducer for FilterFieldsProducer {
    fn check_integrity(&self) -> Result<()> {
        self.inner.check_integrity()
    }

    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        Ok(Box::new(FilterFieldsProducer::new(
            self.inner.get_merge_instance()?,
            &self.filtered,
        )))
    }

    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::postings::NumericDocValues;
    use crate::codecs::stub::FieldInfos;
    use crate::index::IndexOptions;

    fn write_state() -> SegmentWriteState<'static> {
        use crate::codecs::stub::{BufferedUpdates, FieldInfos, SegmentInfo};
        use crate::store::DEFAULT_IO_CONTEXT;
        use crate::util::default_info_stream;

        let dir = crate::store::RamDirectory::default();
        let dir_ref: &'static dyn crate::store::Directory = Box::leak(Box::new(dir));
        let info_stream: &'static dyn crate::util::InfoStream = default_info_stream();
        let segment_info: &'static SegmentInfo = Box::leak(Box::new(SegmentInfo));
        let field_infos: &'static FieldInfos = Box::leak(Box::new(FieldInfos::default()));
        let seg_updates: &'static BufferedUpdates = Box::leak(Box::new(BufferedUpdates));
        let context: &'static dyn crate::store::IOContext = &*DEFAULT_IO_CONTEXT;

        SegmentWriteState::new(
            info_stream,
            dir_ref,
            segment_info,
            field_infos,
            seg_updates,
            context,
        )
    }

    fn read_state() -> SegmentReadState<'static> {
        use crate::codecs::stub::{FieldInfos, SegmentInfo};
        use crate::store::DEFAULT_IO_CONTEXT;

        let dir = crate::store::RamDirectory::default();
        let dir_ref: &'static dyn crate::store::Directory = Box::leak(Box::new(dir));
        let segment_info: &'static SegmentInfo = Box::leak(Box::new(SegmentInfo));
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
        ) -> Result<Box<dyn FieldsProducer + 'a>> {
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

        #[derive(Debug, Default, Clone)]
        struct NoOpNorms;
        impl NormsProducer for NoOpNorms {
            fn get_norms(&self, _field_info: &FieldInfo) -> Result<Box<dyn NumericDocValues>> {
                unreachable!("no norms requested in this test")
            }
        }

        consumer.write(&TwoFields, &NoOpNorms).unwrap();
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

    #[test]
    fn per_field_merge_state_restricts_fields() {
        let body = FieldInfo::new("body", 0);
        let title = FieldInfo::new("title", 1);
        let merge_field_infos = FieldInfos::new(vec![body.clone(), title.clone()]).unwrap();
        let segment_field_infos = FieldInfos::new(vec![body, title]).unwrap();

        let producer_a = Box::new(NamedFieldsProducer("A")) as Box<dyn FieldsProducer>;
        let merge_state = MergeState::new(vec![Some(producer_a)], vec![10])
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
