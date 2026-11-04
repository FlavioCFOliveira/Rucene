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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextStoredFieldsReader`.

use std::sync::{Arc, Mutex};

use rucene::codecs::StoredFieldsReader;
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::{FieldInfos, SegmentInfo, StoredFieldVisitor, StoredFieldVisitorStatus};
use rucene::store::{BufferedChecksumIndexInput, Directory, IOContext, IndexInput};
use rucene::util::java_fmt::java_utf8_to_string;

use super::stored_fields_writer::{
    DOC, END, FIELD, FIELDS_EXTENSION, NAME, TYPE, TYPE_BINARY, TYPE_DOUBLE, TYPE_FLOAT, TYPE_INT,
    TYPE_LONG, TYPE_STRING, VALUE,
};
use super::util::{
    self, parse_double, parse_float, parse_int, parse_int_at, parse_long, read_line,
};

/// Reads plaintext stored fields.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextStoredFieldsReader`.
///
/// **FOR RECREATIONAL USE ONLY**
///
/// Java's `document` moves the reader's one `IndexInput` and reuses its
/// scratch buffers; both sit behind a [`Mutex`] here, because the port's
/// `StoredFieldsReader::document` takes `&self`.
pub struct SimpleTextStoredFieldsReader {
    /// docid -> offset in .fld file
    offsets: Option<Arc<Vec<i64>>>,
    state: Mutex<Option<ReaderState>>,
    field_infos: FieldInfos,
}

struct ReaderState {
    input: Box<dyn IndexInput>,
    scratch: Vec<u8>,
}

impl std::fmt::Debug for SimpleTextStoredFieldsReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `getClass().getSimpleName()`.
        f.write_str("SimpleTextStoredFieldsReader")
    }
}

/// The stored type a field line names, compared by identity in Java.
#[derive(Clone, Copy, PartialEq, Eq)]
enum StoredType {
    String,
    Binary,
    Int,
    Long,
    Float,
    Double,
}

impl SimpleTextStoredFieldsReader {
    /// Opens `si`'s `.fld` file and reads its index.
    ///
    /// # Errors
    ///
    /// Propagates the directory's error, and [`util::check_footer`]'s.
    pub fn new(
        directory: &dyn Directory,
        si: &SegmentInfo,
        fn_: &FieldInfos,
        context: &dyn IOContext,
    ) -> Result<Self> {
        let input =
            directory.open_input(&segment_file_name(&si.name, "", FIELDS_EXTENSION), context)?;
        let mut reader = Self {
            offsets: None,
            state: Mutex::new(Some(ReaderState {
                input,
                scratch: Vec::new(),
            })),
            field_infos: fn_.clone(),
        };
        reader.read_index(si.max_doc()?)?;
        Ok(reader)
    }

    /// We don't actually write a .fdx-like index, instead we read the stored
    /// fields file in entirety up-front and save the offsets so we can seek to
    /// the documents later.
    fn read_index(&mut self, size: i32) -> Result<()> {
        let state = self.state.get_mut().map_err(poisoned)?;
        let state = state.as_mut().ok_or_else(closed)?;
        // Java wraps `in` itself; a clone at the same (initial) position reads
        // the same bytes, and `document` seeks `in` before every read anyway.
        let mut input = BufferedChecksumIndexInput::new(state.input.clone_input()?);
        let mut offsets = vec![0i64; size.max(0) as usize];
        let mut upto = 0usize;
        let scratch = &mut state.scratch;
        while scratch.as_slice() != END {
            read_line(&mut input, scratch)?;
            if scratch.starts_with(DOC) {
                *offsets
                    .get_mut(upto)
                    .ok_or_else(|| array_index_out_of_bounds(upto, size))? = input.file_pointer();
                upto += 1;
            }
        }
        util::check_footer(&mut input)?;
        debug_assert_eq!(upto, offsets.len());
        self.offsets = Some(Arc::new(offsets));
        Ok(())
    }

    fn read_field(
        &self,
        state: &mut ReaderState,
        type_: StoredType,
        field_info: &rucene::index::FieldInfo,
        visitor: &mut dyn StoredFieldVisitor,
    ) -> Result<()> {
        read_line(state.input.as_mut(), &mut state.scratch)?;
        debug_assert!(state.scratch.starts_with(VALUE));
        let value = &state.scratch[VALUE.len()..];
        match type_ {
            StoredType::String => visitor.string_field(field_info, &java_utf8_to_string(value)),
            StoredType::Binary => visitor.binary_field(field_info, value),
            StoredType::Int => {
                let v = parse_int(&java_utf8_to_string(value))?;
                visitor.int_field(field_info, v)
            }
            StoredType::Long => {
                let v = parse_long(&java_utf8_to_string(value))?;
                visitor.long_field(field_info, v)
            }
            StoredType::Float => {
                let v = parse_float(&java_utf8_to_string(value))?;
                visitor.float_field(field_info, v)
            }
            StoredType::Double => {
                let v = parse_double(&java_utf8_to_string(value))?;
                visitor.double_field(field_info, v)
            }
        }
    }
}

/// `Arrays.equals(a, ..., b, b.offset + bOffset, ...)`.
fn equals_at(a: &[u8], b: &[u8], b_offset: usize) -> bool {
    b.len() >= b_offset && a == &b[b_offset..]
}

fn poisoned<T>(_: T) -> LuceneError {
    LuceneError::illegal_state("stored fields reader lock was poisoned".to_string())
}

fn closed() -> LuceneError {
    LuceneError::already_closed("this FieldsReader is closed")
}

/// Java's `ArrayIndexOutOfBoundsException` on `offsets[upto]`.
fn array_index_out_of_bounds(index: usize, length: i32) -> LuceneError {
    LuceneError::illegal_state(format!("Index {index} out of bounds for length {length}"))
}

impl StoredFieldsReader for SimpleTextStoredFieldsReader {
    /// Port of `SimpleTextStoredFieldsReader.document`.
    fn document(&self, n: i32, visitor: &mut dyn StoredFieldVisitor) -> Result<()> {
        let offsets = self.offsets.as_ref().ok_or_else(closed)?;
        let offset = *offsets
            .get(usize::try_from(n).unwrap_or(usize::MAX))
            .ok_or_else(|| array_index_out_of_bounds(n as usize, offsets.len() as i32))?;
        let mut guard = self.state.lock().map_err(poisoned)?;
        let state = guard.as_mut().ok_or_else(closed)?;
        state.input.seek(offset)?;

        loop {
            read_line(state.input.as_mut(), &mut state.scratch)?;
            if !state.scratch.starts_with(FIELD) {
                break;
            }
            let field_number = parse_int_at(&state.scratch, FIELD.len())?;
            let field_info = self.field_infos.field_info_by_number(field_number).ok_or_else(|| {
                LuceneError::illegal_state(format!(
                    "Cannot invoke \"org.apache.lucene.index.FieldInfo\" for field number {field_number}"
                ))
            })?;
            read_line(state.input.as_mut(), &mut state.scratch)?;
            debug_assert!(state.scratch.starts_with(NAME));
            read_line(state.input.as_mut(), &mut state.scratch)?;
            debug_assert!(state.scratch.starts_with(TYPE));

            let type_ = if equals_at(TYPE_STRING, &state.scratch, TYPE.len()) {
                StoredType::String
            } else if equals_at(TYPE_BINARY, &state.scratch, TYPE.len()) {
                StoredType::Binary
            } else if equals_at(TYPE_INT, &state.scratch, TYPE.len()) {
                StoredType::Int
            } else if equals_at(TYPE_LONG, &state.scratch, TYPE.len()) {
                StoredType::Long
            } else if equals_at(TYPE_FLOAT, &state.scratch, TYPE.len()) {
                StoredType::Float
            } else if equals_at(TYPE_DOUBLE, &state.scratch, TYPE.len()) {
                StoredType::Double
            } else {
                return Err(LuceneError::Other("unknown field type".to_string()));
            };

            match visitor.needs_field(field_info)? {
                StoredFieldVisitorStatus::Yes => {
                    self.read_field(state, type_, field_info, visitor)?;
                }
                StoredFieldVisitorStatus::No => {
                    read_line(state.input.as_mut(), &mut state.scratch)?;
                    debug_assert!(state.scratch.starts_with(VALUE));
                }
                StoredFieldVisitorStatus::Stop => return Ok(()),
            }
        }
        Ok(())
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    /// Port of `SimpleTextStoredFieldsReader.clone`.
    ///
    /// **Divergence.** Java's `clone()` throws `AlreadyClosedException("this
    /// FieldsReader is closed")` on a closed reader
    /// (`SimpleTextStoredFieldsReader.java:192-197`); the port's
    /// `clone_reader` cannot fail, so the clone of a closed reader is itself
    /// closed and reports that exception, with the same message, on its first
    /// use.
    fn clone_reader(&self) -> Box<dyn StoredFieldsReader> {
        let guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = guard.as_ref().map(|state| ReaderState {
            input: state
                .input
                .clone_input()
                .expect("INVARIANT: cloning an open IndexInput does not fail"),
            scratch: Vec::new(),
        });
        Box::new(Self {
            offsets: self.offsets.clone(),
            state: Mutex::new(state),
            field_infos: self.field_infos.clone(),
        })
    }

    fn close(&mut self) -> Result<()> {
        let state = self.state.get_mut().map_err(poisoned)?.take();
        self.offsets = None;
        match state {
            Some(mut state) => state.input.close(),
            None => Ok(()),
        }
    }
}
