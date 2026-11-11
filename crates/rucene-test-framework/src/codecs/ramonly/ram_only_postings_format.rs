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

//! Port of `org.apache.lucene.tests.codecs.ramonly.RAMOnlyPostingsFormat`.

use std::any::Any;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::atomic::{AtomicI32, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use randomizedtesting::jdk::exception;
use rucene::codecs::codec_util::{check_header, write_header};
use rucene::codecs::{
    FieldsConsumer, FieldsProducer, NormsProducer, PostingsFormat, SegmentReadState,
    SegmentWriteState, TermStats,
};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::index_reader::java_object_to_string;
use rucene::index::{
    FieldInfo, FieldInfos, Fields, ImpactsEnum, IndexOptions, PostingsEnum, SeekStatus,
    SlowImpactsEnum, Terms, TermsEnum, POSTINGS_ENUM_FREQS, POSTINGS_ENUM_OFFSETS,
    POSTINGS_ENUM_PAYLOADS,
};
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::util::attribute::AttributeSource;
use rucene::util::unicode_util::UnicodeUtil;
use rucene::util::{Accountable, BytesRef, FixedBitSet, IOUtils, RamUsageEstimator};

const RAM_ONLY_NAME: &str = "RAMOnly";
const VERSION_START: i32 = 0;
const VERSION_LATEST: i32 = VERSION_START;

const ID_EXTENSION: &str = "id";

/// `java.lang.NullPointerException` with the JDK's helpful message.
fn npe(message: &str) -> LuceneError {
    LuceneError::unchecked("java.lang.NullPointerException", message)
}

/// A `java.lang.String`: its UTF-16 code units, which may hold unpaired
/// surrogates. The derived order is `String.compareTo` (code unit by code
/// unit, then length), the order of a `TreeMap<String, ?>`, not Rust's UTF-8
/// byte order.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct JavaString(Vec<u16>);

impl JavaString {
    /// A Rust string's UTF-16 code units.
    fn of(s: &str) -> Self {
        JavaString(s.encode_utf16().collect())
    }

    /// `BytesRef.utf8ToString()`: `UnicodeUtil.UTF8toUTF16` into a `char[]`
    /// of the byte length, then `new String(chars, 0, len)`.
    ///
    /// # Errors
    ///
    /// `UnicodeUtil.UTF8toUTF16`'s `AssertionError`s (a continuation byte as
    /// a lead byte, a lead byte past `0xf7`) under `-ea` (`debug_assertions`
    /// here), and its `ArrayIndexOutOfBoundsException` for a truncated
    /// sequence.
    fn utf8_to_string(utf8: &[u8]) -> Result<Self> {
        let byte = |i: usize| -> Result<u32> {
            utf8.get(i).map(|&b| u32::from(b)).ok_or_else(|| {
                LuceneError::unchecked(
                    "java.lang.ArrayIndexOutOfBoundsException",
                    format!("Index {i} out of bounds for length {}", utf8.len()),
                )
            })
        };
        let mut out = Vec::with_capacity(utf8.len());
        let mut offset = 0;
        while offset < utf8.len() {
            let b = byte(offset)?;
            offset += 1;
            if b < 0xc0 {
                java_assert!(b < 0x80);
                out.push(b as u16);
            } else if b < 0xe0 {
                out.push((((b & 0x1f) << 6) + (byte(offset)? & 0x3f)) as u16);
                offset += 1;
            } else if b < 0xf0 {
                out.push(
                    (((b & 0xf) << 12) + ((byte(offset)? & 0x3f) << 6) + (byte(offset + 1)? & 0x3f))
                        as u16,
                );
                offset += 2;
            } else {
                java_assert!(b < 0xf8, "b = 0x{b:x}");
                let ch = ((b & 0x7) << 18)
                    + ((byte(offset)? & 0x3f) << 12)
                    + ((byte(offset + 1)? & 0x3f) << 6)
                    + (byte(offset + 2)? & 0x3f);
                offset += 3;
                if ch < 0xFFFF {
                    out.push(ch as u16);
                } else {
                    let ch_half = ch - 0x0010000;
                    out.push(((ch_half >> 10) + 0xD800) as u16);
                    out.push(((ch_half & 0x3FF) + 0xDC00) as u16);
                }
            }
        }
        Ok(JavaString(out))
    }

    /// `new BytesRef(CharSequence)`: `UnicodeUtil.UTF16toUTF8`, which writes
    /// an unpaired surrogate as U+FFFD.
    fn to_bytes_ref(&self) -> BytesRef {
        let mut bytes = vec![0u8; UnicodeUtil::max_utf8_length(self.0.len())];
        let length = UnicodeUtil::utf16_to_utf8(&self.0, 0, self.0.len(), &mut bytes);
        bytes.truncate(length);
        BytesRef::new(bytes)
    }

    /// The field name a `String` key holds.
    fn to_rust_string(&self) -> String {
        String::from_utf16_lossy(&self.0)
    }
}

/// Stores all postings data in RAM, but writes a small token (header +
/// single int) to identify which "slot" the index is using in RAM HashMap.
///
/// NOTE: this codec sorts terms by reverse-unicode-order! (Java `String`
/// order, that is UTF-16 code unit order, which differs from the UTF-8 byte
/// order of the other formats for terms beyond the BMP.)
///
/// Port of `RAMOnlyPostingsFormat` (`final`, `extends PostingsFormat`, named
/// `RAMOnly`).
///
/// The postings live in this instance: an index written with one instance
/// can be read only through the same instance. Another instance (another
/// process, or a format found by name in another class loader) finds no
/// slot for the id, where Java's `fieldsProducer` returns `null`: its
/// producer then has no terms, and its `checkIntegrity()` and
/// `getMergeInstance()` fail with the `NullPointerException`s
/// `PerFieldPostingsFormat` throws on a `null` producer.
pub struct RAMOnlyPostingsFormat {
    /// Holds all indexes created, keyed by the ID assigned in fieldsConsumer.
    state: Mutex<HashMap<i32, Arc<RAMPostings>>>,
    next_id: AtomicI32,
}

impl RAMOnlyPostingsFormat {
    /// `new RAMOnlyPostingsFormat()`.
    pub fn new() -> Self {
        RAMOnlyPostingsFormat {
            state: Mutex::new(HashMap::new()),
            next_id: AtomicI32::new(0),
        }
    }
}

impl Default for RAMOnlyPostingsFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for RAMOnlyPostingsFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for RAMOnlyPostingsFormat {
    /// `PostingsFormat.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PostingsFormat(name=RAMOnly)")
    }
}

impl PostingsFormat for RAMOnlyPostingsFormat {
    fn name(&self) -> &str {
        "RAMOnly"
    }

    fn fields_consumer<'a>(
        &self,
        write_state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        let id = self.next_id.fetch_add(1, AtomicOrdering::SeqCst);

        // TODO -- ok to do this up front instead of
        // on close....?  should be ok?
        // Write our ID:
        let id_file_name = segment_file_name(
            &write_state.segment_info.name,
            &write_state.segment_suffix,
            ID_EXTENSION,
        );
        let mut out = write_state
            .directory
            .create_output(&id_file_name, write_state.context)?;
        let written = (|| -> Result<()> {
            write_header(out.as_mut(), RAM_ONLY_NAME, VERSION_LATEST)?;
            out.write_v_int(id)
        })();
        match written {
            Ok(()) => IOUtils::close([IOUtils::closeable(|| out.close())])?,
            Err(e) => {
                IOUtils::close_while_handling_exception([IOUtils::closeable(|| out.close())])?;
                return Err(e);
            }
        }

        let postings = Arc::new(RAMPostings::default());
        let consumer = RAMFieldsConsumer {
            postings: Arc::clone(&postings),
            field_infos: write_state.field_infos,
            segment_info: write_state.segment_info,
        };

        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, postings);
        Ok(Box::new(consumer))
    }

    fn fields_producer<'a>(
        &self,
        read_state: &SegmentReadState<'a>,
    ) -> Result<Box<dyn FieldsProducer>> {
        // Load our ID:
        let id_file_name = segment_file_name(
            &read_state.segment_info.name,
            &read_state.segment_suffix,
            ID_EXTENSION,
        );
        let mut input = read_state
            .directory
            .open_input(&id_file_name, read_state.context)?;
        let read = (|| -> Result<i32> {
            check_header(input.as_mut(), RAM_ONLY_NAME, VERSION_START, VERSION_LATEST)?;
            input.read_v_int()
        })();
        let id = match read {
            Ok(id) => {
                IOUtils::close([IOUtils::closeable(|| input.close())])?;
                id
            }
            Err(e) => {
                IOUtils::close_while_handling_exception([IOUtils::closeable(|| input.close())])?;
                return Err(e);
            }
        };

        let postings = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id)
            .cloned();
        Ok(match postings {
            Some(postings) => Box::new(RAMPostingsProducer(postings)),
            None => Box::new(MissingPostings),
        })
    }
}

// ---------------------------------------------------------------------------
// Postings state
// ---------------------------------------------------------------------------

/// Port of the package-private `RAMPostings` (`extends FieldsProducer`): the
/// fields of one segment, filled by its [`RAMFieldsConsumer`].
#[derive(Default)]
struct RAMPostings {
    field_to_terms: RwLock<BTreeMap<JavaString, Arc<RAMField>>>,
}

/// The `RAMPostings` object `fieldsProducer` hands out: the format's own,
/// shared, as Java returns the object of its map.
struct RAMPostingsProducer(Arc<RAMPostings>);

impl Fields for RAMPostingsProducer {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        let names: Vec<String> = self
            .0
            .field_to_terms
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .map(JavaString::to_rust_string)
            .collect();
        Box::new(names.into_iter())
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        Ok(self
            .0
            .field_to_terms
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&JavaString::of(field))
            .map(|f| Box::new(RAMTerms(Arc::clone(f))) as Box<dyn Terms>))
    }

    fn size(&self) -> i32 {
        self.0
            .field_to_terms
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .len() as i32
    }
}

impl FieldsProducer for RAMPostingsProducer {
    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    /// `FieldsProducer.getMergeInstance()`, which `RAMPostings` inherits:
    /// `this`.
    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        Ok(Box::new(RAMPostingsProducer(Arc::clone(&self.0))))
    }

    fn close(&self) -> Result<()> {
        Ok(())
    }
}

/// What `fieldsProducer` returns when the format holds no postings for the
/// segment's id (`state.get(id)` is `null`): Java returns `null`, which its
/// only caller, `PerFieldPostingsFormat.FieldsReader`, keeps as the
/// producer of the segment suffix.
///
/// **Divergence (declared).** A Rust `FieldsProducer` cannot be `null`;
/// this producer behaves as `FieldsReader` does with a `null` one
/// (`PerFieldPostingsFormat.java:340-365`): its fields have no terms
/// (`fieldsProducer == null ? null : ...`), and `checkIntegrity()` and
/// `getMergeInstance()` throw the `NullPointerException`s Java throws on
/// the `null` element.
struct MissingPostings;

impl Fields for MissingPostings {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(std::iter::empty())
    }

    fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
        Ok(None)
    }

    fn size(&self) -> i32 {
        0
    }
}

impl FieldsProducer for MissingPostings {
    fn check_integrity(&self) -> Result<()> {
        Err(npe(
            "Cannot invoke \"org.apache.lucene.codecs.FieldsProducer.checkIntegrity()\" because \
             \"producer\" is null",
        ))
    }

    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        Err(npe(
            "Cannot invoke \"org.apache.lucene.codecs.FieldsProducer.getMergeInstance()\" because \
             the return value of \"java.util.Map$Entry.getValue()\" is null",
        ))
    }

    fn close(&self) -> Result<()> {
        Ok(())
    }
}

/// Port of the package-private `RAMField` (`extends Terms`).
struct RAMField {
    #[allow(dead_code)] // Java's `field`, kept for parity; never read.
    field: String,
    term_to_docs: BTreeMap<JavaString, Arc<RAMTerm>>,
    sum_total_term_freq: i64,
    sum_doc_freq: i64,
    doc_count: i32,
    info: FieldInfo,
}

impl RAMField {
    fn new(field: &str, info: FieldInfo) -> Self {
        RAMField {
            field: field.to_string(),
            term_to_docs: BTreeMap::new(),
            sum_total_term_freq: 0,
            sum_doc_freq: 0,
            doc_count: 0,
            info,
        }
    }
}

/// The `RAMField` a `RAMPostings` hands out as its `Terms`.
struct RAMTerms(Arc<RAMField>);

impl Terms for RAMTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(RAMTermsEnum::new(Arc::clone(&self.0))))
    }

    fn size(&self) -> Result<i64> {
        Ok(self.0.term_to_docs.len() as i64)
    }

    fn sum_total_term_freq(&self) -> Result<i64> {
        Ok(self.0.sum_total_term_freq)
    }

    fn sum_doc_freq(&self) -> Result<i64> {
        Ok(self.0.sum_doc_freq)
    }

    fn doc_count(&self) -> Result<i32> {
        Ok(self.0.doc_count)
    }

    fn has_freqs(&self) -> bool {
        self.0
            .info
            .get_index_options()
            .subsumes(IndexOptions::DOCS_AND_FREQS)
    }

    fn has_offsets(&self) -> bool {
        self.0
            .info
            .get_index_options()
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
    }

    fn has_positions(&self) -> bool {
        self.0
            .info
            .get_index_options()
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
    }

    fn has_payloads(&self) -> bool {
        self.0.info.has_payloads()
    }
}

/// Port of the package-private `RAMTerm`.
struct RAMTerm {
    term: JavaString,
    total_term_freq: i64,
    docs: Vec<RAMDoc>,
}

/// Port of the package-private `RAMDoc` (`implements Accountable`).
struct RAMDoc {
    doc_id: i32,
    positions: Vec<i32>,
    payloads: Option<Vec<Option<Vec<u8>>>>,
}

impl RAMDoc {
    fn new(doc_id: i32, freq: i32) -> Result<Self> {
        if freq < 0 {
            // `new int[freq]` with the `-1` of a field without freqs.
            return Err(LuceneError::unchecked(
                "java.lang.NegativeArraySizeException",
                freq.to_string(),
            ));
        }
        Ok(RAMDoc {
            doc_id,
            positions: vec![0; freq as usize],
            payloads: None,
        })
    }
}

impl Accountable for RAMDoc {
    fn ram_bytes_used(&self) -> i64 {
        let mut size_in_bytes = RamUsageEstimator::size_of_int(&self.positions);
        if let Some(payloads) = &self.payloads {
            for payload in payloads.iter().flatten() {
                size_in_bytes += RamUsageEstimator::size_of(payload);
            }
        }
        size_in_bytes
    }
}

// ---------------------------------------------------------------------------
// Classes for writing to the postings state
// ---------------------------------------------------------------------------

/// Port of the private `RAMFieldsConsumer` (`extends FieldsConsumer`).
struct RAMFieldsConsumer<'a> {
    postings: Arc<RAMPostings>,
    field_infos: &'a FieldInfos,
    segment_info: &'a rucene::index::SegmentInfo,
}

impl FieldsConsumer for RAMFieldsConsumer<'_> {
    fn write(&mut self, fields: &dyn Fields, _norms: Option<&dyn NormsProducer>) -> Result<()> {
        for field in fields.iterator() {
            let Some(terms) = fields.terms(&field)? else {
                continue;
            };

            let mut terms_enum = terms.iterator()?;

            let Some(field_info) = self.field_infos.field_info(&field) else {
                return Err(npe(
                    "Cannot invoke \"org.apache.lucene.index.FieldInfo.getIndexOptions()\" because \
                     \"fieldInfo\" is null",
                ));
            };
            if field_info
                .get_index_options()
                .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
            {
                return Err(LuceneError::unsupported_operation(
                    "this codec cannot index offsets".to_string(),
                ));
            }

            // `postings.fieldToTerms.put(field, ramField)` before the terms
            // are written: the field is published with the terms finished so
            // far even when writing them fails.
            let mut ram_field = RAMField::new(&field, field_info.clone());
            let written = write_field(
                &mut ram_field,
                terms_enum.as_mut(),
                field_info,
                self.segment_info.max_doc()?,
            );
            self.postings
                .field_to_terms
                .write()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(JavaString::of(&field), Arc::new(ram_field));
            written?;
        }
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// The loop of `RAMFieldsConsumer.write` over one field's terms, with
/// `RAMTermsConsumer` and `RAMPostingsWriterImpl` inlined into the
/// [`RAMTerm`] and [`RAMDoc`] they fill.
fn write_field(
    ram_field: &mut RAMField,
    terms_enum: &mut dyn TermsEnum,
    field_info: &FieldInfo,
    max_doc: i32,
) -> Result<()> {
    let mut docs_seen = FixedBitSet::new(max_doc.max(0) as usize);
    let mut sum_total_term_freq = 0i64;
    let mut sum_doc_freq = 0i64;
    let mut postings_enum: Option<Box<dyn PostingsEnum>> = None;

    let index_options = field_info.get_index_options();
    let write_freqs = index_options.subsumes(IndexOptions::DOCS_AND_FREQS);
    let write_positions = index_options.subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
    let write_offsets =
        index_options.subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
    let write_payloads = field_info.has_payloads();

    let enum_flags = if !write_freqs {
        0
    } else if !write_positions {
        POSTINGS_ENUM_FREQS
    } else if !write_offsets {
        if write_payloads {
            POSTINGS_ENUM_PAYLOADS
        } else {
            0
        }
    } else if write_payloads {
        POSTINGS_ENUM_PAYLOADS | POSTINGS_ENUM_OFFSETS
    } else {
        POSTINGS_ENUM_OFFSETS
    };

    while let Some(term) = terms_enum.next()? {
        // termsConsumer.startTerm(term)
        let mut current = RAMTerm {
            term: JavaString::utf8_to_string(term.slice())?,
            total_term_freq: 0,
            docs: Vec::new(),
        };
        let mut pe = terms_enum
            .postings(postings_enum.take(), enum_flags)?
            .ok_or_else(LuceneError::null_pointer)?;

        let mut doc_freq = 0i32;
        let mut total_term_freq = 0i64;
        loop {
            let doc_id = pe.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            docs_seen.set(doc_id as usize);
            doc_freq += 1;

            let freq = if write_freqs {
                let freq = pe.freq()?;
                total_term_freq += i64::from(freq);
                freq
            } else {
                -1
            };

            // postingsWriter.startDoc(docID, freq)
            let mut doc = RAMDoc::new(doc_id, freq)?;
            if write_positions {
                for pos_upto in 0..freq as usize {
                    let pos = pe.next_position()?;
                    let payload = if write_payloads {
                        pe.get_payload()?.map(<[u8]>::to_vec)
                    } else {
                        None
                    };
                    let (start_offset, end_offset) = if write_offsets {
                        (pe.start_offset(), pe.end_offset())
                    } else {
                        (-1, -1)
                    };
                    // postingsWriter.addPosition(pos, payload, startOffset, endOffset)
                    java_assert!(start_offset == -1);
                    java_assert!(end_offset == -1);
                    doc.positions[pos_upto] = pos;
                    if let Some(payload) = payload.filter(|p| !p.is_empty()) {
                        let len = doc.positions.len();
                        doc.payloads.get_or_insert_with(|| vec![None; len])[pos_upto] =
                            Some(payload);
                    }
                }
            }

            // postingsWriter.finishDoc(): `assert posUpto == current.positions.length`
            // holds by construction when positions are written; without
            // positions `posUpto` stays 0.
            java_assert!(write_positions || doc.positions.is_empty());
            current.docs.push(doc);
        }
        postings_enum = Some(pe);
        let stats = TermStats::new(doc_freq, total_term_freq);
        // termsConsumer.finishTerm(term, stats)
        java_assert!(stats.doc_freq > 0);
        java_assert!(stats.doc_freq as usize == current.docs.len());
        current.total_term_freq = stats.total_term_freq;
        ram_field
            .term_to_docs
            .insert(current.term.clone(), Arc::new(current));
        sum_doc_freq += i64::from(doc_freq);
        sum_total_term_freq += total_term_freq;
    }

    // termsConsumer.finish(sumTotalTermFreq, sumDocFreq, docsSeen.cardinality())
    ram_field.sum_total_term_freq = sum_total_term_freq;
    ram_field.sum_doc_freq = sum_doc_freq;
    ram_field.doc_count = docs_seen.cardinality() as i32;
    Ok(())
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// Port of the package-private `RAMTermsEnum` (`extends BaseTermsEnum`).
///
/// Its quirks are Java's: `next()` after a seek starts at the sought term
/// itself (`tailMap(current)` is inclusive), and after a `NOT_FOUND` seek
/// `term()` is the sought term, whose `docFreq()` throws.
struct RAMTermsEnum {
    /// `it`: the keys still to iterate, `None` for Java's `null`.
    it: Option<std::vec::IntoIter<JavaString>>,
    current: Option<JavaString>,
    ram_field: Arc<RAMField>,
    attributes: Option<AttributeSource>,
}

impl RAMTermsEnum {
    fn new(field: Arc<RAMField>) -> Self {
        RAMTermsEnum {
            it: None,
            current: None,
            ram_field: field,
            attributes: None,
        }
    }

    /// `ramField.termToDocs.get(current)`.
    fn current_term(&self) -> Option<&Arc<RAMTerm>> {
        self.ram_field.term_to_docs.get(self.current.as_ref()?)
    }

    /// `new BytesRef(current)`.
    fn current_bytes(&self) -> Result<BytesRef> {
        match &self.current {
            Some(current) => Ok(current.to_bytes_ref()),
            None => Err(npe(
                "Cannot invoke \"java.lang.CharSequence.length()\" because \"text\" is null",
            )),
        }
    }
}

impl fmt::Display for RAMTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&java_object_to_string(
            "org.apache.lucene.tests.codecs.ramonly.RAMOnlyPostingsFormat$RAMTermsEnum",
            self,
        ))
    }
}

impl TermsEnum for RAMTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        self.attributes.get_or_insert_with(AttributeSource::new)
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        if self.it.is_none() {
            let keys: Vec<JavaString> = match &self.current {
                None => self.ram_field.term_to_docs.keys().cloned().collect(),
                Some(current) => self
                    .ram_field
                    .term_to_docs
                    .range(current.clone()..)
                    .map(|(k, _)| k.clone())
                    .collect(),
            };
            self.it = Some(keys.into_iter());
        }
        match self.it.as_mut().and_then(Iterator::next) {
            Some(next) => {
                self.current = Some(next);
                Ok(Some(self.current_bytes()?))
            }
            None => Ok(None),
        }
    }

    fn seek_ceil(&mut self, term: &BytesRef) -> Result<SeekStatus> {
        let key = JavaString::utf8_to_string(term.slice())?;
        self.current = Some(key.clone());
        self.it = None;
        if self.ram_field.term_to_docs.contains_key(&key) {
            Ok(SeekStatus::FOUND)
        } else {
            let Some((last_key, _)) = self.ram_field.term_to_docs.last_key_value() else {
                // `TreeMap.lastKey()` of an empty map.
                return Err(LuceneError::from(LuceneErrorKind::Unchecked {
                    class: "java.util.NoSuchElementException",
                    message: None,
                    cause: None,
                }));
            };
            if key.cmp(last_key) == Ordering::Greater {
                Ok(SeekStatus::END)
            } else {
                Ok(SeekStatus::NOT_FOUND)
            }
        }
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn ord(&self) -> Result<i64> {
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn term(&self) -> Result<Option<BytesRef>> {
        // TODO: reuse BytesRef
        self.current_bytes().map(Some)
    }

    fn doc_freq(&mut self) -> Result<i32> {
        match self.current_term() {
            Some(term) => Ok(term.docs.len() as i32),
            None => Err(npe(
                "Cannot read field \"docs\" because the return value of \
                 \"java.util.SortedMap.get(Object)\" is null",
            )),
        }
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        match self.current_term() {
            Some(term) => Ok(term.total_term_freq),
            None => Err(npe(
                "Cannot read field \"totalTermFreq\" because the return value of \
                 \"java.util.SortedMap.get(Object)\" is null",
            )),
        }
    }

    fn postings(
        &mut self,
        _reuse: Option<Box<dyn PostingsEnum>>,
        _flags: i32,
    ) -> Result<Option<Box<dyn PostingsEnum>>> {
        Ok(Some(Box::new(RAMDocsEnum::new(
            self.current_term().cloned(),
        ))))
    }

    fn impacts(&mut self, _flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        Ok(Box::new(SlowImpactsEnum::new(
            self.postings(None, POSTINGS_ENUM_FREQS)?
                .ok_or_else(LuceneError::null_pointer)?,
        )))
    }
}

/// Port of the private `RAMDocsEnum` (`extends PostingsEnum`).
///
/// As in Java, `docID()` after the last document is still that document
/// (the enum never moves `current` past it).
struct RAMDocsEnum {
    ram_term: Option<Arc<RAMTerm>>,
    current: Option<usize>,
    upto: i32,
    pos_upto: usize,
}

impl RAMDocsEnum {
    fn new(ram_term: Option<Arc<RAMTerm>>) -> Self {
        RAMDocsEnum {
            ram_term,
            current: None,
            upto: -1,
            pos_upto: 0,
        }
    }

    /// `ramTerm`; Java's `null` (a term `RAMTermsEnum` did not find) throws
    /// on the first read of its `docs`.
    fn ram_term(&self) -> Result<&Arc<RAMTerm>> {
        self.ram_term
            .as_ref()
            .ok_or_else(|| npe("Cannot read field \"docs\" because \"this.ramTerm\" is null"))
    }

    /// `current`, the `RAMDoc` of the enum's document; `field` names the
    /// field Java reads from the `null` before the first `nextDoc()`.
    fn current(&self, field: &str) -> Result<&RAMDoc> {
        match (self.current, self.ram_term.as_ref()) {
            (Some(i), Some(term)) => Ok(&term.docs[i]),
            _ => Err(npe(&format!(
                "Cannot read field \"{field}\" because \"this.current\" is null"
            ))),
        }
    }

    /// [`current`](Self::current) from a method that cannot fail: the NPE
    /// is thrown, unchecked.
    fn current_unchecked(&self, field: &str) -> &RAMDoc {
        match self.current(field) {
            Ok(doc) => doc,
            Err(e) => exception::throw(
                "java.lang.NullPointerException",
                java_message(&e).unwrap_or_default(),
            ),
        }
    }
}

/// The message of an unchecked Java error.
fn java_message(e: &LuceneError) -> Option<String> {
    match e {
        LuceneError {
            kind: LuceneErrorKind::Unchecked { message, .. },
            ..
        } => message.clone(),
        _ => None,
    }
}

impl fmt::Display for RAMDocsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&java_object_to_string(
            "org.apache.lucene.tests.codecs.ramonly.RAMOnlyPostingsFormat$RAMDocsEnum",
            self,
        ))
    }
}

impl DocIdSetIterator for RAMDocsEnum {
    fn doc_id(&self) -> i32 {
        self.current_unchecked("docID").doc_id
    }

    // TODO: override bulk read, for better perf
    fn next_doc(&mut self) -> Result<i32> {
        self.upto += 1;
        let docs = self.ram_term()?.docs.len();
        if (self.upto as usize) < docs {
            self.current = Some(self.upto as usize);
            self.pos_upto = 0;
            Ok(self.current("docID")?.doc_id)
        } else {
            Ok(NO_MORE_DOCS)
        }
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.slow_advance(target)
    }

    fn cost(&self) -> i64 {
        match &self.ram_term {
            Some(term) => term.docs.len() as i64,
            None => exception::throw(
                "java.lang.NullPointerException",
                "Cannot read field \"docs\" because \"this.ramTerm\" is null",
            ),
        }
    }
}

impl PostingsEnum for RAMDocsEnum {
    fn freq(&self) -> Result<i32> {
        Ok(self.current("positions")?.positions.len() as i32)
    }

    fn next_position(&mut self) -> Result<i32> {
        let pos_upto = self.pos_upto;
        let doc = self.current("positions")?;
        java_assert!(pos_upto < doc.positions.len());
        let Some(&position) = doc.positions.get(pos_upto) else {
            return Err(LuceneError::unchecked(
                "java.lang.ArrayIndexOutOfBoundsException",
                format!(
                    "Index {pos_upto} out of bounds for length {}",
                    doc.positions.len()
                ),
            ));
        };
        self.pos_upto += 1;
        Ok(position)
    }

    fn start_offset(&self) -> i32 {
        -1
    }

    fn end_offset(&self) -> i32 {
        -1
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        let doc = self.current("payloads")?;
        match &doc.payloads {
            Some(payloads) => {
                let Some(i) = self.pos_upto.checked_sub(1) else {
                    return Err(LuceneError::unchecked(
                        "java.lang.ArrayIndexOutOfBoundsException",
                        format!("Index -1 out of bounds for length {}", payloads.len()),
                    ));
                };
                Ok(payloads.get(i).and_then(|p| p.as_deref()))
            }
            None => Ok(None),
        }
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}
