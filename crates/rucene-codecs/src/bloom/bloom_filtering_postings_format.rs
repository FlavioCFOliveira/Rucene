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

use std::cell::RefCell;
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
    FieldInfo, Fields, ImpactsEnum, PostingsEnum, SeekStatus, TermState, Terms, TermsEnum,
    POSTINGS_ENUM_NONE,
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
/// named `BloomFilter`.
///
/// **Divergence.** Java's `toString()` is
/// `"BloomFilteringPostingsFormat(" + delegatePostingsFormat + ")"`. This
/// type's [`fmt::Display`] prints that; but a `dyn PostingsFormat` prints
/// `PostingsFormat(name=BloomFilter)` (`impl Display for dyn
/// PostingsFormat`), which a format cannot override, as for
/// `FSTPostingsFormat`.
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
///
/// **Divergence.** Java's `toString()` prints the delegate producer's;
/// [`FieldsProducer`] carries no `Display` in this port, so the producer has
/// no `Display` either.
struct BloomFilteredFieldsProducer {
    delegate_fields_producer: Box<dyn FieldsProducer>,
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
                delegate_fields_producer: delegate_fields_producer
                    .expect("INVARIANT: set before the success"),
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

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        match self.blooms_by_field_name.get(field) {
            None => self.delegate_fields_producer.terms(field),
            Some(filter) => {
                let Some(result) = self.delegate_fields_producer.terms(field)? else {
                    return Ok(None);
                };
                Ok(Some(Box::new(BloomFilteredTerms {
                    delegate_terms: Arc::from(result),
                    filter: Arc::clone(filter),
                })))
            }
        }
    }

    fn size(&self) -> i32 {
        self.delegate_fields_producer.size()
    }
}

impl FieldsProducer for BloomFilteredFieldsProducer {
    fn check_integrity(&self) -> Result<()> {
        self.delegate_fields_producer.check_integrity()
    }

    /// `FieldsProducer.getMergeInstance()`, which this producer inherits:
    /// `this`, here a producer over the delegate's merge instance (which is
    /// itself, for every delegate that does not override it) and the same
    /// filters.
    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        Ok(Box::new(Self {
            delegate_fields_producer: self.delegate_fields_producer.get_merge_instance()?,
            blooms_by_field_name: Arc::clone(&self.blooms_by_field_name),
        }))
    }

    fn close(&mut self) -> Result<()> {
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
/// so the lazily pulled enum sits in a [`RefCell`].
struct BloomFilteredTermsEnum {
    delegate_terms: Arc<dyn Terms>,
    delegate_terms_enum: RefCell<Option<Box<dyn TermsEnum>>>,
    filter: Arc<FuzzySet>,
    attributes: Option<AttributeSource>,
}

impl BloomFilteredTermsEnum {
    /// Port of `BloomFilteredTermsEnum(Terms, FuzzySet)`.
    fn new(delegate_terms: Arc<dyn Terms>, filter: Arc<FuzzySet>) -> Self {
        Self {
            delegate_terms,
            delegate_terms_enum: RefCell::new(None),
            filter,
            attributes: None,
        }
    }

    /// Port of the package-private `reset(Terms)`.
    #[allow(dead_code)]
    fn reset(&mut self, delegate_terms: Arc<dyn Terms>) {
        self.delegate_terms = delegate_terms;
        *self.delegate_terms_enum.get_mut() = None;
    }

    /// Port of `delegate()`.
    fn delegate(&mut self) -> Result<&mut dyn TermsEnum> {
        let slot = self.delegate_terms_enum.get_mut();
        if slot.is_none() {
            /* pull the iterator only if we really need it -
             * this can be a relativly heavy operation depending on the
             * delegate postings format and they underlying directory
             * (clone IndexInput) */
            *slot = Some(self.delegate_terms.iterator()?);
        }
        Ok(slot
            .as_deref_mut()
            .expect("INVARIANT: the delegate was pulled above"))
    }

    /// [`delegate`](Self::delegate) for the `&self` methods.
    fn with_delegate<T>(&self, f: impl FnOnce(&dyn TermsEnum) -> Result<T>) -> Result<T> {
        let mut slot = self.delegate_terms_enum.borrow_mut();
        if slot.is_none() {
            *slot = Some(self.delegate_terms.iterator()?);
        }
        f(slot
            .as_deref()
            .expect("INVARIANT: the delegate was pulled above"))
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

    fn next(&mut self) -> Result<Option<BytesRef>> {
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

    /// `BaseTermsEnum.seekExact(BytesRef, TermState)`, which Java's enum
    /// inherits: `seekExact(term)` (through the filter), else
    /// `IllegalArgumentException`.
    fn seek_term_state(&mut self, text: &BytesRef, _state: &dyn TermState) -> Result<()> {
        if !self.seek_exact(text)? {
            return Err(LuceneError::illegal_argument(format!(
                "term={text:?} does not exist"
            )));
        }
        Ok(())
    }

    fn term(&self) -> Result<Option<BytesRef>> {
        self.with_delegate(|d| d.term())
    }

    fn ord(&self) -> Result<i64> {
        self.with_delegate(|d| d.ord())
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
        let right_sized_set = self.bloom_filter_factory.downsize(field_info, bloom_filter);
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
            while let Some(term) = terms_enum.next()? {
                let index = match bloom_filter {
                    Some(index) => index,
                    None => {
                        let set = match &field_info {
                            Some(info) => self
                                .bloom_filter_factory
                                .get_set_for_field(&self.state, info)?,
                            None => {
                                return Err(LuceneError::unchecked(
                                    "java.lang.NullPointerException",
                                    "Cannot read field \"name\" because \"fieldInfo\" is null",
                                ))
                            }
                        };
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
            let info_ref = info.as_ref().ok_or_else(null_field_info)?;
            if !self
                .bloom_filter_factory
                .is_saturated(bloom_filter, info_ref)
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
