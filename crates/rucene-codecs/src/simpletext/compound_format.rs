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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextCompoundFormat`.

use std::collections::HashSet;
use std::sync::Arc;

use rucene::codecs::{CompoundDirectory, CompoundFormat};
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::{segment_file_name, strip_segment_name};
use rucene::index::SegmentInfo;
use rucene::store::java_nio_file::JavaIoException;
use rucene::store::{
    Directory, FileOpenHint, FileTypeHint, IOContext, IndexInput, IndexOutput, Lock,
    DEFAULT_IO_CONTEXT, READONCE_IO_CONTEXT,
};
use rucene::util::java_fmt::java_utf8_to_string;

use super::util::{self, parse_int, parse_long, read_line};

/// Extension of compound file.
pub(crate) const DATA_EXTENSION: &str = "scf";

pub(crate) const HEADER: &[u8] = b"cfs entry for: ";

pub(crate) const TABLE: &[u8] = b"table of contents, size: ";
pub(crate) const TABLENAME: &[u8] = b"  filename: ";
pub(crate) const TABLESTART: &[u8] = b"    start: ";
pub(crate) const TABLEEND: &[u8] = b"    end: ";

pub(crate) const TABLEPOS: &[u8] = b"table of contents begins at offset: ";

/// `SimpleTextCompoundFormat.OFFSETPATTERN`: as many `0`s as
/// `Long.toString(Long.MAX_VALUE)` has digits.
pub(crate) const OFFSETPATTERN_LEN: usize = 19;

/// Plain text compound format.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextCompoundFormat`.
///
/// **FOR RECREATIONAL USE ONLY**
#[derive(Debug, Default, Clone, Copy)]
pub struct SimpleTextCompoundFormat;

impl SimpleTextCompoundFormat {
    /// Sole constructor.
    pub fn new() -> Self {
        Self
    }
}

/// Port of the private `SimpleTextCompoundFormat.stripPrefix`.
fn strip_prefix(scratch: &[u8], prefix: &[u8]) -> String {
    java_utf8_to_string(&scratch[prefix.len().min(scratch.len())..])
}

/// `new DecimalFormat(OFFSETPATTERN, ROOT).parse(s).longValue()`:
/// `DecimalFormat.parse(String)` reads the longest number at the start of
/// `s` and throws `ParseException` when there is none.
fn parse_offset(s: &str) -> Option<i64> {
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
    let value = if negative { -value } else { value };
    Some(value as i64)
}

impl CompoundFormat for SimpleTextCompoundFormat {
    fn name(&self) -> &str {
        "SimpleTextCompoundFormat"
    }

    /// Port of `SimpleTextCompoundFormat.getCompoundReader`.
    fn get_compound_reader(
        &self,
        dir: &dyn Directory,
        si: &SegmentInfo,
    ) -> Result<Box<dyn CompoundDirectory>> {
        let data_file = segment_file_name(&si.name, "", DATA_EXTENSION);
        let hints: [Arc<dyn FileOpenHint>; 1] = [Arc::new(FileTypeHint::Data)];
        let mut input =
            dir.open_input(&data_file, DEFAULT_IO_CONTEXT.with_hints(&hints).as_ref())?;

        let mut scratch = Vec::new();

        // first get to TOC:
        let pos = input.length() - TABLEPOS.len() as i64 - OFFSETPATTERN_LEN as i64 - 1;
        input.seek(pos)?;
        read_line(input.as_mut(), &mut scratch)?;
        debug_assert!(scratch.starts_with(TABLEPOS));
        let table_pos = parse_offset(&strip_prefix(&scratch, TABLEPOS)).ok_or_else(|| {
            LuceneError::corrupt_index(
                format!(
                    "can't parse CFS trailer, got: {}",
                    java_utf8_to_string(&scratch)
                ),
                input.resource_description(),
            )
        })?;

        // seek to TOC and read it
        input.seek(table_pos)?;
        read_line(input.as_mut(), &mut scratch)?;
        debug_assert!(scratch.starts_with(TABLE));
        let num_entries = parse_int(&strip_prefix(&scratch, TABLE))?;

        let mut file_names = Vec::with_capacity(num_entries.max(0) as usize);
        let mut start_offsets = Vec::with_capacity(num_entries.max(0) as usize);
        let mut end_offsets = Vec::with_capacity(num_entries.max(0) as usize);

        for i in 0..num_entries as usize {
            read_line(input.as_mut(), &mut scratch)?;
            debug_assert!(scratch.starts_with(TABLENAME));
            let table_name = strip_prefix(&scratch, TABLENAME);
            file_names.push(format!("{}{}", si.name, strip_segment_name(&table_name)));

            if i > 0 {
                // files must be unique and in sorted order
                debug_assert!(file_names[i] > file_names[i - 1]);
            }

            read_line(input.as_mut(), &mut scratch)?;
            debug_assert!(scratch.starts_with(TABLESTART));
            start_offsets.push(parse_long(&strip_prefix(&scratch, TABLESTART))?);

            read_line(input.as_mut(), &mut scratch)?;
            debug_assert!(scratch.starts_with(TABLEEND));
            end_offsets.push(parse_long(&strip_prefix(&scratch, TABLEEND))?);
        }

        Ok(Box::new(SimpleTextCompoundDirectory {
            input,
            file_names,
            start_offsets,
            end_offsets,
        }))
    }

    /// Port of `SimpleTextCompoundFormat.write`.
    fn write(&self, dir: &dyn Directory, si: &SegmentInfo, context: &dyn IOContext) -> Result<()> {
        let data_file = segment_file_name(&si.name, "", DATA_EXTENSION);

        let files = si.files_ordered()?;
        let num_files = files.len();
        let mut names: Vec<String> = files.iter().cloned().collect();
        // `Arrays.sort(names)`: `String.compareTo`, UTF-16 code unit order.
        names.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        let mut start_offsets = vec![0i64; num_files];
        let mut end_offsets = vec![0i64; num_files];

        let mut output = dir.create_output(&data_file, context)?;
        let out: &mut dyn IndexOutput = output.as_mut();
        for (i, name) in names.iter().enumerate() {
            // write header for file
            util::write_bytes(out, HEADER)?;
            util::write_str(out, name)?;
            util::write_newline(out)?;

            // write bytes for file
            start_offsets[i] = out.file_pointer();
            let mut input = dir.open_input(name, &*READONCE_IO_CONTEXT)?;
            let length = input.length();
            out.copy_bytes(input.as_mut(), length)?;
            input.close()?;
            end_offsets[i] = out.file_pointer();
        }

        let toc_pos = out.file_pointer();

        // write CFS table
        util::write_bytes(out, TABLE)?;
        util::write_str(out, &num_files.to_string())?;
        util::write_newline(out)?;

        for (i, name) in names.iter().enumerate() {
            util::write_bytes(out, TABLENAME)?;
            util::write_str(out, name)?;
            util::write_newline(out)?;

            util::write_bytes(out, TABLESTART)?;
            util::write_str(out, &start_offsets[i].to_string())?;
            util::write_newline(out)?;

            util::write_bytes(out, TABLEEND)?;
            util::write_str(out, &end_offsets[i].to_string())?;
            util::write_newline(out)?;
        }

        // `new DecimalFormat(OFFSETPATTERN, ROOT).format(tocPos)`: zero-padded
        // to the pattern's 19 digits.
        util::write_bytes(out, TABLEPOS)?;
        util::write_str(
            out,
            &format!("{toc_pos:0width$}", width = OFFSETPATTERN_LEN),
        )?;
        util::write_newline(out)?;
        output.close()
    }
}

/// The anonymous `CompoundDirectory` `getCompoundReader` returns.
struct SimpleTextCompoundDirectory {
    input: Box<dyn IndexInput>,
    file_names: Vec<String>,
    start_offsets: Vec<i64>,
    end_offsets: Vec<i64>,
}

impl std::fmt::Debug for SimpleTextCompoundDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleTextCompoundDirectory")
            .field("file_names", &self.file_names)
            .finish_non_exhaustive()
    }
}

impl SimpleTextCompoundDirectory {
    /// `getIndex`: `Arrays.binarySearch(fileNames, name)`.
    fn get_index(&self, name: &str) -> Result<usize> {
        self.file_names
            .binary_search_by(|probe| probe.encode_utf16().cmp(name.encode_utf16()))
            .map_err(|_| {
                LuceneError::Io(
                    JavaIoException::new(
                        "java.io.FileNotFoundException",
                        Some(format!(
                            "No sub-file found (fileName={name} files: [{}])",
                            self.file_names.join(", ")
                        )),
                    )
                    .into_io_error(),
                )
            })
    }
}

impl CompoundDirectory for SimpleTextCompoundDirectory {
    fn check_integrity(&self) -> Result<()> {
        // No checksums for SimpleText
        Ok(())
    }
}

impl Directory for SimpleTextCompoundDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        Ok(self.file_names.clone())
    }

    fn delete_file(&self, _name: &str) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        let index = self.get_index(name)?;
        Ok(self.end_offsets[index] - self.start_offsets[index])
    }

    fn create_output(&self, _name: &str, _context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn create_temp_output(
        &self,
        _prefix: &str,
        _suffix: &str,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn sync(&self, _names: &[String]) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn sync_metadata(&self) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn rename(&self, _source: &str, _dest: &str) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        let index = self.get_index(name)?;
        self.input.slice_with_context(
            name,
            self.start_offsets[index],
            self.end_offsets[index] - self.start_offsets[index],
            context,
        )
    }

    fn obtain_lock(&self, _name: &str) -> Result<Box<dyn Lock>> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn close(&mut self) -> Result<()> {
        self.input.close()
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        Ok(HashSet::new())
    }
}
