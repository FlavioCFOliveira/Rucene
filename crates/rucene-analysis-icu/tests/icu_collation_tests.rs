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

//! Port of the collation test classes of Lucene 10.5.0
//! `lucene/analysis/icu/src/test`, each run as a suite by the ported runner:
//! `org.apache.lucene.analysis.icu.TestICUCollationDocValuesField` and
//! `TestICUCollationKeyAnalyzer` (over the ported
//! `org.apache.lucene.tests.analysis.CollationTestBase`).
//!
//! Java reuses one `Document` whose fields it mutates between
//! `addDocument` calls; [`ReusedDocument`] is that document over the same
//! field objects. `Collator.getInstance()` reads the JVM default locale,
//! which `TestRuleSetupAndRestoreClassEnv` set from `-Dtests.locale`; the
//! port passes the framework's default locale explicitly ([`default_locale`]),
//! as the `TestCollationDocValuesField` port of `rucene-analysis-common` does.

mod support;

use std::cell::RefCell;
use std::sync::Arc;

use icu4j::text::collator::{self, Collator, IDENTICAL, PRIMARY};
use icu4j::util::u_locale::ENGLISH;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::{Field, StringField};
use rucene::error::Result;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::{IndexableField, IndexableFieldIterable};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::{Sort, SortField, SortFieldType};
use rucene::util::BytesRef;
use rucene_analysis_icu::analysis::icu::icu_collation_doc_values_field::ICUCollationDocValuesField;
use rucene_analysis_icu::analysis::icu::icu_collation_key_analyzer::ICUCollationKeyAnalyzer;
use rucene_queryparser::jdk::locale::Locale;
use rucene_test_framework::analysis::CollationTestBase;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::RANDOM_MULTIPLIER;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};
use support::{hook, lucene_class, run, t};

/// A Java `String` as UTF-16 code units.
fn u(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// `new BytesRef(collator.getCollationKey(s).toByteArray())`.
fn key_bytes(collator: &dyn Collator, s: &str) -> BytesRef {
    let key = collator
        .get_collation_key(Some(&u(s)))
        .expect("a non-null source yields a key");
    BytesRef::new(key.to_byte_array().iter().map(|&b| b as u8).collect())
}

// -----------------------------------------------------------------------------
// TestICUCollationDocValuesField
// -----------------------------------------------------------------------------

/// The `Document` Java reuses: the same field objects, in order.
struct ReusedDocument<'a>(Vec<&'a dyn IndexableField>);

impl IndexableFieldIterable for ReusedDocument<'_> {
    fn iterator(&self) -> Box<dyn Iterator<Item = Result<&dyn IndexableField>> + '_> {
        Box::new(self.0.iter().map(|f| Ok(*f)))
    }
}

/// `assertEquals(expected, actual)` of two strings.
fn eq(expected: &str, actual: &str) {
    Assert::assert_equals_str(None, Some(expected), Some(actual));
}

fn dvf_test_basic() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), dir.as_directory())?;
    let field: Field = LuceneTestCase::new_field("field", "", StringField::type_stored());
    let collation_field = ICUCollationDocValuesField::new(
        "collated",
        &*collator::get_instance_ulocale(Some(&ENGLISH))?,
    )?;
    let doc = ReusedDocument(vec![&field, &collation_field]);

    field.set_string_value("ABC".to_string())?;
    collation_field.set_string_value("ABC")?;
    iw.add_document(&doc)?;

    field.set_string_value("abc".to_string())?;
    collation_field.set_string_value("abc")?;
    iw.add_document(&doc)?;

    let ir = iw.get_reader()?;
    iw.close()?;

    let is = LuceneTestCase::new_searcher(Arc::clone(&ir));

    let sort_field = SortField::new(Some("collated".to_string()), SortFieldType::String)?;

    let td = is.search_with_sort(
        Arc::new(MatchAllDocsQuery::instance()),
        5,
        &Sort::new_fields(vec![sort_field])?,
    )?;
    let stored_fields = ir.stored_fields()?;
    eq(
        "abc",
        &stored_fields
            .document(td.score_docs[0].doc())?
            .get("field")
            .unwrap_or_default(),
    );
    eq(
        "ABC",
        &stored_fields
            .document(td.score_docs[1].doc())?
            .get("field")
            .unwrap_or_default(),
    );
    drop(is);
    ir.close()?;
    dir.close()
}

/// `Locale.getDefault()` (`-Dtests.locale`), as the JDK `Locale` of the
/// `icu4j` port.
fn default_locale() -> Result<Locale> {
    let tag = rucene_test_framework::jdk::locale::get_default()
        .expect("TestRuleSetupAndRestoreClassEnv sets the default locale")
        .to_language_tag();
    Locale::builder_from_language_tag(&tag)
}

fn dvf_test_ranges() -> Result<()> {
    let random = LuceneTestCase::random();
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random, dir.as_directory())?;
    let field: Field = LuceneTestCase::new_field("field", "", StringField::type_stored());
    // uses -Dtests.locale
    let mut collator = collator::get_instance_locale(Some(&default_locale()?))?;
    if random.next_boolean() {
        collator.set_strength(PRIMARY)?;
    }
    let collation_field = ICUCollationDocValuesField::new("collated", &*collator)?;
    let doc = ReusedDocument(vec![&field, &collation_field]);

    let num_docs = LuceneTestCase::at_least(500);
    for _ in 0..num_docs {
        let value = TestUtil::random_simple_string(&*random);
        field.set_string_value(value.clone())?;
        collation_field.set_string_value(&value)?;
        iw.add_document(&doc)?;
    }

    let ir = iw.get_reader()?;
    iw.close()?;
    let is = LuceneTestCase::new_searcher(Arc::clone(&ir));

    let num_checks = LuceneTestCase::at_least(100);

    let result = (|| -> Result<()> {
        for _ in 0..num_checks {
            let start = TestUtil::random_simple_string(&*random);
            let end = TestUtil::random_simple_string(&*random);
            let lower_val = key_bytes(&*collator, &start);
            let upper_val = key_bytes(&*collator, &end);
            do_test_ranges(&is, &start, &end, &lower_val, &upper_val, &*collator)?;
        }
        Ok(())
    })();
    drop(is);
    let closed = ir.close();
    let dir_closed = dir.close();
    result?;
    closed?;
    dir_closed
}

fn do_test_ranges(
    is: &IndexSearcher,
    start_point: &str,
    end_point: &str,
    start_br: &BytesRef,
    end_br: &BytesRef,
    collator: &dyn Collator,
) -> Result<()> {
    let mut dvs = MultiDocValues::get_sorted_values(is.get_index_reader(), "collated")?
        .expect("collated doc values");
    let stored_fields = is.stored_fields()?;
    let (start, end) = (u(start_point), u(end_point));
    for doc_id in 0..is.get_index_reader().max_doc() {
        let doc = stored_fields.document(doc_id)?;
        let s = u(&doc.get("field").unwrap_or_default());
        let collator_accepts = collator.compare(&s, &start) >= 0 && collator.compare(&s, &end) <= 0;
        Assert::assert_equals(doc_id, dvs.next_doc()?);
        let br = dvs.lookup_ord(dvs.ord_value()?)?;
        let lucene_accepts = *br >= *start_br && *br <= *end_br;
        Assert::assert_equals(collator_accepts, lucene_accepts);
    }
    Ok(())
}

fn collation_doc_values_field_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.icu.TestICUCollationDocValuesField")
        .method(t("testBasic", dvf_test_basic))
        .method(t("testRanges", dvf_test_ranges))
        .build()
}

// -----------------------------------------------------------------------------
// TestICUCollationKeyAnalyzer
// -----------------------------------------------------------------------------

/// The instance fields of `TestICUCollationKeyAnalyzer`.
struct KeyAnalyzerTest {
    base: CollationTestBase,
    analyzer: Arc<dyn Analyzer>,
    first_range_beginning: BytesRef,
    first_range_end: BytesRef,
    second_range_beginning: BytesRef,
    second_range_end: BytesRef,
}

thread_local! {
    static KA: RefCell<Option<KeyAnalyzerTest>> = const { RefCell::new(None) };
}

/// The field initializers and `setUp()`.
fn ka_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    let collator: Arc<dyn Collator> = Arc::from(collator::get_instance_locale(Some(
        &Locale::builder_from_language_tag("fa")?,
    ))?);
    let base = CollationTestBase::default();
    let state = KeyAnalyzerTest {
        first_range_beginning: key_bytes(&*collator, &base.first_range_beginning_original),
        first_range_end: key_bytes(&*collator, &base.first_range_end_original),
        second_range_beginning: key_bytes(&*collator, &base.second_range_beginning_original),
        second_range_end: key_bytes(&*collator, &base.second_range_end_original),
        analyzer: Arc::new(ICUCollationKeyAnalyzer::new(Arc::clone(&collator))),
        base,
    };
    KA.with(|ka| *ka.borrow_mut() = Some(state));
    Ok(())
}

fn ka_tear_down() -> Result<()> {
    if let Some(state) = KA.with(|ka| ka.borrow_mut().take()) {
        state.analyzer.close();
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}

fn with_state<R>(f: impl FnOnce(&KeyAnalyzerTest) -> R) -> R {
    KA.with(|ka| f(ka.borrow().as_ref().expect("setUp")))
}

fn ka_test_farsi_range_filter_collating() -> Result<()> {
    with_state(|s| {
        s.base.test_farsi_range_filter_collating(
            Arc::clone(&s.analyzer),
            &s.first_range_beginning,
            &s.first_range_end,
            &s.second_range_beginning,
            &s.second_range_end,
        )
    })
}

fn ka_test_farsi_range_query_collating() -> Result<()> {
    with_state(|s| {
        s.base.test_farsi_range_query_collating(
            Arc::clone(&s.analyzer),
            &s.first_range_beginning,
            &s.first_range_end,
            &s.second_range_beginning,
            &s.second_range_end,
        )
    })
}

fn ka_test_farsi_term_range_query() -> Result<()> {
    with_state(|s| {
        s.base.test_farsi_term_range_query(
            Arc::clone(&s.analyzer),
            &s.first_range_beginning,
            &s.first_range_end,
            &s.second_range_beginning,
            &s.second_range_end,
        )
    })
}

fn ka_test_thread_safe() -> Result<()> {
    let iters = 20 * *RANDOM_MULTIPLIER;
    for _ in 0..iters {
        let locale = Locale::of("de", "", ""); // Locale.GERMAN
        let mut collator = collator::get_instance_locale(Some(&locale))?;
        collator.set_strength(IDENTICAL)?;
        let a: Arc<dyn Analyzer> = Arc::new(ICUCollationKeyAnalyzer::new(Arc::from(collator)));
        with_state(|s| s.base.assert_thread_safe(Arc::clone(&a)))?;
        a.close();
    }
    Ok(())
}

fn collation_key_analyzer_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.icu.TestICUCollationKeyAnalyzer")
        .method(hook("setUp", ka_set_up))
        .method(hook("tearDown", ka_tear_down))
        .method(t(
            "testFarsiRangeFilterCollating",
            ka_test_farsi_range_filter_collating,
        ))
        .method(t(
            "testFarsiRangeQueryCollating",
            ka_test_farsi_range_query_collating,
        ))
        .method(t("testFarsiTermRangeQuery", ka_test_farsi_term_range_query))
        .method(t("testThreadSafe", ka_test_thread_safe))
        .build()
}

// -----------------------------------------------------------------------------
// Suites
// -----------------------------------------------------------------------------

#[test]
fn test_icu_collation_doc_values_field() {
    run(collation_doc_values_field_class);
}

#[test]
fn test_icu_collation_key_analyzer() {
    run(collation_key_analyzer_class);
}
