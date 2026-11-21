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

//! Port of `org.apache.lucene.tests.analysis.VocabularyAssert`.

use std::io::Read;

use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::error::Result;
use rucene::store::java_nio_file::{FileSystems, Files, InputStream, Path};
use rucene_analysis_common::jdk::io::{BufferedReader, InputStreamReader};

use super::base_token_stream_test_case::BaseTokenStreamTestCase;
use crate::util::java_exceptions::try_with;
use crate::util::{LuceneTestCase, TestUtil};

/// Utility class for doing vocabulary-based stemming tests.
///
/// Equivalent to `org.apache.lucene.tests.analysis.VocabularyAssert`.
pub struct VocabularyAssert;

/// `new BufferedReader(new InputStreamReader(in, StandardCharsets.UTF_8))`:
/// JDK `readLine()` semantics (`\n`, `\r` and `\r\n` end a line;
/// malformed UTF-8 decodes to U+FFFD).
fn reader<R: Read>(input: R) -> BufferedReader<InputStreamReader<R>> {
    BufferedReader::new(InputStreamReader::new_utf8(input))
}

/// `readLine()`, as a Rust string (UTF-8 decoding never yields a lone
/// surrogate).
fn read_line<R: Read>(r: &mut BufferedReader<InputStreamReader<R>>) -> Result<Option<String>> {
    Ok(r.read_line()?.map(|units| String::from_utf16_lossy(&units)))
}

/// A `java.io.InputStream` of the nio filesystem as a [`Read`].
struct NioRead(Box<dyn InputStream>);

impl Read for NioRead {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let len = buf.len();
        let n = self.0.read(buf, 0, len)?;
        Ok(if n < 0 { 0 } else { n as usize })
    }
}

/// `Files.newInputStream(path)`.
fn new_input_stream(path: &Path) -> Result<NioRead> {
    Ok(NioRead(Files::new_input_stream(path, &[])?))
}

/// `InputStream.close()`.
fn close(stream: &mut NioRead) -> Result<()> {
    Ok(stream.0.close()?)
}

impl VocabularyAssert {
    /// Run a vocabulary test against two data files.
    ///
    /// Equivalent to `assertVocabulary(Analyzer, InputStream, InputStream)`.
    ///
    /// # Errors
    ///
    /// The I/O errors of the readers and what the analyzer throws.
    ///
    /// # Panics
    ///
    /// The `AssertionError` of a missing expected word or of `checkOneTerm`.
    pub fn assert_vocabulary(a: &dyn Analyzer, voc: impl Read, out: impl Read) -> Result<()> {
        let mut voc_reader = reader(voc);
        let mut output_reader = reader(out);
        while let Some(input_word) = read_line(&mut voc_reader)? {
            let expected_word = read_line(&mut output_reader)?;
            // assertNotNull(expectedWord): an AssertionError without a message
            Assert::assert_true(expected_word.is_some());
            let expected_word = expected_word.unwrap_or_default();
            BaseTokenStreamTestCase::check_one_term(a, &input_word, &expected_word)?;
        }
        Ok(())
    }

    /// Run a vocabulary test against one file: tab separated.
    ///
    /// Equivalent to `assertVocabulary(Analyzer, InputStream)`.
    ///
    /// # Errors
    ///
    /// The I/O errors of the reader and what the analyzer throws.
    ///
    /// # Panics
    ///
    /// The `ArrayIndexOutOfBoundsException` of a line without a second
    /// field, and the `AssertionError` of `checkOneTerm`.
    pub fn assert_vocabulary_tsv(a: &dyn Analyzer, voc_out: impl Read) -> Result<()> {
        let mut voc_reader = reader(voc_out);
        while let Some(input_line) = read_line(&mut voc_reader)? {
            // String.trim() strips every char <= ' '
            if input_line.starts_with('#') || input_line.trim_matches(|c| c <= ' ').is_empty() {
                continue; /* comment */
            }
            // `inputLine.split("\t")`: trailing empty strings are removed.
            let mut words: Vec<&str> = input_line.split('\t').collect();
            while words.len() > 1 && words.last() == Some(&"") {
                words.pop();
            }
            if words.len() < 2 {
                randomizedtesting::jdk::exception::Throwable::new(
                    "java.lang.ArrayIndexOutOfBoundsException",
                    format!("Index 1 out of bounds for length {}", words.len()),
                )
                .throw();
            }
            BaseTokenStreamTestCase::check_one_term(a, words[0], words[1])?;
        }
        Ok(())
    }

    /// Run a vocabulary test against two data files inside a zip file.
    ///
    /// Equivalent to `assertVocabulary(Analyzer, Path, String, String)`.
    ///
    /// # Errors
    ///
    /// The I/O errors of the files and what the analyzer throws.
    pub fn assert_vocabulary_zip(
        a: &dyn Analyzer,
        zip_file: &std::path::Path,
        voc: &str,
        out: &str,
    ) -> Result<()> {
        let tmp = LuceneTestCase::create_temp_dir();
        Self::unzip(zip_file, &tmp)?;
        // try (InputStream v = ...; InputStream o = ...)
        try_with(new_input_stream(&tmp.resolve_str(voc))?, close, |v| {
            try_with(new_input_stream(&tmp.resolve_str(out))?, close, |o| {
                Self::assert_vocabulary(a, &mut *v, &mut *o)
            })
        })
    }

    /// Run a vocabulary test against a tab-separated data file inside a zip
    /// file.
    ///
    /// Equivalent to `assertVocabulary(Analyzer, Path, String)`.
    ///
    /// # Errors
    ///
    /// The I/O errors of the files and what the analyzer throws.
    pub fn assert_vocabulary_zip_tsv(
        a: &dyn Analyzer,
        zip_file: &std::path::Path,
        voc_out: &str,
    ) -> Result<()> {
        let tmp = LuceneTestCase::create_temp_dir();
        Self::unzip(zip_file, &tmp)?;
        // try (InputStream in = Files.newInputStream(tmp.resolve(vocOut)))
        try_with(
            new_input_stream(&tmp.resolve_str(voc_out))?,
            close,
            |input| Self::assert_vocabulary_tsv(a, &mut *input),
        )
    }

    /// `try (InputStream in = Files.newInputStream(zipFile)) {
    /// TestUtil.unzip(in, tmp); }`.
    fn unzip(zip_file: &std::path::Path, tmp: &Path) -> Result<()> {
        let zip = FileSystems::get_default().get_path(&zip_file.to_string_lossy(), &[]);
        try_with(new_input_stream(&zip)?, close, |input| {
            let mut bytes = Vec::new();
            input.read_to_end(&mut bytes)?;
            Ok(TestUtil::unzip(&bytes, tmp.as_std_path())?)
        })
    }
}
