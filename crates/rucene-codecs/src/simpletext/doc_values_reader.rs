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
use super::java_decimal_format::{DecimalFormat, ParsedNumber};
use super::util::{self, java_split, parse_int, parse_long, read_line, JavaScratch, CHECKSUM};
use rucene::util::extra::ParseException;

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

impl OneField {
    /// `pattern.length()`, in UTF-16 units.
    fn pattern_len(&self) -> i32 {
        self.pattern.encode_utf16().count() as i32
    }

    /// `(1 + pattern.length() + 2)`, an `int`: the bytes of one numeric
    /// document.
    fn numeric_stride(&self) -> i64 {
        i64::from(1i32.wrapping_add(self.pattern_len()).wrapping_add(2))
    }

    /// `(9 + pattern.length() + maxLength + 2)`, an `int`: the bytes of one
    /// binary document.
    fn binary_stride(&self) -> i64 {
        i64::from(
            9i32.wrapping_add(self.pattern_len())
                .wrapping_add(self.max_length)
                .wrapping_add(2),
        )
    }

    /// `(9 + pattern.length() + maxLength)`, an `int`: the bytes of one
    /// sorted value.
    fn value_stride(&self) -> i64 {
        i64::from(
            9i32.wrapping_add(self.pattern_len())
                .wrapping_add(self.max_length),
        )
    }

    /// `(1 + ordPattern.length())`, an `int`: the bytes of one document's
    /// ords.
    fn ord_stride(&self) -> i64 {
        i64::from(1i32.wrapping_add(self.ord_pattern.encode_utf16().count() as i32))
    }

    /// `dataStartFilePointer + numValues * valueStride + i * ordStride`, in
    /// Java's wrapping `long` arithmetic.
    fn ord_position(&self, i: i32) -> i64 {
        self.data_start_file_pointer
            .wrapping_add(self.num_values.wrapping_mul(self.value_stride()))
            .wrapping_add(i64::from(i).wrapping_mul(self.ord_stride()))
    }
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
fn strip_prefix(scratch: &JavaScratch, prefix: &[u8]) -> Result<String> {
    scratch.new_string(prefix.len())
}

/// `decoder.parse(text)`, a `ParseException` becoming the cause of a
/// `CorruptIndexException(message, in, pe)`; any other exception passes.
fn decimal_parse(
    decoder: &DecimalFormat,
    text: &str,
    message: &str,
    resource: &str,
) -> Result<ParsedNumber> {
    decoder.parse(text).map_err(|e| {
        if ParseException::of(&e).is_some() {
            LuceneError::corrupt_index_with_cause(message, resource, e)
        } else {
            e
        }
    })
}

/// `new DecimalFormat(pattern, new DecimalFormatSymbols(Locale.ROOT))`.
fn decoder(pattern: &str) -> Result<DecimalFormat> {
    DecimalFormat::new(pattern)
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
        let mut scratch = JavaScratch::new();
        let enums = SimpleTextFieldInfosFormat::new();
        loop {
            scratch.read_line(data.as_mut())?;
            if &*scratch == END {
                break;
            }
            debug_assert!(
                scratch.starts_with(FIELD),
                "{}",
                java_utf8_to_string(&scratch)
            );
            let field_name = strip_prefix(&scratch, FIELD)?;

            let mut field = OneField::default();

            scratch.read_line(data.as_mut())?;
            debug_assert!(
                scratch.starts_with(TYPE),
                "{}",
                java_utf8_to_string(&scratch)
            );

            let dv_type = enums.doc_values_type(&strip_prefix(&scratch, TYPE)?)?;
            debug_assert_ne!(dv_type, DocValuesType::NONE);

            if dv_type == DocValuesType::NUMERIC || dv_type == DocValuesType::SORTED_NUMERIC {
                scratch.read_line(data.as_mut())?;
                debug_assert!(scratch.starts_with(MINVALUE));
                field.min_value = parse_long(&strip_prefix(&scratch, MINVALUE)?)?;
                scratch.read_line(data.as_mut())?;
                debug_assert!(scratch.starts_with(MAXVALUE));
                field.max_value = parse_long(&strip_prefix(&scratch, MAXVALUE)?)?;
            }

            scratch.read_line(data.as_mut())?;
            debug_assert!(scratch.starts_with(DOCCOUNT));
            field.doc_count = parse_int(&strip_prefix(&scratch, DOCCOUNT)?)?;

            scratch.read_line(data.as_mut())?;
            if scratch.starts_with(MAXVALUECOUNT) {
                field.max_value_count = parse_int(&strip_prefix(&scratch, MAXVALUECOUNT)?)?;
                scratch.read_line(data.as_mut())?;
            } else if field.doc_count == 0 {
                field.max_value_count = 0;
            } else if dv_type == DocValuesType::NUMERIC || dv_type == DocValuesType::SORTED {
                field.max_value_count = 1;
            } else {
                field.max_value_count = -1;
            }

            if dv_type == DocValuesType::NUMERIC {
                debug_assert!(scratch.starts_with(ORIGIN));
                field.origin = parse_long(&strip_prefix(&scratch, ORIGIN)?)?;
                scratch.read_line(data.as_mut())?;
                debug_assert!(scratch.starts_with(PATTERN));
                field.pattern = strip_prefix(&scratch, PATTERN)?;
                field.data_start_file_pointer = data.file_pointer()?;
                let fp = data
                    .file_pointer()?
                    .wrapping_add(field.numeric_stride().wrapping_mul(i64::from(max_doc)));
                data.seek(fp)?;
                field.skip_intervals = Arc::new(read_skip_data(data.as_mut(), &mut scratch)?);
            } else if dv_type == DocValuesType::BINARY || dv_type == DocValuesType::SORTED_NUMERIC {
                debug_assert!(scratch.starts_with(MAXLENGTH));
                field.max_length = parse_int(&strip_prefix(&scratch, MAXLENGTH)?)?;
                scratch.read_line(data.as_mut())?;
                debug_assert!(scratch.starts_with(PATTERN));
                field.pattern = strip_prefix(&scratch, PATTERN)?;
                field.data_start_file_pointer = data.file_pointer()?;
                let fp = data
                    .file_pointer()?
                    .wrapping_add(field.binary_stride().wrapping_mul(i64::from(max_doc)));
                data.seek(fp)?;
                if dv_type == DocValuesType::SORTED_NUMERIC {
                    field.skip_intervals = Arc::new(read_skip_data(data.as_mut(), &mut scratch)?);
                }
            } else if dv_type == DocValuesType::SORTED || dv_type == DocValuesType::SORTED_SET {
                debug_assert!(scratch.starts_with(NUMVALUES));
                field.num_values = parse_long(&strip_prefix(&scratch, NUMVALUES)?)?;
                scratch.read_line(data.as_mut())?;
                debug_assert!(scratch.starts_with(MAXLENGTH));
                field.max_length = parse_int(&strip_prefix(&scratch, MAXLENGTH)?)?;
                scratch.read_line(data.as_mut())?;
                debug_assert!(scratch.starts_with(PATTERN));
                field.pattern = strip_prefix(&scratch, PATTERN)?;
                scratch.read_line(data.as_mut())?;
                debug_assert!(scratch.starts_with(ORDPATTERN));
                field.ord_pattern = strip_prefix(&scratch, ORDPATTERN)?;
                field.data_start_file_pointer = data.file_pointer()?;
                let fp = data
                    .file_pointer()?
                    .wrapping_add(field.value_stride().wrapping_mul(field.num_values))
                    .wrapping_add(field.ord_stride().wrapping_mul(i64::from(max_doc)));
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

    /// `fields.get(fieldInfo.name)` followed by `assert field != null;`
    /// (null message) and, without `-ea`, the `NullPointerException` of the
    /// first read of `field.<member>`.
    fn field(&self, field_info: &FieldInfo, member: &str) -> Result<&OneField> {
        // SegmentCoreReaders already verifies this field is valid:
        self.fields.get(&field_info.name).ok_or_else(|| {
            if cfg!(debug_assertions) {
                LuceneError::from(LuceneErrorKind::AssertionError(String::new()))
            } else {
                util::null_pointer(format!(
                    "Cannot read field \"{member}\" because \"field\" is null"
                ))
            }
        })
    }

    /// Port of `SimpleTextDocValuesReader.getNumericNonIterator`.
    fn get_numeric_non_iterator(&self, field_info: &FieldInfo) -> Result<NumericValues> {
        let field = self.field(field_info, "pattern")?.clone();
        let input = RefCell::new(self.data.clone_input()?);
        let mut decoder = decoder(&field.pattern)?;
        decoder.set_parse_big_decimal(true);
        Ok(NumericValues {
            input,
            decoder,
            field,
            max_doc: self.max_doc,
        })
    }

    /// Port of the private `getNumericDocsWithField`.
    fn get_numeric_docs_with_field(&self, field_info: &FieldInfo) -> Result<DocsWithField> {
        let field = self.field(field_info, "pattern")?.clone();
        Ok(DocsWithField {
            input: self.data.clone_input()?,
            scratch: JavaScratch::new(),
            stride: field.numeric_stride(),
            decoder: None,
            field,
            max_doc: self.max_doc,
            doc: -1,
        })
    }

    /// Port of the private `getBinaryDocsWithField`.
    fn get_binary_docs_with_field(&self, field_info: &FieldInfo) -> Result<DocsWithField> {
        let field = self.field(field_info, "pattern")?.clone();
        let input = self.data.clone_input()?;
        let decoder = decoder(&field.pattern)?;
        Ok(DocsWithField {
            input,
            scratch: JavaScratch::new(),
            stride: field.binary_stride(),
            decoder: Some(decoder),
            field,
            max_doc: self.max_doc,
            doc: -1,
        })
    }

    /// Port of `getBinary`, typed.
    fn get_binary_impl(&self, field_info: &FieldInfo) -> Result<TextBinaryDocValues> {
        let field = self.field(field_info, "pattern")?.clone();
        let input = RefCell::new(self.data.clone_input()?);
        let values_decoder = decoder(&field.pattern)?;
        let docs_with_field = self.get_binary_docs_with_field(field_info)?;
        Ok(TextBinaryDocValues {
            values: BinaryValues {
                input,
                scratch: RefCell::new(JavaScratch::new()),
                decoder: values_decoder,
                field,
                max_doc: self.max_doc,
            },
            docs_with_field,
        })
    }
}

/// Port of the private `readSkipData`.
fn read_skip_data(
    data: &mut dyn IndexInput,
    scratch: &mut JavaScratch,
) -> Result<Vec<SkipInterval>> {
    scratch.read_line(data)?;
    debug_assert!(
        scratch.starts_with(SKIP_DATA),
        "got {}",
        java_utf8_to_string(&scratch[..])
    );
    let mut intervals = Vec::new();
    loop {
        scratch.read_line(data)?;
        if scratch.starts_with(SKIP_END) {
            break;
        }
        debug_assert!(
            scratch.starts_with(SKIP_INTERVAL),
            "got {}",
            java_utf8_to_string(&scratch[..])
        );
        scratch.read_line(data)?;
        debug_assert!(scratch.starts_with(SKIP_MIN_DOC_ID));
        let min_doc_id = parse_int(&strip_prefix(scratch, SKIP_MIN_DOC_ID)?)?;
        scratch.read_line(data)?;
        debug_assert!(scratch.starts_with(SKIP_MAX_DOC_ID));
        let max_doc_id = parse_int(&strip_prefix(scratch, SKIP_MAX_DOC_ID)?)?;
        scratch.read_line(data)?;
        debug_assert!(scratch.starts_with(SKIP_MIN_VALUE));
        let min_value = parse_long(&strip_prefix(scratch, SKIP_MIN_VALUE)?)?;
        scratch.read_line(data)?;
        debug_assert!(scratch.starts_with(SKIP_MAX_VALUE));
        let max_value = parse_long(&strip_prefix(scratch, SKIP_MAX_VALUE)?)?;
        scratch.read_line(data)?;
        debug_assert!(scratch.starts_with(SKIP_DOC_COUNT));
        let doc_count = parse_int(&strip_prefix(scratch, SKIP_DOC_COUNT)?)?;
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
    decoder: DecimalFormat,
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
            self.field
                .data_start_file_pointer
                .wrapping_add(self.field.numeric_stride().wrapping_mul(i64::from(doc_id))),
        )?;
        read_line(input.as_mut(), &mut scratch)?;
        let bd = decimal_parse(
            &self.decoder,
            &java_utf8_to_string(&scratch),
            "failed to parse BigDecimal value",
            input.resource_description(),
        )?;
        read_line(input.as_mut(), &mut scratch)?; // read the line telling us if it's real or not
                                                  // `BigInteger.valueOf(origin).add(bd.toBigIntegerExact()).longValue()`.
        bd.add_big_integer_exact(self.field.origin)
    }
}

/// The `IntFunction<BytesRef>` of `getBinary`.
struct BinaryValues {
    input: RefCell<Box<dyn IndexInput>>,
    scratch: RefCell<JavaScratch>,
    decoder: DecimalFormat,
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
        let mut scratch = self.scratch.borrow_mut();
        input.seek(
            self.field
                .data_start_file_pointer
                .wrapping_add(self.field.binary_stride().wrapping_mul(i64::from(doc_id))),
        )?;
        scratch.read_line(input.as_mut())?;
        debug_assert!(scratch.starts_with(LENGTH));
        let len = grow_no_copy_length(parse_length(
            &self.decoder,
            &scratch,
            input.resource_description(),
        )?)?;
        let mut term_byte_array = vec![0u8; len];
        input.read_bytes(Some(&mut term_byte_array), 0, len)?;
        util::from_bytes_ref_string(&java_utf8_to_string(&term_byte_array))
    }
}

/// `decoder.parse(new String(scratch, LENGTH.length, ...)).intValue()`.
fn parse_length(decoder: &DecimalFormat, scratch: &JavaScratch, resource: &str) -> Result<i32> {
    decimal_parse(
        decoder,
        &strip_prefix(scratch, LENGTH)?,
        "failed to parse int length",
        resource,
    )?
    .int_value()
}

/// `term.growNoCopy(len); term.setLength(len); in.readBytes(..., len)` for
/// a parsed length: `ArrayUtil.growNoCopy` asserts it is not negative
/// (`ArrayUtil.java:458`), which a debug build reproduces.
///
/// **Divergence.** Without `-ea`, Java goes on with a negative length:
/// `setLength(len)` gives the builder a negative-length `BytesRef`, which
/// `getBinary` turns into a `NegativeArraySizeException` in `utf8ToString()`
/// (after a `readBytes(..., len)` whose outcome depends on the `IndexInput`)
/// and `lookupOrd` of `getSorted`/`getSortedSet` returns as is
/// (`SimpleTextDocValuesReader.java:416-419`, `:657-660`, `:844-847`). A Rust
/// `BytesRef` cannot have a negative length, so every one of these paths
/// raises `NegativeArraySizeException` with the length.
fn grow_no_copy_length(len: i32) -> Result<usize> {
    debug_assert!(len >= 0, "{}", util::negative_grow_message(i64::from(len)));
    usize::try_from(len).map_err(|_| util::negative_array_size(len))
}

/// The `DocValuesIterator`s of `getNumericDocsWithField` and
/// `getBinaryDocsWithField`.
struct DocsWithField {
    input: Box<dyn IndexInput>,
    scratch: JavaScratch,
    field: OneField,
    /// The bytes one document takes.
    stride: i64,
    /// The binary flavour's length decoder: its documents start with a
    /// length line and hold that many bytes.
    decoder: Option<DecimalFormat>,
    max_doc: i32,
    doc: i32,
}

impl DocsWithField {
    /// Reads document `i`'s entry and returns whether it has a value.
    fn read_entry(&mut self, i: i32) -> Result<bool> {
        self.input.seek(
            self.field
                .data_start_file_pointer
                .wrapping_add(self.stride.wrapping_mul(i64::from(i))),
        )?;
        self.scratch.read_line(self.input.as_mut())?; // data
        if let Some(decoder) = &self.decoder {
            debug_assert!(self.scratch.starts_with(LENGTH));
            let len = parse_length(decoder, &self.scratch, self.input.resource_description())?;
            // skip past bytes: `new byte[len]`
            let len = usize::try_from(len).map_err(|_| util::negative_array_size(len))?;
            let mut bytes = vec![0u8; len];
            self.input.read_bytes(Some(&mut bytes), 0, len)?;
            self.scratch.read_line(self.input.as_mut())?; // newline
        }
        self.scratch.read_line(self.input.as_mut())?; // 'T' or 'F'
                                                      // `scratch.byteAt(0) == (byte) 'T'`, which reads the array's first
                                                      // byte whatever the line's length.
        Ok(self.scratch.byte_at(0)? == b'T')
    }
}

impl DocIdSetIterator for DocsWithField {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        // `advance(docID() + 1)`, wrapping past `NO_MORE_DOCS` as Java does.
        self.advance(self.doc.wrapping_add(1))
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
    scratch: RefCell<JavaScratch>,
    decoder: DecimalFormat,
    ord_decoder: DecimalFormat,
    field: OneField,
    max_doc: i32,
    doc: i32,
    ord: i32,
}

impl TextSortedDocValues {
    fn read_ord(&mut self, i: i32) -> Result<i32> {
        let pos = self.field.ord_position(i);
        let input = self.input.get_mut();
        input.seek(pos)?;
        let scratch = self.scratch.get_mut();
        scratch.read_line(input.as_mut())?;
        let parsed = decimal_parse(
            &self.ord_decoder,
            &java_utf8_to_string(scratch),
            "failed to parse ord",
            input.resource_description(),
        )?;
        // `(int) ordDecoder.parse(...).longValue() - 1`.
        Ok((parsed.long_value()? as i32).wrapping_sub(1))
    }
}

impl DocIdSetIterator for TextSortedDocValues {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        // `advance(docID() + 1)`, wrapping past `NO_MORE_DOCS` as Java does.
        self.advance(self.doc.wrapping_add(1))
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
        // `ord * (long) (9 + pattern.length() + maxLength)`.
        input.seek(
            self.field
                .data_start_file_pointer
                .wrapping_add(i64::from(ord).wrapping_mul(self.field.value_stride())),
        )?;
        let mut scratch = self.scratch.borrow_mut();
        scratch.read_line(input.as_mut())?;
        debug_assert!(
            scratch.starts_with(LENGTH),
            "got {} in={}",
            java_utf8_to_string(&scratch),
            input.resource_description()
        );
        let len = grow_no_copy_length(parse_length(
            &self.decoder,
            &scratch,
            input.resource_description(),
        )?)?;
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
    scratch: RefCell<JavaScratch>,
    decoder: DecimalFormat,
    field: OneField,
    max_doc: i32,
    current_ords: Vec<String>,
    current_index: usize,
    doc: i32,
}

impl TextSortedSetDocValues {
    /// Reads document `i`'s ord list, trimmed.
    fn read_ord_list(&mut self, i: i32) -> Result<String> {
        let pos = self.field.ord_position(i);
        let input = self.input.get_mut();
        input.seek(pos)?;
        let scratch = self.scratch.get_mut();
        scratch.read_line(input.as_mut())?;
        // `String.trim()`.
        Ok(java_utf8_to_string(scratch)
            .trim_matches(|c: char| c <= '\u{20}')
            .to_string())
    }
}

impl DocIdSetIterator for TextSortedSetDocValues {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        // `advance(docID() + 1)`, wrapping past `NO_MORE_DOCS` as Java does.
        self.advance(self.doc.wrapping_add(1))
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
        // `ord * (9 + pattern.length() + maxLength)`.
        input.seek(
            self.field
                .data_start_file_pointer
                .wrapping_add(ord.wrapping_mul(self.field.value_stride())),
        )?;
        let mut scratch = self.scratch.borrow_mut();
        scratch.read_line(input.as_mut())?;
        debug_assert!(
            scratch.starts_with(LENGTH),
            "got {} in={}",
            java_utf8_to_string(&scratch),
            input.resource_description()
        );
        let len = grow_no_copy_length(parse_length(
            &self.decoder,
            &scratch,
            input.resource_description(),
        )?)?;
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
        let field = self.field(field_info, "pattern")?.clone();
        let input = RefCell::new(self.data.clone_input()?);
        let decoder = decoder(&field.pattern)?;
        let ord_decoder = self::decoder(&field.ord_pattern)?;
        Ok(Some(Box::new(TextSortedDocValues {
            input,
            scratch: RefCell::new(JavaScratch::new()),
            decoder,
            ord_decoder,
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
        let field = self.field(field_info, "pattern")?.clone();
        let input = RefCell::new(self.data.clone_input()?);
        let decoder = decoder(&field.pattern)?;
        Ok(Some(Box::new(TextSortedSetDocValues {
            input,
            scratch: RefCell::new(JavaScratch::new()),
            decoder,
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
        let field = self.field(field_info, "skipIntervals")?.clone();
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
            if input.file_pointer()? >= footer_start_pos {
                // Make sure we landed at precisely the right location:
                if input.file_pointer()? != footer_start_pos {
                    return Err(LuceneError::corrupt_index(
                        format!(
                            "SimpleText failure: footer does not start at expected position \
                             current={} vs expected={footer_start_pos}",
                            input.file_pointer()?
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simpletext::test_support::{directory, segment_info, unchecked, write_file};
    use rucene::index::FieldInfos;
    use rucene::store::DEFAULT_IO_CONTEXT;

    /// A `.dat` file with one numeric field `f` over one document whose
    /// value line is `value` (3 chars, pattern `000`).
    fn numeric_reader(value: &str) -> SimpleTextDocValuesReader {
        let dir = directory();
        write_file(
            dir.as_ref(),
            "_0.dat",
            &[
                "field f",
                "  type NUMERIC",
                "  minalue 0",
                "  maxvalue 9",
                "  doccount 1",
                "  maxvaluecount 1",
                "  origin 0",
                "  pattern 000",
                value,
                "T",
                "  skipdata",
                "  skipend",
                "END",
            ],
            true,
        );
        let si = segment_info(&dir, "_0", 1);
        let infos = FieldInfos::new(Vec::new()).unwrap();
        let state = SegmentReadState::new(dir.as_ref(), &si, &infos, &*DEFAULT_IO_CONTEXT);
        SimpleTextDocValuesReader::new(&state, "dat").unwrap()
    }

    /// Regression (rmp #526): numeric values go through a `BigDecimal`
    /// decoder and `toBigIntegerExact()`, so `1.5` is `ArithmeticException:
    /// Rounding necessary` (`SimpleTextDocValuesReader.java:303-311`); the
    /// port read the digits before the `.`.
    #[test]
    fn fractional_numeric_value_needs_rounding() {
        let reader = numeric_reader("1.5");
        let mut values = reader
            .get_numeric(&FieldInfo::new("f", 0))
            .unwrap()
            .expect("values");
        assert_eq!(values.next_doc().unwrap(), 0);
        let error = values.long_value().unwrap_err();
        assert_eq!(
            unchecked(&error),
            (
                "java.lang.ArithmeticException",
                "Rounding necessary".to_string()
            )
        );
        let reader = numeric_reader("2E1");
        let mut values = reader
            .get_numeric(&FieldInfo::new("f", 0))
            .unwrap()
            .expect("values");
        values.next_doc().unwrap();
        assert_eq!(values.long_value().unwrap(), 20);
    }

    /// Regression (rmp #526): the decoder is `new DecimalFormat(pattern, ...)`
    /// over the header's pattern, so a malformed pattern fails `getNumeric`
    /// with `applyPattern`'s `IllegalArgumentException`, and a `#,##0`
    /// pattern parses grouped values (`SimpleTextDocValuesReader.java:286-311`).
    #[test]
    fn decoder_is_built_from_the_header_pattern() {
        let open = |pattern: &str, value: &str| {
            let dir = directory();
            let pattern = format!("  pattern {pattern}");
            write_file(
                dir.as_ref(),
                "_0.dat",
                &[
                    "field f",
                    "  type NUMERIC",
                    "  minalue 0",
                    "  maxvalue 9",
                    "  doccount 1",
                    "  maxvaluecount 1",
                    "  origin 1",
                    &pattern,
                    value,
                    "T",
                    "  skipdata",
                    "  skipend",
                    "END",
                ],
                true,
            );
            let si = segment_info(&dir, "_0", 1);
            let infos = FieldInfos::new(Vec::new()).unwrap();
            let state = SegmentReadState::new(dir.as_ref(), &si, &infos, &*DEFAULT_IO_CONTEXT);
            SimpleTextDocValuesReader::new(&state, "dat").unwrap()
        };
        let reader = open("0,", "12");
        let error = reader
            .get_numeric(&FieldInfo::new("f", 0))
            .err()
            .expect("an error");
        match &error.kind {
            LuceneErrorKind::IllegalArgument { message, .. } => {
                assert_eq!(message, "Malformed pattern \"0,\"")
            }
            other => panic!("{other:?}"),
        }
        let reader = open("#,##0", "1,234");
        let mut values = reader
            .get_numeric(&FieldInfo::new("f", 0))
            .unwrap()
            .expect("values");
        assert_eq!(values.next_doc().unwrap(), 0);
        assert_eq!(values.long_value().unwrap(), 1235);
    }

    /// Regression (rmp #526): `assert field != null;` carries no message
    /// (`SimpleTextDocValuesReader.java:283-285`); the port's error named
    /// the field and the map size.
    #[cfg(debug_assertions)]
    #[test]
    fn missing_field_is_a_bare_assertion() {
        let reader = numeric_reader("001");
        let error = reader
            .get_numeric(&FieldInfo::new("g", 1))
            .err()
            .expect("an error");
        match &error.kind {
            LuceneErrorKind::AssertionError(message) => assert_eq!(message, ""),
            other => panic!("{other:?}"),
        }
    }

    /// Regression (rmp #526): `nextDoc()` past `NO_MORE_DOCS` is
    /// `advance(Integer.MIN_VALUE)` in Java's wrapping arithmetic, whose
    /// negative seek fails; the port overflowed (a panic in debug builds).
    #[test]
    fn next_doc_after_exhaustion_wraps() {
        let reader = numeric_reader("001");
        let mut values = reader
            .get_numeric(&FieldInfo::new("f", 0))
            .unwrap()
            .expect("values");
        assert_eq!(values.next_doc().unwrap(), 0);
        assert_eq!(values.next_doc().unwrap(), NO_MORE_DOCS);
        assert!(values.next_doc().is_err());
    }
}
