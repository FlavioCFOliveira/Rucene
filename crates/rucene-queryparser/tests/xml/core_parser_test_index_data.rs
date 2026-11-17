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

//! Port of `CoreParserTestIndexData.java`.

use std::path::PathBuf;
use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::document::{Document, IntPoint, Store};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::search::index_searcher::IndexSearcher;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

/// The directory of the test resources of the Java package
/// `org.apache.lucene.queryparser.xml` (byte-identical copies, SHA-1
/// checked by the XML oracle).
pub fn resource_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/resources/org/apache/lucene/queryparser/xml")
}

/// `TestCoreParser.class.getResourceAsStream(name)`: the bytes, or `None`
/// for a resource that does not exist.
pub fn resource(name: &str) -> Option<Vec<u8>> {
    std::fs::read(resource_dir().join(name)).ok()
}

/// `class CoreParserTestIndexData implements Closeable`.
pub struct CoreParserTestIndexData {
    /// `dir`.
    pub dir: TestDirectory,
    /// `reader`.
    pub reader: Arc<dyn DirectoryReader>,
    /// `searcher`.
    pub searcher: Arc<IndexSearcher>,
}

impl CoreParserTestIndexData {
    /// `new CoreParserTestIndexData(Analyzer analyzer)`.
    pub fn new(analyzer: Arc<dyn Analyzer>) -> Result<Self> {
        // `new BufferedReader(new InputStreamReader(..., US_ASCII))`: the
        // file is ASCII; `readLine` splits on `\n`, `\r` and `\r\n`.
        let bytes = resource("reuters21578.txt")
            .ok_or_else(|| LuceneError::null_pointer_with_message("reuters21578.txt"))?;
        let text: String = bytes
            .iter()
            .map(|&b| if b < 0x80 { b as char } else { '\u{FFFD}' })
            .collect();
        let dir = LuceneTestCase::new_directory();
        let writer = IndexWriter::new(
            dir.as_directory(),
            LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer)),
        )?;
        for line in text.lines() {
            let end_of_date = line.find('\t').ok_or_else(|| {
                LuceneError::from(LuceneErrorKind::StringIndexOutOfBounds(format!(
                    "begin 0, end -1, length {}",
                    line.len()
                )))
            })?;
            let date = java_trim(&line[..end_of_date]);
            let content = java_trim(&line[end_of_date..]);
            let mut doc = Document::new();
            doc.add(LuceneTestCase::new_text_field("date", date, Store::YES));
            doc.add(LuceneTestCase::new_text_field(
                "contents",
                content,
                Store::YES,
            ));
            let date3 = rucene::util::extra::java_parse_int(date).ok_or_else(|| {
                LuceneError::number_format(format!("For input string: \"{date}\""))
            })?;
            doc.add(IntPoint::new("date3", &[date3])?);
            writer.add_document(&doc)?;
        }
        writer.close()?;
        let reader = directory_reader::open(dir.as_directory())?;
        let searcher = Arc::new(LuceneTestCase::new_searcher_wrap(
            Arc::clone(&reader),
            false,
        ));
        Ok(CoreParserTestIndexData {
            dir,
            reader,
            searcher,
        })
    }

    /// `close()`.
    pub fn close(&self) -> Result<()> {
        self.reader.close()?;
        self.dir.close()
    }
}

/// `String.trim()`: strips the code units `<= ' '` at both ends.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}
