/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `org.apache.lucene.codecs.bloom.BloomFilteringPostingsFormat`.

use std::cell::OnceCell;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use rucene::codecs::codec_util::{
    check_footer, check_index_header, write_footer, write_index_header,
};
use rucene::codecs::{
    FieldsConsumer, FieldsProducer, NormsProducer, PostingsFormat, SegmentReadState,
    SegmentWriteState,
};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::{
    FieldInfo, Fields, ImpactsEnum, PostingsEnum, SeekStatus, Terms, TermsEnum, POSTINGS_ENUM_NONE,
};
use rucene::search::NO_MORE_DOCS;
use rucene::store::{ChecksumIndexInput, IndexOutput};
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::CompiledAutomaton;
use rucene::util::io_function::IOBooleanSupplier;
use rucene::util::{BytesRef, IOUtils};

use super::bloom_filter_factory::BloomFilterFactory;
use super::default_bloom_filter_factory::DefaultBloomFilterFactory;
use super::fuzzy_set::{ContainsResult, FuzzySet};
use crate::memory::direct_postings_format::postings_format_for_name;

/// The codec name of the `.blm` file, and the format's name.
pub const BLOOM_CODEC_NAME: &str = "BloomFilter";
/// The first version of the `.blm` file.
pub const VERSION_START: i32 = 3;
/// The current version of the `.blm` file.
pub const VERSION_CURRENT: i32 = VERSION_START;

/// Extension of Bloom Filters file.
const BLOOM_EXTENSION: &str = "blm";

/// A [`PostingsFormat`] useful for low doc-frequency fields such as primary
/// keys. Bloom filters are maintained in a ".blm" file which offers
/// "fast-fail" for reads in segments known to have no record of the key. A
/// choice of delegate PostingsFormat is used to record all other Postings
/// data.
///
/// A choice of [`BloomFilterFactory`] can be passed to tailor Bloom Filter
/// settings on a per-field basis. The default configuration is
/// [`DefaultBloomFilterFactory`] which allocates a ~8mb bitset and hashes
/// values using `StringHelper.murmurhash3_x64_128(BytesRef)`. This should
/// be suitable for most purposes.
///
/// The format of the blm file is as follows:
///
/// * BloomFilter (.blm) --> Header, DelegatePostingsFormatName,
///   NumFilteredFields, Filter<sup>NumFilteredFields</sup>, Footer
/// * Filter --> FieldNumber, FuzzySet
/// * FuzzySet --> See [`FuzzySet::serialize`]
/// * Header --> `CodecUtil.writeIndexHeader` IndexHeader
/// * DelegatePostingsFormatName --> `DataOutput.writeString(String)`
///   String: the name of a ServiceProvider registered `PostingsFormat`
/// * NumFilteredFields --> `DataOutput.writeInt` Uint32
/// * FieldNumber --> `DataOutput.writeInt` Uint32: the number of the field
///   in this segment
/// * Footer --> `CodecUtil.writeFooter` CodecFooter
///
/// Port of `org.apache.lucene.codecs.bloom.BloomFilteringPostingsFormat`,
/// named `BloomFilter`. Its `toString()` is this type's [`fmt::Display`],
/// which a `dyn PostingsFormat` reaches through
/// [`PostingsFormat::to_java_string`].
#[derive(Debug, Clone)]
pub struct BloomFilteringPostingsFormat {
    bloom_filter_factory: Arc<dyn BloomFilterFactory>,
    delegate_postings_format: Option<Arc<dyn PostingsFormat>>,
}

impl BloomFilteringPostingsFormat {
    /// Creates Bloom filters for a selection of fields created in the
    /// index. This is recorded as a set of Bitsets held as a segment summary
    /// in an additional "blm" file. This PostingsFormat delegates to a
    /// choice of delegate PostingsFormat for encoding all other postings
    /// data.
    ///
    /// `delegate_postings_format` records all the non-bloom filter data
    /// i.e. postings info; `bloom_filter_factory` is responsible for sizing
    /// BloomFilters appropriately.
    ///
    /// Port of `BloomFilteringPostingsFormat(PostingsFormat,
    /// BloomFilterFactory)`.
    pub fn with_factory(
        delegate_postings_format: Arc<dyn PostingsFormat>,
        bloom_filter_factory: Arc<dyn BloomFilterFactory>,
    ) -> Self {
        Self {
            bloom_filter_factory,
            delegate_postings_format: Some(delegate_postings_format),
        }
    }

    /// As [`with_factory`](Self::with_factory), with the
    /// [`DefaultBloomFilterFactory`] for configuring per-field
    /// BloomFilters.
    ///
    /// Port of `BloomFilteringPostingsFormat(PostingsFormat)`.
    pub fn new(delegate_postings_format: Arc<dyn PostingsFormat>) -> Self {
        Self::with_factory(
            delegate_postings_format,
            Arc::new(DefaultBloomFilterFactory::new()),
        )
    }

    /// Used only by core Lucene at read-time via Service Provider
    /// instantiation - do not use at Write-time in application code.
    ///
    /// Port of `BloomFilteringPostingsFormat()`.
    pub fn for_reading() -> Self {
        Self {
            bloom_filter_factory: Arc::new(DefaultBloomFilterFactory::new()),
            delegate_postings_format: None,
        }
    }
}

impl fmt::Display for BloomFilteringPostingsFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.delegate_postings_format {
            Some(delegate) => write!(f, "BloomFilteringPostingsFormat({})", &**delegate),
            None => f.write_str("BloomFilteringPostingsFormat(null)"),
        }
    }
}

impl PostingsFormat for BloomFilteringPostingsFormat {
    /// `toString()`: this format's [`fmt::Display`].
    fn to_java_string(&self) -> String {
        self.to_string()
    }

    fn name(&self) -> &str {
        BLOOM_CODEC_NAME
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        let Some(delegate_postings_format) = &self.delegate_postings_format else {
            return Err(LuceneError::unsupported_operation(
                "Error - org.apache.lucene.codecs.bloom.BloomFilteringPostingsFormat has been \
                 constructed without a choice of PostingsFormat"
                    .to_string(),
            ));
        };
        let fields_consumer = delegate_postings_format.fields_consumer(state)?;
        Ok(Box::new(BloomFilteredFieldsConsumer {
            delegate_fields_consumer: fields_consumer,
            bloom_filters: Vec::new(),
            state: state.clone(),
            bloom_filter_factory: Arc::clone(&self.bloom_filter_factory),
            delegate_postings_format_name: delegate_postings_format.name().to_string(),
            closed: false,
        }))
    }

    fn fields_producer<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn FieldsProducer>> {
        Ok(Box::new(BloomFilteredFieldsProducer::new(state)?))
    }
}

/// Port of the static inner class `BloomFilteredFieldsProducer`.
struct BloomFilteredFieldsProducer {
    /// Shared with the merge instance, which in Java is `this`.
    delegate_fields_producer: Arc<dyn FieldsProducer>,
    blooms_by_field_name: Arc<HashMap<String, Arc<FuzzySet>>>,
}

impl BloomFilteredFieldsProducer {
    /// Port of `BloomFilteredFieldsProducer(SegmentReadState)`.
    fn new(state: &SegmentReadState<'_>) -> Result<Self> {
        let bloom_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            BLOOM_EXTENSION,
        );
        let mut bloom_in: Option<Box<dyn ChecksumIndexInput>> = None;
        let mut delegate_fields_producer: Option<Box<dyn FieldsProducer>> = None;
        let mut blooms_by_field_name = HashMap::new();
        let result = (|| -> Result<()> {
            let input = bloom_in.insert(state.directory.open_checksum_input(&bloom_file_name)?);
            check_index_header(
                input.as_mut(),
                BLOOM_CODEC_NAME,
                VERSION_START,
                VERSION_CURRENT,
                &state.segment_info.id(),
                &state.segment_suffix,
            )?;
            // // Load the hash function used in the BloomFilter
            // hashFunction = HashFunction.forName(bloomIn.readString());
            // Load the delegate postings format
            let delegate_postings_format = postings_format_for_name(&input.read_string()?)?;

            delegate_fields_producer = Some(delegate_postings_format.fields_producer(state)?);
            let num_blooms = input.read_int()?;
            for _ in 0..num_blooms {
                let field_num = input.read_int()?;
                let bloom = FuzzySet::deserialize(input.as_mut())?;
                if field_num < 0 {
                    return Err(LuceneError::illegal_argument(format!(
                        "Illegal field number: {field_num}"
                    )));
                }
                let field_info = state
                    .field_infos
                    .field_info_by_number(field_num)
                    .ok_or_else(|| {
                        LuceneError::unchecked(
                            "java.lang.NullPointerException",
                            "Cannot read field \"name\" because \"fieldInfo\" is null",
                        )
                    })?;
                blooms_by_field_name.insert(field_info.name.clone(), Arc::new(bloom));
            }
            check_footer(input.as_mut())?;
            let mut input = bloom_in.take().expect("INVARIANT: opened above");
            IOUtils::close([IOUtils::closeable(move || input.close())])?;
            Ok(())
        })();
        match result {
            Ok(()) => Ok(Self {
                delegate_fields_producer: Arc::from(
                    delegate_fields_producer.expect("INVARIANT: set before the success"),
                ),
                blooms_by_field_name: Arc::new(blooms_by_field_name),
            }),
            Err(e) => {
                IOUtils::close_while_handling_exception([
                    bloom_in
                        .as_mut()
                        .and_then(|i| IOUtils::closeable(move || i.close())),
                    delegate_fields_producer
                        .as_mut()
                        .and_then(|p| IOUtils::closeable(move || p.close())),
                ])?;
                Err(e)
            }
        }
    }
}

impl Fields for BloomFilteredFieldsProducer {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        self.delegate_fields_producer.iterator()
    }

    fn terms(&self, field: &str) -> Result<Option<Arc<dyn Terms>>> {
        match self.blooms_by_field_name.get(field) {
            None => self.delegate_fields_producer.terms(field),
            Some(filter) => {
                let Some(result) = self.delegate_fields_producer.terms(field)? else {
                    return Ok(None);
                };
                Ok(Some(Arc::new(BloomFilteredTerms {
                    delegate_terms: result,
                    filter: Arc::clone(filter),
                })))
            }
        }
    }

    fn size(&self) -> i32 {
        self.delegate_fields_producer.size()
    }
}

impl fmt::Display for BloomFilteredFieldsProducer {
    /// `BloomFilteredFieldsProducer.toString()` (`BloomFilteringPostingsFormat.java:399-407`):
    /// `getClass().getSimpleName() + "(fields=" + bloomsByFieldName.size()
    /// + ",delegate=" + delegateFieldsProducer + ")"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "BloomFilteredFieldsProducer(fields={},delegate={})",
            self.blooms_by_field_name.len(),
            self.delegate_fields_producer.to_java_string()
        )
    }
}

impl FieldsProducer for BloomFilteredFieldsProducer {
    /// `toString()`: this producer's [`Display`](fmt::Display).
    fn to_java_string(&self) -> String {
        format!("{self}")
    }

    fn check_integrity(&self) -> Result<()> {
        self.delegate_fields_producer.check_integrity()
    }

    /// `FieldsProducer.getMergeInstance()`, which this producer inherits:
    /// `this` — here a handle on the same delegate and the same filters.
    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        Ok(Box::new(Self {
            delegate_fields_producer: Arc::clone(&self.delegate_fields_producer),
            blooms_by_field_name: Arc::clone(&self.blooms_by_field_name),
        }))
    }

    fn close(&self) -> Result<()> {
        self.delegate_fields_producer.close()
    }
}

/// Port of `BloomFilteredFieldsProducer.BloomFilteredTerms`.
///
/// The delegate is shared with the enums the terms hand out, which Java's
/// `BloomFilteredTermsEnum` holds as `delegateTerms`.
struct BloomFilteredTerms {
    delegate_terms: Arc<dyn Terms>,
    filter: Arc<FuzzySet>,
}

impl Terms for BloomFilteredTerms {
    fn intersect(
        &self,
        compiled: &CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<Box<dyn TermsEnum>> {
        self.delegate_terms.intersect(compiled, start_term)
    }

    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(BloomFilteredTermsEnum::new(
            Arc::clone(&self.delegate_terms),
            Arc::clone(&self.filter),
        )))
    }

    fn size(&self) -> Result<i64> {
        self.delegate_terms.size()
    }

    fn sum_total_term_freq(&self) -> Result<i64> {
        self.delegate_terms.sum_total_term_freq()
    }

    fn sum_doc_freq(&self) -> Result<i64> {
        self.delegate_terms.sum_doc_freq()
    }

    fn doc_count(&self) -> Result<i32> {
        self.delegate_terms.doc_count()
    }

    fn has_freqs(&self) -> bool {
        self.delegate_terms.has_freqs()
    }

    fn has_offsets(&self) -> bool {
        self.delegate_terms.has_offsets()
    }

    fn has_positions(&self) -> bool {
        self.delegate_terms.has_positions()
    }

    fn has_payloads(&self) -> bool {
        self.delegate_terms.has_payloads()
    }

    fn min(&self) -> Result<Option<BytesRef>> {
        self.delegate_terms.min()
    }

    fn max(&self) -> Result<Option<BytesRef>> {
        self.delegate_terms.max()
    }
}

/// Port of `BloomFilteredFieldsProducer.BloomFilteredTermsEnum`.
///
/// Java pulls the delegate's iterator only when it is first needed, from
/// any method, `term()` and `ord()` included; those two take `&self` here,
/// so the lazily pulled enum sits in a [`OnceCell`], which can lend the
/// delegate's term from `&self`.
struct BloomFilteredTermsEnum {
    delegate_terms: Arc<dyn Terms>,
    delegate_terms_enum: OnceCell<Box<dyn TermsEnum>>,
    filter: Arc<FuzzySet>,
    attributes: Option<AttributeSource>,
}

impl BloomFilteredTermsEnum {
    /// Port of `BloomFilteredTermsEnum(Terms, FuzzySet)`.
    fn new(delegate_terms: Arc<dyn Terms>, filter: Arc<FuzzySet>) -> Self {
        Self {
            delegate_terms,
            delegate_terms_enum: OnceCell::new(),
            filter,
            attributes: None,
        }
    }

    /// Port of the package-private `reset(Terms)`.
    #[allow(dead_code)]
    fn reset(&mut self, delegate_terms: Arc<dyn Terms>) {
        self.delegate_terms = delegate_terms;
        self.delegate_terms_enum = OnceCell::new();
    }

    /// Port of `delegate()`.
    fn delegate(&mut self) -> Result<&mut dyn TermsEnum> {
        if self.delegate_terms_enum.get().is_none() {
            /* pull the iterator only if we really need it -
             * this can be a relativly heavy operation depending on the
             * delegate postings format and they underlying directory
             * (clone IndexInput) */
            let iterator = self.delegate_terms.iterator()?;
            let _ = self.delegate_terms_enum.set(iterator);
        }
        Ok(self
            .delegate_terms_enum
            .get_mut()
            .expect("INVARIANT: the delegate was pulled above")
            .as_mut())
    }

    /// [`delegate`](Self::delegate) for the `&self` methods.
    fn delegate_ref(&self) -> Result<&dyn TermsEnum> {
        if self.delegate_terms_enum.get().is_none() {
            let iterator = self.delegate_terms.iterator()?;
            let _ = self.delegate_terms_enum.set(iterator);
        }
        Ok(self
            .delegate_terms_enum
            .get()
            .expect("INVARIANT: the delegate was pulled above")
            .as_ref())
    }
}

impl fmt::Display for BloomFilteredTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BloomFilteredTermsEnum(filter={})", self.filter)
    }
}

impl TermsEnum for BloomFilteredTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        self.attributes.get_or_insert_with(AttributeSource::new)
    }

    fn next(&mut self) -> Result<Option<&BytesRef>> {
        self.delegate()?.next()
    }

    fn prepare_seek_exact<'a>(
        &'a mut self,
        text: &BytesRef,
    ) -> Result<Option<Box<dyn IOBooleanSupplier + 'a>>> {
        // The magical fail-fast speed up that is the entire point of all of
        // this code - save a disk seek if there is a match on an in-memory
        // structure
        // that may occasionally give a false positive but guaranteed no false
        // negatives
        if self.filter.contains(text) == ContainsResult::NO {
            return Ok(None);
        }
        self.delegate()?.prepare_seek_exact(text)
    }

    fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
        // See #prepareSeekExact
        if self.filter.contains(text) == ContainsResult::NO {
            return Ok(false);
        }
        self.delegate()?.seek_exact(text)
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        self.delegate()?.seek_ceil(text)
    }

    fn seek_ord(&mut self, ord: i64) -> Result<()> {
        self.delegate()?.seek_ord(ord)
    }

    fn term(&self) -> Result<Option<&BytesRef>> {
        self.delegate_ref()?.term()
    }

    fn ord(&self) -> Result<i64> {
        self.delegate_ref()?.ord()
    }

    fn doc_freq(&mut self) -> Result<i32> {
        self.delegate()?.doc_freq()
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        self.delegate()?.total_term_freq()
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Option<Box<dyn PostingsEnum>>> {
        self.delegate()?.postings(reuse, flags)
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        self.delegate()?.impacts(flags)
    }

    fn prefer_seek_exact(&self) -> bool {
        // Prefer seekExact() to seekCeil() when processing updates and deletes,
        // since seekExact() passes through the bloom filter.
        true
    }
}

/// Port of the inner class `BloomFilteredFieldsConsumer`.
///
/// **Divergence.** Java keeps the filters in a `HashMap<FieldInfo,
/// FuzzySet>` and writes them in its iteration order, which follows
/// `FieldInfo`'s identity hash codes and so changes from run to run (the
/// fixtures of one Lucene run hold the filters of the same segment in two
/// orders). The port writes them in the order the fields were filtered: a
/// `.blm` file holds the same filters, in an order Java's reader ignores.
struct BloomFilteredFieldsConsumer<'a> {
    delegate_fields_consumer: Box<dyn FieldsConsumer + 'a>,
    bloom_filters: Vec<(Option<FieldInfo>, FuzzySet)>,
    state: SegmentWriteState<'a>,
    bloom_filter_factory: Arc<dyn BloomFilterFactory>,
    delegate_postings_format_name: String,
    closed: bool,
}

impl BloomFilteredFieldsConsumer<'_> {
    /// Port of `saveAppropriatelySizedBloomFilter(IndexOutput, FuzzySet,
    /// FieldInfo)`.
    fn save_appropriately_sized_bloom_filter(
        &self,
        bloom_output: &mut dyn IndexOutput,
        bloom_filter: &FuzzySet,
        field_info: &FieldInfo,
    ) -> Result<()> {
        let right_sized_set = self
            .bloom_filter_factory
            .downsize(Some(field_info), bloom_filter);
        match &right_sized_set {
            Some(set) => set.serialize(bloom_output),
            None => bloom_filter.serialize(bloom_output),
        }
    }
}

fn null_field_info() -> LuceneError {
    LuceneError::unchecked(
        "java.lang.NullPointerException",
        "Cannot read field \"number\" because \"fieldInfo\" is null",
    )
}

impl FieldsConsumer for BloomFilteredFieldsConsumer<'_> {
    fn write(&mut self, fields: &dyn Fields, norms: Option<&dyn NormsProducer>) -> Result<()> {
        // Delegate must write first: it may have opened files
        // on creating the class
        // (e.g. Lucene41PostingsConsumer), and write() will
        // close them; alternatively, if we delayed pulling
        // the fields consumer until here, we could do it
        // afterwards:
        self.delegate_fields_consumer.write(fields, norms)?;

        for field in fields.iterator() {
            let Some(terms) = fields.terms(&field)? else {
                continue;
            };
            let field_info = self.state.field_infos.field_info(&field).cloned();
            let mut terms_enum = terms.iterator()?;

            let mut bloom_filter: Option<usize> = None;

            let mut postings_enum: Option<Box<dyn PostingsEnum>> = None;
            while let Some(term) = terms_enum.next()?.cloned() {
                let index = match bloom_filter {
                    Some(index) => index,
                    None => {
                        let set = self
                            .bloom_filter_factory
                            .get_set_for_field(&self.state, field_info.as_ref())?;
                        let Some(set) = set else {
                            // Field not bloom'd
                            break;
                        };
                        if cfg!(debug_assertions)
                            && self.bloom_filters.iter().any(|(info, _)| {
                                info.as_ref().map(|i| i.number)
                                    == field_info.as_ref().map(|i| i.number)
                            })
                        {
                            return Err(LuceneError::from(LuceneErrorKind::AssertionError(
                                String::new(),
                            )));
                        }
                        self.bloom_filters.push((field_info.clone(), set));
                        let index = self.bloom_filters.len() - 1;
                        bloom_filter = Some(index);
                        index
                    }
                };
                // Make sure there's at least one doc for this term:
                let mut pe = terms_enum
                    .postings(postings_enum.take(), POSTINGS_ENUM_NONE)?
                    .ok_or_else(LuceneError::null_pointer)?;
                if pe.next_doc()? != NO_MORE_DOCS {
                    self.bloom_filters[index].1.add_value(&term);
                }
                postings_enum = Some(pe);
            }
        }
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        self.delegate_fields_consumer.close()?;

        // Now we are done accumulating values for these fields
        let bloom_filters = std::mem::take(&mut self.bloom_filters);
        let mut non_saturated_blooms: Vec<(&Option<FieldInfo>, &FuzzySet)> = Vec::new();

        for (info, bloom_filter) in &bloom_filters {
            if !self
                .bloom_filter_factory
                .is_saturated(bloom_filter, info.as_ref())
            {
                non_saturated_blooms.push((info, bloom_filter));
            }
        }
        let bloom_file_name = segment_file_name(
            &self.state.segment_info.name,
            &self.state.segment_suffix,
            BLOOM_EXTENSION,
        );
        let mut bloom_output = self
            .state
            .directory
            .create_output(&bloom_file_name, self.state.context)?;
        let written = (|| -> Result<()> {
            write_index_header(
                bloom_output.as_mut(),
                BLOOM_CODEC_NAME,
                VERSION_CURRENT,
                &self.state.segment_info.id(),
                &self.state.segment_suffix,
            )?;
            // remember the name of the postings format we will delegate to
            bloom_output.write_string(&self.delegate_postings_format_name)?;

            // First field in the output file is the number of fields+blooms saved
            bloom_output.write_int(non_saturated_blooms.len() as i32)?;
            for (field_info, bloom_filter) in &non_saturated_blooms {
                let field_info = field_info.as_ref().ok_or_else(null_field_info)?;
                bloom_output.write_int(field_info.number)?;
                self.save_appropriately_sized_bloom_filter(
                    bloom_output.as_mut(),
                    bloom_filter,
                    field_info,
                )?;
            }
            write_footer(bloom_output.as_mut())
        })();
        // try-with-resources: the body's exception wins, `close()`'s is
        // suppressed.
        match written {
            Ok(()) => bloom_output.close()?,
            Err(e) => {
                let _ = bloom_output.close();
                return Err(e);
            }
        }
        // We are done with large bitsets so no need to keep them hanging around
        Ok(())
    }
}

#[cfg(test)]
mod producer_to_string_tests {
    use super::*;

    /// A delegate whose class overrides `toString()`.
    struct Named;

    impl Fields for Named {
        fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
            Box::new(std::iter::empty())
        }
        fn terms(&self, _field: &str) -> Result<Option<Arc<dyn Terms>>> {
            Ok(None)
        }
        fn size(&self) -> i32 {
            0
        }
    }

    impl FieldsProducer for Named {
        fn to_java_string(&self) -> String {
            "Named(fields=2)".to_string()
        }
        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }
        fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
            Ok(Box::new(Named))
        }
        fn close(&self) -> Result<()> {
            Ok(())
        }
    }

    /// `getClass().getSimpleName() + "(fields=" + bloomsByFieldName.size()
    /// + ",delegate=" + delegateFieldsProducer + ")"`.
    #[test]
    fn to_string_matches_java() {
        let mut blooms = HashMap::new();
        blooms.insert(
            "id".to_string(),
            Arc::new(FuzzySet::create_set_based_on_max_memory(1024)),
        );
        blooms.insert(
            "title".to_string(),
            Arc::new(FuzzySet::create_set_based_on_max_memory(1024)),
        );
        let producer = BloomFilteredFieldsProducer {
            delegate_fields_producer: Arc::new(Named),
            blooms_by_field_name: Arc::new(blooms),
        };
        assert_eq!(
            producer.to_string(),
            "BloomFilteredFieldsProducer(fields=2,delegate=Named(fields=2))"
        );
        assert_eq!(producer.to_java_string(), producer.to_string());

        let empty = BloomFilteredFieldsProducer {
            delegate_fields_producer: Arc::new(Named),
            blooms_by_field_name: Arc::new(HashMap::new()),
        };
        assert_eq!(
            empty.to_string(),
            "BloomFilteredFieldsProducer(fields=0,delegate=Named(fields=2))"
        );
    }
}

#[cfg(test)]
mod fidelity_tests {
    use super::*;
    use rucene::error::LuceneErrorKind;
    use rucene::index::OrdTermState;

    /// A delegate the filter must never reach.
    struct UnusedTerms;

    impl Terms for UnusedTerms {
        fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
            panic!("the bloom filter answered NO; the delegate is not pulled")
        }
        fn size(&self) -> Result<i64> {
            Ok(0)
        }
        fn sum_total_term_freq(&self) -> Result<i64> {
            Ok(0)
        }
        fn sum_doc_freq(&self) -> Result<i64> {
            Ok(0)
        }
        fn doc_count(&self) -> Result<i32> {
            Ok(0)
        }
        fn has_freqs(&self) -> bool {
            false
        }
        fn has_offsets(&self) -> bool {
            false
        }
        fn has_positions(&self) -> bool {
            false
        }
        fn has_payloads(&self) -> bool {
            false
        }
    }

    /// Regression (rmp #526): the inherited `BaseTermsEnum.seekExact(term,
    /// state)` reports `"term=" + term + " does not exist"` with
    /// `BytesRef.toString()`; the port's override printed Rust's `Debug`.
    #[test]
    fn seek_term_state_message_uses_bytes_ref_to_string() {
        let mut terms_enum = BloomFilteredTermsEnum::new(
            Arc::new(UnusedTerms),
            Arc::new(FuzzySet::create_set_based_on_max_memory(1024)),
        );
        let error = terms_enum
            .seek_term_state(&BytesRef::new(b"ab".to_vec()), &OrdTermState::default())
            .unwrap_err();
        match &error.kind {
            LuceneErrorKind::IllegalArgument { message, .. } => {
                assert_eq!(message, "term=[61 62] does not exist")
            }
            other => panic!("{other:?}"),
        }
    }

    /// A delegate whose merge instance is a different producer.
    struct Original;

    impl Fields for Original {
        fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
            Box::new(std::iter::empty())
        }
        fn terms(&self, _field: &str) -> Result<Option<Arc<dyn Terms>>> {
            Ok(None)
        }
        fn size(&self) -> i32 {
            0
        }
    }

    impl FieldsProducer for Original {
        fn to_java_string(&self) -> String {
            "Original".to_string()
        }
        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }
        fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
            Ok(Box::new(super::producer_to_string_tests_support::Merged))
        }
        fn close(&self) -> Result<()> {
            Ok(())
        }
    }

    /// Regression (rmp #526): `getMergeInstance()` is inherited from
    /// `FieldsProducer` and returns `this`, so the merge instance keeps the
    /// original delegate; the port swapped in the delegate's merge instance.
    #[test]
    fn merge_instance_keeps_the_delegate() {
        let producer = BloomFilteredFieldsProducer {
            delegate_fields_producer: Arc::new(Original),
            blooms_by_field_name: Arc::new(HashMap::new()),
        };
        let merge = producer.get_merge_instance().unwrap();
        assert_eq!(
            merge.to_java_string(),
            "BloomFilteredFieldsProducer(fields=0,delegate=Original)"
        );
    }

    /// Regression (rmp #526): a field without a `FieldInfo` reaches the
    /// factory as `null` (the default factory ignores it), so `write`
    /// succeeds; the port threw a `NullPointerException` there.
    #[test]
    fn factories_accept_a_missing_field_info() {
        let factory = DefaultBloomFilterFactory::new();
        let set = FuzzySet::create_set_based_on_max_memory(1024);
        assert!(!factory.is_saturated(&set, None));
        assert!(factory.downsize(None, &set).is_some());
    }
}

#[cfg(test)]
mod producer_to_string_tests_support {
    use super::*;

    /// The merge instance `Original` hands out.
    pub(super) struct Merged;

    impl Fields for Merged {
        fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
            Box::new(std::iter::empty())
        }
        fn terms(&self, _field: &str) -> Result<Option<Arc<dyn Terms>>> {
            Ok(None)
        }
        fn size(&self) -> i32 {
            0
        }
    }

    impl FieldsProducer for Merged {
        fn to_java_string(&self) -> String {
            "Merged".to_string()
        }
        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }
        fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
            Ok(Box::new(Merged))
        }
        fn close(&self) -> Result<()> {
            Ok(())
        }
    }
}
