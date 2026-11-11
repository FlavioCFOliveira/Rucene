//! Port of `lucene/core/src/test/org/apache/lucene/index/TestMixedDocValuesUpdates.java`.
//!
//! * `TestBinaryDocValuesUpdates.toBytes`/`getValue` are the package-private
//!   helpers of that port (`crate::test_binary_doc_values_updates`).
//! * `IndexWriter.updateDocValues(Term, Field...)` takes `Field`s; a
//!   `NumericDocValuesField`/`BinaryDocValuesField` passed there is the
//!   `Field` of the same name, type and value (`ndv_update`, `bdv_update`),
//!   and a `null` value is a `Field` without a value
//!   (`Field::new_with_type`), as Java's `fieldsData == null`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{
    BinaryDocValuesField, Document, Field, NumericDocValuesField, NumericValue, Store, StringField,
};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader, IndexWriter as IndexWriterTrait};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig};
use rucene::index::multi_reader::reader_util::sub_index_from_leaves;
use rucene::index::terms::Term;
use rucene::index::IndexReader;
use rucene::search::field_exists_query::FieldExistsQuery;
use rucene::search::term_query::TermQuery;
use rucene::search::{IndexSearcher, TopDocs, NO_MORE_DOCS};
use rucene::util::{BytesRef, IOUtils};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::{expect_throws, is_instance, java_throwable};
use rucene_test_framework::util::lucene_test_case::{Nightly, TEST_NIGHTLY};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_null, assert_null, class, random, test,
    JavaHashSet, IAE,
};
use crate::support::{npe, CountDownLatch, CyclicBarrier};
use crate::test_binary_doc_values_updates::{get_value, to_bytes};

/// `TestMixedDocValuesUpdates`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestMixedDocValuesUpdates",
        vec![
            test("testManyReopensAndFields", test_many_reopens_and_fields),
            test("testStressMultiThreading", test_stress_multi_threading),
            test(
                "testUpdateDifferentDocsInDifferentGens",
                test_update_different_docs_in_different_gens,
            ),
            test("testTonsOfUpdates", test_tons_of_updates).annotate(Nightly),
            test("testTryUpdateDocValues", test_try_update_doc_values),
            test("testTryUpdateMultiThreaded", test_try_update_multi_threaded),
            test("testResetValue", test_reset_value),
            test("testResetValueMultipleDocs", test_reset_value_multiple_docs),
            test(
                "testUpdateNotExistingFieldDV",
                test_update_not_existing_field_dv,
            ),
            test(
                "testUpdateFieldWithNoPreviousDocValuesThrowsError",
                test_update_field_with_no_previous_doc_values_throws_error,
            ),
            test("testLongRunValuesReset", test_long_run_values_reset),
        ],
    )
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `new NumericDocValuesField(name, value)` as the `Field` an update takes.
fn ndv_update(name: &str, value: Option<i64>) -> Result<Field> {
    match value {
        Some(v) => Field::new_with_number(
            name,
            NumericValue::Long(v),
            NumericDocValuesField::type_().clone(),
        ),
        None => Field::new_with_type(name, NumericDocValuesField::type_().clone()),
    }
}

/// `new BinaryDocValuesField(name, value)` as the `Field` an update takes.
fn bdv_update(name: &str, value: Option<BytesRef>) -> Result<Field> {
    match value {
        Some(v) => Field::new_with_bytes(name, v, BinaryDocValuesField::type_().clone()),
        None => Field::new_with_type(name, BinaryDocValuesField::type_().clone()),
    }
}

/// `new StringField(name, value, store)`.
fn string_field(name: &str, value: String, store: Store) -> Result<Box<StringField>> {
    Ok(Box::new(StringField::new(name, value, store)?))
}

/// Port of `TestMixedDocValuesUpdates.testManyReopensAndFields`.
fn test_many_reopens_and_fields() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let random = random();
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random,
    ))));
    let lmp = LuceneTestCase::new_log_merge_policy();
    lmp.set_merge_factor(3)?; // merge often
    conf.set_merge_policy(Some(Arc::new(lmp)))?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    let is_nrt = random.next_boolean();
    let mut reader = if is_nrt {
        directory_reader::open_with_writer(&*writer)?
    } else {
        writer.commit()?;
        directory_reader::open(dir.as_directory())?
    };

    let num_fields = random.next_int_bound(4) + 3; // 3-7
    let num_ndv_fields = random.next_int_bound(num_fields / 2) + 1; // 1-3
    let mut field_values = vec![1i64; num_fields as usize];

    let num_rounds = LuceneTestCase::at_least(15);
    let mut doc_id = 0;
    for _ in 0..num_rounds {
        let num_docs = LuceneTestCase::at_least(5);
        for _ in 0..num_docs {
            let mut doc = Document::new();
            doc.add(string_field("id", format!("doc-{doc_id}"), Store::NO)?);
            doc.add(string_field("key", "all".into(), Store::NO)?); // update key
                                                                    // add all fields with their current value
            for f in 0..field_values.len() {
                if (f as i32) < num_ndv_fields {
                    doc.add(Box::new(NumericDocValuesField::new(
                        &format!("f{f}"),
                        field_values[f],
                    )));
                } else {
                    doc.add(Box::new(BinaryDocValuesField::new(
                        &format!("f{f}"),
                        to_bytes(field_values[f]),
                    )));
                }
            }
            writer.add_document(&doc)?;
            doc_id += 1;
        }

        let field_idx = random.next_int_bound(field_values.len() as i32) as usize;
        let update_field = format!("f{field_idx}");
        field_values[field_idx] += 1;
        if (field_idx as i32) < num_ndv_fields {
            writer.update_numeric_doc_value(
                Term::from_text("key", "all"),
                &update_field,
                field_values[field_idx],
            )?;
        } else {
            writer.update_binary_doc_value(
                Term::from_text("key", "all"),
                &update_field,
                to_bytes(field_values[field_idx]),
            )?;
        }

        if random.next_double() < 0.2 {
            let delete_doc = random.next_int_bound(doc_id); // might also delete an already deleted document, ok!
            writer.delete_documents_by_terms(vec![Term::from_text(
                "id",
                &format!("doc-{delete_doc}"),
            )])?;
        }

        // verify reader
        if !is_nrt {
            writer.commit()?;
        }

        let new_reader = directory_reader::open_if_changed(Arc::clone(&reader))?;
        assert_not_null(&new_reader);
        reader.close()?;
        reader = new_reader.unwrap_or_else(|| npe("newReader"));
        Assert::assert_true(reader.num_docs() > 0); // we delete at most one document per round
        for context in Arc::clone(&reader).leaves()? {
            let r = context.leaf_reader();
            let live_docs = r.get_live_docs();
            for field in 0..field_values.len() {
                let f = format!("f{field}");
                let bdv = r.get_binary_doc_values(&f)?;
                let ndv = r.get_numeric_doc_values(&f)?;
                if (field as i32) < num_ndv_fields {
                    assert_not_null(&ndv);
                    assert_null(&bdv.as_ref().map(|_| ()));
                } else {
                    assert_null(&ndv.as_ref().map(|_| ()));
                    assert_not_null(&bdv);
                }
                let mut ndv = ndv;
                let mut bdv = bdv;
                let max_doc = r.max_doc();
                for doc in 0..max_doc {
                    if live_docs.as_ref().is_none_or(|l| l.get(doc as usize)) {
                        if (field as i32) < num_ndv_fields {
                            let ndv = ndv.as_mut().unwrap_or_else(|| npe("ndv"));
                            assert_equals(&doc, &ndv.advance(doc)?);
                            assert_equals_msg(
                                Some(&format!(
                                    "invalid numeric value for doc={doc}, field={f}, reader={r}"
                                )),
                                &field_values[field],
                                &ndv.long_value()?,
                            );
                        } else {
                            let bdv = bdv.as_mut().unwrap_or_else(|| npe("bdv"));
                            assert_equals(&doc, &bdv.advance(doc)?);
                            assert_equals_msg(
                                Some(&format!(
                                    "invalid binary value for doc={doc}, field={f}, reader={r}"
                                )),
                                &field_values[field],
                                &get_value(&**bdv)?,
                            );
                        }
                    }
                }
            }
        }
    }

    writer.close()?;
    IOUtils::close([
        IOUtils::closeable(|| reader.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// The update group of a draw: `g0` to `g3`.
fn group_of(group: f64) -> &'static str {
    if group < 0.1 {
        "g0"
    } else if group < 0.5 {
        "g1"
    } else if group < 0.8 {
        "g2"
    } else {
        "g3"
    }
}

/// Port of `TestMixedDocValuesUpdates.testStressMultiThreading`.
fn test_stress_multi_threading() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let conf = mock_iwc();
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    // create index
    let num_fields = TestUtil::next_int(&*random(), 2, 4);
    let num_threads = TestUtil::next_int(&*random(), 3, 6);
    let num_docs = LuceneTestCase::at_least(2000);
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(string_field("id", format!("doc{i}"), Store::NO)?);
        let group = random().next_double();
        let g = group_of(group);
        doc.add(string_field("updKey", g.into(), Store::NO)?);
        for j in 0..num_fields {
            let value = i64::from(random().next_int());
            doc.add(Box::new(BinaryDocValuesField::new(
                &format!("f{j}"),
                to_bytes(value),
            )));
            doc.add(Box::new(NumericDocValuesField::new(
                &format!("cf{j}"),
                value.wrapping_mul(2),
            ))); // control, always updated to f * 2
        }
        writer.add_document(&doc)?;
    }

    let done = Arc::new(CountDownLatch::new(i64::from(num_threads)));
    let num_updates = Arc::new(AtomicI32::new(LuceneTestCase::at_least(100)));

    // same thread updates a field as well as reopens
    let mut threads = Vec::with_capacity(num_threads as usize);
    for i in 0..num_threads {
        let writer = Arc::clone(&writer);
        let done = Arc::clone(&done);
        let num_updates = Arc::clone(&num_updates);
        threads.push(Thread::new(format!("UpdateThread-{i}"), move || {
            let mut reader: Option<Arc<dyn DirectoryReader>> = None;
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                    let random = random();
                    while num_updates.fetch_sub(1, Ordering::SeqCst) > 0 {
                        let group = random.next_double();
                        let t = Term::from_text("updKey", group_of(group));
                        let field = crate::core_support::random().next_int_bound(num_fields);
                        let f = format!("f{field}");
                        let cf = format!("cf{field}");
                        let upd_value = i64::from(random.next_int());
                        writer.update_doc_values(
                            t,
                            &[
                                bdv_update(&f, Some(to_bytes(upd_value)))?,
                                ndv_update(&cf, Some(upd_value.wrapping_mul(2)))?,
                            ],
                        )?;

                        if random.next_double() < 0.2 {
                            // delete a random document
                            let doc = random.next_int_bound(num_docs);
                            writer.delete_documents_by_terms(vec![Term::from_text(
                                "id",
                                &format!("doc{doc}"),
                            )])?;
                        }

                        if random.next_double() < 0.05 {
                            // commit every 20 updates on average
                            writer.commit()?;
                        }

                        if random.next_double() < 0.1 {
                            // reopen NRT reader (apply updates), on average once every 10
                            // updates
                            match &reader {
                                None => {
                                    reader = Some(directory_reader::open_with_writer(&*writer)?);
                                }
                                Some(current) => {
                                    let r2 = directory_reader::open_if_changed_with_writer(
                                        Arc::clone(current),
                                        &*writer,
                                        true,
                                    )?;
                                    if let Some(r2) = r2 {
                                        current.close()?;
                                        reader = Some(r2);
                                    }
                                }
                            }
                        }
                    }
                    Ok(())
                }));
            let success = matches!(outcome, Ok(Ok(())));
            // `finally`
            let mut close_failure = None;
            if let Some(reader) = reader.take() {
                if let Err(e) = reader.close() {
                    if success {
                        // suppress this exception only if there was another exception
                        close_failure = Some(e);
                    }
                }
            }
            done.count_down();
            match outcome {
                Ok(Ok(())) => {
                    if let Some(e) = close_failure {
                        Throwable::from_cause("java.lang.RuntimeException", java_throwable(&e))
                            .throw()
                    }
                }
                Ok(Err(e)) if is_instance(&e, "java.io.IOException") => {
                    Throwable::from_cause("java.lang.RuntimeException", java_throwable(&e)).throw()
                }
                Ok(Err(e)) => java_throwable(&e).throw(),
                Err(payload) => std::panic::resume_unwind(payload),
            }
        }));
    }

    for t in &threads {
        t.start();
    }
    done.await_();
    writer.close()?;

    let reader = directory_reader::open(dir.as_directory())?;
    for context in Arc::clone(&reader).leaves()? {
        let r = context.leaf_reader();
        for i in 0..num_fields {
            let mut bdv = r
                .get_binary_doc_values(&format!("f{i}"))?
                .unwrap_or_else(|| npe("bdv"));
            let mut control = r
                .get_numeric_doc_values(&format!("cf{i}"))?
                .unwrap_or_else(|| npe("control"));
            let live_docs = r.get_live_docs();
            for j in 0..r.max_doc() {
                if live_docs.as_ref().is_none_or(|l| l.get(j as usize)) {
                    assert_equals(&j, &control.advance(j)?);
                    let ctrl_value = control.long_value()?;
                    assert_equals(&j, &bdv.advance(j)?);
                    let bdv_value = get_value(&*bdv)?.wrapping_mul(2);
                    assert_equals(&ctrl_value, &bdv_value);
                }
            }
        }
    }
    reader.close()?;

    dir.close()
}

/// Port of `TestMixedDocValuesUpdates.testUpdateDifferentDocsInDifferentGens`.
fn test_update_different_docs_in_different_gens() -> Result<()> {
    // update same document multiple times across generations
    let dir = LuceneTestCase::new_directory();
    let conf = mock_iwc();
    conf.set_max_buffered_docs(4)?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    let num_docs = LuceneTestCase::at_least(10);
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(string_field("id", format!("doc{i}"), Store::NO)?);
        let value = i64::from(random().next_int());
        doc.add(Box::new(BinaryDocValuesField::new("f", to_bytes(value))));
        doc.add(Box::new(NumericDocValuesField::new(
            "cf",
            value.wrapping_mul(2),
        )));
        writer.add_document(&doc)?;
    }

    let num_gens = LuceneTestCase::at_least(5);
    for _ in 0..num_gens {
        let doc = random().next_int_bound(num_docs);
        let t = Term::from_text("id", &format!("doc{doc}"));
        let value = random().next_long();
        if random().next_boolean() {
            do_update(
                t,
                &writer,
                &[
                    bdv_update("f", Some(to_bytes(value)))?,
                    ndv_update("cf", Some(value.wrapping_mul(2)))?,
                ],
            )?;
        } else {
            writer.update_doc_values(
                t,
                &[
                    bdv_update("f", Some(to_bytes(value)))?,
                    ndv_update("cf", Some(value.wrapping_mul(2)))?,
                ],
            )?;
        }

        let reader = directory_reader::open_with_writer(&*writer)?;
        for context in Arc::clone(&reader).leaves()? {
            let r = context.leaf_reader();
            let mut fbdv = r.get_binary_doc_values("f")?.unwrap_or_else(|| npe("fbdv"));
            let mut cfndv = r
                .get_numeric_doc_values("cf")?
                .unwrap_or_else(|| npe("cfndv"));
            for j in 0..r.max_doc() {
                assert_equals(&j, &cfndv.next_doc()?);
                assert_equals(&j, &fbdv.next_doc()?);
                assert_equals(&cfndv.long_value()?, &get_value(&*fbdv)?.wrapping_mul(2));
            }
        }
        reader.close()?;
    }
    writer.close()?;
    dir.close()
}

/// `RandomPicks.randomFrom(random, set)` over a Java `HashSet<String>`, in
/// its iteration order.
fn random_from_set(random: &dyn Random, set: &JavaHashSet<String>) -> String {
    RandomPicks::random_from_list(random, &set.to_vec()).clone()
}

/// Port of `TestMixedDocValuesUpdates.testTonsOfUpdates` (`@Nightly`).
fn test_tons_of_updates() -> Result<()> {
    // LUCENE-5248: make sure that when there are many updates, we don't use too much RAM
    let dir = LuceneTestCase::new_directory();
    let random = random();
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random,
    ))));
    conf.set_ram_buffer_size_mb(LiveIndexWriterConfig::DEFAULT_RAM_BUFFER_SIZE_MB)?;
    conf.set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)?; // don't flush by doc
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    // test data: lots of documents (few 10Ks) and lots of update terms (few hundreds)
    let num_docs = LuceneTestCase::at_least(20000);
    let num_binary_fields = LuceneTestCase::at_least(5);
    let num_terms = TestUtil::next_int(&*random, 10, 100); // terms should affect many docs
    let mut update_terms: JavaHashSet<String> = JavaHashSet::new();
    while (update_terms.size() as i32) < num_terms {
        update_terms.add(TestUtil::random_simple_string(&*random));
    }

    // build a large index with many BDV fields and update terms
    for _ in 0..num_docs {
        let mut doc = Document::new();
        let num_update_terms = TestUtil::next_int(&*random, 1, num_terms / 10);
        for _ in 0..num_update_terms {
            doc.add(string_field(
                "upd",
                random_from_set(&*random, &update_terms),
                Store::NO,
            )?);
        }
        for j in 0..num_binary_fields {
            let val = i64::from(random.next_int());
            doc.add(Box::new(BinaryDocValuesField::new(
                &format!("f{j}"),
                to_bytes(val),
            )));
            doc.add(Box::new(NumericDocValuesField::new(
                &format!("cf{j}"),
                val.wrapping_mul(2),
            )));
        }
        writer.add_document(&doc)?;
    }

    writer.commit()?; // commit so there's something to apply to

    // set to flush every 2048 bytes (approximately every 12 updates), so we get
    // many flushes during binary updates
    writer
        .get_config()
        .set_ram_buffer_size_mb(2048.0 / 1024.0 / 1024.0)?;
    let num_updates = LuceneTestCase::at_least(100);
    for _ in 0..num_updates {
        let field = random.next_int_bound(num_binary_fields);
        let update_term = Term::from_text("upd", &random_from_set(&*random, &update_terms));
        let value = i64::from(random.next_int());
        writer.update_doc_values(
            update_term,
            &[
                bdv_update(&format!("f{field}"), Some(to_bytes(value)))?,
                ndv_update(&format!("cf{field}"), Some(value.wrapping_mul(2)))?,
            ],
        )?;
    }

    writer.close()?;

    let reader = directory_reader::open(dir.as_directory())?;
    for context in Arc::clone(&reader).leaves()? {
        for i in 0..num_binary_fields {
            let r = context.leaf_reader();
            let mut f = r
                .get_binary_doc_values(&format!("f{i}"))?
                .unwrap_or_else(|| npe("f"));
            let mut cf = r
                .get_numeric_doc_values(&format!("cf{i}"))?
                .unwrap_or_else(|| npe("cf"));
            for j in 0..r.max_doc() {
                assert_equals(&j, &cf.next_doc()?);
                assert_equals(&j, &f.next_doc()?);
                assert_equals_msg(
                    Some(&format!("reader={r}, field=f{i}, doc={j}")),
                    &cf.long_value()?,
                    &get_value(&*f)?.wrapping_mul(2),
                );
            }
        }
    }
    reader.close()?;

    dir.close()
}

/// `new IndexSearcher(reader).search(new TermQuery(term), 10)`.
fn search_term(reader: Arc<dyn IndexReader>, term: Term) -> Result<TopDocs> {
    IndexSearcher::new(reader)?.search(Arc::new(TermQuery::new(term)), 10)
}

/// Port of `TestMixedDocValuesUpdates.testTryUpdateDocValues`.
fn test_try_update_doc_values() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let conf = LuceneTestCase::new_index_writer_config();
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    let num_docs = 1 + random().next_int_bound(128);
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(string_field("id", i.to_string(), Store::YES)?);
        doc.add(Box::new(NumericDocValuesField::new(
            "numericId",
            i64::from(i),
        )));
        doc.add(Box::new(BinaryDocValuesField::new(
            "binaryId",
            BytesRef::new(vec![i as u8]),
        )));
        writer.add_document(&doc)?;
        if random().next_boolean() {
            writer.flush()?;
        }
    }
    let doc = random().next_int_bound(num_docs);
    do_update(
        Term::from_text("id", &doc.to_string()),
        &writer,
        &[
            ndv_update("numericId", Some(i64::from(doc + 1)))?,
            bdv_update("binaryId", Some(BytesRef::new(vec![(doc + 1) as u8])))?,
        ],
    )?;
    let reader: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*writer)?;
    let mut numeric_id_values = None;
    let mut binary_id_values = None;
    for c in Arc::clone(&reader).leaves()? {
        let top_docs = search_term(
            c.leaf_reader().as_index_reader(),
            Term::from_text("id", &doc.to_string()),
        )?;
        if top_docs.total_hits.value() == 1 {
            assert_null(&numeric_id_values.as_ref().map(|_: &Box<_>| ()));
            assert_null(&binary_id_values.as_ref().map(|_: &Box<_>| ()));
            let mut ndv = c
                .leaf_reader()
                .get_numeric_doc_values("numericId")?
                .unwrap_or_else(|| npe("numericIdValues"));
            assert_equals(
                &top_docs.score_docs[0].doc,
                &ndv.advance(top_docs.score_docs[0].doc)?,
            );
            numeric_id_values = Some(ndv);
            let mut bdv = c
                .leaf_reader()
                .get_binary_doc_values("binaryId")?
                .unwrap_or_else(|| npe("binaryIdValues"));
            assert_equals(
                &top_docs.score_docs[0].doc,
                &bdv.advance(top_docs.score_docs[0].doc)?,
            );
            binary_id_values = Some(bdv);
        } else {
            assert_equals(&0, &top_docs.total_hits.value());
        }
    }

    assert_not_null(&numeric_id_values);
    assert_not_null(&binary_id_values);

    let numeric_id_values = numeric_id_values.unwrap_or_else(|| npe("numericIdValues"));
    let binary_id_values = binary_id_values.unwrap_or_else(|| npe("binaryIdValues"));
    assert_equals(&i64::from(doc + 1), &numeric_id_values.long_value()?);
    assert_equals(
        &BytesRef::new(vec![(doc + 1) as u8]),
        &binary_id_values.binary_value()?,
    );
    IOUtils::close([
        IOUtils::closeable(|| reader.close()),
        IOUtils::closeable(|| writer.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// `java.util.concurrent.locks.ReentrantLock`, held across statements:
/// `lock()` waits while another thread holds it, `unlock()` releases.
#[derive(Default)]
struct HeldLock {
    held: Mutex<bool>,
    released: std::sync::Condvar,
}

impl HeldLock {
    fn lock(&self) {
        let mut held = self.held.lock().unwrap_or_else(|p| p.into_inner());
        while *held {
            held = self.released.wait(held).unwrap_or_else(|p| p.into_inner());
        }
        *held = true;
    }

    fn unlock(&self) {
        *self.held.lock().unwrap_or_else(|p| p.into_inner()) = false;
        self.released.notify_one();
    }
}

/// Port of `TestMixedDocValuesUpdates.testTryUpdateMultiThreaded`.
fn test_try_update_multi_threaded() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let conf = LuceneTestCase::new_index_writer_config();
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    let num_locks = 25 + random().next_int_bound(50);
    let locks: Arc<Vec<HeldLock>> = Arc::new((0..num_locks).map(|_| HeldLock::default()).collect());
    let values: Arc<Vec<Mutex<Option<i64>>>> =
        Arc::new((0..num_locks).map(|_| Mutex::new(None)).collect());

    for i in 0..num_locks as usize {
        let mut doc = Document::new();
        let value = random().next_long();
        *values[i].lock().unwrap_or_else(|p| p.into_inner()) = Some(value);
        doc.add(string_field("id", i.to_string(), Store::NO)?);
        doc.add(Box::new(NumericDocValuesField::new("value", value)));
        writer.add_document(&doc)?;
    }

    let num_threads = if *TEST_NIGHTLY {
        2 + random().next_int_bound(3)
    } else {
        2
    };
    let mut threads = Vec::with_capacity(num_threads as usize);
    let barrier = Arc::new(CyclicBarrier::new(num_threads + 1));
    for _ in 0..num_threads {
        let (writer, locks, values, barrier) = (
            Arc::clone(&writer),
            Arc::clone(&locks),
            Arc::clone(&values),
            Arc::clone(&barrier),
        );
        let t = Thread::unnamed(move || {
            let outcome =
                crate::core_support::try_catch("java.lang.Exception", || -> Result<()> {
                    if let Err(e) = barrier.await_() {
                        Throwable::new(e.java_class(), "").throw();
                    }
                    for _ in 0..1000 {
                        let doc_id = random().next_int_bound(locks.len() as i32) as usize;
                        locks[doc_id].lock();
                        let updated = (|| -> Result<()> {
                            let value = if LuceneTestCase::rarely() {
                                None
                            } else {
                                Some(random().next_long())
                            }; // sometimes reset it
                            if random().next_boolean() {
                                writer.update_doc_values(
                                    Term::from_text("id", &doc_id.to_string()),
                                    &[ndv_update("value", value)?],
                                )?;
                            } else {
                                do_update(
                                    Term::from_text("id", &doc_id.to_string()),
                                    &writer,
                                    &[ndv_update("value", value)?],
                                )?;
                            }
                            *values[doc_id].lock().unwrap_or_else(|p| p.into_inner()) = value;
                            Ok(())
                        })();
                        // finally
                        locks[doc_id].unlock();
                        if let Err(e) = updated {
                            Throwable::from_cause("java.lang.AssertionError", java_throwable(&e))
                                .throw();
                        }
                        if LuceneTestCase::rarely() {
                            writer.flush()?;
                        }
                    }
                    Ok(())
                });
            let caught = match outcome {
                Ok(Ok(())) => None,
                Ok(Err(e)) => Some(java_throwable(&e)),
                Err(t) => Some(t),
            };
            if let Some(e) = caught {
                Throwable::from_cause("java.lang.AssertionError", e).throw();
            }
        });
        t.start();
        threads.push(t);
    }

    if let Err(e) = barrier.await_() {
        Throwable::new(e.java_class(), "").throw();
    }
    for t in &threads {
        let _ = t.join();
    }
    let reader = directory_reader::open_with_writer(&*writer)?;
    let checked = (|| -> Result<()> {
        for i in 0..locks.len() {
            locks[i].lock();
            let one = (|| -> Result<()> {
                let value = *values[i].lock().unwrap_or_else(|p| p.into_inner());
                let top_docs = search_term(
                    Arc::clone(&reader) as Arc<dyn IndexReader>,
                    Term::from_text("id", &i.to_string()),
                )?;
                assert_equals(&top_docs.total_hits.value(), &1);
                let mut doc_id = top_docs.score_docs[0].doc;
                let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
                let sub_index = sub_index_from_leaves(doc_id, &leaves) as usize;
                let leaf_reader = leaves[sub_index].leaf_reader();
                doc_id -= leaves[sub_index].doc_base();
                let mut numeric_doc_values = leaf_reader
                    .get_numeric_doc_values("value")?
                    .unwrap_or_else(|| npe("numericDocValues"));
                match value {
                    None => Assert::assert_false_msg(
                        &format!("docID: {doc_id}"),
                        numeric_doc_values.advance_exact(doc_id)?,
                    ),
                    Some(value) => {
                        Assert::assert_true_msg(
                            &format!("docID: {doc_id}"),
                            numeric_doc_values.advance_exact(doc_id)?,
                        );
                        assert_equals(&numeric_doc_values.long_value()?, &value);
                    }
                }
                Ok(())
            })();
            // finally
            locks[i].unlock();
            one?;
        }
        Ok(())
    })();
    // try-with-resources
    let closed = reader.close();
    checked?;
    closed?;

    IOUtils::close([
        IOUtils::closeable(|| writer.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// Port of `TestMixedDocValuesUpdates.doUpdate` (`static`, package-private).
pub(crate) fn do_update(doc: Term, writer: &IndexWriter, fields: &[Field]) -> Result<()> {
    let mut seq_id = -1;
    while seq_id == -1 {
        // retry if we just committing a merge
        let reader: Arc<dyn IndexReader> = directory_reader::open_with_writer(writer)?;
        let attempt = (|| -> Result<i64> {
            let top_docs = search_term(Arc::clone(&reader), doc.clone())?;
            assert_equals(&1, &top_docs.total_hits.value());
            let the_doc = top_docs.score_docs[0].doc;
            writer.try_update_doc_value(&reader, the_doc, fields)
        })();
        let closed = reader.close();
        seq_id = attempt?;
        closed?;
    }
    Ok(())
}

/// Port of `TestMixedDocValuesUpdates.testResetValue`.
fn test_reset_value() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let conf = mock_iwc();
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    let mut doc = Document::new();
    doc.add(string_field("id", "0".into(), Store::NO)?);
    doc.add(Box::new(NumericDocValuesField::new("val", 5)));
    doc.add(Box::new(BinaryDocValuesField::new(
        "val-bin",
        BytesRef::new(vec![5]),
    )));
    writer.add_document(&doc)?;

    if random().next_boolean() {
        writer.commit()?;
    }
    {
        let reader = directory_reader::open_with_writer(&*writer)?;
        let checked = (|| -> Result<()> {
            let leaves = Arc::clone(&reader).leaves()?;
            assert_equals(&1, &leaves.len());
            let r = leaves[0].leaf_reader();
            let mut ndv = r
                .get_numeric_doc_values("val")?
                .unwrap_or_else(|| npe("ndv"));
            assert_equals(&0, &ndv.next_doc()?);
            assert_equals(&5, &ndv.long_value()?);
            assert_equals(&NO_MORE_DOCS, &ndv.next_doc()?);

            let mut bdv = r
                .get_binary_doc_values("val-bin")?
                .unwrap_or_else(|| npe("bdv"));
            assert_equals(&0, &bdv.next_doc()?);
            assert_equals(&BytesRef::new(vec![5]), &bdv.binary_value()?);
            assert_equals(&NO_MORE_DOCS, &bdv.next_doc()?);
            Ok(())
        })();
        let closed = reader.close();
        checked?;
        closed?;
    }

    writer.update_doc_values(Term::from_text("id", "0"), &[bdv_update("val-bin", None)?])?;
    {
        let reader = directory_reader::open_with_writer(&*writer)?;
        let checked = (|| -> Result<()> {
            let leaves = Arc::clone(&reader).leaves()?;
            assert_equals(&1, &leaves.len());
            let r = leaves[0].leaf_reader();
            let mut ndv = r
                .get_numeric_doc_values("val")?
                .unwrap_or_else(|| npe("ndv"));
            assert_equals(&0, &ndv.next_doc()?);
            assert_equals(&5, &ndv.long_value()?);
            assert_equals(&NO_MORE_DOCS, &ndv.next_doc()?);

            let mut bdv = r
                .get_binary_doc_values("val-bin")?
                .unwrap_or_else(|| npe("bdv"));
            assert_equals(&NO_MORE_DOCS, &bdv.next_doc()?);
            Ok(())
        })();
        let closed = reader.close();
        checked?;
        closed?;
    }
    IOUtils::close([
        IOUtils::closeable(|| writer.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// Port of `TestMixedDocValuesUpdates.testResetValueMultipleDocs`.
fn test_reset_value_multiple_docs() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let conf = mock_iwc();
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    let num_docs = 10 + random().next_int_bound(50);
    let mut current_seq_id = 0;
    let mut seq_id = [-1i32; 5];
    for i in 0..num_docs {
        let mut doc = Document::new();
        let id = random().next_int_bound(5);
        seq_id[id as usize] = current_seq_id;
        doc.add(string_field("id", id.to_string(), Store::YES)?);
        doc.add(Box::new(NumericDocValuesField::new(
            "seqID",
            i64::from(current_seq_id),
        )));
        current_seq_id += 1;
        doc.add(Box::new(NumericDocValuesField::new("is_live", 1)));
        if i > 0 {
            writer.update_doc_values(
                Term::from_text("id", &id.to_string()),
                &[ndv_update("is_live", None)?],
            )?;
        }
        writer.add_document(&doc)?;
        if random().next_boolean() {
            writer.flush()?;
        }
    }

    if random().next_boolean() {
        writer.commit()?;
    }
    let mut num_hits = 0; // check if every doc has been selected at least once
    for i in seq_id {
        if i > -1 {
            num_hits += 1;
        }
    }
    let reader = directory_reader::open_with_writer(&*writer)?;
    let checked = (|| -> Result<()> {
        let ir: Arc<dyn IndexReader> = Arc::clone(&reader) as Arc<dyn IndexReader>;
        let searcher = IndexSearcher::new(Arc::clone(&ir))?;

        let is_live = searcher.search(Arc::new(FieldExistsQuery::new("is_live")), 5)?;
        assert_equals(&num_hits, &is_live.total_hits.value());
        let stored_fields = ir.stored_fields()?;
        for doc in &is_live.score_docs {
            let id: i32 = stored_fields
                .document(doc.doc)?
                .get("id")
                .unwrap_or_else(|| npe("id"))
                .parse()
                .unwrap_or_else(|_| {
                    Throwable::new("java.lang.NumberFormatException", "id").throw()
                });
            let leaves = Arc::clone(&ir).leaves()?;
            let i = sub_index_from_leaves(doc.doc, &leaves) as usize;
            let leaf_reader_context = &leaves[i];
            let seq_id_values = leaf_reader_context
                .leaf_reader()
                .get_numeric_doc_values("seqID")?;
            assert_not_null(&seq_id_values);
            let mut seq_id_values = seq_id_values.unwrap_or_else(|| npe("seqID"));
            Assert::assert_true(
                seq_id_values.advance_exact(doc.doc - leaf_reader_context.doc_base())?,
            );
            assert_equals(
                &i64::from(seq_id[id as usize]),
                &seq_id_values.long_value()?,
            );
        }
        Ok(())
    })();
    let closed = reader.close();
    checked?;
    closed?;
    IOUtils::close([
        IOUtils::closeable(|| writer.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// `try (Directory dir = newDirectory(); IndexWriter writer = new IndexWriter(dir, conf)) { body }`.
fn with_dir_and_writer(
    conf: IndexWriterConfig,
    body: impl FnOnce(&IndexWriter) -> Result<()>,
) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = match IndexWriter::new(dir.as_directory(), conf) {
        Ok(w) => w,
        Err(e) => {
            let _ = dir.close();
            return Err(e);
        }
    };
    let outcome = body(&writer);
    let closed_writer = writer.close();
    drop(writer);
    let closed_dir = dir.close();
    outcome?;
    closed_writer?;
    closed_dir
}

/// Port of `TestMixedDocValuesUpdates.testUpdateNotExistingFieldDV`.
fn test_update_not_existing_field_dv() -> Result<()> {
    let conf = mock_iwc();
    with_dir_and_writer(conf, |writer| {
        let mut doc = Document::new();
        doc.add(string_field("id", "1".into(), Store::YES)?);
        doc.add(Box::new(NumericDocValuesField::new("test", 1)));
        writer.add_document(&doc)?;
        if random().next_boolean() {
            writer.commit()?;
        }
        writer.update_doc_values(
            Term::from_text("id", "1"),
            &[ndv_update("not_existing", Some(1))?],
        )?;

        let mut doc1 = Document::new();
        doc1.add(string_field("id", "2".into(), Store::YES)?);
        doc1.add(Box::new(BinaryDocValuesField::new(
            "not_existing",
            BytesRef::default(),
        )));
        let iae = expect_throws(IAE, || writer.add_document(&doc1));
        assert_equals(
            "cannot change field \"not_existing\" from doc values type=NUMERIC to inconsistent doc values type=BINARY",
            iae.message().unwrap_or_default(),
        );

        let iae = expect_throws(IAE, || {
            writer.update_doc_values(
                Term::from_text("id", "1"),
                &[bdv_update("not_existing", Some(BytesRef::default()))?],
            )
        });
        assert_equals(
            "Can't update [BINARY] doc values; the field [not_existing] has inconsistent doc values' type of [NUMERIC].",
            iae.message().unwrap_or_default(),
        );
        Ok(())
    })
}

/// Port of `TestMixedDocValuesUpdates.testUpdateFieldWithNoPreviousDocValuesThrowsError`.
fn test_update_field_with_no_previous_doc_values_throws_error() -> Result<()> {
    let conf = mock_iwc();
    with_dir_and_writer(conf, |writer| {
        let mut doc = Document::new();
        doc.add(string_field("id", "1".into(), Store::YES)?);
        writer.add_document(&doc)?;
        if random().next_boolean() {
            let reader = directory_reader::open_with_writer(writer)?;
            let checked = (|| -> Result<()> {
                let id = Arc::clone(&reader).leaves()?[0]
                    .leaf_reader()
                    .get_numeric_doc_values("id")?;
                assert_null(&id.as_ref().map(|_| ()));
                Ok(())
            })();
            let closed = reader.close();
            checked?;
            closed?;
        } else if random().next_boolean() {
            writer.commit()?;
        }
        let exception = expect_throws(IAE, || {
            writer.update_doc_values(Term::from_text("id", "1"), &[ndv_update("id", Some(1))?])
        });
        assert_equals(
            "Can't update [NUMERIC] doc values; the field [id] has inconsistent doc values' type of [NONE].",
            exception.message().unwrap_or_default(),
        );
        Ok(())
    })
}

/// Port of `TestMixedDocValuesUpdates.testLongRunValuesReset`.
fn test_long_run_values_reset() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), IndexWriterConfig::new())?;
    let num_docs = LuceneTestCase::at_least(65536);
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(string_field("id", i.to_string(), Store::NO)?);
        if i == 0 || i == num_docs - 1 || random().next_boolean() {
            doc.add(Box::new(NumericDocValuesField::new("numeric", 1)));
        }
        writer.add_document(&doc)?;
    }
    writer.flush()?;

    for i in 1..num_docs - 1 {
        writer.update_doc_values(
            Term::from_text("id", &i.to_string()),
            &[ndv_update("numeric", None)?],
        )?;
    }

    {
        let reader = IndexWriterTrait::get_reader(&*writer, true, true)?;
        let checked = (|| -> Result<()> {
            let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
            assert_equals(
                &2,
                &searcher.count(Arc::new(FieldExistsQuery::new("numeric")))?,
            );
            Ok(())
        })();
        let closed = reader.close();
        checked?;
        closed?;
    }

    IOUtils::close([
        IOUtils::closeable(|| writer.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}
