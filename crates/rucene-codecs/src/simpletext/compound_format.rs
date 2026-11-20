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
use std::sync::{Arc, Mutex, PoisonError};

use rucene::codecs::{CompoundDirectory, CompoundFormat};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::index_file_names::{segment_file_name, strip_segment_name};
use rucene::index::SegmentInfo;
use rucene::store::java_nio_file::JavaIoException;
use rucene::store::{
    Directory, FileOpenHint, FileTypeHint, IOContext, IndexInput, IndexOutput, Lock,
    DEFAULT_IO_CONTEXT, READONCE_IO_CONTEXT,
};
use rucene::util::java_fmt::java_utf8_to_string;

use super::java_decimal_format::DecimalFormat;
use super::util::{self, parse_int, parse_long, JavaScratch};

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

/// Port of the private `SimpleTextCompoundFormat.stripPrefix`: `new
/// String(scratch.bytes(), prefix.length, scratch.length() -
/// prefix.length, UTF_8)`.
fn strip_prefix(scratch: &JavaScratch, prefix: &[u8]) -> Result<String> {
    scratch.new_string(prefix.len())
}

impl CompoundFormat for SimpleTextCompoundFormat {
    fn name(&self) -> &str {
        "SimpleTextCompoundFormat"
    }

    /// Port of `SimpleTextCompoundFormat.getCompoundReader`.
    fn get_compound_reader(
        &self,
        dir: &Arc<dyn Directory>,
        si: &SegmentInfo,
    ) -> Result<Box<dyn CompoundDirectory>> {
        let data_file = segment_file_name(&si.name, "", DATA_EXTENSION);
        let hints: [Arc<dyn FileOpenHint>; 1] = [Arc::new(FileTypeHint::Data)];
        let mut input =
            dir.open_input(&data_file, DEFAULT_IO_CONTEXT.with_hints(&hints).as_ref())?;

        let mut scratch = JavaScratch::new();
        let df = DecimalFormat::new(&"0".repeat(OFFSETPATTERN_LEN))?;

        // first get to TOC:
        let pos = input.length() - TABLEPOS.len() as i64 - OFFSETPATTERN_LEN as i64 - 1;
        input.seek(pos)?;
        scratch.read_line(input.as_mut())?;
        debug_assert!(scratch.starts_with(TABLEPOS));
        // `df.parse(stripPrefix(scratch, TABLEPOS)).longValue()`, its
        // `ParseException` the cause of the `CorruptIndexException`
        // (`SimpleTextCompoundFormat.java:84-90`).
        let table_pos = match df.parse(&strip_prefix(&scratch, TABLEPOS)?) {
            Ok(number) => number.long_value()?,
            Err(pe) => {
                return Err(LuceneError::corrupt_index_with_cause(
                    format!(
                        "can't parse CFS trailer, got: {}",
                        java_utf8_to_string(&scratch)
                    ),
                    input.resource_description(),
                    pe,
                ))
            }
        };

        // seek to TOC and read it
        input.seek(table_pos)?;
        scratch.read_line(input.as_mut())?;
        debug_assert!(scratch.starts_with(TABLE));
        let num_entries = parse_int(&strip_prefix(&scratch, TABLE)?)?;
        // `new String[numEntries]` (`SimpleTextCompoundFormat.java:97`).
        if num_entries < 0 {
            return Err(util::negative_array_size(num_entries));
        }

        let mut file_names = Vec::with_capacity(num_entries.max(0) as usize);
        let mut start_offsets = Vec::with_capacity(num_entries.max(0) as usize);
        let mut end_offsets = Vec::with_capacity(num_entries.max(0) as usize);

        for i in 0..num_entries as usize {
            scratch.read_line(input.as_mut())?;
            debug_assert!(scratch.starts_with(TABLENAME));
            let table_name = strip_prefix(&scratch, TABLENAME)?;
            file_names.push(format!("{}{}", si.name, strip_segment_name(&table_name)));

            if i > 0 {
                // files must be unique and in sorted order
                debug_assert!(file_names[i] > file_names[i - 1]);
            }

            scratch.read_line(input.as_mut())?;
            debug_assert!(scratch.starts_with(TABLESTART));
            start_offsets.push(parse_long(&strip_prefix(&scratch, TABLESTART)?)?);

            scratch.read_line(input.as_mut())?;
            debug_assert!(scratch.starts_with(TABLEEND));
            end_offsets.push(parse_long(&strip_prefix(&scratch, TABLEEND)?)?);
        }

        Ok(Box::new(SimpleTextCompoundDirectory {
            input: Mutex::new(input),
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

        // `try (IndexOutput out = dir.createOutput(dataFile, context))`
        // (`SimpleTextCompoundFormat.java:171-212`): the output is closed
        // whether or not the body throws; a failure of the body wins, and the
        // failure of the close it triggers is suppressed.
        let mut output = dir.create_output(&data_file, context)?;
        let written = (|| -> Result<()> {
            let out: &mut dyn IndexOutput = output.as_mut();
            for (i, name) in names.iter().enumerate() {
                // write header for file
                util::write_bytes(out, HEADER)?;
                util::write_str(out, name)?;
                util::write_newline(out)?;

                // write bytes for file
                start_offsets[i] = out.file_pointer();
                // `try (IndexInput in = dir.openInput(names[i], IOContext.READONCE))`
                // (`SimpleTextCompoundFormat.java:180-182`): closed on every exit.
                let mut input = dir.open_input(name, &*READONCE_IO_CONTEXT)?;
                let length = input.length();
                let copied = out.copy_bytes(input.as_mut(), length);
                rucene::store::close_after(input.as_mut(), copied)?;
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
            util::write_newline(out)
        })();
        let closed = output.close();
        written?;
        closed
    }
}

/// The anonymous `CompoundDirectory` `getCompoundReader` returns.
struct SimpleTextCompoundDirectory {
    /// Behind a mutex because `close()` closes it through the shared
    /// directory, as Java's `in.close()` does.
    input: Mutex<Box<dyn IndexInput>>,
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
                LuceneError::from(LuceneErrorKind::Io(
                    JavaIoException::new(
                        "java.io.FileNotFoundException",
                        Some(format!(
                            "No sub-file found (fileName={name} files: [{}])",
                            self.file_names.join(", ")
                        )),
                    )
                    .into_io_error(),
                ))
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
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        let index = self.get_index(name)?;
        Ok(self.end_offsets[index] - self.start_offsets[index])
    }

    fn create_output(&self, _name: &str, _context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn create_temp_output(
        &self,
        _prefix: &str,
        _suffix: &str,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn sync(&self, _names: &[String]) -> Result<()> {
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn sync_metadata(&self) -> Result<()> {
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn rename(&self, _source: &str, _dest: &str) -> Result<()> {
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        let index = self.get_index(name)?;
        self.input
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .slice_with_context(
                name,
                self.start_offsets[index],
                self.end_offsets[index] - self.start_offsets[index],
                context,
            )
    }

    fn obtain_lock(&self, _name: &str) -> Result<Box<dyn Lock>> {
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn close(&self) -> Result<()> {
        self.input
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .close()
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        Ok(HashSet::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simpletext::test_support::{directory, segment_info, unchecked, write_file};
    use rucene::util::extra::ParseException;

    /// Writes `_0.scf` holding a 10-byte filler line, a table of contents of
    /// `size` entries at offset 10, and the trailer `offset` (19 chars).
    fn compound(dir: &dyn Directory, size: &str, offset: &str) {
        let toc = format!("table of contents, size: {size}");
        let trailer = format!("table of contents begins at offset: {offset}");
        write_file(dir, "_0.scf", &["xxxxxxxxx", &toc, &trailer], false);
    }

    /// Regression (rmp #526): the trailer is parsed by `DecimalFormat`, which
    /// reads `…1E1` as 10 (the port read 1), and `new String[numEntries]`
    /// rejects a negative count with `NegativeArraySizeException`
    /// (`SimpleTextCompoundFormat.java:84-99`).
    #[test]
    fn trailer_exponent_and_negative_entry_count_follow_java() {
        let dir = directory();
        compound(dir.as_ref(), "-1", "00000000000000001E1");
        let si = segment_info(&dir, "_0", 1);
        let error = SimpleTextCompoundFormat::new()
            .get_compound_reader(&dir, &si)
            .unwrap_err();
        assert_eq!(
            unchecked(&error),
            ("java.lang.NegativeArraySizeException", "-1".to_string())
        );
    }

    /// Regression (rmp #526): an unparseable trailer is a
    /// `CorruptIndexException` whose cause is the `ParseException`
    /// (`SimpleTextCompoundFormat.java:87-90`).
    #[test]
    fn unparseable_trailer_keeps_the_parse_exception_as_cause() {
        let dir = directory();
        compound(dir.as_ref(), "0", "abcdefghijklmnopqrs");
        let si = segment_info(&dir, "_0", 1);
        let error = SimpleTextCompoundFormat::new()
            .get_compound_reader(&dir, &si)
            .unwrap_err();
        let LuceneErrorKind::CorruptIndex {
            original_message,
            cause,
            ..
        } = &error.kind
        else {
            panic!("{error:?}");
        };
        assert_eq!(
            original_message,
            "can't parse CFS trailer, got: table of contents begins at offset: abcdefghijklmnopqrs"
        );
        let cause = cause.as_deref().expect("the ParseException cause");
        assert_eq!(
            ParseException::of(cause).map(ParseException::get_message),
            Some("Unparseable number: \"abcdefghijklmnopqrs\"")
        );
    }
}
