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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextDocValuesWriter`.

use std::collections::HashSet;

use rucene::codecs::{DocValuesConsumer, DocValuesProducer, SegmentWriteState};
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::{
    BinaryDocValues, DocValuesIterator, DocValuesType, FieldInfo, SortedNumericDocValues,
};
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::store::IndexOutput;
use rucene::util::BytesRef;

use super::field_infos_format::enum_name;
use super::util;

pub(crate) const END: &[u8] = b"END";
pub(crate) const FIELD: &[u8] = b"field ";
pub(crate) const TYPE: &[u8] = b"  type ";
pub(crate) const DOCCOUNT: &[u8] = b"  doccount ";
// used for numerics
pub(crate) const ORIGIN: &[u8] = b"  origin "; // for deltas

pub(crate) const MINVALUE: &[u8] = b"  minalue ";
pub(crate) const MAXVALUE: &[u8] = b"  maxvalue ";
pub(crate) const MAXVALUECOUNT: &[u8] = b"  maxvaluecount ";

pub(crate) const PATTERN: &[u8] = b"  pattern ";
// used for bytes
pub(crate) const LENGTH: &[u8] = b"length ";
pub(crate) const MAXLENGTH: &[u8] = b"  maxlength ";
// used for sorted bytes
pub(crate) const NUMVALUES: &[u8] = b"  numvalues ";
pub(crate) const ORDPATTERN: &[u8] = b"  ordpattern ";
// used for skip data
pub(crate) const SKIP_DATA: &[u8] = b"  skipdata";
pub(crate) const SKIP_INTERVAL: &[u8] = b"    interval";
pub(crate) const SKIP_MIN_DOC_ID: &[u8] = b"      minDocID ";
pub(crate) const SKIP_MAX_DOC_ID: &[u8] = b"      maxDocID ";
pub(crate) const SKIP_MIN_VALUE: &[u8] = b"      minValue ";
pub(crate) const SKIP_MAX_VALUE: &[u8] = b"      maxValue ";
pub(crate) const SKIP_DOC_COUNT: &[u8] = b"      docCount ";
pub(crate) const SKIP_END: &[u8] = b"  skipend";

pub(crate) const SKIP_INTERVAL_SIZE: i32 = 8;

/// Port of the record `org.apache.lucene.codecs.simpletext.SimpleTextDocValuesWriter.SkipInterval`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SkipInterval {
    pub(crate) min_doc_id: i32,
    pub(crate) max_doc_id: i32,
    pub(crate) min_value: i64,
    pub(crate) max_value: i64,
    pub(crate) doc_count: i32,
}

/// Accumulates the skip intervals of a field, as every `add*Field` does
/// inline in Java.
struct SkipIntervals {
    intervals: Vec<SkipInterval>,
    interval_doc_count: i32,
    interval_min_doc_id: i32,
    interval_max_doc_id: i32,
    interval_min_value: i64,
    interval_max_value: i64,
}

impl SkipIntervals {
    fn new() -> Self {
        Self {
            intervals: Vec::new(),
            interval_doc_count: 0,
            interval_min_doc_id: -1,
            interval_max_doc_id: -1,
            interval_min_value: i64::MAX,
            interval_max_value: i64::MIN,
        }
    }

    /// The per-document head of the loop: the interval's first and last doc.
    fn start_doc(&mut self, doc: i32) {
        if self.interval_doc_count == 0 {
            self.interval_min_doc_id = doc;
        }
        self.interval_max_doc_id = doc;
    }

    /// One value of the document.
    fn value(&mut self, v: i64) {
        self.interval_min_value = self.interval_min_value.min(v);
        self.interval_max_value = self.interval_max_value.max(v);
    }

    /// The per-document tail of the loop.
    fn end_doc(&mut self) {
        self.interval_doc_count += 1;
        if self.interval_doc_count == SKIP_INTERVAL_SIZE {
            self.push();
            self.interval_doc_count = 0;
            self.interval_min_value = i64::MAX;
            self.interval_max_value = i64::MIN;
        }
    }

    fn push(&mut self) {
        self.intervals.push(SkipInterval {
            min_doc_id: self.interval_min_doc_id,
            max_doc_id: self.interval_max_doc_id,
            min_value: self.interval_min_value,
            max_value: self.interval_max_value,
            doc_count: self.interval_doc_count,
        });
    }

    /// The trailing partial interval.
    fn finish(mut self) -> Vec<SkipInterval> {
        if self.interval_doc_count > 0 {
            self.push();
        }
        self.intervals
    }
}

/// `new DecimalFormat("0…0", ROOT).format(value)` for a non-negative value:
/// at least as many digits as the pattern has zeros.
pub(crate) fn decimal_format(pattern_len: usize, value: i128) -> String {
    if value < 0 {
        format!("-{:0width$}", -value, width = pattern_len)
    } else {
        format!("{value:0width$}", width = pattern_len)
    }
}

/// Writes plain-text doc values.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextDocValuesWriter`.
pub struct SimpleTextDocValuesWriter {
    data: Option<Box<dyn IndexOutput>>,
    num_docs: i32,
    fields_seen: HashSet<String>, // for asserting
}

impl std::fmt::Debug for SimpleTextDocValuesWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleTextDocValuesWriter")
            .field("num_docs", &self.num_docs)
            .finish_non_exhaustive()
    }
}

fn missing(what: &str, field: &FieldInfo) -> LuceneError {
    LuceneError::illegal_state(format!("no {what} doc values for field \"{}\"", field.name))
}

impl SimpleTextDocValuesWriter {
    /// Port of `SimpleTextDocValuesWriter(SegmentWriteState, String)`.
    ///
    /// # Errors
    ///
    /// Propagates the directory's error.
    pub fn new(state: &SegmentWriteState<'_>, ext: &str) -> Result<Self> {
        let data = state.directory.create_output(
            &segment_file_name(&state.segment_info.name, &state.segment_suffix, ext),
            state.context,
        )?;
        Ok(Self {
            data: Some(data),
            num_docs: state.segment_info.max_doc()?,
            fields_seen: HashSet::new(),
        })
    }

    fn data(&mut self) -> Result<&mut dyn IndexOutput> {
        match self.data.as_deref_mut() {
            Some(data) => Ok(data),
            None => Err(LuceneError::already_closed("this writer is closed")),
        }
    }

    fn write_bytes(&mut self, b: &[u8]) -> Result<()> {
        util::write_bytes(self.data()?, b)
    }

    fn write_str(&mut self, s: &str) -> Result<()> {
        util::write_str(self.data()?, s)
    }

    fn newline(&mut self) -> Result<()> {
        util::write_newline(self.data()?)
    }

    fn line(&mut self, prefix: &[u8], value: &str) -> Result<()> {
        self.write_bytes(prefix)?;
        self.write_str(value)?;
        self.newline()
    }

    /// for asserting
    fn field_seen(&mut self, field: &str) -> bool {
        debug_assert!(
            !self.fields_seen.contains(field),
            "field \"{field}\" was added more than once during flush"
        );
        self.fields_seen.insert(field.to_string());
        true
    }

    /// Port of the private `doAddBinaryField(FieldInfo, DocValuesProducer,
    /// int)`; `values` stands for `valuesProducer.getBinary(field)`.
    fn do_add_binary_field(
        &mut self,
        values: &dyn Fn() -> Result<Box<dyn BinaryDocValues>>,
        max_value_count: i32,
    ) -> Result<()> {
        let mut max_length = 0usize;
        let mut it = values()?;
        let mut doc_count = 0i32;
        let mut doc = it.next_doc()?;
        while doc != NO_MORE_DOCS {
            doc_count += 1;
            max_length = max_length.max(it.binary_value()?.to_string().encode_utf16().count());
            doc = it.next_doc()?;
        }

        self.line(DOCCOUNT, &doc_count.to_string())?;

        let count = if max_value_count == -1 {
            if doc_count == 0 {
                0
            } else {
                1
            }
        } else {
            max_value_count
        };
        self.line(MAXVALUECOUNT, &count.to_string())?;

        // write maxLength
        self.line(MAXLENGTH, &max_length.to_string())?;

        let max_bytes_length = max_length.to_string().len();
        let pattern = "0".repeat(max_bytes_length);
        // write our pattern for encoding lengths
        self.line(PATTERN, &pattern)?;

        let mut it = values()?;
        let mut num_docs_written = 0;
        for i in 0..self.num_docs {
            if it.doc_id() < i {
                it.next_doc()?;
                debug_assert!(it.doc_id() >= i);
            }
            let string_val = if it.doc_id() == i {
                Some(it.binary_value()?.to_string())
            } else {
                None
            };
            // write length
            let length = string_val.as_ref().map_or(0, |s| s.encode_utf16().count());
            self.line(LENGTH, &decimal_format(max_bytes_length, length as i128))?;

            // write bytes as hex array
            if let Some(string_val) = &string_val {
                self.write_str(string_val)?;
            }

            // pad to fit
            for _ in length..max_length {
                self.data()?.write_byte(b' ')?;
            }
            self.newline()?;
            if string_val.is_none() {
                self.write_str("F")?;
            } else {
                self.write_str("T")?;
            }
            self.newline()?;
            num_docs_written += 1;
        }

        debug_assert_eq!(self.num_docs, num_docs_written);
        Ok(())
    }

    /// Port of the private `writeSkipData`.
    fn write_skip_data(&mut self, intervals: &[SkipInterval]) -> Result<()> {
        self.write_bytes(SKIP_DATA)?;
        self.newline()?;
        for interval in intervals {
            self.write_bytes(SKIP_INTERVAL)?;
            self.newline()?;
            self.line(SKIP_MIN_DOC_ID, &interval.min_doc_id.to_string())?;
            self.line(SKIP_MAX_DOC_ID, &interval.max_doc_id.to_string())?;
            self.line(SKIP_MIN_VALUE, &interval.min_value.to_string())?;
            self.line(SKIP_MAX_VALUE, &interval.max_value.to_string())?;
            self.line(SKIP_DOC_COUNT, &interval.doc_count.to_string())?;
        }
        self.write_bytes(SKIP_END)?;
        self.newline()
    }

    /// write the header for this field
    fn write_field_entry(&mut self, field: &FieldInfo, type_: DocValuesType) -> Result<()> {
        self.line(FIELD, &field.name)?;
        self.line(TYPE, &enum_name(type_))
    }

    /// Writes the sorted values of a `SORTED` or `SORTED_SET` field, each
    /// padded to `max_length`, raw (Java writes them with
    /// `data.writeBytes`, which does not escape).
    fn write_sorted_values(
        &mut self,
        mut terms: Box<dyn rucene::index::TermsEnum>,
        encoder_len: usize,
        max_length: i32,
    ) -> Result<i64> {
        let mut values_seen = 0i64;
        while let Some(value) = terms.next()? {
            // write length
            self.line(LENGTH, &decimal_format(encoder_len, value.length as i128))?;

            // write bytes -- don't use SimpleText.write because it escapes:
            self.data()?.write_bytes(value.slice(), 0, value.length)?;

            // pad to fit
            for _ in value.length as i32..max_length {
                self.data()?.write_byte(b' ')?;
            }
            self.newline()?;
            values_seen += 1;
        }
        Ok(values_seen)
    }
}

impl DocValuesConsumer for SimpleTextDocValuesWriter {
    /// Port of `SimpleTextDocValuesWriter.addNumericField`.
    fn add_numeric_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
    ) -> Result<()> {
        debug_assert!(self.field_seen(&field.name));
        debug_assert!(field.get_doc_values_type() == DocValuesType::NUMERIC || field.has_norms());
        self.write_field_entry(field, DocValuesType::NUMERIC)?;

        // first pass to find min/max and accumulate skip intervals
        let mut min_value = i64::MAX;
        let mut max_value = i64::MIN;
        let mut values = values_producer
            .get_numeric(field)?
            .ok_or_else(|| missing("numeric", field))?;
        let mut num_values = 0i32;
        let mut skip = SkipIntervals::new();
        let mut doc = values.next_doc()?;
        while doc != NO_MORE_DOCS {
            let v = values.long_value()?;
            min_value = min_value.min(v);
            max_value = max_value.max(v);
            num_values += 1;
            skip.start_doc(doc);
            skip.value(v);
            skip.end_doc();
            doc = values.next_doc()?;
        }
        let skip_intervals = skip.finish();

        // write absolute min and max for skipper
        self.line(MINVALUE, &min_value.to_string())?;
        self.line(MAXVALUE, &max_value.to_string())?;
        self.line(DOCCOUNT, &num_values.to_string())?;
        self.line(
            MAXVALUECOUNT,
            &(if num_values == 0 { 0 } else { 1 }).to_string(),
        )?;

        if num_values != self.num_docs {
            min_value = min_value.min(0);
            max_value = max_value.max(0);
        }

        // write our minimum value to the .dat, all entries are deltas from that
        self.line(ORIGIN, &min_value.to_string())?;

        // build up our fixed-width "simple text packed ints" format
        let diff_big = i128::from(max_value) - i128::from(min_value);
        let max_bytes_per_value = diff_big.to_string().len();
        let pattern_string = "0".repeat(max_bytes_per_value);

        // write our pattern to the .dat
        self.line(PATTERN, &pattern_string)?;

        let mut num_docs_written = 0;

        // second pass to write the values
        let mut values = values_producer
            .get_numeric(field)?
            .ok_or_else(|| missing("numeric", field))?;
        for i in 0..self.num_docs {
            if values.doc_id() < i {
                values.next_doc()?;
                debug_assert!(values.doc_id() >= i);
            }
            let value = if values.doc_id() != i {
                0
            } else {
                values.long_value()?
            };
            debug_assert!(value >= min_value);
            let delta = i128::from(value) - i128::from(min_value);
            let s = decimal_format(pattern_string.len(), delta);
            debug_assert_eq!(s.len(), pattern_string.len());
            self.write_str(&s)?;
            self.newline()?;
            if values.doc_id() != i {
                self.write_str("F")?;
            } else {
                self.write_str("T")?;
            }
            self.newline()?;
            num_docs_written += 1;
            debug_assert!(num_docs_written <= self.num_docs);
        }

        debug_assert_eq!(
            self.num_docs, num_docs_written,
            "numDocs={} numDocsWritten={num_docs_written}",
            self.num_docs
        );

        self.write_skip_data(&skip_intervals)
    }

    /// Port of `SimpleTextDocValuesWriter.addBinaryField`.
    fn add_binary_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
    ) -> Result<()> {
        debug_assert!(self.field_seen(&field.name));
        debug_assert_eq!(field.get_doc_values_type(), DocValuesType::BINARY);
        self.write_field_entry(field, DocValuesType::BINARY)?;
        let values = || -> Result<Box<dyn BinaryDocValues>> {
            values_producer
                .get_binary(field)?
                .ok_or_else(|| missing("binary", field))
        };
        self.do_add_binary_field(&values, -1)
    }

    /// Port of `SimpleTextDocValuesWriter.addSortedField`.
    fn add_sorted_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
    ) -> Result<()> {
        debug_assert!(self.field_seen(&field.name));
        debug_assert_eq!(field.get_doc_values_type(), DocValuesType::SORTED);
        self.write_field_entry(field, DocValuesType::SORTED)?;

        let sorted = || {
            values_producer
                .get_sorted(field)?
                .ok_or_else(|| missing("sorted", field))
        };

        let mut doc_count = 0i32;
        let mut skip = SkipIntervals::new();
        let mut values = sorted()?;
        let mut doc = values.next_doc()?;
        while doc != NO_MORE_DOCS {
            doc_count += 1;
            let ord = values.ord_value()?;
            skip.start_doc(doc);
            skip.value(i64::from(ord));
            skip.end_doc();
            doc = values.next_doc()?;
        }
        let skip_intervals = skip.finish();

        self.line(DOCCOUNT, &doc_count.to_string())?;
        self.line(
            MAXVALUECOUNT,
            &(if doc_count == 0 { 0 } else { 1 }).to_string(),
        )?;

        let mut value_count = 0i32;
        let mut max_length = -1i32;
        let mut terms = sorted()?.terms_enum()?;
        while let Some(value) = terms.next()? {
            max_length = max_length.max(value.length as i32);
            value_count += 1;
        }

        // write numValues
        self.line(NUMVALUES, &value_count.to_string())?;

        // write maxLength
        self.line(MAXLENGTH, &max_length.to_string())?;

        let max_bytes_length = max_length.to_string().len();
        let pattern = "0".repeat(max_bytes_length);

        // write our pattern for encoding lengths
        self.line(PATTERN, &pattern)?;

        let max_ord_bytes = (i64::from(value_count) + 1).to_string().len();
        let ord_pattern = "0".repeat(max_ord_bytes);

        // write our pattern for ords
        self.line(ORDPATTERN, &ord_pattern)?;

        // for asserts:
        let values_seen =
            self.write_sorted_values(sorted()?.terms_enum()?, max_bytes_length, max_length)?;

        debug_assert_eq!(values_seen, i64::from(value_count));

        let mut values = sorted()?;
        for i in 0..self.num_docs {
            if values.doc_id() < i {
                values.next_doc()?;
                debug_assert!(values.doc_id() >= i);
            }
            let mut ord = -1i32;
            if values.doc_id() == i {
                ord = values.ord_value()?;
            }
            self.write_str(&decimal_format(max_ord_bytes, i128::from(ord) + 1))?;
            self.newline()?;
        }

        self.write_skip_data(&skip_intervals)
    }

    /// Port of `SimpleTextDocValuesWriter.addSortedNumericField`.
    fn add_sorted_numeric_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
    ) -> Result<()> {
        debug_assert!(self.field_seen(&field.name));
        debug_assert_eq!(field.get_doc_values_type(), DocValuesType::SORTED_NUMERIC);
        self.write_field_entry(field, DocValuesType::SORTED_NUMERIC)?;

        let sorted_numeric = || {
            values_producer
                .get_sorted_numeric(field)?
                .ok_or_else(|| missing("sorted numeric", field))
        };

        let mut min_value = i64::MAX;
        let mut max_value = i64::MIN;
        let mut max_value_count = 0i32;
        let mut skip = SkipIntervals::new();
        let mut values = sorted_numeric()?;
        let mut doc = values.next_doc()?;
        while doc != NO_MORE_DOCS {
            let value_count = values.doc_value_count()?;
            max_value_count = max_value_count.max(value_count);
            skip.start_doc(doc);
            for _ in 0..value_count {
                let v = values.next_value()?;
                min_value = min_value.min(v);
                max_value = max_value.max(v);
                skip.value(v);
            }
            skip.end_doc();
            doc = values.next_doc()?;
        }
        let skip_intervals = skip.finish();

        // write absolute min and max for skipper
        self.line(MINVALUE, &min_value.to_string())?;
        self.line(MAXVALUE, &max_value.to_string())?;

        let binary = || -> Result<Box<dyn BinaryDocValues>> {
            Ok(Box::new(SortedNumericAsBinary {
                values: sorted_numeric()?,
                binary_value: BytesRef::default(),
            }))
        };
        self.do_add_binary_field(&binary, max_value_count)?;

        self.write_skip_data(&skip_intervals)
    }

    /// Port of `SimpleTextDocValuesWriter.addSortedSetField`.
    fn add_sorted_set_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
    ) -> Result<()> {
        debug_assert!(self.field_seen(&field.name));
        debug_assert_eq!(field.get_doc_values_type(), DocValuesType::SORTED_SET);
        self.write_field_entry(field, DocValuesType::SORTED_SET)?;

        let sorted_set = || {
            values_producer
                .get_sorted_set(field)?
                .ok_or_else(|| missing("sorted set", field))
        };

        let mut doc_count = 0i32;
        let mut max_value_count = 0i32;
        let mut skip = SkipIntervals::new();
        let mut values = sorted_set()?;
        let mut doc = values.next_doc()?;
        while doc != NO_MORE_DOCS {
            doc_count += 1;
            let value_count = values.doc_value_count()?;
            max_value_count = max_value_count.max(value_count);
            skip.start_doc(doc);
            for _ in 0..value_count {
                let ord = values.next_ord()?;
                skip.value(ord);
            }
            skip.end_doc();
            doc = values.next_doc()?;
        }
        let skip_intervals = skip.finish();

        self.line(DOCCOUNT, &doc_count.to_string())?;
        self.line(MAXVALUECOUNT, &max_value_count.to_string())?;

        let mut value_count = 0i64;
        let mut max_length = 0i32;
        let mut terms = sorted_set()?.terms_enum()?;
        while let Some(value) = terms.next()? {
            max_length = max_length.max(value.length as i32);
            value_count += 1;
        }

        // write numValues
        self.line(NUMVALUES, &value_count.to_string())?;

        // write maxLength
        self.line(MAXLENGTH, &max_length.to_string())?;

        let max_bytes_length = max_length.to_string().len();
        let pattern = "0".repeat(max_bytes_length);

        // write our pattern for encoding lengths
        self.line(PATTERN, &pattern)?;

        // compute ord pattern: this is funny, we encode all values for all
        // docs to find the maximum length
        let mut max_ord_list_length = 0usize;
        let mut sb2 = String::new();
        let mut values = sorted_set()?;
        let mut doc = values.next_doc()?;
        while doc != NO_MORE_DOCS {
            sb2.clear();
            let mut i = 0;
            while i < values.doc_value_count()? {
                if !sb2.is_empty() {
                    sb2.push(',');
                }
                sb2.push_str(&values.next_ord()?.to_string());
                i += 1;
            }
            max_ord_list_length = max_ord_list_length.max(sb2.len());
            doc = values.next_doc()?;
        }

        let ord_pattern = "X".repeat(max_ord_list_length);

        // write our pattern for ord lists
        self.line(ORDPATTERN, &ord_pattern)?;

        // for asserts:
        let values_seen =
            self.write_sorted_values(sorted_set()?.terms_enum()?, max_bytes_length, max_length)?;

        debug_assert_eq!(values_seen, value_count);

        let mut values = sorted_set()?;

        // write the ords for each doc comma-separated
        for i in 0..self.num_docs {
            if values.doc_id() < i {
                values.next_doc()?;
                debug_assert!(values.doc_id() >= i);
            }
            sb2.clear();
            if values.doc_id() == i {
                let mut j = 0;
                while j < values.doc_value_count()? {
                    if !sb2.is_empty() {
                        sb2.push(',');
                    }
                    sb2.push_str(&values.next_ord()?.to_string());
                    j += 1;
                }
            }
            // now pad to fit: these are numbers so spaces work well. reader
            // calls trim()
            let num_padding = max_ord_list_length.saturating_sub(sb2.len());
            for _ in 0..num_padding {
                sb2.push(' ');
            }
            self.write_str(&sb2)?;
            self.newline()?;
        }

        self.write_skip_data(&skip_intervals)
    }

    /// Port of `SimpleTextDocValuesWriter.close`.
    fn close(&mut self) -> Result<()> {
        if self.data.is_some() {
            debug_assert!(!self.fields_seen.is_empty());
            // TODO: sheisty to do this here?
            let result = (|| {
                self.write_bytes(END)?;
                self.newline()?;
                util::write_checksum(self.data()?)
            })();
            let mut data = self.data.take().expect("INVARIANT: checked above");
            let closed = data.close();
            result?;
            closed?;
        }
        Ok(())
    }
}

/// The anonymous `BinaryDocValues` of `addSortedNumericField`: each
/// document's values, comma-separated with `Long.toString`.
struct SortedNumericAsBinary {
    values: Box<dyn SortedNumericDocValues>,
    binary_value: BytesRef,
}

impl SortedNumericAsBinary {
    fn set_current_doc(&mut self) -> Result<()> {
        if self.values.doc_id() == NO_MORE_DOCS {
            return Ok(());
        }
        let mut builder = String::new();
        let count = self.values.doc_value_count()?;
        for i in 0..count {
            if i > 0 {
                builder.push(',');
            }
            builder.push_str(&self.values.next_value()?.to_string());
        }
        self.binary_value = BytesRef::new(builder.into_bytes());
        Ok(())
    }
}

impl DocIdSetIterator for SortedNumericAsBinary {
    fn doc_id(&self) -> i32 {
        self.values.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.values.next_doc()?;
        self.set_current_doc()?;
        Ok(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.values.advance(target)?;
        self.set_current_doc()?;
        Ok(doc)
    }

    fn cost(&self) -> i64 {
        self.values.cost()
    }
}

impl DocValuesIterator for SortedNumericAsBinary {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        if self.values.advance_exact(target)? {
            self.set_current_doc()?;
            return Ok(true);
        }
        Ok(false)
    }
}

impl BinaryDocValues for SortedNumericAsBinary {
    fn binary_value(&self) -> Result<BytesRef> {
        Ok(self.binary_value.clone())
    }
}
