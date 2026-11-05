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

use std::io::{BufRead, BufReader, Read};

use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::error::Result;

use super::base_token_stream_test_case::BaseTokenStreamTestCase;
use crate::util::{LuceneTestCase, TestUtil};

/// Utility class for doing vocabulary-based stemming tests.
///
/// Equivalent to `org.apache.lucene.tests.analysis.VocabularyAssert`.
pub struct VocabularyAssert;

fn lines(input: impl Read) -> impl Iterator<Item = std::io::Result<String>> {
    BufReader::new(input).lines()
}

impl VocabularyAssert {
    /// Run a vocabulary test against two data files.
    ///
    /// # Errors
    ///
    /// I/O errors and what the analyzer throws.
    pub fn assert_vocabulary(a: &dyn Analyzer, voc: impl Read, out: impl Read) -> Result<()> {
        let mut output_reader = lines(out);
        for input_word in lines(voc) {
            let input_word = input_word?;
            let expected_word = output_reader.next().transpose()?;
            // assertNotNull(expectedWord): an AssertionError without a message
            Assert::assert_true(expected_word.is_some());
            let expected_word = expected_word.unwrap_or_default();
            BaseTokenStreamTestCase::check_one_term(a, &input_word, &expected_word)?;
        }
        Ok(())
    }

    /// Run a vocabulary test against one file: tab separated.
    ///
    /// # Errors
    ///
    /// I/O errors and what the analyzer throws.
    pub fn assert_vocabulary_tsv(a: &dyn Analyzer, voc_out: impl Read) -> Result<()> {
        for input_line in lines(voc_out) {
            let input_line = input_line?;
            // String.trim() strips every char <= ' '
            if input_line.starts_with('#') || input_line.trim_matches(|c| c <= ' ').is_empty() {
                continue; /* comment */
            }
            let words: Vec<&str> = input_line.split('\t').collect();
            BaseTokenStreamTestCase::check_one_term(a, words[0], words[1])?;
        }
        Ok(())
    }

    /// Run a vocabulary test against two data files inside a zip file.
    ///
    /// # Errors
    ///
    /// I/O errors and what the analyzer throws.
    pub fn assert_vocabulary_zip(
        a: &dyn Analyzer,
        zip_file: &std::path::Path,
        voc: &str,
        out: &str,
    ) -> Result<()> {
        let tmp = LuceneTestCase::create_temp_dir();
        let tmp = tmp.as_std_path();
        TestUtil::unzip(&std::fs::read(zip_file)?, tmp)?;
        let v = std::fs::File::open(tmp.join(voc))?;
        let o = std::fs::File::open(tmp.join(out))?;
        Self::assert_vocabulary(a, v, o)
    }

    /// Run a vocabulary test against a tab-separated data file inside a zip
    /// file.
    ///
    /// # Errors
    ///
    /// I/O errors and what the analyzer throws.
    pub fn assert_vocabulary_zip_tsv(
        a: &dyn Analyzer,
        zip_file: &std::path::Path,
        voc_out: &str,
    ) -> Result<()> {
        let tmp = LuceneTestCase::create_temp_dir();
        let tmp = tmp.as_std_path();
        TestUtil::unzip(&std::fs::read(zip_file)?, tmp)?;
        let input = std::fs::File::open(tmp.join(voc_out))?;
        Self::assert_vocabulary_tsv(a, input)
    }
}
