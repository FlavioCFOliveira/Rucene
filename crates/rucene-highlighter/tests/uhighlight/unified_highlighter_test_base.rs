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

//! Port of `UnifiedHighlighterTestBase.java` (the parent class of the unified
//! highlighter tests), with the static randomized-highlighter helpers of
//! `TestUnifiedHighlighter` that the other test classes call
//! (`randomUnifiedHighlighter`, `overriddenBuilderForTests`, …).

use std::any::Any;
use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use randomizedtesting::annotations::ParametersFactory;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method, Value};
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::{Document, Field, FieldType, TextField};
use rucene::error::{LuceneError, Result};
use rucene::index::{IndexOptions, IndexableField};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::sort::{Sort, SortField};
use rucene::search::TopDocs;
use rucene_highlighter::uhighlight::uh_components::FieldMatcher;
use rucene_highlighter::uhighlight::unified_highlighter::{
    Builder, HighlightFlag, UnifiedHighlighter,
};
use rucene_test_framework::analysis::mock_tokenizer::SIMPLE;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::highlighter_support::core_support::random;
use crate::highlighter_support::search_support::term;
use crate::highlighter_support::ti;

/// `FieldType` of `type` frozen behind an `Arc`, built once.
fn frozen(cell: &'static OnceLock<Arc<FieldType>>, init: fn(&FieldType)) -> Arc<FieldType> {
    Arc::clone(cell.get_or_init(|| {
        let ft = FieldType::new_from(TextField::type_stored());
        init(&ft);
        ft.freeze();
        Arc::new(ft)
    }))
}

const SETTER: &str = "INVARIANT: an unfrozen FieldType accepts the setter";

/// `postingsType`.
pub fn postings_type() -> Arc<FieldType> {
    static CELL: OnceLock<Arc<FieldType>> = OnceLock::new();
    frozen(&CELL, |ft| {
        ft.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
            .expect(SETTER);
    })
}

/// `tvType`.
pub fn tv_type() -> Arc<FieldType> {
    static CELL: OnceLock<Arc<FieldType>> = OnceLock::new();
    frozen(&CELL, |ft| {
        ft.set_store_term_vectors(true).expect(SETTER);
        ft.set_store_term_vector_positions(true).expect(SETTER);
        ft.set_store_term_vector_offsets(true).expect(SETTER);
    })
}

/// `postingsWithTvType`.
pub fn postings_with_tv_type() -> Arc<FieldType> {
    static CELL: OnceLock<Arc<FieldType>> = OnceLock::new();
    frozen(&CELL, |ft| {
        ft.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
            .expect(SETTER);
        ft.set_store_term_vectors(true).expect(SETTER);
    })
}

/// `reanalysisType`: `TextField.TYPE_STORED` itself (one shared instance,
/// compared by identity).
pub fn reanalysis_type() -> Arc<FieldType> {
    static CELL: OnceLock<Arc<FieldType>> = OnceLock::new();
    Arc::clone(CELL.get_or_init(|| {
        let ft = FieldType::new_from(TextField::type_stored());
        ft.freeze();
        Arc::new(ft)
    }))
}

/// `randomFieldType(random, typePossibilities...)`: an empty slice is Java's
/// empty (or `null`) varargs.
pub fn random_field_type(random: &dyn Random, type_possibilities: &[Arc<FieldType>]) -> Arc<FieldType> {
    let defaults;
    let possibilities = if type_possibilities.is_empty() {
        defaults = [
            postings_type(),
            tv_type(),
            postings_with_tv_type(),
            reanalysis_type(),
        ];
        &defaults[..]
    } else {
        type_possibilities
    };
    Arc::clone(&possibilities[random.next_int_bound(possibilities.len() as i32) as usize])
}

/// The fields of a `UnifiedHighlighterTestBase` instance.
pub struct UHBase {
    /// Randomized field type for the "body" field, generally.
    pub field_type: Arc<FieldType>,
    pub index_analyzer: Option<Arc<MockAnalyzer>>,
    pub dir: Option<TestDirectory>,
}

impl UHBase {
    /// `UnifiedHighlighterTestBase(FieldType fieldType)`.
    pub fn new(field_type: Arc<FieldType>) -> Self {
        Self {
            field_type,
            index_analyzer: None,
            dir: None,
        }
    }

    /// `indexAnalyzer`, as an `Analyzer`.
    pub fn analyzer(&self) -> Arc<dyn Analyzer> {
        let a = Arc::clone(self.index_analyzer.as_ref().expect("setUp ran"));
        a
    }

    /// `indexAnalyzer`.
    pub fn mock(&self) -> &MockAnalyzer {
        self.index_analyzer.as_ref().expect("setUp ran")
    }

    /// `dir`.
    pub fn dir(&self) -> &TestDirectory {
        self.dir.as_ref().expect("setUp ran")
    }

    /// `fieldType == reanalysisType`.
    pub fn is_reanalysis(&self) -> bool {
        Arc::ptr_eq(&self.field_type, &reanalysis_type())
    }

    /// `newIndexOrderPreservingWriter()`.
    pub fn writer(&self) -> Result<RandomIndexWriter> {
        new_index_order_preserving_writer(self.dir(), self.analyzer())
    }

    /// `new Field(name, value, fieldType)`, shareable by documents.
    pub fn field(&self, name: &str, value: &str) -> Result<Arc<Field>> {
        field(name, value, &self.field_type)
    }
}

/// `new Field(name, value, type)`, shareable by documents.
pub fn field(name: &str, value: &str, field_type: &Arc<FieldType>) -> Result<Arc<Field>> {
    Ok(Arc::new(Field::new(
        name.to_string(),
        value.to_string(),
        Arc::clone(field_type),
    )?))
}

/// `doc.add(field)` of a field a test changes later (`setStringValue`).
pub fn add(doc: &mut Document, field: &Arc<Field>) {
    let shared: Arc<dyn IndexableField> = Arc::clone(field) as Arc<dyn IndexableField>;
    doc.add_shared(shared);
}

/// `field.setStringValue(value)`.
pub fn set(field: &Arc<Field>, value: &str) -> Result<()> {
    field.set_string_value(value.to_string())
}

/// `newIndexOrderPreservingWriter(dir, indexAnalyzer)`: a
/// `RandomIndexWriter` that avoids the random merge policy, which may reorder
/// documents (which assertions rely on).
pub fn new_index_order_preserving_writer(
    dir: &TestDirectory,
    index_analyzer: Arc<dyn Analyzer>,
) -> Result<RandomIndexWriter> {
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(index_analyzer));
    config.set_merge_policy(Some(LuceneTestCase::new_merge_policy_include_mock(
        &*random(),
        false,
    )))?;
    RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)
}

/// `UnifiedHighlighterTestBase.setUp()`.
pub fn base_set_up(base: &mut UHBase) -> Result<()> {
    LuceneTestCase::super_set_up();
    base.dir = Some(LuceneTestCase::new_directory());
    base.index_analyzer = Some(Arc::new(MockAnalyzer::with_automaton(
        &*random(),
        Arc::clone(&SIMPLE),
        true,
    )));
    Ok(())
}

/// `UnifiedHighlighterTestBase.tearDown()`.
pub fn base_tear_down(base: &mut UHBase) -> Result<()> {
    let closed = match base.dir.take() {
        Some(dir) => dir.close(),
        None => Ok(()),
    };
    closed?;
    LuceneTestCase::super_tear_down();
    Ok(())
}

/// A `UnifiedHighlighterTestBase` subclass instance.
pub trait UHCase: Any + Send {
    fn base(&self) -> &UHBase;
    fn base_mut(&mut self) -> &mut UHBase;

    /// `setUp()`.
    fn set_up(&mut self) -> Result<()> {
        base_set_up(self.base_mut())
    }

    /// `tearDown()`.
    fn tear_down(&mut self) -> Result<()> {
        base_tear_down(self.base_mut())
    }
}

/// The plain instance of a subclass that adds no field.
pub struct UHTest {
    pub b: UHBase,
}

impl UHCase for UHTest {
    fn base(&self) -> &UHBase {
        &self.b
    }

    fn base_mut(&mut self) -> &mut UHBase {
        &mut self.b
    }
}

impl std::ops::Deref for UHTest {
    type Target = UHBase;

    fn deref(&self) -> &UHBase {
        &self.b
    }
}

/// The concrete class `name` (`extends UnifiedHighlighterTestBase`) built by
/// `constructor`, with its `setUp`/`tearDown` and `methods`.
pub fn uh_class<T: UHCase>(name: &str, constructor: Constructor, methods: Vec<Method>) -> Arc<Class> {
    let mut builder = Class::builder(name)
        .extends(&LuceneTestCase::class())
        .constructor(constructor)
        .method(ti::<T>("setUp", T::set_up))
        .method(ti::<T>("tearDown", T::tear_down));
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

/// The constructor `Test…(@Name("fieldType") FieldType fieldType)` of a
/// parameterized subclass.
pub fn parameterized_constructor<T: UHCase>(make: fn(Arc<FieldType>) -> T) -> Constructor {
    Constructor::with_params(
        &[("org.apache.lucene.document.FieldType", Some("fieldType"))],
        move |args: &[Value]| match &args[0] {
            Value::Object(o, _) => make(Arc::clone(
                o.downcast_ref::<Arc<FieldType>>()
                    .expect("a FieldType argument"),
            )),
            _ => panic!("java.lang.IllegalArgumentException: argument type mismatch"),
        },
    )
}

/// The no-argument constructor `Test…() { super(randomFieldType(random())); }`.
pub fn random_type_constructor<T: UHCase>(make: fn(Arc<FieldType>) -> T) -> Constructor {
    Constructor::no_args(move || make(random_field_type(&*random(), &[])))
}

fn ft_value(ft: Arc<FieldType>) -> Value {
    let name = ft.to_string();
    Value::Object(Arc::new(ft), name)
}

/// `@ParametersFactory public static Iterable<Object[]> parameters() {
/// return parametersFactoryList(); }`.
pub fn parameters_method() -> Method {
    Method::new("parameters")
        .modifiers(modifier::PUBLIC | modifier::STATIC)
        .returns("java.lang.Iterable<java.lang.Object[]>")
        .annotate(ParametersFactory::default())
        .static_value(|_| {
            Value::List(
                [
                    postings_type(),
                    tv_type(),
                    postings_with_tv_type(),
                    reanalysis_type(),
                ]
                .into_iter()
                .map(|ft| Value::Array(vec![ft_value(ft)]))
                .collect(),
            )
        })
}

/// `Sort.INDEXORDER`.
pub fn index_order() -> Sort {
    Sort::new_fields(vec![SortField::FIELD_DOC.clone()]).expect("INVARIANT: one sort field")
}

/// `searcher.search(query, n, Sort.INDEXORDER)`.
pub fn search_index_order(searcher: &IndexSearcher, query: &Arc<dyn Query>, n: i32) -> Result<TopDocs> {
    Ok(searcher
        .search_with_sort(Arc::clone(query), n, &index_order())?
        .to_top_docs())
}

/// `new TermQuery(new Term(field, text))`.
pub fn tq(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(rucene::search::TermQuery::new(term(field, text)))
}

/// `assertEquals(expected, snippet)` of a `String` snippet that may be null.
pub fn assert_snippet(expected: &str, actual: &Option<String>) {
    Assert::assert_equals_str(None, Some(expected), actual.as_deref());
}

/// `assertEquals(message, expected, snippet)`.
pub fn assert_snippet_msg(message: &str, expected: &str, actual: &Option<String>) {
    Assert::assert_equals_str(Some(message), Some(expected), actual.as_deref());
}

/// `assertNull(snippet)`.
pub fn assert_null_snippet(actual: &Option<String>) {
    if let Some(s) = actual {
        Assert::fail(&format!("expected null, but was:<{s}>"));
    }
}

/// `snippet` dereferenced, as Java does on a call on it.
pub fn s(actual: &Option<String>) -> &str {
    actual
        .as_deref()
        .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
}

/// `assertArrayEquals(expected, snippets)` of `String[]` snippets.
pub fn assert_snippets(expected: &[&str], actual: &[Option<String>]) {
    let expected: Vec<Option<String>> = expected.iter().map(|e| Some(e.to_string())).collect();
    if expected.as_slice() != actual {
        Assert::fail(&format!(
            "arrays differ: expected:<{expected:?}> but was:<{actual:?}>"
        ));
    }
}

/// `snippets.get(field)` of a highlight map (`null` when absent).
pub fn get<'a>(
    snippets: &'a std::collections::HashMap<String, Vec<Option<String>>>,
    field: &str,
) -> &'a [Option<String>] {
    snippets
        .get(field)
        .map(Vec::as_slice)
        .unwrap_or_else(|| panic!("java.lang.NullPointerException: no snippets of {field}"))
}

/// `generateRandomHighlightFlags(requiredFlags)`.
pub fn generate_random_highlight_flags(required_flags: &[HighlightFlag]) -> HashSet<HighlightFlag> {
    let mut result: HashSet<HighlightFlag> = required_flags.iter().copied().collect();
    let r = random().next_int();
    for (ordinal, highlight_flag) in [
        HighlightFlag::PHRASES,
        HighlightFlag::MULTI_TERM_QUERY,
        HighlightFlag::PASSAGE_RELEVANCY_OVER_SPEED,
        HighlightFlag::WEIGHT_MATCHES,
    ]
    .into_iter()
    .enumerate()
    {
        if ((1 << ordinal) & r) == 0 {
            result.insert(highlight_flag);
        }
    }
    if result.contains(&HighlightFlag::WEIGHT_MATCHES) {
        // these two are required for WEIGHT_MATCHES
        result.insert(HighlightFlag::MULTI_TERM_QUERY);
        result.insert(HighlightFlag::PHRASES);
    }
    result
}

/// `randomUnifiedHighlighter(searcher, indexAnalyzer)`: this randomized
/// test method uses builder from the UH class.
pub fn random_unified_highlighter(
    searcher: &Arc<IndexSearcher>,
    index_analyzer: Arc<dyn Analyzer>,
) -> UnifiedHighlighter {
    random_unified_highlighter_flags(searcher, index_analyzer, &[], None)
}

/// `randomUnifiedHighlighter(searcher, indexAnalyzer, mandatoryFlags,
/// requireFieldMatch)`.
pub fn random_unified_highlighter_flags(
    searcher: &Arc<IndexSearcher>,
    index_analyzer: Arc<dyn Analyzer>,
    mandatory_flags: &[HighlightFlag],
    require_field_match: Option<bool>,
) -> UnifiedHighlighter {
    let uh_builder = Builder::new(Some(Arc::clone(searcher)), index_analyzer);
    random_uh_from_builder_flags(uh_builder, mandatory_flags, require_field_match)
}

/// `randomUnifiedHighlighter(uhBuilder)`.
pub fn random_uh_from_builder(uh_builder: Builder) -> UnifiedHighlighter {
    random_uh_from_builder_flags(uh_builder, &[], None)
}

/// `randomUnifiedHighlighter(uhBuilder, mandatoryFlags, requireFieldMatch)`.
pub fn random_uh_from_builder_flags(
    mut uh_builder: Builder,
    mandatory_flags: &[HighlightFlag],
    require_field_match: Option<bool>,
) -> UnifiedHighlighter {
    uh_builder = uh_builder.with_cache_field_val_chars_threshold(random().next_int_bound(100));
    if require_field_match == Some(false)
        || (require_field_match.is_none() && random().next_boolean())
    {
        let matcher: FieldMatcher = Arc::new(|_f: &str| true);
        uh_builder = uh_builder.with_field_matcher(Some(matcher)); // requireFieldMatch==false
    }
    overridden_builder_for_tests(uh_builder, mandatory_flags).build()
}

/// `overrideFieldMatcherForTests(original, value, fieldName)`.
pub fn override_field_matcher_for_tests(
    original: &UnifiedHighlighter,
    value: Option<FieldMatcher>,
    field_name: &str,
) -> UnifiedHighlighter {
    UnifiedHighlighter::builder(
        Arc::clone(original.get_index_searcher().expect("a searcher")),
        Arc::clone(original.get_index_analyzer()),
    )
    .with_flags(original.get_flags(field_name))
    .with_cache_field_val_chars_threshold(original.get_cache_field_val_chars_threshold())
    .with_field_matcher(value)
    .build()
}

/// The anonymous `UnifiedHighlighter.Builder` of
/// `overriddenBuilderForTests(uhBuilder, mandatoryFlags)`: its `build()`
/// returns `new UnifiedHighlighter(uhBuilder)` (of the *given* builder) whose
/// `evaluateFlags(Builder)` returns the flags drawn once by
/// `generateRandomHighlightFlags(mandatoryFlags)`.
pub struct OverriddenBuilder {
    uh_builder: Builder,
    mandatory_flags: Vec<HighlightFlag>,
    flags: Option<HashSet<HighlightFlag>>,
}

/// `overriddenBuilderForTests(uhBuilder, mandatoryFlags)`.
pub fn overridden_builder_for_tests(
    uh_builder: Builder,
    mandatory_flags: &[HighlightFlag],
) -> OverriddenBuilder {
    OverriddenBuilder {
        uh_builder,
        mandatory_flags: mandatory_flags.to_vec(),
        flags: None,
    }
}

impl OverriddenBuilder {
    /// `build()`: the overridden `evaluateFlags` returns the drawn flags,
    /// which a builder's `withFlags` gives `new UnifiedHighlighter(uhBuilder)`
    /// as well.
    pub fn build(mut self) -> UnifiedHighlighter {
        let flags = match &self.flags {
            Some(flags) => flags.clone(),
            None => {
                let flags = generate_random_highlight_flags(&self.mandatory_flags);
                self.flags = Some(flags.clone());
                flags
            }
        };
        self.uh_builder.with_flags(flags).build()
    }
}

/// `CharSequence` values of `loadFieldValues`, from a `String`.
pub fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// `fail` with an `AssertionError`.
pub fn assertion_error(message: &str) -> LuceneError {
    crate::highlighter_support::core_support::assertion_error(message)
}
