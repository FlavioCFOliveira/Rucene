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

//! Port of `org.apache.lucene.tests.index.BaseTestCheckIndex`: base class
//! for CheckIndex tests.
//!
//! Java's class (`public class BaseTestCheckIndex extends LuceneTestCase`)
//! declares no JUnit test: its four methods take the `Directory` to check, so
//! the runner never calls them; a subclass (`TestCheckIndex`) wraps each in a
//! test of its own. [`class()`] declares the class; the four methods are free
//! functions taking the directory.

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::reflect::{modifier, Class};
use randomizedtesting::junit::Assert;
use rucene::document::{Document, FieldType, TextField};
use rucene::error::{LuceneError, Result};
use rucene::index::check_index::{CheckIndex, Level};
use rucene::index::index_writer::{IndexWriter, MAX_TERM_LENGTH};
use rucene::index::terms::Term;
use rucene::store::Directory;
use rucene::util::{BytesRef, InfoStream, PrintStreamInfoStream};

use super::base_index_file_format_test_case::{assert_not_null, assert_null, expect_throws_lucene};
use crate::analysis::MockAnalyzer;
use crate::util::lucene_test_case::{LuceneTestCase, VERBOSE};
use crate::util::{LineFileDocs, PrintStreamLines, TestUtil};

/// The declaration of `org.apache.lucene.tests.index.BaseTestCheckIndex`
/// (`public class BaseTestCheckIndex extends LuceneTestCase`), which declares
/// no test.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.tests.index.BaseTestCheckIndex")
            .modifiers(modifier::PUBLIC)
            .extends(&LuceneTestCase::class())
            .build()
    });
    Arc::clone(&CLASS)
}

/// `new PrintStream(bos, …, UTF_8)` over a `ByteArrayOutputStream(1024)`,
/// or `System.out` when `VERBOSE`.
fn info_stream(bos: &Arc<Mutex<Vec<u8>>>) -> Arc<dyn InfoStream> {
    if *VERBOSE {
        Arc::new(PrintStreamInfoStream::new(Box::new(std::io::stdout())))
    } else {
        Arc::new(PrintStreamLines(Arc::clone(bos)))
    }
}

/// `bos.toString(UTF_8)`.
fn bos_string(bos: &Arc<Mutex<Vec<u8>>>) -> String {
    String::from_utf8_lossy(&bos.lock().unwrap_or_else(|e| e.into_inner())).into_owned()
}

/// The error of a status, as `assertNull(status.error)` renders it.
fn error_string(error: &Option<LuceneError>) -> Option<String> {
    error.as_ref().map(|e| format!("{e}"))
}

/// `testDeletedDocs(dir)`.
///
/// # Errors
///
/// The errors of indexing and of `CheckIndex`.
pub fn test_deleted_docs(dir: Arc<dyn Directory>) -> Result<()> {
    let random = LuceneTestCase::random();
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::new(&*random),
    )));
    config.set_max_buffered_docs(2)?;
    let writer = IndexWriter::new(Arc::clone(&dir), config)?;
    for i in 0..19 {
        let mut doc = Document::new();
        let custom_type = FieldType::new_from(TextField::type_stored());
        custom_type.set_store_term_vectors(true)?;
        custom_type.set_store_term_vector_positions(true)?;
        custom_type.set_store_term_vector_offsets(true)?;
        doc.add(Box::new(LuceneTestCase::new_field(
            "field",
            &format!("aaa{i}"),
            &custom_type,
        )));
        writer.add_document(&doc)?;
    }
    writer.force_merge(1)?;
    writer.commit()?;
    writer.delete_documents_by_terms(vec![Term::new("field", BytesRef::new(b"aaa5".to_vec()))])?;
    writer.close()?;

    let bos = Arc::new(Mutex::new(Vec::with_capacity(1024)));
    let mut checker = CheckIndex::new(Arc::clone(&dir))?;
    checker.set_info_stream(Arc::new(PrintStreamLines(Arc::clone(&bos))));
    checker.set_level(Level::MIN_LEVEL_FOR_INTEGRITY_CHECKS)?;
    if *VERBOSE {
        checker.set_info_stream(info_stream(&bos));
    }
    let index_status = checker.check_index()?;
    if !index_status.clean {
        println!("CheckIndex failed");
        println!("{}", bos_string(&bos));
        Assert::fail_without_message();
    }

    let seg = &index_status.segment_infos[0];
    Assert::assert_true(seg.open_reader_passed);

    // `assertNotNull(seg.diagnostics)`: the port's map is never `null`.
    assert_not_null(true);

    assert_not_null(seg.field_norm_status.is_some());
    let field_norm_status = seg.field_norm_status.as_ref().expect("checked above");
    assert_null(error_string(&field_norm_status.error));
    Assert::assert_equals(1i64, field_norm_status.tot_fields);

    assert_not_null(seg.term_index_status.is_some());
    let term_index_status = seg.term_index_status.as_ref().expect("checked above");
    assert_null(error_string(&term_index_status.error));
    Assert::assert_equals(18i64, term_index_status.term_count);
    Assert::assert_equals(18i64, term_index_status.tot_freq);
    Assert::assert_equals(18i64, term_index_status.tot_pos);

    assert_not_null(seg.stored_field_status.is_some());
    let stored_field_status = seg.stored_field_status.as_ref().expect("checked above");
    assert_null(error_string(&stored_field_status.error));
    Assert::assert_equals(18i64, i64::from(stored_field_status.doc_count));
    Assert::assert_equals(18i64, stored_field_status.tot_fields);

    assert_not_null(seg.term_vector_status.is_some());
    let term_vector_status = seg.term_vector_status.as_ref().expect("checked above");
    assert_null(error_string(&term_vector_status.error));
    Assert::assert_equals(18i64, i64::from(term_vector_status.doc_count));
    Assert::assert_equals(18i64, term_vector_status.tot_vectors);

    assert_not_null(seg.diagnostics.contains_key("java.runtime.version"));

    Assert::assert_true(!seg.diagnostics.is_empty());
    let only_segments = vec!["_0".to_string()];

    Assert::assert_true(checker.check_index_segments(Some(&only_segments))?.clean);
    checker.close()
}

/// `testChecksumsOnly(dir)`, which `verbose` turns into
/// `testChecksumsOnlyVerbose(dir)`: the two differ only in the autoflush
/// flag of the `PrintStream`, which the buffer has no use for.
fn checksums_only(dir: Arc<dyn Directory>) -> Result<()> {
    let random = LuceneTestCase::random();
    let lf = LineFileDocs::with_default_file(&*random).map_err(LuceneError::from)?;
    let analyzer = Arc::new(MockAnalyzer::new(&*random));
    analyzer.set_max_token_length(TestUtil::next_int(&*random, 1, MAX_TERM_LENGTH as i32));
    let iw = IndexWriter::new(
        Arc::clone(&dir),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&analyzer) as _)),
    )?;
    for _ in 0..100 {
        iw.add_document(&lf.next_doc()?)?;
    }
    iw.add_document(&Document::new())?;
    iw.commit()?;
    iw.close()?;
    lf.close();

    let bos = Arc::new(Mutex::new(Vec::with_capacity(1024)));
    let mut checker = CheckIndex::new(Arc::clone(&dir))?;
    checker.set_info_stream(Arc::new(PrintStreamLines(Arc::clone(&bos))));
    if *VERBOSE {
        checker.set_info_stream(info_stream(&bos));
    }
    let index_status = checker.check_index()?;
    Assert::assert_true(index_status.clean);
    checker.close()?;
    rucene::analysis::Analyzer::close(&*analyzer);
    Ok(())
}

/// `testChecksumsOnly(dir)`.
///
/// # Errors
///
/// The errors of indexing and of `CheckIndex`.
pub fn test_checksums_only(dir: Arc<dyn Directory>) -> Result<()> {
    checksums_only(dir)
}

/// `testChecksumsOnlyVerbose(dir)`.
///
/// # Errors
///
/// The errors of indexing and of `CheckIndex`.
pub fn test_checksums_only_verbose(dir: Arc<dyn Directory>) -> Result<()> {
    checksums_only(dir)
}

/// `testObtainsLock(dir)`.
///
/// # Errors
///
/// The errors of indexing.
pub fn test_obtains_lock(dir: Arc<dyn Directory>) -> Result<()> {
    let iw = IndexWriter::new(
        Arc::clone(&dir),
        LuceneTestCase::new_index_writer_config_analyzer(None),
    )?;
    iw.add_document(&Document::new())?;
    iw.commit()?;

    // keep IW open... should not be able to obtain write lock
    expect_throws_lucene(
        "org.apache.lucene.store.LockObtainFailedException",
        |e| matches!(e, LuceneError::LockObtainFailed { .. }),
        || CheckIndex::new(Arc::clone(&dir)),
    );

    iw.close()
}
