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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextDocValuesReader`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rucene::codecs::{DocValuesProducer, SegmentReadState};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::{
    BinaryDocValues, DocValuesIterator, DocValuesSkipper, DocValuesType, FieldInfo,
    NumericDocValues, SortedDocValues, SortedNumericDocValues, SortedSetDocValues,
};
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::store::{BufferedChecksumIndexInput, IndexInput};
use rucene::util::java_fmt::java_utf8_to_string;
use rucene::util::BytesRef;

use super::doc_values_writer::{
    SkipInterval, DOCCOUNT, END, FIELD, LENGTH, MAXLENGTH, MAXVALUE, MAXVALUECOUNT, MINVALUE,
    NUMVALUES, ORDPATTERN, ORIGIN, PATTERN, SKIP_DATA, SKIP_DOC_COUNT, SKIP_END, SKIP_INTERVAL,
    SKIP_MAX_DOC_ID, SKIP_MAX_VALUE, SKIP_MIN_DOC_ID, SKIP_MIN_VALUE, TYPE,
};
use super::field_infos_format::SimpleTextFieldInfosFormat;
use super::util::{self, java_split, parse_int, parse_long, read_line, CHECKSUM};

/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextDocValuesReader.OneField`.
#[derive(Debug, Clone, Default)]
pub(crate) struct OneField {
    doc_count: i32,
    data_start_file_pointer: i64,
    pattern: String,
    ord_pattern: String,
    max_length: i32,
    origin: i64,
    min_value: i64,
    max_value: i64,
    max_value_count: i32,
    num_values: i64,
    skip_intervals: Arc<Vec<SkipInterval>>,
}

/// The reader's `data`: every iterator clones it, and `close` closes it.
struct SharedData {
    data: Mutex<Option<Box<dyn IndexInput>>>,
}

impl SharedData {
    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        let guard = self.data.lock().map_err(|_| {
            LuceneError::illegal_state("doc values input lock was poisoned".to_string())
        })?;
        match guard.as_ref() {
            Some(data) => data.clone_input(),
            None => Err(LuceneError::already_closed("this IndexInput is closed")),
        }
    }
}

/// Reads plain-text doc values.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextDocValuesReader`.
///
/// `getMergeInstance()` returns the reader itself in Java; the port's
/// returns a handle that shares all of this reader's state.
#[derive(Clone)]
pub struct SimpleTextDocValuesReader {
    max_doc: i32,
    data: Arc<SharedData>,
    fields: Arc<HashMap<String, OneField>>,
}

impl std::fmt::Debug for SimpleTextDocValuesReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

impl std::fmt::Display for SimpleTextDocValuesReader {
    /// `getClass().getSimpleName() + "(fields=" + fields.size() + ")"`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SimpleTextDocValuesReader(fields={})", self.fields.len())
    }
}

/// `new String(scratch.bytes(), prefix.length, ..., UTF_8)`.
fn strip_prefix(scratch: &[u8], prefix: &[u8]) -> String {
    util::read_string(scratch, prefix.len())
}

/// `DecimalFormat.parse(String)` with a pattern of zeros (or `X`s): the
/// longest number at the start of the text, an optional leading `-`, and the
/// `ParseException` Java throws when there is none.
fn decimal_parse(s: &str) -> Option<i128> {
    let bytes = s.as_bytes();
    let (negative, digits) = match bytes.first() {
        Some(b'-') => (true, &bytes[1..]),
        _ => (false, bytes),
    };
    let count = digits.iter().take_while(|b| b.is_ascii_digit()).count();
    if count == 0 {
        return None;
    }
    let mut value: i128 = 0;
    for &d in &digits[..count] {
        value = value
            .saturating_mul(10)
            .saturating_add(i128::from(d - b'0'));
    }
    Some(if negative { -value } else { value })
}

impl SimpleTextDocValuesReader {
    /// Port of `SimpleTextDocValuesReader(SegmentReadState, String)`.
    ///
    /// # Errors
    ///
    /// Propagates the directory's and the parsers' errors.
    pub fn new(state: &SegmentReadState<'_>, ext: &str) -> Result<Self> {
        let mut data = state.directory.open_input(
            &segment_file_name(&state.segment_info.name, &state.segment_suffix, ext),
            state.context,
        )?;
        let max_doc = state.segment_info.max_doc()?;
        let mut fields: HashMap<String, OneField> = HashMap::new();
        let mut scratch = Vec::new();
        let enums = SimpleTextFieldInfosFormat::new();
        loop {
            read_line(data.as_mut(), &mut scratch)?;
            if scratch.as_slice() == END {
                break;
            }
            debug_assert!(
                scratch.starts_with(FIELD),
                "{}",
                java_utf8_to_string(&scratch)
            );
            let field_name = strip_prefix(&scratch, FIELD);

            let mut field = OneField::default();

            read_line(data.as_mut(), &mut scratch)?;
            debug_assert!(
                scratch.starts_with(TYPE),
                "{}",
                java_utf8_to_string(&scratch)
            );

            let dv_type = enums.doc_values_type(&strip_prefix(&scratch, TYPE))?;
            debug_assert_ne!(dv_type, DocValuesType::NONE);

            if dv_type == DocValuesType::NUMERIC || dv_type == DocValuesType::SORTED_NUMERIC {
                read_line(data.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(MINVALUE));
                field.min_value = parse_long(&strip_prefix(&scratch, MINVALUE))?;
                read_line(data.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(MAXVALUE));
                field.max_value = parse_long(&strip_prefix(&scratch, MAXVALUE))?;
            }

            read_line(data.as_mut(), &mut scratch)?;
            debug_assert!(scratch.starts_with(DOCCOUNT));
            field.doc_count = parse_int(&strip_prefix(&scratch, DOCCOUNT))?;

            read_line(data.as_mut(), &mut scratch)?;
            if scratch.starts_with(MAXVALUECOUNT) {
                field.max_value_count = parse_int(&strip_prefix(&scratch, MAXVALUECOUNT))?;
                read_line(data.as_mut(), &mut scratch)?;
            } else if field.doc_count == 0 {
                field.max_value_count = 0;
            } else if dv_type == DocValuesType::NUMERIC || dv_type == DocValuesType::SORTED {
                field.max_value_count = 1;
            } else {
                field.max_value_count = -1;
            }

            if dv_type == DocValuesType::NUMERIC {
                debug_assert!(scratch.starts_with(ORIGIN));
                field.origin = parse_long(&strip_prefix(&scratch, ORIGIN))?;
                read_line(data.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(PATTERN));
                field.pattern = strip_prefix(&scratch, PATTERN);
                field.data_start_file_pointer = data.file_pointer();
                let fp = data.file_pointer()
                    + (1 + field.pattern.encode_utf16().count() as i64 + 2) * i64::from(max_doc);
                data.seek(fp)?;
                field.skip_intervals = Arc::new(read_skip_data(data.as_mut(), &mut scratch)?);
            } else if dv_type == DocValuesType::BINARY || dv_type == DocValuesType::SORTED_NUMERIC {
                debug_assert!(scratch.starts_with(MAXLENGTH));
                field.max_length = parse_int(&strip_prefix(&scratch, MAXLENGTH))?;
                read_line(data.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(PATTERN));
                field.pattern = strip_prefix(&scratch, PATTERN);
                field.data_start_file_pointer = data.file_pointer();
                let fp = data.file_pointer()
                    + (9 + field.pattern.encode_utf16().count() as i64
                        + i64::from(field.max_length)
                        + 2)
                        * i64::from(max_doc);
                data.seek(fp)?;
                if dv_type == DocValuesType::SORTED_NUMERIC {
                    field.skip_intervals = Arc::new(read_skip_data(data.as_mut(), &mut scratch)?);
                }
            } else if dv_type == DocValuesType::SORTED || dv_type == DocValuesType::SORTED_SET {
                debug_assert!(scratch.starts_with(NUMVALUES));
                field.num_values = parse_long(&strip_prefix(&scratch, NUMVALUES))?;
                read_line(data.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(MAXLENGTH));
                field.max_length = parse_int(&strip_prefix(&scratch, MAXLENGTH))?;
                read_line(data.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(PATTERN));
                field.pattern = strip_prefix(&scratch, PATTERN);
                read_line(data.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(ORDPATTERN));
                field.ord_pattern = strip_prefix(&scratch, ORDPATTERN);
                field.data_start_file_pointer = data.file_pointer();
                let fp = data.file_pointer()
                    + (9 + field.pattern.encode_utf16().count() as i64
                        + i64::from(field.max_length))
                        * field.num_values
                    + (1 + field.ord_pattern.encode_utf16().count() as i64) * i64::from(max_doc);
                data.seek(fp)?;
                field.skip_intervals = Arc::new(read_skip_data(data.as_mut(), &mut scratch)?);
            } else {
                return Err(LuceneError::from(LuceneErrorKind::AssertionError(
                    String::new(),
                )));
            }
            fields.insert(field_name, field);
        }

        // We should only be called from above if at least one field has DVs:
        debug_assert!(!fields.is_empty());
        Ok(Self {
            max_doc,
            data: Arc::new(SharedData {
                data: Mutex::new(Some(data)),
            }),
            fields: Arc::new(fields),
        })
    }

    fn field(&self, field_info: &FieldInfo) -> Result<&OneField> {
        // SegmentCoreReaders already verifies this field is valid:
        self.fields.get(&field_info.name).ok_or_else(|| {
            LuceneError::from(LuceneErrorKind::AssertionError(format!(
                "field={} fields={}",
                field_info.name,
                self.fields.len()
            )))
        })
    }

    /// Port of `SimpleTextDocValuesReader.getNumericNonIterator`.
    fn get_numeric_non_iterator(&self, field_info: &FieldInfo) -> Result<NumericValues> {
        let field = self.field(field_info)?.clone();
        Ok(NumericValues {
            input: RefCell::new(self.data.clone_input()?),
            field,
            max_doc: self.max_doc,
        })
    }

    /// Port of the private `getNumericDocsWithField`.
    fn get_numeric_docs_with_field(&self, field_info: &FieldInfo) -> Result<DocsWithField> {
        let field = self.field(field_info)?.clone();
        Ok(DocsWithField {
            input: self.data.clone_input()?,
            scratch: Vec::new(),
            stride: 1 + field.pattern.encode_utf16().count() as i64 + 2,
            binary: false,
            field,
            max_doc: self.max_doc,
            doc: -1,
        })
    }

    /// Port of the private `getBinaryDocsWithField`.
    fn get_binary_docs_with_field(&self, field_info: &FieldInfo) -> Result<DocsWithField> {
        let field = self.field(field_info)?.clone();
        Ok(DocsWithField {
            input: self.data.clone_input()?,
            scratch: Vec::new(),
            stride: 9
                + field.pattern.encode_utf16().count() as i64
                + i64::from(field.max_length)
                + 2,
            binary: true,
            field,
            max_doc: self.max_doc,
            doc: -1,
        })
    }

    /// Port of `getBinary`, typed.
    fn get_binary_impl(&self, field_info: &FieldInfo) -> Result<TextBinaryDocValues> {
        let field = self.field(field_info)?.clone();
        let docs_with_field = self.get_binary_docs_with_field(field_info)?;
        Ok(TextBinaryDocValues {
            values: BinaryValues {
                input: RefCell::new(self.data.clone_input()?),
                field,
                max_doc: self.max_doc,
            },
            docs_with_field,
        })
    }
}

/// Port of the private `readSkipData`.
fn read_skip_data(data: &mut dyn IndexInput, scratch: &mut Vec<u8>) -> Result<Vec<SkipInterval>> {
    read_line(data, scratch)?;
    debug_assert!(
        scratch.starts_with(SKIP_DATA),
        "got {}",
        java_utf8_to_string(scratch)
    );
    let mut intervals = Vec::new();
    loop {
        read_line(data, scratch)?;
        if scratch.starts_with(SKIP_END) {
            break;
        }
        debug_assert!(
            scratch.starts_with(SKIP_INTERVAL),
            "got {}",
            java_utf8_to_string(scratch)
        );
        read_line(data, scratch)?;
        debug_assert!(scratch.starts_with(SKIP_MIN_DOC_ID));
        let min_doc_id = parse_int(&strip_prefix(scratch, SKIP_MIN_DOC_ID))?;
        read_line(data, scratch)?;
        debug_assert!(scratch.starts_with(SKIP_MAX_DOC_ID));
        let max_doc_id = parse_int(&strip_prefix(scratch, SKIP_MAX_DOC_ID))?;
        read_line(data, scratch)?;
        debug_assert!(scratch.starts_with(SKIP_MIN_VALUE));
        let min_value = parse_long(&strip_prefix(scratch, SKIP_MIN_VALUE))?;
        read_line(data, scratch)?;
        debug_assert!(scratch.starts_with(SKIP_MAX_VALUE));
        let max_value = parse_long(&strip_prefix(scratch, SKIP_MAX_VALUE))?;
        read_line(data, scratch)?;
        debug_assert!(scratch.starts_with(SKIP_DOC_COUNT));
        let doc_count = parse_int(&strip_prefix(scratch, SKIP_DOC_COUNT))?;
        intervals.push(SkipInterval {
            min_doc_id,
            max_doc_id,
            min_value,
            max_value,
            doc_count,
        });
    }
    Ok(intervals)
}

/// The `IndexOutOfBoundsException` the value functions throw.
fn doc_id_out_of_bounds(max_doc: i32, doc_id: i32) -> LuceneError {
    LuceneError::unchecked(
        INDEX_OUT_OF_BOUNDS,
        format!("docID must be 0 .. {}; got {doc_id}", max_doc - 1),
    )
}

/// The `IndexOutOfBoundsException` `lookupOrd` throws.
fn ord_out_of_bounds(num_values: i64, ord: i64) -> LuceneError {
    LuceneError::unchecked(
        INDEX_OUT_OF_BOUNDS,
        format!("ord must be 0 .. {}; got {ord}", num_values - 1),
    )
}

/// The `ArrayIndexOutOfBoundsException` of `values[index++]` past the end of
/// a document's values.
fn array_index_out_of_bounds(index: usize, length: usize) -> LuceneError {
    LuceneError::unchecked(
        ARRAY_INDEX_OUT_OF_BOUNDS,
        format!("Index {index} out of bounds for length {length}"),
    )
}

const INDEX_OUT_OF_BOUNDS: &str = "java.lang.IndexOutOfBoundsException";
const ARRAY_INDEX_OUT_OF_BOUNDS: &str = "java.lang.ArrayIndexOutOfBoundsException";

/// The `catch (IOException ioe) { throw new RuntimeException(ioe); }` around
/// the body of the value functions: `IntFunction.apply` cannot throw a
/// checked exception (`SimpleTextDocValuesReader.java:310-314`, `:420-423`).
fn wrap_io_exception(error: LuceneError) -> LuceneError {
    if error.is_io_exception() {
        LuceneError::runtime_exception(error)
    } else {
        error
    }
}

/// The `IntFunction<Long>` of `getNumericNonIterator`.
struct NumericValues {
    input: RefCell<Box<dyn IndexInput>>,
    field: OneField,
    max_doc: i32,
}

impl NumericValues {
    fn apply(&self, doc_id: i32) -> Result<i64> {
        if doc_id < 0 || doc_id >= self.max_doc {
            return Err(doc_id_out_of_bounds(self.max_doc, doc_id));
        }
        self.read(doc_id).map_err(wrap_io_exception)
    }

    /// The part of `apply` inside its `try`, after the bounds check.
    fn read(&self, doc_id: i32) -> Result<i64> {
        let mut input = self.input.borrow_mut();
        let mut scratch = Vec::new();
        input.seek(
            self.field.data_start_file_pointer
                + (1 + self.field.pattern.encode_utf16().count() as i64 + 2) * i64::from(doc_id),
        )?;
        read_line(input.as_mut(), &mut scratch)?;
        let bd = decimal_parse(&java_utf8_to_string(&scratch)).ok_or_else(|| {
            LuceneError::corrupt_index(
                "failed to parse BigDecimal value",
                input.resource_description(),
            )
        })?;
        read_line(input.as_mut(), &mut scratch)?; // read the line telling us if it's real or not
                                                  // `BigInteger.valueOf(origin).add(bd.toBigIntegerExact()).longValue()`.
        Ok((i128::from(self.field.origin) + bd) as i64)
    }
}

/// The `IntFunction<BytesRef>` of `getBinary`.
struct BinaryValues {
    input: RefCell<Box<dyn IndexInput>>,
    field: OneField,
    max_doc: i32,
}

impl BinaryValues {
    fn apply(&self, doc_id: i32) -> Result<BytesRef> {
        if doc_id < 0 || doc_id >= self.max_doc {
            return Err(doc_id_out_of_bounds(self.max_doc, doc_id));
        }
        self.read(doc_id).map_err(wrap_io_exception)
    }

    /// The part of `apply` inside its `try`, after the bounds check.
    fn read(&self, doc_id: i32) -> Result<BytesRef> {
        let mut input = self.input.borrow_mut();
        let mut scratch = Vec::new();
        input.seek(
            self.field.data_start_file_pointer
                + (9 + self.field.pattern.encode_utf16().count() as i64
                    + i64::from(self.field.max_length)
                    + 2)
                    * i64::from(doc_id),
        )?;
        read_line(input.as_mut(), &mut scratch)?;
        debug_assert!(scratch.starts_with(LENGTH));
        let len = parse_length(&scratch, input.resource_description())?;
        let mut term_byte_array = vec![0u8; len];
        input.read_bytes(Some(&mut term_byte_array), 0, len)?;
        util::from_bytes_ref_string(&java_utf8_to_string(&term_byte_array))
    }
}

/// `decoder.parse(new String(scratch, LENGTH.length, ...)).intValue()`.
fn parse_length(scratch: &[u8], resource: &str) -> Result<usize> {
    let value = decimal_parse(&strip_prefix(scratch, LENGTH))
        .ok_or_else(|| LuceneError::corrupt_index("failed to parse int length", resource))?;
    let len = value as i32;
    usize::try_from(len).map_err(|_| LuceneError::illegal_state(format!("negative length {len}")))
}

/// The `DocValuesIterator`s of `getNumericDocsWithField` and
/// `getBinaryDocsWithField`.
struct DocsWithField {
    input: Box<dyn IndexInput>,
    scratch: Vec<u8>,
    field: OneField,
    /// The bytes one document takes.
    stride: i64,
    /// Whether this is the binary flavour, whose documents start with a
    /// length line and hold that many bytes.
    binary: bool,
    max_doc: i32,
    doc: i32,
}

impl DocsWithField {
    /// Reads document `i`'s entry and returns whether it has a value.
    fn read_entry(&mut self, i: i32) -> Result<bool> {
        self.input
            .seek(self.field.data_start_file_pointer + self.stride * i64::from(i))?;
        read_line(self.input.as_mut(), &mut self.scratch)?; // data
        if self.binary {
            debug_assert!(self.scratch.starts_with(LENGTH));
            let len = parse_length(&self.scratch, self.input.resource_description())?;
            // skip past bytes
            let mut bytes = vec![0u8; len];
            self.input.read_bytes(Some(&mut bytes), 0, len)?;
            read_line(self.input.as_mut(), &mut self.scratch)?; // newline
        }
        read_line(self.input.as_mut(), &mut self.scratch)?; // 'T' or 'F'
        Ok(self.scratch.first() == Some(&b'T'))
    }
}

impl DocIdSetIterator for DocsWithField {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        for i in target..self.max_doc {
            if self.read_entry(i)? {
                self.doc = i;
                return Ok(i);
            }
        }
        self.doc = NO_MORE_DOCS;
        Ok(NO_MORE_DOCS)
    }

    fn cost(&self) -> i64 {
        i64::from(self.max_doc)
    }
}

impl DocValuesIterator for DocsWithField {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc = target;
        self.read_entry(target)
    }
}

/// The anonymous `NumericDocValues` of `getNumeric`.
struct TextNumericDocValues {
    values: NumericValues,
    docs_with_field: DocsWithField,
}

impl DocIdSetIterator for TextNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.docs_with_field.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.docs_with_field.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.docs_with_field.advance(target)
    }

    fn cost(&self) -> i64 {
        self.docs_with_field.cost()
    }
}

impl DocValuesIterator for TextNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.docs_with_field.advance_exact(target)
    }
}

impl NumericDocValues for TextNumericDocValues {
    fn long_value(&self) -> Result<i64> {
        self.values.apply(self.docs_with_field.doc_id())
    }
}

/// The anonymous `BinaryDocValues` of `getBinary`.
struct TextBinaryDocValues {
    values: BinaryValues,
    docs_with_field: DocsWithField,
}

impl DocIdSetIterator for TextBinaryDocValues {
    fn doc_id(&self) -> i32 {
        self.docs_with_field.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.docs_with_field.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.docs_with_field.advance(target)
    }

    fn cost(&self) -> i64 {
        self.docs_with_field.cost()
    }
}

impl DocValuesIterator for TextBinaryDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.docs_with_field.advance_exact(target)
    }
}

impl BinaryDocValues for TextBinaryDocValues {
    fn binary_value(&self) -> Result<BytesRef> {
        self.values.apply(self.docs_with_field.doc_id())
    }
}

/// The anonymous `SortedDocValues` of `getSorted`.
struct TextSortedDocValues {
    input: RefCell<Box<dyn IndexInput>>,
    field: OneField,
    max_doc: i32,
    doc: i32,
    ord: i32,
}

impl TextSortedDocValues {
    fn value_stride(&self) -> i64 {
        9 + self.field.pattern.encode_utf16().count() as i64 + i64::from(self.field.max_length)
    }

    fn read_ord(&mut self, i: i32) -> Result<i32> {
        let pos = self.field.data_start_file_pointer
            + self.field.num_values * self.value_stride()
            + i64::from(i) * (1 + self.field.ord_pattern.encode_utf16().count() as i64);
        let input = self.input.get_mut();
        input.seek(pos)?;
        let mut scratch = Vec::new();
        read_line(input.as_mut(), &mut scratch)?;
        let parsed = decimal_parse(&java_utf8_to_string(&scratch)).ok_or_else(|| {
            LuceneError::corrupt_index("failed to parse ord", input.resource_description())
        })?;
        // `(int) ordDecoder.parse(...).longValue() - 1`.
        Ok((parsed as i64 as i32).wrapping_sub(1))
    }
}

impl DocIdSetIterator for TextSortedDocValues {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        for i in target..self.max_doc {
            self.ord = self.read_ord(i)?;
            if self.ord >= 0 {
                self.doc = i;
                return Ok(i);
            }
        }
        self.doc = NO_MORE_DOCS;
        Ok(NO_MORE_DOCS)
    }

    fn cost(&self) -> i64 {
        i64::from(self.max_doc)
    }
}

impl DocValuesIterator for TextSortedDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc = target;
        self.ord = self.read_ord(target)?;
        Ok(self.ord >= 0)
    }
}

impl SortedDocValues for TextSortedDocValues {
    fn ord_value(&self) -> Result<i32> {
        Ok(self.ord)
    }

    fn get_value_count(&self) -> Result<i32> {
        Ok(self.field.num_values as i32)
    }

    fn lookup_ord(&self, ord: i32) -> Result<BytesRef> {
        if ord < 0 || i64::from(ord) >= self.field.num_values {
            return Err(ord_out_of_bounds(self.field.num_values, i64::from(ord)));
        }
        let mut input = self.input.borrow_mut();
        input.seek(self.field.data_start_file_pointer + i64::from(ord) * self.value_stride())?;
        let mut scratch = Vec::new();
        read_line(input.as_mut(), &mut scratch)?;
        debug_assert!(
            scratch.starts_with(LENGTH),
            "got {} in={}",
            java_utf8_to_string(&scratch),
            input.resource_description()
        );
        let len = parse_length(&scratch, input.resource_description())?;
        let mut term = vec![0u8; len];
        input.read_bytes(Some(&mut term), 0, len)?;
        Ok(BytesRef::new(term))
    }
}

/// The anonymous `SortedNumericDocValues` of `getSortedNumeric`.
struct TextSortedNumericDocValues {
    binary: TextBinaryDocValues,
    values: Vec<i64>,
    index: usize,
}

impl TextSortedNumericDocValues {
    fn set_current_doc(&mut self) -> Result<()> {
        if self.binary.doc_id() == NO_MORE_DOCS {
            return Ok(());
        }
        let csv = java_utf8_to_string(self.binary.binary_value()?.slice());
        if csv.is_empty() {
            self.values = Vec::new();
        } else {
            let s = java_split(&csv, ',');
            self.values = s
                .iter()
                .map(|v| parse_long(v))
                .collect::<Result<Vec<_>>>()?;
        }
        self.index = 0;
        Ok(())
    }
}

impl DocIdSetIterator for TextSortedNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.binary.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.binary.next_doc()?;
        self.set_current_doc()?;
        Ok(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.binary.advance(target)?;
        self.set_current_doc()?;
        Ok(doc)
    }

    fn cost(&self) -> i64 {
        self.binary.cost()
    }
}

impl DocValuesIterator for TextSortedNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        if self.binary.advance_exact(target)? {
            self.set_current_doc()?;
            return Ok(true);
        }
        Ok(false)
    }
}

impl SortedNumericDocValues for TextSortedNumericDocValues {
    fn next_value(&mut self) -> Result<i64> {
        // `values[index++]`: the increment happens before the access.
        let index = self.index;
        self.index += 1;
        self.values
            .get(index)
            .copied()
            .ok_or_else(|| array_index_out_of_bounds(index, self.values.len()))
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(self.values.len() as i32)
    }
}

/// The anonymous `SortedSetDocValues` of `getSortedSet`.
struct TextSortedSetDocValues {
    input: RefCell<Box<dyn IndexInput>>,
    field: OneField,
    max_doc: i32,
    current_ords: Vec<String>,
    current_index: usize,
    doc: i32,
}

impl TextSortedSetDocValues {
    fn value_stride(&self) -> i64 {
        9 + self.field.pattern.encode_utf16().count() as i64 + i64::from(self.field.max_length)
    }

    /// Reads document `i`'s ord list, trimmed.
    fn read_ord_list(&mut self, i: i32) -> Result<String> {
        let pos = self.field.data_start_file_pointer
            + self.field.num_values * self.value_stride()
            + i64::from(i) * (1 + self.field.ord_pattern.encode_utf16().count() as i64);
        let input = self.input.get_mut();
        input.seek(pos)?;
        let mut scratch = Vec::new();
        read_line(input.as_mut(), &mut scratch)?;
        // `String.trim()`.
        Ok(java_utf8_to_string(&scratch)
            .trim_matches(|c: char| c <= '\u{20}')
            .to_string())
    }
}

impl DocIdSetIterator for TextSortedSetDocValues {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        for i in target..self.max_doc {
            let ord_list = self.read_ord_list(i)?;
            if !ord_list.is_empty() {
                self.current_ords = java_split(&ord_list, ',')
                    .into_iter()
                    .map(str::to_string)
                    .collect();
                self.current_index = 0;
                self.doc = i;
                return Ok(i);
            }
        }
        self.doc = NO_MORE_DOCS;
        Ok(NO_MORE_DOCS)
    }

    fn cost(&self) -> i64 {
        i64::from(self.max_doc)
    }
}

impl DocValuesIterator for TextSortedSetDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        let ord_list = self.read_ord_list(target)?;
        self.doc = target;
        if !ord_list.is_empty() {
            self.current_ords = java_split(&ord_list, ',')
                .into_iter()
                .map(str::to_string)
                .collect();
            self.current_index = 0;
            return Ok(true);
        }
        Ok(false)
    }
}

impl SortedSetDocValues for TextSortedSetDocValues {
    fn next_ord(&mut self) -> Result<i64> {
        // `Long.parseLong(currentOrds[currentIndex++])`: the increment
        // happens before the access and the parse.
        let index = self.current_index;
        self.current_index += 1;
        let ord = self
            .current_ords
            .get(index)
            .ok_or_else(|| array_index_out_of_bounds(index, self.current_ords.len()))?;
        parse_long(ord)
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(self.current_ords.len() as i32)
    }

    fn lookup_ord(&self, ord: i64) -> Result<BytesRef> {
        if ord < 0 || ord >= self.field.num_values {
            return Err(ord_out_of_bounds(self.field.num_values, ord));
        }
        let mut input = self.input.borrow_mut();
        input.seek(self.field.data_start_file_pointer + ord * self.value_stride())?;
        let mut scratch = Vec::new();
        read_line(input.as_mut(), &mut scratch)?;
        debug_assert!(
            scratch.starts_with(LENGTH),
            "got {} in={}",
            java_utf8_to_string(&scratch),
            input.resource_description()
        );
        let len = parse_length(&scratch, input.resource_description())?;
        let mut term = vec![0u8; len];
        input.read_bytes(Some(&mut term), 0, len)?;
        Ok(BytesRef::new(term))
    }

    fn get_value_count(&self) -> Result<i64> {
        Ok(self.field.num_values)
    }
}

/// The anonymous `DocValuesSkipper` of `getSkipper`.
struct TextDocValuesSkipper {
    numeric: bool,
    field: OneField,
    cursor: i32,
}

impl TextDocValuesSkipper {
    fn interval(&self) -> Option<&SkipInterval> {
        if self.cursor < 0 {
            None
        } else {
            self.field.skip_intervals.get(self.cursor as usize)
        }
    }
}

impl DocValuesSkipper for TextDocValuesSkipper {
    fn advance(&mut self, target: i32) -> Result<()> {
        let intervals = &self.field.skip_intervals;
        if intervals.is_empty() {
            self.cursor = 0;
            return Ok(());
        }
        if self.cursor == -1 {
            self.cursor = 0;
        }
        while (self.cursor as usize) < intervals.len()
            && intervals[self.cursor as usize].max_doc_id < target
        {
            self.cursor += 1;
        }
        Ok(())
    }

    fn num_levels(&self) -> i32 {
        1
    }

    fn min_doc_id(&self, _level: i32) -> i32 {
        if self.cursor == -1 {
            -1
        } else {
            self.interval().map_or(NO_MORE_DOCS, |i| i.min_doc_id)
        }
    }

    fn max_doc_id(&self, _level: i32) -> i32 {
        if self.cursor == -1 {
            -1
        } else {
            self.interval().map_or(NO_MORE_DOCS, |i| i.max_doc_id)
        }
    }

    fn min_value(&self, _level: i32) -> i64 {
        self.interval()
            .map_or_else(|| self.global_min_value(), |i| i.min_value)
    }

    fn max_value(&self, _level: i32) -> i64 {
        self.interval()
            .map_or_else(|| self.global_max_value(), |i| i.max_value)
    }

    fn doc_count(&self, _level: i32) -> i32 {
        self.interval()
            .map_or_else(|| self.global_doc_count(), |i| i.doc_count)
    }

    fn global_min_value(&self) -> i64 {
        if self.numeric {
            self.field.min_value
        } else {
            0
        }
    }

    fn global_max_value(&self) -> i64 {
        if self.numeric {
            self.field.max_value
        } else {
            self.field.num_values - 1
        }
    }

    fn global_doc_count(&self) -> i32 {
        self.field.doc_count
    }

    fn max_value_count(&self) -> i32 {
        self.field.max_value_count
    }
}

impl DocValuesProducer for SimpleTextDocValuesReader {
    /// Port of `SimpleTextDocValuesReader.getNumeric`.
    fn get_numeric(&self, field_info: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        let values = self.get_numeric_non_iterator(field_info)?;
        let docs_with_field = self.get_numeric_docs_with_field(field_info)?;
        Ok(Some(Box::new(TextNumericDocValues {
            values,
            docs_with_field,
        })))
    }

    /// Port of the `synchronized` `SimpleTextDocValuesReader.getBinary`.
    fn get_binary(&self, field_info: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
        Ok(Some(Box::new(self.get_binary_impl(field_info)?)))
    }

    /// Port of `SimpleTextDocValuesReader.getSorted`.
    fn get_sorted(&self, field_info: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
        let field = self.field(field_info)?.clone();
        Ok(Some(Box::new(TextSortedDocValues {
            input: RefCell::new(self.data.clone_input()?),
            field,
            max_doc: self.max_doc,
            doc: -1,
            ord: 0,
        })))
    }

    /// Port of `SimpleTextDocValuesReader.getSortedNumeric`.
    fn get_sorted_numeric(
        &self,
        field_info: &FieldInfo,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        let binary = self.get_binary_impl(field_info)?;
        Ok(Some(Box::new(TextSortedNumericDocValues {
            binary,
            values: Vec::new(),
            index: 0,
        })))
    }

    /// Port of `SimpleTextDocValuesReader.getSortedSet`.
    fn get_sorted_set(
        &self,
        field_info: &FieldInfo,
    ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        let field = self.field(field_info)?.clone();
        Ok(Some(Box::new(TextSortedSetDocValues {
            input: RefCell::new(self.data.clone_input()?),
            field,
            max_doc: self.max_doc,
            current_ords: Vec::new(),
            current_index: 0,
            doc: -1,
        })))
    }

    /// Port of `SimpleTextDocValuesReader.getSkipper`.
    fn get_skipper(&self, field_info: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        let numeric = field_info.get_doc_values_type() == DocValuesType::NUMERIC
            || field_info.get_doc_values_type() == DocValuesType::SORTED_NUMERIC;
        let field = self.field(field_info)?.clone();
        Ok(Some(Box::new(TextDocValuesSkipper {
            numeric,
            field,
            cursor: -1,
        })))
    }

    /// Port of `SimpleTextDocValuesReader.checkIntegrity`.
    fn check_integrity(&self) -> Result<()> {
        let mut scratch = Vec::new();
        let mut clone = self.data.clone_input()?;
        clone.seek(0)?;
        // checksum is fixed-width encoded with 20 bytes, plus 1 byte for
        // newline (the space is included in SimpleTextUtil.CHECKSUM):
        let footer_start_pos = clone.length() - (CHECKSUM.len() as i64 + 21);
        let mut input = BufferedChecksumIndexInput::new(clone);
        loop {
            read_line(&mut input, &mut scratch)?;
            if input.file_pointer() >= footer_start_pos {
                // Make sure we landed at precisely the right location:
                if input.file_pointer() != footer_start_pos {
                    return Err(LuceneError::corrupt_index(
                        format!(
                            "SimpleText failure: footer does not start at expected position \
                             current={} vs expected={footer_start_pos}",
                            input.file_pointer()
                        ),
                        rucene::store::IndexInput::resource_description(&input),
                    ));
                }
                util::check_footer(&mut input)?;
                break;
            }
        }
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(self.clone()))
    }

    fn close(&self) -> Result<()> {
        let data = self
            .data
            .data
            .lock()
            .map_err(|_| {
                LuceneError::illegal_state("doc values input lock was poisoned".to_string())
            })?
            .take();
        match data {
            Some(mut data) => data.close(),
            None => Ok(()),
        }
    }
}
