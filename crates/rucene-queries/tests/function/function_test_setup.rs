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

//! Port of `FunctionTestSetup.java`: setup for function tests.
//!
//! The class is declared by [`class()`] (its `@AfterClass` hook); its static
//! fields `dir` and `anlzr` are [`STATE`], its protected instance fields
//! (the value sources) are functions, as they read no instance state.

#![allow(dead_code)]

use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use randomizedtesting::junit::AfterClass;
use rucene::analysis::Analyzer;
use rucene::document::{
    Document, FieldType, NumericDocValuesField, NumericValue, SortedDocValuesField,
    SortedNumericDocValuesField, StoredField, TextField,
};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::search::sorted_numeric_selector::SortedNumericSelectorType;
use rucene::store::Directory;
use rucene::util::{BytesRef, NumericUtils};
use rucene_queries::function::value_source::ValueSource;
use rucene_queries::function::valuesource::float_field_source::FloatFieldSource;
use rucene_queries::function::valuesource::int_field_source::IntFieldSource;
use rucene_queries::function::valuesource::multi_valued_float_field_source::MultiValuedFloatFieldSource;
use rucene_queries::function::valuesource::multi_valued_int_field_source::MultiValuedIntFieldSource;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::queries_support::core_support::{random, verbose};

/// Actual score computation order is slightly different than assumptios
/// this allows for a small amount of variation.
pub const TEST_SCORE_TOLERANCE_DELTA: f32 = 0.001;

/// `N_DOCS`: select a primary number > 2.
pub const N_DOCS: i32 = 17;
/// `ID_FIELD`.
pub const ID_FIELD: &str = "id";
/// `TEXT_FIELD`.
pub const TEXT_FIELD: &str = "text";
/// `INT_FIELD`.
pub const INT_FIELD: &str = "iii";
/// This field is multiValued and should give the exact same results as
/// [`INT_FIELD`] when used with MIN selector.
pub const INT_FIELD_MV_MIN: &str = "iii_min";
/// This field is multiValued and should give the exact same results as
/// [`INT_FIELD`] when used with MAX selector.
pub const INT_FIELD_MV_MAX: &str = "iii_max";
/// `FLOAT_FIELD`.
pub const FLOAT_FIELD: &str = "fff";
/// This field is multiValued and should give the exact same results as
/// [`FLOAT_FIELD`] when used with MIN selector.
pub const FLOAT_FIELD_MV_MIN: &str = "fff_min";
/// This field is multiValued and should give the exact same results as
/// [`FLOAT_FIELD`] when used with MAX selector.
pub const FLOAT_FIELD_MV_MAX: &str = "fff_max";

/// `INT_VALUESOURCE`.
pub fn int_valuesource() -> Arc<dyn ValueSource> {
    Arc::new(IntFieldSource::new(INT_FIELD))
}

/// `INT_MV_MIN_VALUESOURCE`.
pub fn int_mv_min_valuesource() -> Arc<dyn ValueSource> {
    Arc::new(MultiValuedIntFieldSource::new(
        INT_FIELD_MV_MIN,
        SortedNumericSelectorType::MIN,
    ))
}

/// `INT_MV_MAX_VALUESOURCE`.
pub fn int_mv_max_valuesource() -> Arc<dyn ValueSource> {
    Arc::new(MultiValuedIntFieldSource::new(
        INT_FIELD_MV_MAX,
        SortedNumericSelectorType::MAX,
    ))
}

/// `FLOAT_VALUESOURCE`.
pub fn float_valuesource() -> Arc<dyn ValueSource> {
    Arc::new(FloatFieldSource::new(FLOAT_FIELD))
}

/// `FLOAT_MV_MIN_VALUESOURCE`.
pub fn float_mv_min_valuesource() -> Arc<dyn ValueSource> {
    Arc::new(MultiValuedFloatFieldSource::new(
        FLOAT_FIELD_MV_MIN,
        SortedNumericSelectorType::MIN,
    ))
}

/// `FLOAT_MV_MAX_VALUESOURCE`.
pub fn float_mv_max_valuesource() -> Arc<dyn ValueSource> {
    Arc::new(MultiValuedFloatFieldSource::new(
        FLOAT_FIELD_MV_MAX,
        SortedNumericSelectorType::MAX,
    ))
}

/// `DOC_TEXT_LINES`.
const DOC_TEXT_LINES: [&str; 14] = [
    "Well, this is just some plain text we use for creating the ",
    "test documents. It used to be a text from an online collection ",
    "devoted to first aid, but if there was there an (online) lawyers ",
    "first aid collection with legal advices, \"it\" might have quite ",
    "probably advised one not to include \"it\"'s text or the text of ",
    "any other online collection in one's code, unless one has money ",
    "that one don't need and one is happy to donate for lawyers ",
    "charity. Anyhow at some point, rechecking the usage of this text, ",
    "it became uncertain that this text is free to use, because ",
    "the web site in the disclaimer of he eBook containing that text ",
    "was not responding anymore, and at the same time, in projGut, ",
    "searching for first aid no longer found that eBook as well. ",
    "So here we are, with a perhaps much less interesting ",
    "text for the test, but oh much much safer. ",
];

/// The static fields `dir` and `anlzr`.
pub struct FunctionStatics {
    /// `dir`.
    pub dir: TestDirectory,
    /// `anlzr`.
    pub anlzr: Arc<dyn Analyzer>,
}

/// The static state of the class.
pub static STATE: Mutex<Option<FunctionStatics>> = Mutex::new(None);

fn state() -> MutexGuard<'static, Option<FunctionStatics>> {
    STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `dir`.
pub fn dir() -> Arc<dyn Directory> {
    state()
        .as_ref()
        .expect("createIndex ran")
        .dir
        .as_directory()
}

/// The declaration of `@Ignore public abstract class FunctionTestSetup
/// extends LuceneTestCase`, with its `@AfterClass` hook.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.queries.function.FunctionTestSetup")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&LuceneTestCase::class())
            .method(
                Method::new("afterClassFunctionTestSetup")
                    .modifiers(modifier::PUBLIC | modifier::STATIC)
                    .throws(&["java.lang.Exception"])
                    .annotate(AfterClass)
                    .static_body(|| {
                        after_class_function_test_setup().unwrap_or_else(|e| rethrow_java(e))
                    }),
            )
            .build()
    });
    Arc::clone(&CLASS)
}

/// A subclass `name` of `FunctionTestSetup` with `methods`.
pub fn subclass(name: &str, methods: Vec<Method>) -> Arc<Class> {
    let mut builder = Class::builder(name)
        .extends(&class())
        .constructor(Constructor::no_args(|| ()));
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

/// `afterClassFunctionTestSetup()`.
pub fn after_class_function_test_setup() -> Result<()> {
    if let Some(FunctionStatics { dir, anlzr }) = state().take() {
        dir.close()?;
        anlzr.close();
    }
    Ok(())
}

/// `createIndex(boolean doMultiSegment)`.
pub fn create_index(do_multi_segment: bool) -> Result<()> {
    if verbose() {
        println!("TEST: setUp");
    }
    let dir = LuceneTestCase::new_directory();
    let anlzr: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&anlzr)));
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    if do_multi_segment {
        iwc.set_max_buffered_docs(TestUtil::next_int(&*random(), 2, 7))?;
    }
    let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
    let mut remaining = N_DOCS;
    let mut done = [false; N_DOCS as usize];
    let mut i = 0usize;
    while remaining > 0 {
        if done[i] {
            return Err(LuceneError::from(LuceneErrorKind::Unchecked {
                class: "java.lang.Exception",
                message: Some(format!(
                    "to set this test correctly N_DOCS={N_DOCS} must be primary and greater than 2!"
                )),
                cause: None,
            }));
        }
        add_doc(&iw, i as i32)?;
        done[i] = true;
        i = (i + 4) % N_DOCS as usize;
        remaining -= 1;
    }
    if !do_multi_segment {
        if verbose() {
            println!("TEST: setUp full merge");
        }
        iw.force_merge(1)?;
    }
    iw.close()?;
    if verbose() {
        println!("TEST: setUp done close");
    }
    *state() = Some(FunctionStatics { dir, anlzr });
    Ok(())
}

fn stored_int(name: &str, value: i32) -> Result<StoredField> {
    StoredField::new_number(name, NumericValue::Int(value))
}

fn add_doc(iw: &RandomIndexWriter, i: i32) -> Result<()> {
    let mut d = Document::new();
    let score_and_id = i + 1;

    let custom_type = FieldType::new_from(TextField::type_stored());
    custom_type.set_tokenized(false)?;
    custom_type.set_omit_norms(true)?;

    let f = LuceneTestCase::new_field(ID_FIELD, &id2_string(score_and_id), &custom_type); // for debug purposes
    d.add(f);
    d.add(SortedDocValuesField::new(
        ID_FIELD,
        BytesRef::from_char_sequence(&id2_string(score_and_id)),
    ));

    let custom_type2 = FieldType::new_from(TextField::type_not_stored());
    custom_type2.set_omit_norms(true)?;
    let f = LuceneTestCase::new_field(
        TEXT_FIELD,
        &format!("text of doc{score_and_id}{}", text_line(i)),
        &custom_type2,
    ); // for regular search
    d.add(f);

    d.add(stored_int(INT_FIELD, score_and_id)?); // for function scoring
    d.add(NumericDocValuesField::new(
        INT_FIELD,
        i64::from(score_and_id),
    ));

    d.add(stored_int(FLOAT_FIELD, score_and_id)?); // for function scoring
    d.add(NumericDocValuesField::new(
        FLOAT_FIELD,
        i64::from((score_and_id as f32).to_bits() as i32),
    ));

    d.add(stored_int(INT_FIELD_MV_MIN, score_and_id)?);
    d.add(stored_int(INT_FIELD_MV_MIN, score_and_id + 1)?);
    d.add(SortedNumericDocValuesField::new(
        INT_FIELD_MV_MIN,
        i64::from(score_and_id),
    ));
    d.add(SortedNumericDocValuesField::new(
        INT_FIELD_MV_MIN,
        i64::from(score_and_id + 1),
    ));

    d.add(stored_int(INT_FIELD_MV_MAX, score_and_id)?);
    d.add(stored_int(INT_FIELD_MV_MAX, score_and_id - 1)?);
    d.add(SortedNumericDocValuesField::new(
        INT_FIELD_MV_MAX,
        i64::from(score_and_id),
    ));
    d.add(SortedNumericDocValuesField::new(
        INT_FIELD_MV_MAX,
        i64::from(score_and_id - 1),
    ));

    d.add(stored_int(FLOAT_FIELD_MV_MIN, score_and_id)?);
    d.add(stored_int(FLOAT_FIELD_MV_MIN, score_and_id + 1)?);
    d.add(SortedNumericDocValuesField::new(
        FLOAT_FIELD_MV_MIN,
        i64::from(NumericUtils::float_to_sortable_int(score_and_id as f32)),
    ));
    d.add(SortedNumericDocValuesField::new(
        FLOAT_FIELD_MV_MIN,
        i64::from(NumericUtils::float_to_sortable_int(
            (score_and_id + 1) as f32,
        )),
    ));

    d.add(stored_int(FLOAT_FIELD_MV_MAX, score_and_id)?);
    d.add(stored_int(FLOAT_FIELD_MV_MAX, score_and_id - 1)?);
    d.add(SortedNumericDocValuesField::new(
        FLOAT_FIELD_MV_MAX,
        i64::from(NumericUtils::float_to_sortable_int(score_and_id as f32)),
    ));
    d.add(SortedNumericDocValuesField::new(
        FLOAT_FIELD_MV_MAX,
        i64::from(NumericUtils::float_to_sortable_int(
            (score_and_id - 1) as f32,
        )),
    ));

    iw.add_document(&d)?;
    log(|| format!("added: {d:?}"));
    Ok(())
}

/// `id2String(int scoreAndID)`.
pub fn id2_string(score_and_id: i32) -> String {
    let s = format!("000000000{score_and_id}");
    let n = N_DOCS.to_string().len() + 3;
    let k = s.len() - n;
    format!("ID{}", &s[k..])
}

fn text_line(doc_num: i32) -> &'static str {
    DOC_TEXT_LINES[doc_num as usize % DOC_TEXT_LINES.len()]
}

/// `expectedFieldScore(String docIDFieldVal)`: `"ID7"` → `7.0`.
pub fn expected_field_score(doc_id_field_val: &str) -> f32 {
    randomizedtesting::jdk::lang::parse_float(&doc_id_field_val[2..]).unwrap_or_else(|| {
        randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.NumberFormatException",
            format!("For input string: \"{}\"", &doc_id_field_val[2..]),
        )
        .throw()
    })
}

/// `log(Object o)`: printed when `VERBOSE`.
pub fn log(o: impl FnOnce() -> String) {
    if verbose() {
        println!("{}", o());
    }
}
