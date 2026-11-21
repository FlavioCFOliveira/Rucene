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

//! Port of the test classes of Lucene 10.5.0
//! `lucene/analysis/common/src/test/org/apache/lucene/analysis/hunspell`
//! that rmp tasks #462 (`TestAffixCondition`) and #463 (the other 46) list,
//! each run as a suite by the ported runner. The dictionaries and their
//! `.good`/`.wrong`/`.sug` files are copied byte for byte to
//! `tests/resources/org/apache/lucene/analysis/hunspell/`; the tests read them
//! from there as Java reads them from the test class path
//! (`getResourceAsStream`, `Path.of(getResource(name).toURI())`).
//!
//! * `StemmerTestBase` is the abstract base of the stemming suites: its
//!   static `stemmer` is [`STEMMER`], `init` and `assertStemsTo` are
//!   [`init`] and [`assert_stems_to`].
//! * `TestBaseUtf.good()` is neither `public void test*()` nor `@Test`: the
//!   runner does not run it, here as in Java.
//! * `TestAllDictionaries` and `TestPerformance` read the dictionaries (and
//!   corpora) named by the system properties `hunspell.dictionaries` and
//!   `hunspell.corpora`, assuming them away when unset, as Lucene does
//!   (`testMaxPrologueNeeded` is `@Ignore`d in Lucene). The memory summary
//!   `TestAllDictionaries` prints through `RamUsageTester` is not printed:
//!   the ported `RamUsageTester` has no object model of `Dictionary`.
//! * `TestHunspellRepositoryTestCases` is a JUnit `Parameterized` class over
//!   the checked-out Hunspell repository (`hunspell.repo.path`); its
//!   `data()` and `test()` are driven by [`test_hunspell_repository_test_cases`]
//!   as `Parameterized` drives them.

mod support;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{AfterClass, Assert, Assume, BeforeClass, Ignore, Test};
use rucene::analysis::{Analyzer, CharArraySet};
use rucene::error::{LuceneError, Result};
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::extra::ParseException;
use rucene::util::IntsRef;
use rucene_analysis_common::analysis::hunspell::affix_condition::{
    always_false, compile, unique_key, ALWAYS_TRUE_KEY,
};
use rucene_analysis_common::analysis::hunspell::affix_kind::AffixKind;
use rucene_analysis_common::analysis::hunspell::affixed_word::AffixedWord;
use rucene_analysis_common::analysis::hunspell::conv_table::ConvTable;
use rucene_analysis_common::analysis::hunspell::dictionary::{
    extract_language_code, get_flag_parsing_strategy, Dictionary, DictionarySubclass, FLAG_UNSET,
    MAX_PROLOGUE_SCAN_WINDOW,
};
use rucene_analysis_common::analysis::hunspell::fragment_checker::FragmentChecker;
use rucene_analysis_common::analysis::hunspell::generating_suggester::ngram_score;
use rucene_analysis_common::analysis::hunspell::hunspell::{
    no_check_canceled, CheckCanceled, Hunspell, SUGGEST_TIME_LIMIT,
};
use rucene_analysis_common::analysis::hunspell::hunspell_stem_filter::{
    new_hunspell_stem_filter, new_hunspell_stem_filter_with,
};
use rucene_analysis_common::analysis::hunspell::ngram_fragment_checker::NGramFragmentChecker;
use rucene_analysis_common::analysis::hunspell::sorting_strategy::SortingStrategy;
use rucene_analysis_common::analysis::hunspell::stemmer::Stemmer;
use rucene_analysis_common::analysis::hunspell::suggester::Suggester;
use rucene_analysis_common::analysis::hunspell::suggestible_entry_cache::SuggestibleEntryCache;
use rucene_analysis_common::analysis::hunspell::suggestion_timeout_exception::SuggestionTimeoutException;
use rucene_analysis_common::analysis::hunspell::timeout_policy::TimeoutPolicy;
use rucene_analysis_common::analysis::hunspell::trigram_automaton::TrigramAutomaton;
use rucene_analysis_common::analysis::hunspell::word_form_generator::WordFormGenerator;
use rucene_analysis_common::analysis::miscellaneous::set_keyword_marker_filter::new_set_keyword_marker_filter;
use rucene_analysis_common::jdk::character::{is_letter, is_whitespace};
use rucene_analysis_common::jdk::charset::Charset;
use rucene_analysis_common::jdk::io::{BufferedReader, InputStreamReader};
use rucene_analysis_common::jdk::string::{to_rust_string, units};
use rucene_test_framework::analysis::mock_tokenizer::MockTokenizer;
use rucene_test_framework::analysis::BaseTokenStreamTestCase;
use rucene_test_framework::util::java_exceptions::{expect_throws, rethrow_java};
use rucene_test_framework::util::lucene_test_case::RANDOM_MULTIPLIER;
use rucene_test_framework::util::LuceneTestCase;
use support::{
    boxed, btstc_class, factory_class, lucene_class, t, token_filter_factory, tokenizer_analyzer,
};

type B = BaseTokenStreamTestCase;

/// The package of the test classes, where their resources are.
const PACKAGE: &str = "org.apache.lucene.analysis.hunspell";

// -----------------------------------------------------------------------------
// Resources
// -----------------------------------------------------------------------------

/// The file of the test resource `name` of the package.
fn resource_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/resources")
        .join(PACKAGE.replace('.', "/"))
        .join(name)
}

/// `getClass().getResourceAsStream(name)`: `None` for a missing resource.
fn resource_stream(name: &str) -> Option<Cursor<Vec<u8>>> {
    std::fs::read(resource_path(name)).ok().map(Cursor::new)
}

/// `new FileNotFoundException(message)`.
fn file_not_found(message: String) -> LuceneError {
    LuceneError::from(std::io::Error::new(std::io::ErrorKind::NotFound, message))
}

/// `Files.readAllLines(path)` (UTF-8, `BufferedReader` line terminators).
fn read_all_lines(path: &Path) -> Result<Vec<String>> {
    read_lines_with(path, Charset::utf_8())
}

/// The lines of `path` decoded with `charset` (`Files.lines(path, charset)`).
fn read_lines_with(path: &Path, charset: Charset) -> Result<Vec<String>> {
    let bytes = std::fs::read(path)?;
    let mut reader =
        BufferedReader::new(InputStreamReader::with_charset(Cursor::new(bytes), charset));
    let mut lines = Vec::new();
    while let Some(line) = reader.read_line()? {
        lines.push(to_rust_string(&line));
    }
    Ok(lines)
}

/// Java's `String.trim()`: strips the units up to U+0020 at both ends.
fn java_trim(s: &str) -> String {
    let u = units(s);
    let mut start = 0;
    let mut end = u.len();
    while start < end && u[start] <= 0x20 {
        start += 1;
    }
    while start < end && u[end - 1] <= 0x20 {
        end -= 1;
    }
    to_rust_string(&u[start..end])
}

/// Java's `List.toString()` / `Arrays.toString(Object[])`: `[a, b]`.
fn java_list<T: std::fmt::Display>(items: &[T]) -> String {
    let items: Vec<String> = items.iter().map(ToString::to_string).collect();
    format!("[{}]", items.join(", "))
}

/// `assertEquals(message, expected, actual)` of two lists of strings.
fn assert_list_equals(message: Option<&str>, expected: &[String], actual: &[String]) {
    if expected != actual {
        Assert::assert_equals_msg(message, java_list(expected), java_list(actual));
    }
}

/// `List.of(items...)` as owned strings.
fn list(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

/// `Arrays.sort(String[])`: UTF-16 order.
fn sort_utf16(v: &mut [String]) {
    v.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
}

/// `assertEquals(expected, actual)` of two strings.
fn eq_str(expected: &str, actual: &str) {
    Assert::assert_equals_str(None, Some(expected), Some(actual));
}

/// `assertEquals(message, expected, actual)` of two strings.
fn eq_str_msg(message: &str, expected: &str, actual: &str) {
    Assert::assert_equals_str(Some(message), Some(expected), Some(actual));
}

/// `() -> {}`.
fn noop() -> CheckCanceled {
    no_check_canceled()
}

// -----------------------------------------------------------------------------
// StemmerTestBase
// -----------------------------------------------------------------------------

/// `StemmerTestBase.stemmer`, the static field the suites share (one suite
/// of the binary runs at a time).
static STEMMER: Mutex<Option<Stemmer>> = Mutex::new(None);

/// `@AfterClass afterClass()`: `stemmer = null`.
fn stemmer_after_class() {
    *STEMMER.lock().unwrap() = None;
}

/// `init(affix, dictionary)`: `init(false, affix, dictionary)`.
fn init(affix: &str, dictionary: &str) -> Result<()> {
    init_ignore_case(false, affix, &[dictionary])
}

/// `init(ignoreCase, affix, dictionaries...)`.
fn init_ignore_case(ignore_case: bool, affix: &str, dictionaries: &[&str]) -> Result<()> {
    let stemmer = Stemmer::new(Arc::new(load_dictionary_names(
        ignore_case,
        affix,
        dictionaries,
    )?));
    *STEMMER.lock().unwrap() = Some(stemmer);
    Ok(())
}

/// `StemmerTestBase.loadDictionary(ignoreCase, affix, dictionaries...)`.
fn load_dictionary_names(
    ignore_case: bool,
    affix: &str,
    dictionaries: &[&str],
) -> Result<Dictionary> {
    if dictionaries.is_empty() {
        return Err(LuceneError::illegal_argument(
            "there must be at least one dictionary",
        ));
    }

    let Some(mut affix_stream) = resource_stream(affix) else {
        return Err(file_not_found(format!("file not found: {affix}")));
    };

    let mut dict_streams = Vec::with_capacity(dictionaries.len());
    for dictionary in dictionaries {
        match resource_stream(dictionary) {
            Some(stream) => dict_streams.push(stream),
            // Java prints the `null` stream, not the name.
            None => return Err(file_not_found("file not found: null".to_string())),
        }
    }

    let mut dicts: Vec<&mut dyn Read> = dict_streams
        .iter_mut()
        .map(|s| s as &mut dyn Read)
        .collect();
    load_dictionary_streams(ignore_case, &mut affix_stream, std::mem::take(&mut dicts))
}

/// `StemmerTestBase.loadDictionary(ignoreCase, affixStream, dictStreams...)`.
fn load_dictionary_streams(
    ignore_case: bool,
    affix_stream: &mut dyn Read,
    dict_streams: Vec<&mut dyn Read>,
) -> Result<Dictionary> {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    Dictionary::with_dictionaries(dir, "dictionary", affix_stream, dict_streams, ignore_case)
}

/// `assertStemsTo(s, expected...)`.
fn assert_stems_to(s: &str, expected: &[&str]) -> Result<()> {
    let guard = STEMMER.lock().unwrap();
    let Some(stemmer) = guard.as_ref() else {
        Assert::fail_without_message()
    };
    let mut expected = list(expected);
    sort_utf16(&mut expected);

    let word = units(s);
    let stems = stemmer.stem(&word, word.len())?;
    let mut actual: Vec<String> = stems.iter().map(ToString::to_string).collect();
    sort_utf16(&mut actual);

    if expected != actual {
        Assert::fail(&format!(
            "expected={},actual={}: arrays first differed",
            java_list(&expected),
            java_list(&actual)
        ));
    }
    Ok(())
}

/// A test method: its name and body.
type TestMethod = (&'static str, fn() -> Result<()>);

/// The builder of a `StemmerTestBase` subclass: its `@BeforeClass
/// beforeClass()` runs `before_class`, its test methods are `tests`.
fn stemmer_class(name: &str, before_class: fn() -> Result<()>, tests: &[TestMethod]) -> Arc<Class> {
    let mut b = lucene_class(&format!("{PACKAGE}.{name}"))
        .method(
            Method::new("beforeClass")
                .modifiers(modifier::PUBLIC | modifier::STATIC)
                .throws(&["java.lang.Exception"])
                .annotate(BeforeClass)
                .static_body(move || before_class().unwrap_or_else(|e| rethrow_java(e))),
        )
        .method(
            Method::new("afterClass")
                .modifiers(modifier::PUBLIC | modifier::STATIC)
                .annotate(AfterClass)
                .static_body(stemmer_after_class),
        );
    for (method, body) in tests {
        b = b.method(t(method, *body));
    }
    b.build()
}

/// Expands `stem!(word => stems...)` lines into `assert_stems_to` calls.
macro_rules! stems {
    ($($word:expr => [$($stem:expr),* $(,)?]);* $(;)?) => {{
        $(assert_stems_to($word, &[$($stem),*])?;)*
        Ok(())
    }};
}

// TestAllCaps

fn all_caps_good() -> Result<()> {
    stems! {
        "OpenOffice.org" => ["OpenOffice.org"];
        "UNICEF's" => ["UNICEF"];
        "L'Afrique" => ["Afrique"];
        "L'AFRIQUE" => ["Afrique"];
        "OPENOFFICE.ORG" => ["Openoffice.org"];
        "UNICEF'S" => ["Unicef"];
    }
}

fn all_caps_wrong() -> Result<()> {
    stems! {
        "Openoffice.org" => ["Openoffice.org"];
        "Unicef" => ["Unicef"];
        "Unicef's" => ["Unicef"];
    }
}

// TestAlternateCasing

fn alternate_casing_possibilities() -> Result<()> {
    stems! {
        "drink" => ["drink"];
        "DR\u{130}NK" => ["drink"];
        "DRINK" => [];
        "drinki" => ["drink"];
        "DR\u{130}NK\u{130}" => ["drink"];
        "DR\u{130}NKI" => [];
        "DRINKI" => [];
        "DRINK\u{130}" => [];
        "idrink" => ["drink"];
        "\u{130}DR\u{130}NK" => ["drink"];
        "IDR\u{130}NK" => [];
        "IDRINK" => [];
        "\u{130}DRINK" => [];
        "idrinki" => ["drink"];
        "\u{130}DR\u{130}NK\u{130}" => ["drink"];
        "r\u{131}ver" => ["r\u{131}ver"];
        "RIVER" => ["r\u{131}ver"];
        "R\u{130}VER" => [];
        "r\u{131}ver\u{131}" => ["r\u{131}ver"];
        "RIVERI" => ["r\u{131}ver"];
        "R\u{130}VERI" => [];
        "R\u{130}VER\u{130}" => [];
        "RIVER\u{130}" => [];
        "\u{131}r\u{131}ver" => ["r\u{131}ver"];
        "IRIVER" => ["r\u{131}ver"];
        "IR\u{130}VER" => [];
        "\u{130}R\u{130}VER" => [];
        "\u{130}RIVER" => [];
        "\u{131}r\u{131}ver\u{131}" => ["r\u{131}ver"];
        "IRIVERI" => ["r\u{131}ver"];
        "Ir\u{131}ver\u{131}" => ["r\u{131}ver"];
    }
}

// TestBaseUtf

/// `good()`: not a test method (no `test` prefix, no `@Test`).
fn base_utf_good() -> Result<()> {
    stems! {
        "imply" => ["imply"];
        "Imply" => ["imply"];
        "IMPLY" => ["imply"];
        "\u{130}ZM\u{130}R" => ["\u{130}zmir"];
    }
}

fn base_utf_no_dotted_i_decapitalization() -> Result<()> {
    stems! {
        "\u{130}mply" => [];
        "\u{130}MPLY" => [];
    }
}

// TestCaseInsensitive

fn case_insensitive_case_insensitivity() -> Result<()> {
    stems! {
        "lucene" => ["lucene", "lucen"];
        "LuCeNe" => ["lucene", "lucen"];
        "mahoute" => ["mahout"];
        "MaHoUte" => ["mahout"];
    }
}

fn simple_prefix() -> Result<()> {
    stems! { "solr" => ["olr"]; }
}

fn recursive_suffix() -> Result<()> {
    // we should not recurse here, as the suffix has no continuation!
    stems! { "abcd" => []; }
}

// all forms unmunched from dictionary
fn all_stems() -> Result<()> {
    stems! {
        "ab" => ["ab"];
        "abc" => ["ab"];
        "apach" => ["apach"];
        "apache" => ["apach"];
        "foo" => ["foo", "foo"];
        "food" => ["foo"];
        "foos" => ["foo"];
        "lucen" => ["lucen"];
        "lucene" => ["lucen", "lucene"];
        "mahout" => ["mahout"];
        "mahoute" => ["mahout"];
        "moo" => ["moo"];
        "mood" => ["moo"];
        "olr" => ["olr"];
        "solr" => ["olr"];
    }
}

// some bogus stuff that should not stem (empty lists)!
fn bogus_stems() -> Result<()> {
    stems! {
        "abs" => [];
        "abe" => [];
        "sab" => [];
        "sapach" => [];
        "sapache" => [];
        "apachee" => [];
        "sfoo" => [];
        "sfoos" => [];
        "fooss" => [];
        "lucenee" => [];
        "solre" => [];
    }
}

// TestCaseSensitive

fn case_sensitive_all_possibilities() -> Result<()> {
    stems! {
        "drink" => ["drink"];
        "drinks" => ["drink"];
        "drinkS" => ["drink"];
        "gooddrinks" => ["drink"];
        "Gooddrinks" => ["drink", "drink"];
        "GOODdrinks" => ["drink"];
        "gooddrinkS" => ["drink"];
        "GooddrinkS" => ["drink"];
        "gooddrink" => ["drink"];
        "Gooddrink" => ["drink", "drink"];
        "GOODdrink" => ["drink"];
        "Drink" => ["drink", "Drink"];
        "Drinks" => ["drink", "Drink"];
        "DrinkS" => ["Drink"];
        "goodDrinks" => ["Drink"];
        "GoodDrinks" => ["Drink"];
        "GOODDrinks" => ["Drink"];
        "goodDrinkS" => ["Drink"];
        "GoodDrinkS" => ["Drink"];
        "GOODDrinkS" => ["Drink"];
        "goodDrink" => ["Drink"];
        "GoodDrink" => ["Drink"];
        "GOODDrink" => ["Drink"];
        "DRINK" => ["DRINK", "drink", "Drink"];
        "DRINKs" => ["DRINK"];
        "DRINKS" => ["DRINK", "drink", "Drink"];
        "goodDRINKs" => ["DRINK"];
        "GoodDRINKs" => ["DRINK"];
        "GOODDRINKs" => ["DRINK"];
        "goodDRINKS" => ["DRINK"];
        "GoodDRINKS" => ["DRINK"];
        "GOODDRINKS" => ["DRINK", "drink", "drink"];
        "goodDRINK" => ["DRINK"];
        "GoodDRINK" => ["DRINK"];
        "GOODDRINK" => ["DRINK", "drink", "drink"];
    }
}

// TestCheckSharpS

fn check_sharp_s_sharp_s() -> Result<()> {
    stems! {
        "M\u{fc}\u{df}ig" => ["m\u{fc}\u{df}ig"];
        "M\u{dc}SSIG" => ["m\u{fc}\u{df}ig"];
        "M\u{fc}ssig" => [];
        "PROZESSIONSSTRASSE" => ["Prozessionsstra\u{df}e"];
    }
}

// TestCircumfix

fn circumfix_circumfix() -> Result<()> {
    stems! {
        "nagy" => ["nagy"];
        "nagyobb" => ["nagy"];
        "legnagyobb" => ["nagy"];
        "legeslegnagyobb" => ["nagy"];
        "nagyobbobb" => [];
        "legnagy" => [];
        "legeslegnagy" => [];
    }
}

// TestComplexPrefix

fn complex_prefix_prefixes() -> Result<()> {
    stems! {
        "ptwofoo" => ["foo"];
        "poneptwofoo" => ["foo"];
        "foosuf" => ["foo"];
        "ptwofoosuf" => ["foo"];
        "poneptwofoosuf" => ["foo"];
        "ponefoo" => [];
        "ponefoosuf" => [];
        "ptwoponefoo" => [];
        "ptwoponefoosuf" => [];
    }
}

// TestCompressed

fn compressed_test() -> Result<()> {
    stems! {
        "apach" => ["apach"];
        "apache" => ["apach"];
        "apachee" => [];
        "XYZ" => ["XYZ", "Xyz"];
        "XYZs" => ["XYZ"];
        "XYZS" => ["Xyz"];
        "xyz" => [];
        "mixedCase" => ["mixedCase"];
        "MIXEDCASE" => ["Mixedcase"];
    }
}

// TestCondition, TestOptionalCondition

fn condition_stemming() -> Result<()> {
    stems! {
        "hello" => ["hello"];
        "try" => ["try"];
        "tried" => ["try"];
        "work" => ["work"];
        "worked" => ["work"];
        "rework" => ["work"];
        "reworked" => ["work"];
        "retried" => [];
        "workied" => [];
        "tryed" => [];
        "tryied" => [];
        "helloed" => [];
    }
}

// TestCondition2

fn condition2_stemming() -> Result<()> {
    stems! { "monopolies" => ["monopoly"]; }
}

// TestConv

fn conv_conversion() -> Result<()> {
    stems! {
        "drink" => ["drInk"];
        "drInk" => ["drInk"];
        "drInkAble" => ["drInk"];
        "drInkABle" => ["drInk"];
        "drinkABle" => ["drInk"];
    }
}

// TestDependencies

fn dependencies_dependencies() -> Result<()> {
    stems! {
        "drink" => ["drink", "drink"];
        "drinks" => ["drink", "drink"];
        "drinkable" => ["drink"];
        "drinkables" => ["drink"];
        "undrinkable" => ["drink"];
        "undrinkables" => ["drink"];
        "undrink" => [];
        "undrinks" => [];
        "hydration" => ["hydrate"];
        "dehydrate" => ["hydrate"];
        "dehydration" => ["hydrate"];
        "calorie" => ["calorie", "calorie"];
        "calories" => ["calorie"];
    }
}

// TestDoubleEscape

fn double_escape_stemming() -> Result<()> {
    stems! { "adubo" => ["adubar"]; }
}

// TestDutchIJ

fn dutch_ij_stemming() -> Result<()> {
    stems! {
        "ijs" => ["ijs"];
        "IJs" => ["ijs"];
    }
}

// TestEscaped

fn escaped_stemming() -> Result<()> {
    stems! {
        "/" => ["/"];
        "works" => ["work"];
        "work" => ["work"];
        "R2/D2" => ["R2/D2", "R2/d2"];
        "R2/D2s" => ["R2/D2"];
        "N/A" => ["N/A"];
        "N/As" => [];
        "/" => ["/"];
        "/a" => ["/a"];
        "//" => [];
    }
}

// TestFlagLong

fn flag_long_long_flags() -> Result<()> {
    stems! {
        "foo" => ["foo"];
        "foos" => ["foo"];
        "fooss" => [];
        "foobogus" => [];
    }
}

// TestFlagNum

fn flag_num_num_flags() -> Result<()> {
    stems! {
        "foo" => ["foo"];
        "foos" => ["foo"];
        "fooss" => [];
    }
}

// TestFullStrip

fn full_strip_stemming() -> Result<()> {
    stems! {
        "tasty" => ["beer"];
        "as" => ["a"];
        "s" => [];
    }
}

// TestHomonyms

fn homonyms_examples() -> Result<()> {
    stems! { "works" => ["work", "work"]; }
}

// TestIgnore

fn ignore_examples() -> Result<()> {
    stems! {
        "drink" => ["drink"];
        "drinkable" => ["drink"];
        "dr'ink-able" => ["drink"];
        "drank-able" => ["drank"];
        "'-'-'-" => [];
    }
}

// TestKeepCase

fn keep_case_possibilities() -> Result<()> {
    stems! {
        "drink" => ["drink"];
        "Drink" => ["drink"];
        "DRINK" => ["drink"];
        "drinks" => ["drink"];
        "Drinks" => ["drink"];
        "DRINKS" => ["drink"];
        "walk" => ["walk"];
        "walks" => ["walk"];
        "Walk" => ["walk"];
        "Walks" => ["walk"];
        "WALKS" => ["walk"];
        "test" => ["test"];
        "Test" => ["test"];
        "TEST" => ["test"];
        "baz." => ["baz."];
        "Baz." => ["baz."];
        "Quux." => ["Quux."];
        "QUUX." => ["Quux."];
        "Ways" => ["way", "ways"];
        "WAYS" => ["way", "ways"];
    }
}

// TestMorph

fn morph_examples() -> Result<()> {
    stems! {
        "drink" => ["drink"];
        "drinkable" => ["drink"];
        "drinkableable" => [];
    }
}

// TestMorphAlias, TestMorphData

fn morph_alias_stemming() -> Result<()> {
    stems! {
        "feet" => ["foot"];
        "feetscratcher" => ["foot"];
        "work" => ["workverb", "worknoun"];
        "works" => ["workverb", "worknoun"];
        "notspecial" => ["notspecial"];
        "simplenoun" => ["simplenoun"];
        "simplenouns" => ["simplenoun"];
        "simplenounscratcher" => [];
    }
}

// TestNeedAffix

fn need_affix_possibilities() -> Result<()> {
    stems! {
        "drink" => ["drink"];
        "drinks" => ["drink"];
        "walk" => [];
        "walks" => ["walk"];
        "prewalk" => ["walk"];
        "prewalks" => ["walk"];
        "test" => [];
        "pretest" => [];
        "tests" => [];
        "pretests" => [];
    }
}

// TestOnlyInCompound

fn only_in_compound_possibilities() -> Result<()> {
    stems! {
        "drink" => ["drink"];
        "drinks" => ["drink"];
        "drinked" => [];
        "predrink" => [];
        "predrinked" => [];
        "walk" => [];
    }
}

// TestSpaces

fn spaces_stemming() -> Result<()> {
    stems! {
        "four" => ["four"];
        "fours" => ["four"];
        "five" => ["five"];
        "forty four" => ["forty four"];
        "forty fours" => ["forty four"];
        "forty five" => ["forty five"];
        "fifty" => ["50"];
        "fiftys" => ["50"];
        "sixty" => ["60"];
        "sixty four" => ["64"];
        "fifty four" => ["54"];
        "fifty fours" => ["54"];
    }
}

// TestStemmer

fn stemmer_simple_suffix() -> Result<()> {
    stems! {
        "lucene" => ["lucene", "lucen"];
        "mahoute" => ["mahout"];
    }
}

// TestStrangeOvergeneration

fn strange_overgeneration_stemming() -> Result<()> {
    stems! {
        "btasty" => ["beer"];
        "tasty" => [];
        "yuck" => [];
        "foo" => [];
    }
}

// TestTwoFold

fn two_fold_examples() -> Result<()> {
    stems! {
        "drink" => ["drink"];
        "drinkable" => ["drink"];
        "drinkables" => ["drink"];
        "drinksable" => [];
        "drinkableable" => [];
        "drinks" => [];
    }
}

// TestTwoSuffixes

fn two_suffixes_examples() -> Result<()> {
    stems! {
        "drink" => ["drink"];
        "drinkable" => ["drink"];
        "drinks" => ["drink"];
        "drinkableable" => [];
        "drinkss" => [];
    }
}

// TestZeroAffix, TestZeroAffix2

fn zero_affix_stemming() -> Result<()> {
    stems! { "drink" => ["drinksierranevada"]; }
}

fn zero_affix2_stemming() -> Result<()> {
    stems! { "b" => ["beer"]; }
}

/// The `StemmerTestBase` suites: name, `beforeClass` body, test methods.
type StemmerSuite = (&'static str, fn() -> Result<()>, &'static [TestMethod]);

static STEMMER_SUITES: &[StemmerSuite] = &[
    (
        "TestAllCaps",
        || init("allcaps.aff", "allcaps.dic"),
        &[("testGood", all_caps_good), ("testWrong", all_caps_wrong)],
    ),
    (
        "TestAlternateCasing",
        || init("alternate-casing.aff", "alternate-casing.dic"),
        &[("testPossibilities", alternate_casing_possibilities)],
    ),
    (
        "TestCaseInsensitive",
        || init_ignore_case(true, "simple.aff", &["mixedcase.dic"]),
        &[
            ("testCaseInsensitivity", case_insensitive_case_insensitivity),
            ("testSimplePrefix", simple_prefix),
            ("testRecursiveSuffix", recursive_suffix),
            ("testAllStems", all_stems),
            ("testBogusStems", bogus_stems),
        ],
    ),
    (
        "TestCaseSensitive",
        || init("casesensitive.aff", "casesensitive.dic"),
        &[("testAllPossibilities", case_sensitive_all_possibilities)],
    ),
    (
        "TestCheckSharpS",
        || init("checksharps.aff", "checksharps.dic"),
        &[("testSharpS", check_sharp_s_sharp_s)],
    ),
    (
        "TestCircumfix",
        || init("circumfix.aff", "circumfix.dic"),
        &[("testCircumfix", circumfix_circumfix)],
    ),
    (
        "TestComplexPrefix",
        || init("complexprefix.aff", "complexprefix.dic"),
        &[("testPrefixes", complex_prefix_prefixes)],
    ),
    (
        "TestCompressed",
        || init("compressed.aff", "compressed.dic"),
        &[("test", compressed_test)],
    ),
    (
        "TestCondition",
        || init("condition.aff", "condition.dic"),
        &[("testStemming", condition_stemming)],
    ),
    (
        "TestCondition2",
        || init("condition2.aff", "condition2.dic"),
        &[("testStemming", condition2_stemming)],
    ),
    (
        "TestConv",
        || init("conv.aff", "conv.dic"),
        &[("testConversion", conv_conversion)],
    ),
    (
        "TestDependencies",
        || init("dependencies.aff", "dependencies.dic"),
        &[("testDependencies", dependencies_dependencies)],
    ),
    (
        "TestDoubleEscape",
        || init("double-escaped.aff", "double-escaped.dic"),
        &[("testStemming", double_escape_stemming)],
    ),
    (
        "TestDutchIJ",
        || init("IJ.aff", "IJ.dic"),
        &[("testStemming", dutch_ij_stemming)],
    ),
    (
        "TestEscaped",
        || init("escaped.aff", "escaped.dic"),
        &[("testStemming", escaped_stemming)],
    ),
    (
        "TestFlagLong",
        || init("flaglong.aff", "flaglong.dic"),
        &[("testLongFlags", flag_long_long_flags)],
    ),
    (
        "TestFlagNum",
        || init("flagnum.aff", "flagnum.dic"),
        &[("testNumFlags", flag_num_num_flags)],
    ),
    (
        "TestFullStrip",
        || init("fullstrip.aff", "fullstrip.dic"),
        &[("testStemming", full_strip_stemming)],
    ),
    (
        "TestHomonyms",
        || init("homonyms.aff", "homonyms.dic"),
        &[("testExamples", homonyms_examples)],
    ),
    (
        "TestIgnore",
        || init("ignore.aff", "ignore.dic"),
        &[("testExamples", ignore_examples)],
    ),
    (
        "TestKeepCase",
        || init("keepcase.aff", "keepcase.dic"),
        &[("testPossibilities", keep_case_possibilities)],
    ),
    (
        "TestMorph",
        || init("morph.aff", "morph.dic"),
        &[("testExamples", morph_examples)],
    ),
    (
        "TestMorphAlias",
        || init("morphalias.aff", "morphalias.dic"),
        &[("testStemming", morph_alias_stemming)],
    ),
    (
        "TestMorphData",
        || init("morphdata.aff", "morphdata.dic"),
        &[("testStemming", morph_alias_stemming)],
    ),
    (
        "TestNeedAffix",
        || init("needaffix.aff", "needaffix.dic"),
        &[("testPossibilities", need_affix_possibilities)],
    ),
    (
        "TestOnlyInCompound",
        || init("onlyincompound.aff", "onlyincompound.dic"),
        &[("testPossibilities", only_in_compound_possibilities)],
    ),
    (
        "TestOptionalCondition",
        || init("optional-condition.aff", "condition.dic"),
        &[("testStemming", condition_stemming)],
    ),
    (
        "TestSpaces",
        || init("spaces.aff", "spaces.dic"),
        &[("testStemming", spaces_stemming)],
    ),
    (
        "TestStemmer",
        || init("simple.aff", "simple.dic"),
        &[
            ("testSimpleSuffix", stemmer_simple_suffix),
            ("testSimplePrefix", simple_prefix),
            ("testRecursiveSuffix", recursive_suffix),
            ("testAllStems", all_stems),
            ("testBogusStems", bogus_stems),
        ],
    ),
    (
        "TestStrangeOvergeneration",
        || init("strange-overgeneration.aff", "strange-overgeneration.dic"),
        &[("testStemming", strange_overgeneration_stemming)],
    ),
    (
        "TestTwoFold",
        || init("twofold.aff", "morph.dic"),
        &[("testExamples", two_fold_examples)],
    ),
    (
        "TestTwoSuffixes",
        || init("twosuffixes.aff", "twosuffixes.dic"),
        &[("testExamples", two_suffixes_examples)],
    ),
    (
        "TestZeroAffix",
        || init("zeroaffix.aff", "zeroaffix.dic"),
        &[("testStemming", zero_affix_stemming)],
    ),
    (
        "TestZeroAffix2",
        || init("zeroaffix2.aff", "zeroaffix2.dic"),
        &[("testStemming", zero_affix2_stemming)],
    ),
];

/// The suite `name` of [`STEMMER_SUITES`].
fn stemmer_suite(name: &str) -> Arc<Class> {
    let (_, before, tests) = STEMMER_SUITES
        .iter()
        .find(|(n, _, _)| *n == name)
        .expect("a stemmer suite");
    stemmer_class(name, *before, tests)
}

/// `TestBaseUtf`: `good()` is a plain method, `no_dottedI_decapitalization`
/// a JUnit 4 `@Test`.
fn base_utf_class() -> Arc<Class> {
    lucene_class(&format!("{PACKAGE}.TestBaseUtf"))
        .method(
            Method::new("beforeClass")
                .modifiers(modifier::PUBLIC | modifier::STATIC)
                .throws(&["java.lang.Exception"])
                .annotate(BeforeClass)
                .static_body(|| {
                    init("base_utf.aff", "base_utf.dic").unwrap_or_else(|e| rethrow_java(e))
                }),
        )
        .method(
            Method::new("afterClass")
                .modifiers(modifier::PUBLIC | modifier::STATIC)
                .annotate(AfterClass)
                .static_body(stemmer_after_class),
        )
        .method(t("good", base_utf_good))
        .method(
            t(
                "no_dottedI_decapitalization",
                base_utf_no_dotted_i_decapitalization,
            )
            .annotate(Test::new()),
        )
        .build()
}

// -----------------------------------------------------------------------------
// TestAffixCondition
// -----------------------------------------------------------------------------

fn compiled(
    kind: AffixKind,
    strip: &str,
    condition: &str,
) -> Result<Arc<dyn rucene_analysis_common::analysis::hunspell::affix_condition::AffixCondition>> {
    compile(kind, &units(strip), &units(condition), &units(""))
}

fn accepts(
    c: &Arc<dyn rucene_analysis_common::analysis::hunspell::affix_condition::AffixCondition>,
    stem: &str,
) -> bool {
    c.accepts_stem_string(&units(stem))
}

fn ac_test_plain_suffix_matching() -> Result<()> {
    let condition = compiled(AffixKind::Suffix, "b", "ab")?;
    Assert::assert_true(accepts(&condition, "a"));
    Assert::assert_false(accepts(&condition, "b"));
    Assert::assert_false(accepts(&condition, "ab"));
    Ok(())
}

fn ac_test_plain_prefix_matching() -> Result<()> {
    let condition = compiled(AffixKind::Prefix, "a", "ab")?;
    Assert::assert_false(accepts(&condition, "ab"));
    Assert::assert_true(accepts(&condition, "b"));
    Assert::assert_false(accepts(&condition, "a"));
    Ok(())
}

fn ac_test_dot_matching() -> Result<()> {
    let condition = compiled(AffixKind::Prefix, "", "wr.")?;
    Assert::assert_true(accepts(&condition, "wry"));
    Assert::assert_true(accepts(&condition, "wrong"));
    Assert::assert_false(accepts(&condition, "white"));
    Ok(())
}

fn key(kind: AffixKind, strip: &str, condition: &str) -> String {
    to_rust_string(&unique_key(kind, &units(strip), &units(condition)))
}

fn ac_test_unique_key() -> Result<()> {
    Assert::assert_true(key(AffixKind::Prefix, "", "x") != key(AffixKind::Suffix, "", "x"));
    Assert::assert_true(key(AffixKind::Suffix, "y", "x") != key(AffixKind::Suffix, "", "x"));
    eq_str(ALWAYS_TRUE_KEY, &key(AffixKind::Prefix, "", "."));
    eq_str(ALWAYS_TRUE_KEY, &key(AffixKind::Suffix, "abc", "abc"));

    eq_str(ALWAYS_TRUE_KEY, &key(AffixKind::Suffix, "abc", "bc"));
    eq_str(ALWAYS_TRUE_KEY, &key(AffixKind::Prefix, "abc", "ab"));
    Ok(())
}

fn ac_test_condition_has_brackets_intersecting_with_strip() -> Result<()> {
    Assert::assert_true(accepts(
        &compiled(AffixKind::Suffix, "oj", "[io]j")?,
        "whatever",
    ));
    Assert::assert_true(accepts(
        &compiled(AffixKind::Suffix, "oj", "o[ioj")?,
        "whatever",
    ));
    Ok(())
}

/// `assertEquals(ALWAYS_FALSE, condition)`: the same instance.
fn assert_always_false(
    condition: Arc<dyn rucene_analysis_common::analysis::hunspell::affix_condition::AffixCondition>,
) {
    if !Arc::ptr_eq(&condition, &always_false()) {
        Assert::fail(&format!("expected:<ALWAYS_FALSE> but was:<{condition:?}>"));
    }
}

fn ac_test_impossible_condition() -> Result<()> {
    assert_always_false(compiled(AffixKind::Suffix, "a", "b")?);
    Ok(())
}

fn ac_test_non_hunspell_pattern_characters() -> Result<()> {
    assert_always_false(compiled(AffixKind::Suffix, "x", "(^ax)")?);
    assert_always_false(compiled(AffixKind::Suffix, "x", "(^.x)")?);
    assert_always_false(compiled(AffixKind::Suffix, "x", "[z](^ax)")?);
    assert_always_false(compiled(AffixKind::Suffix, "x", "(^ax)[z]")?);
    Ok(())
}

fn affix_condition_class() -> Arc<Class> {
    lucene_class(&format!("{PACKAGE}.TestAffixCondition"))
        .method(t("testPlainSuffixMatching", ac_test_plain_suffix_matching))
        .method(t("testPlainPrefixMatching", ac_test_plain_prefix_matching))
        .method(t("testDotMatching", ac_test_dot_matching))
        .method(t("testUniqueKey", ac_test_unique_key))
        .method(t(
            "testConditionHasBracketsIntersectingWithStrip",
            ac_test_condition_has_brackets_intersecting_with_strip,
        ))
        .method(t("testImpossibleCondition", ac_test_impossible_condition))
        .method(t(
            "testNonHunspellPatternCharacters",
            ac_test_non_hunspell_pattern_characters,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// Test64kAffixes
// -----------------------------------------------------------------------------

fn k64_test() -> Result<()> {
    let temp_dir = LuceneTestCase::create_temp_dir_prefix("64kaffixes");
    let affix = temp_dir.resolve_str("64kaffixes.aff");
    let dict = temp_dir.resolve_str("64kaffixes.dic");

    // 65k affixes with flag 1, then an affix with flag 2
    let mut affix_text = String::from("SET UTF-8\nFLAG num\nSFX 1 Y 65536\n");
    for i in 0..65536 {
        affix_text.push_str(&format!("SFX 1 0 {i:x} .\n"));
    }
    affix_text.push_str("SFX 2 Y 1\nSFX 2 0 s\n");
    std::fs::write(affix.as_std_path(), affix_text)?;

    // drink signed with affix 2 (takes -s)
    std::fs::write(dict.as_std_path(), "1\ndrink/2\n")?;

    let mut aff_stream = std::fs::File::open(affix.as_std_path())?;
    let mut dict_stream = std::fs::File::open(dict.as_std_path())?;
    let temp_dir2 = LuceneTestCase::new_directory();
    let dictionary = Dictionary::new(
        temp_dir2.as_directory(),
        "dictionary",
        &mut aff_stream,
        &mut dict_stream,
    )?;
    let stemmer = Stemmer::new(Arc::new(dictionary));
    // drinks should still stem to drink
    let word = units("drinks");
    let stems = stemmer.stem(&word, word.len())?;
    Assert::assert_equals(1, stems.len());
    eq_str("drink", &stems[0].to_string());
    temp_dir2.close()?;
    Ok(())
}

fn k64_class() -> Arc<Class> {
    lucene_class(&format!("{PACKAGE}.Test64kAffixes"))
        .method(t("test", k64_test))
        .build()
}

// -----------------------------------------------------------------------------
// TestDictionary
// -----------------------------------------------------------------------------

/// `getDirectory()`: `newDirectory()`.
fn get_directory() -> rucene_test_framework::util::lucene_test_case::TestDirectory {
    LuceneTestCase::new_directory()
}

/// `TestDictionary.loadDictionary(aff, dic)`.
fn td_load_dictionary(aff: &str, dic: &str) -> Result<Dictionary> {
    let mut affix_stream = resource_stream(aff).ok_or_else(LuceneError::null_pointer)?;
    let mut dic_stream = resource_stream(dic).ok_or_else(LuceneError::null_pointer)?;
    let temp_dir = get_directory();
    let result = Dictionary::new(
        temp_dir.as_directory(),
        "dictionary",
        &mut affix_stream,
        &mut dic_stream,
    );
    let closed = temp_dir.close();
    let dictionary = result?;
    closed?;
    Ok(dictionary)
}

/// The anonymous subclass of `loadForgivingDictionary`.
struct Forgiving;

impl DictionarySubclass for Forgiving {
    fn tolerate_affix_rule_count_mismatches(&self) -> bool {
        true
    }

    fn tolerate_rep_rule_count_mismatches(&self) -> bool {
        true
    }

    fn tolerate_duplicate_conversion_mappings(&self) -> bool {
        true
    }
}

/// `TestDictionary.loadForgivingDictionary(aff, dic)`.
fn td_load_forgiving_dictionary(aff: &str, dic: &str) -> Result<Dictionary> {
    let mut affix_stream = resource_stream(aff).ok_or_else(LuceneError::null_pointer)?;
    let mut dic_stream = resource_stream(dic).ok_or_else(LuceneError::null_pointer)?;
    let temp_dir = get_directory();
    let result = Dictionary::with_subclass(
        &mut affix_stream,
        vec![&mut dic_stream],
        false,
        SortingStrategy::offline(temp_dir.as_directory(), "dictionary"),
        Arc::new(Forgiving),
    );
    let closed = temp_dir.close();
    let dictionary = result?;
    closed?;
    Ok(dictionary)
}

fn ints_len(r: &Option<IntsRef>) -> usize {
    r.as_ref().expect("a non-null IntsRef").length
}

fn assert_single_flag(dictionary: &Dictionary, ord_list: &IntsRef) -> u16 {
    let entry_id = ord_list.ints[0];
    let flags = dictionary.flag_lookup.get_flags(entry_id);
    Assert::assert_equals(1, flags.len());
    flags[0]
}

fn td_test_simple_dictionary() -> Result<()> {
    let dictionary = td_load_dictionary("simple.aff", "simple.dic")?;
    Assert::assert_equals(3, ints_len(&dictionary.lookup_suffix(&units("e"))?));
    Assert::assert_equals(1, ints_len(&dictionary.lookup_prefix(&units("s"))?));
    let ord_list = dictionary.lookup_word(&units("olr"), 0, 3);
    Assert::assert_true(ord_list.is_some());
    let ord_list = ord_list.unwrap();
    Assert::assert_equals(1, ord_list.length);

    Assert::assert_equals(u16::from(b'B'), assert_single_flag(&dictionary, &ord_list));

    let offset = LuceneTestCase::random().next_int_bound(10) as usize;
    let text = units(&(" ".repeat(offset) + "lucen"));
    let ord_list = dictionary.lookup_word(&text, offset, 5);
    Assert::assert_true(ord_list.is_some());
    let ord_list = ord_list.unwrap();
    Assert::assert_equals(1, ord_list.length);
    Assert::assert_equals(u16::from(b'A'), assert_single_flag(&dictionary, &ord_list));

    Assert::assert_true(dictionary.lookup_word(&units("ab"), 0, 2).is_some());
    Assert::assert_true(dictionary.lookup_word(&units("db"), 0, 2).is_some());
    Assert::assert_true(dictionary.lookup_word(&units("b"), 0, 1).is_none());
    Ok(())
}

/// `processSuggestibleWords(dictionary, minLength, maxLength)`.
fn process_suggestible_words(
    dictionary: &Dictionary,
    min_length: i32,
    max_length: i32,
) -> Result<HashSet<String>> {
    let mut processed = HashSet::new();
    dictionary
        .words
        .process_suggestible_words(min_length, max_length, &mut |e| {
            processed.insert(to_rust_string(e.root()));
            Ok(())
        })?;

    let mut cached = HashSet::new();
    SuggestibleEntryCache::build_cache(&dictionary.words)?.process_suggestible_words(
        min_length,
        max_length,
        &mut |e| {
            cached.insert(to_rust_string(e.root()));
            Ok(())
        },
    )?;
    if processed != cached {
        Assert::fail(&format!(
            "expected:<{}> but was:<{}>",
            java_list(&sorted(&processed)),
            java_list(&sorted(&cached))
        ));
    }

    Ok(processed)
}

fn sorted(set: &HashSet<String>) -> Vec<String> {
    let mut v: Vec<String> = set.iter().cloned().collect();
    sort_utf16(&mut v);
    v
}

fn check_process_words(
    dictionary: &Dictionary,
    all_words: &HashSet<String>,
    min_length: i32,
    max_length: i32,
) -> Result<()> {
    let processed = process_suggestible_words(dictionary, min_length, max_length)?;

    let filtered: HashSet<String> = all_words
        .iter()
        .filter(|s| {
            let len = s.encode_utf16().count() as i32;
            min_length <= len && len <= max_length
        })
        .cloned()
        .collect();

    if filtered != processed {
        Assert::fail(&format!(
            "For lengths [{min_length},{max_length}] expected:<{}> but was:<{}>",
            java_list(&sorted(&filtered)),
            java_list(&sorted(&processed))
        ));
    }
    Ok(())
}

fn td_test_process_all_words() -> Result<()> {
    let dictionary = td_load_dictionary("simple.aff", "simple.dic")?;

    let lines = read_all_lines(&resource_path("simple.dic"))?;
    let all_words: HashSet<String> = lines
        .iter()
        .skip(1)
        .map(|s| {
            // `s.split("/")[0]`
            s.split('/').next().unwrap_or("").to_string()
        })
        .collect();
    let max_length = all_words
        .iter()
        .map(|s| s.encode_utf16().count() as i32)
        .max()
        .expect("a word");

    for min in 1..=max_length + 1 {
        for max in min..=max_length + 1 {
            check_process_words(&dictionary, &all_words, min, max)?;
        }
    }
    Ok(())
}

fn td_test_process_suggestible_words() -> Result<()> {
    let dictionary = td_load_dictionary("suggestible.aff", "suggestible.dic")?;

    let processed = process_suggestible_words(&dictionary, 1, 100)?;
    let expected: HashSet<String> = ["normal", "ambiguous"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    Assert::assert_equals(
        java_list(&sorted(&expected)),
        java_list(&sorted(&processed)),
    );
    Ok(())
}

fn compressed_dictionary_checks(aff: &str) -> Result<()> {
    let dictionary = td_load_dictionary(aff, "compressed.dic")?;
    Assert::assert_equals(3, ints_len(&dictionary.lookup_suffix(&units("e"))?));
    Assert::assert_equals(1, ints_len(&dictionary.lookup_prefix(&units("s"))?));
    let ord_list = dictionary
        .lookup_word(&units("olr"), 0, 3)
        .ok_or_else(LuceneError::null_pointer)?;
    assert_single_flag(&dictionary, &ord_list);
    Ok(())
}

fn td_test_compressed_dictionary() -> Result<()> {
    compressed_dictionary_checks("compressed.aff")
}

fn td_test_compressed_before_set_dictionary() -> Result<()> {
    compressed_dictionary_checks("compressed-before-set.aff")
}

fn td_test_compressed_empty_alias_dictionary() -> Result<()> {
    compressed_dictionary_checks("compressed-empty-alias.aff")
}

// malformed rule causes ParseException
fn td_test_invalid_data() -> Result<()> {
    let expected = match td_load_dictionary("broken.aff", "simple.dic") {
        Ok(_) => {
            Assert::fail("Expected exception java.text.ParseException but no exception was thrown")
        }
        Err(e) => e,
    };
    if ParseException::of(&expected).is_none() {
        rethrow_java(expected)
    }
    let parse = ParseException::of(&expected).expect("a ParseException");
    Assert::assert_true(parse.get_message().starts_with("Invalid syntax"));
    Assert::assert_equals(24, parse.get_error_offset());

    let names = [
        "broken_missingAffRule.aff",
        "broken_extraAffRule.aff",
        "broken_extraAffRule_last.aff",
        "broken_extraAffRule_beforeAnother.aff",
        "broken_mismatchedAffix.aff",
    ];

    for name in names {
        let msg = format!("Expected ParseException on {name}");
        match td_load_dictionary(name, "simple.dic") {
            Ok(_) => Assert::fail(&msg),
            Err(e) => {
                if ParseException::of(&e).is_none() {
                    rethrow_java(e)
                }
            }
        }
    }
    Ok(())
}

fn td_test_using_flags_before_flag_directive() -> Result<()> {
    let aff = b"KEEPCASE 42\nFLAG num".to_vec();
    let dic = b"1\nfoo/42".to_vec();

    let dictionary = Dictionary::new(
        Arc::new(ByteBuffersDirectory::new()),
        "",
        &mut Cursor::new(aff),
        &mut Cursor::new(dic),
    )?;

    Assert::assert_equals(42, dictionary.keepcase);
    Ok(())
}

fn td_test_forgivable_errors() -> Result<()> {
    let dictionary =
        td_load_forgiving_dictionary("forgivable-errors.aff", "forgivable-errors.dic")?;
    Assert::assert_equals(1, dictionary.rep_table.len());
    Assert::assert_equals(2, dictionary.compound_max);

    td_load_forgiving_dictionary("forgivable-errors-long.aff", "single-word.dic")?;
    td_load_forgiving_dictionary("forgivable-errors-num.aff", "single-word.dic")?;
    Ok(())
}

/// simple tests for dictionary problems seen in the wild
fn td_test_common_forgivable_errors() -> Result<()> {
    let dictionary = td_load_forgiving_dictionary("common-errors.aff", "common-errors.dic")?;
    // try to ensure we still parsed the affixes correctly, despite the problems
    let expected_suffixes = ["ing", "ed"];
    for suffix in expected_suffixes {
        // `new StringBuilder(suffix).reverse()`
        let reversed: Vec<u16> = units(suffix).into_iter().rev().collect();
        Assert::assert_true_msg(
            &format!("checking for {suffix}"),
            dictionary.lookup_suffix(&reversed)?.is_some(),
        );
    }
    Ok(())
}

/// `CloseCheckInputStream`: a stream that records `close()`.
struct CloseCheckInputStream {
    delegate: Cursor<Vec<u8>>,
    closed: bool,
}

impl CloseCheckInputStream {
    fn new(delegate: Cursor<Vec<u8>>) -> Self {
        CloseCheckInputStream {
            delegate,
            closed: false,
        }
    }

    fn close(&mut self) {
        self.closed = true;
    }

    fn is_closed(&self) -> bool {
        self.closed
    }
}

impl Read for CloseCheckInputStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.delegate.read(buf)
    }
}

fn td_test_resource_cleanup() -> Result<()> {
    let mut affix_stream =
        CloseCheckInputStream::new(resource_stream("compressed.aff").expect("compressed.aff"));
    let mut dict_stream =
        CloseCheckInputStream::new(resource_stream("compressed.dic").expect("compressed.dic"));
    let temp_dir = get_directory();

    Dictionary::new(
        temp_dir.as_directory(),
        "dictionary",
        &mut affix_stream,
        &mut dict_stream,
    )?;

    Assert::assert_false(affix_stream.is_closed());
    Assert::assert_false(dict_stream.is_closed());

    affix_stream.close();
    dict_stream.close();
    temp_dir.close()?;

    Assert::assert_true(affix_stream.is_closed());
    Assert::assert_true(dict_stream.is_closed());
    Ok(())
}

fn td_test_replacements() -> Result<()> {
    let mut map: BTreeMap<Vec<u16>, Vec<u16>> = BTreeMap::new();
    map.insert(units("a"), units("b"));
    map.insert(units("ab"), units("c"));
    map.insert(units("c"), units("de"));
    map.insert(units("def"), units("gh"));
    let table = ConvTable::new(&map)?;

    let apply = |s: &str| -> Result<String> {
        let mut sb = units(s);
        table.apply_mappings(&mut sb)?;
        Ok(to_rust_string(&sb))
    };

    eq_str("btestbnother", &apply("atestanother")?);
    eq_str("ctestbnother", &apply("abtestanother")?);
    eq_str("btestcnother", &apply("atestabnother")?);
    eq_str("ctestcnother", &apply("abtestabnother")?);
    eq_str("ctestcdenother", &apply("abtestabcnother")?);
    eq_str("ghghghde", &apply("defdefdefc")?);
    Ok(())
}

fn get_dictionary_encoding(aff_file: &str) -> Result<String> {
    let dictionary = Dictionary::new(
        Arc::new(ByteBuffersDirectory::new()),
        "",
        &mut Cursor::new(aff_file.as_bytes().to_vec()),
        &mut Cursor::new(b"1\nmock".to_vec()),
    )?;
    Ok(dictionary.decoder.charset().name().to_string())
}

fn td_test_set_with_crazy_whitespace_and_boms() -> Result<()> {
    eq_str("UTF-8", &get_dictionary_encoding("SET\tUTF-8\n")?);
    eq_str("UTF-8", &get_dictionary_encoding("SET\t UTF-8\n")?);
    eq_str("UTF-8", &get_dictionary_encoding("\u{FEFF}SET\tUTF-8\n")?);
    eq_str("UTF-8", &get_dictionary_encoding("\u{FEFF}SET\tUTF-8\r\n")?);
    // `Dictionary.DEFAULT_CHARSET` is `StandardCharsets.ISO_8859_1`.
    eq_str(Charset::iso_8859_1().name(), &get_dictionary_encoding("")?);
    Ok(())
}

fn td_test_flag_with_crazy_whitespace() -> Result<()> {
    Assert::assert_true(get_flag_parsing_strategy(&units("FLAG\tUTF-8"), Charset::utf_8()).is_ok());
    Assert::assert_true(
        get_flag_parsing_strategy(&units("FLAG    UTF-8"), Charset::utf_8()).is_ok(),
    );
    Ok(())
}

fn td_test_utf8_flag() -> Result<()> {
    // `Dictionary.DEFAULT_CHARSET` is `StandardCharsets.ISO_8859_1`.
    let strategy = get_flag_parsing_strategy(&units("FLAG\tUTF-8"), Charset::iso_8859_1())?;

    let src = "\u{43f}\u{440}\u{438}\u{432}\u{435}\u{442}";
    // `new String(src.getBytes(UTF_8), DEFAULT_CHARSET)`: one unit per byte
    let as_ascii: Vec<u16> = src.as_bytes().iter().map(|&b| u16::from(b)).collect();
    Assert::assert_true(units(src) != as_ascii);
    eq_str(src, &to_rust_string(&strategy.parse_flags(&as_ascii)?));
    Ok(())
}

fn td_test_custom_morphological_data() -> Result<()> {
    let dic = td_load_dictionary("morphdata.aff", "morphdata.dic")?;
    Assert::assert_true(dic.lookup_entries("nonexistent").is_none());

    let simple_noun = dic.lookup_entries("simplenoun").expect("simplenoun");
    Assert::assert_equals(1, simple_noun.size());
    assert_list_equals(None, &[], &simple_noun.get_morphological_values(0, "aa:"));
    assert_list_equals(
        None,
        &list(&["42"]),
        &simple_noun.get_morphological_values(0, "fr:"),
    );
    assert_list_equals(
        None,
        &list(&["42"]),
        &simple_noun.get(0).get_morphological_values("fr:"),
    );
    eq_str("A", &simple_noun.get(0).get_flags());

    let lay = dic.lookup_entries("lay").expect("lay");
    let mut data: Vec<String> = (0..3).map(|i| lay.get_morphological_data(i)).collect();
    sort_utf16(&mut data);
    eq_str(
        "is:past_2 po:verb st:lie; is:present po:verb; po:noun",
        &data.join("; "),
    );

    let sing = dic.lookup_entries("sing").expect("sing");
    Assert::assert_equals(1, sing.size());
    assert_list_equals(
        None,
        &list(&["sang", "sung"]),
        &sing.get_morphological_values(0, "al:"),
    );

    eq_str(
        "al:abal\u{e9}ar po:verbo ts:transitiva",
        &dic.lookup_entries("unsupported1")
            .expect("unsupported1")
            .get_morphological_data(0),
    );

    eq_str(
        "",
        &dic.lookup_entries("unsupported2")
            .expect("unsupported2")
            .get_morphological_data(0),
    );
    Ok(())
}

fn dictionary_class() -> Arc<Class> {
    lucene_class(&format!("{PACKAGE}.TestDictionary"))
        .method(t("testSimpleDictionary", td_test_simple_dictionary))
        .method(t("testProcessAllWords", td_test_process_all_words))
        .method(t(
            "testProcessSuggestibleWords",
            td_test_process_suggestible_words,
        ))
        .method(t("testCompressedDictionary", td_test_compressed_dictionary))
        .method(t(
            "testCompressedBeforeSetDictionary",
            td_test_compressed_before_set_dictionary,
        ))
        .method(t(
            "testCompressedEmptyAliasDictionary",
            td_test_compressed_empty_alias_dictionary,
        ))
        .method(t("testInvalidData", td_test_invalid_data))
        .method(t(
            "testUsingFlagsBeforeFlagDirective",
            td_test_using_flags_before_flag_directive,
        ))
        .method(t("testForgivableErrors", td_test_forgivable_errors))
        .method(t(
            "testCommonForgivableErrors",
            td_test_common_forgivable_errors,
        ))
        .method(t("testResourceCleanup", td_test_resource_cleanup))
        .method(t("testReplacements", td_test_replacements))
        .method(t(
            "testSetWithCrazyWhitespaceAndBOMs",
            td_test_set_with_crazy_whitespace_and_boms,
        ))
        .method(t(
            "testFlagWithCrazyWhitespace",
            td_test_flag_with_crazy_whitespace,
        ))
        .method(t("testUtf8Flag", td_test_utf8_flag).annotate(Test::new()))
        .method(
            t(
                "testCustomMorphologicalData",
                td_test_custom_morphological_data,
            )
            .annotate(Test::new()),
        )
        .build()
}

// -----------------------------------------------------------------------------
// TestHunspell
// -----------------------------------------------------------------------------

/// `new CancellationException()`.
fn cancellation_exception() -> LuceneError {
    LuceneError::unchecked("java.util.concurrent.CancellationException", "")
}

fn h_test_check_canceled() -> Result<()> {
    let canceled = Arc::new(AtomicBool::new(false));
    let c = Arc::clone(&canceled);
    let check_canceled: CheckCanceled = Arc::new(move || {
        if c.load(Ordering::SeqCst) {
            return Err(cancellation_exception());
        }
        Ok(())
    });
    let dictionary = Arc::new(load_dictionary_names(false, "simple.aff", &["simple.dic"])?);
    let hunspell = Hunspell::with_policy(dictionary, TimeoutPolicy::NoTimeout, check_canceled);

    Assert::assert_true(hunspell.spell("apache")?);
    assert_list_equals(None, &list(&["apach"]), &hunspell.suggest("apac")?);

    canceled.store(true, Ordering::SeqCst);
    expect_throws("java.util.concurrent.CancellationException", || {
        hunspell.spell("apache")
    });
    expect_throws("java.util.concurrent.CancellationException", || {
        hunspell.suggest("apac")
    });
    Ok(())
}

fn h_test_custom_check_canceled_gives_partial_result() -> Result<()> {
    let dictionary = Arc::new(load_dictionary_names(false, "simple.aff", &["simple.dic"])?);

    let expected = list(&["apach"]);
    assert_list_equals(
        None,
        &expected,
        &Hunspell::with_policy(Arc::clone(&dictionary), TimeoutPolicy::NoTimeout, noop())
            .suggest("apac")?,
    );

    let counter = Arc::new(AtomicI32::new(0));
    let msg = "msg";
    let c = Arc::clone(&counter);
    let check_canceled: CheckCanceled = Arc::new(move || {
        if c.fetch_add(1, Ordering::SeqCst) + 1 > 400 {
            return Err(SuggestionTimeoutException::error(
                Some(msg.to_string()),
                None,
            ));
        }
        Ok(())
    });

    let hunspell = Hunspell::with_policy(
        Arc::clone(&dictionary),
        TimeoutPolicy::ReturnPartialResult,
        Arc::clone(&check_canceled),
    );
    // pass a long timeout so that slower CI servers are more predictable.
    assert_list_equals(
        None,
        &expected,
        &hunspell.suggest_with_time_limit("apac", 24 * 60 * 60 * 1000)?,
    );

    counter.store(0, Ordering::SeqCst);
    let e = match Suggester::new(Arc::clone(&dictionary)).suggest_no_timeout("apac", check_canceled)
    {
        Ok(_) => Assert::fail(
            "Expected exception SuggestionTimeoutException but no exception was thrown",
        ),
        Err(e) => e,
    };
    if SuggestionTimeoutException::of(&e).is_none() {
        rethrow_java(e)
    }
    let e = SuggestionTimeoutException::of(&e).expect("a SuggestionTimeoutException");
    assert_list_equals(
        None,
        &expected,
        e.get_partial_result().expect("a partial result"),
    );
    eq_str("msg", e.get_message().unwrap_or("null"));
    Ok(())
}

fn h_test_suggestion_time_limit() -> Result<()> {
    let time_limit_ms = 10;

    let dictionary = Arc::new(load_dictionary_names(
        false,
        "timelimit.aff",
        &["simple.dic"],
    )?);
    let word = "qazwsxedcrfvtgbyhnujm";

    let incomplete = Hunspell::with_policy(
        Arc::clone(&dictionary),
        TimeoutPolicy::ReturnPartialResult,
        noop(),
    );
    Assert::assert_false(incomplete.spell(word)?);
    assert_list_equals(
        None,
        &[],
        &incomplete.suggest_with_time_limit(word, time_limit_ms)?,
    );

    let throwing = Hunspell::with_policy(
        Arc::clone(&dictionary),
        TimeoutPolicy::ThrowException,
        noop(),
    );
    Assert::assert_false(throwing.spell(word)?);

    let exception = match throwing.suggest_with_time_limit(word, time_limit_ms) {
        Ok(_) => Assert::fail(
            "Expected exception SuggestionTimeoutException but no exception was thrown",
        ),
        Err(e) => e,
    };
    if SuggestionTimeoutException::of(&exception).is_none() {
        rethrow_java(exception)
    }
    let exception =
        SuggestionTimeoutException::of(&exception).expect("a SuggestionTimeoutException");
    assert_list_equals(
        None,
        &[],
        exception.get_partial_result().expect("a partial result"),
    );

    let no_limit = Hunspell::with_policy(Arc::clone(&dictionary), TimeoutPolicy::NoTimeout, noop());
    assert_list_equals(
        None,
        &[],
        &no_limit.suggest_with_time_limit(&word[..5], time_limit_ms)?,
    );
    Ok(())
}

/// `loadNoTimeout(name)`.
fn load_no_timeout(name: &str) -> Result<Hunspell> {
    Ok(load_no_timeout_dictionary(Arc::new(load_dictionary_names(
        false,
        &format!("{name}.aff"),
        &[&format!("{name}.dic")],
    )?)))
}

/// `loadNoTimeout(dictionary)`.
fn load_no_timeout_dictionary(dictionary: Arc<Dictionary>) -> Hunspell {
    Hunspell::with_policy(dictionary, TimeoutPolicy::NoTimeout, noop())
}

fn h_test_stemming_api() -> Result<()> {
    let hunspell = load_no_timeout("simple")?;
    assert_list_equals(None, &list(&["apach"]), &hunspell.get_roots("apache")?);
    assert_list_equals(None, &list(&["foo"]), &hunspell.get_roots("foo")?);
    Ok(())
}

fn check_affixed_word(
    word: &AffixedWord,
    stem: &str,
    prefix_flags: &[&str],
    suffix_flags: &[&str],
) {
    eq_str(stem, &word.get_dict_entry().get_stem());
    let prefixes: Vec<String> = word.get_prefixes().iter().map(|a| a.get_flag()).collect();
    assert_list_equals(None, &list(prefix_flags), &prefixes);
    let suffixes: Vec<String> = word.get_suffixes().iter().map(|a| a.get_flag()).collect();
    assert_list_equals(None, &list(suffix_flags), &suffixes);
}

fn h_test_analysis_api() -> Result<()> {
    let hunspell = load_no_timeout("base")?;
    Assert::assert_true(hunspell.analyze_simple_word("nonexistent")?.is_empty());
    let word = hunspell.analyze_simple_word("recreated")?.remove(0);
    check_affixed_word(&word, "create", &["A"], &["D"]);
    Ok(())
}

fn h_test_analysis_several_suffixes() -> Result<()> {
    let hunspell = load_no_timeout("needaffix5")?;
    let word = hunspell
        .analyze_simple_word("pseudoprefoopseudosufbar")?
        .remove(0);
    check_affixed_word(&word, "foo", &["C"], &["B", "A"]);
    Ok(())
}

fn h_test_analysis_flag_long() -> Result<()> {
    let word = load_no_timeout("flaglong")?
        .analyze_simple_word("foos")?
        .remove(0);
    check_affixed_word(&word, "foo", &[], &["Y1"]);
    Ok(())
}

fn h_test_analysis_flag_num() -> Result<()> {
    let word = load_no_timeout("flagnum")?
        .analyze_simple_word("foos")?
        .remove(0);
    check_affixed_word(&word, "foo", &[], &["65000"]);
    Ok(())
}

fn h_test_analysis_morph_data() -> Result<()> {
    let words = load_no_timeout("morphdata")?.analyze_simple_word("works")?;
    Assert::assert_equals(2, words.len());
    let verb_index = if words[0]
        .get_dict_entry()
        .get_morphological_data()
        .contains("verb")
    {
        0
    } else {
        1
    };
    let verb = &words[verb_index];
    let noun = &words[if verb_index != 0 { 0 } else { 1 }];
    check_affixed_word(verb, "work", &[], &["A"]);
    check_affixed_word(noun, "work", &[], &["B"]);

    assert_list_equals(
        None,
        &list(&["worknoun"]),
        &noun.get_dict_entry().get_morphological_values("st:"),
    );
    assert_list_equals(
        None,
        &list(&["workverb"]),
        &verb.get_dict_entry().get_morphological_values("st:"),
    );
    eq_str(
        "st:worknoun",
        &noun.get_dict_entry().get_morphological_data(),
    );
    eq_str(
        "st:workverb",
        &verb.get_dict_entry().get_morphological_data(),
    );
    Ok(())
}

fn word_set(words: &[AffixedWord]) -> Vec<String> {
    let set: HashSet<String> = words.iter().map(AffixedWord::get_word).collect();
    sorted(&set)
}

fn h_test_expand_root_api() -> Result<()> {
    let h = load_no_timeout("base")?;
    let create_forms_base = [
        "create",
        "created",
        "creates",
        "creating",
        "creation",
        "creations",
    ];
    let mut expected: Vec<String> = create_forms_base
        .iter()
        .flat_map(|s| [s.to_string(), format!("pro{s}"), format!("re{s}")])
        .chain(std::iter::once("creative".to_string()))
        .collect();
    sort_utf16(&mut expected);

    let expanded: HashMap<String, AffixedWord> =
        check_expansion_generates_correct_words(&h, "create", "base")?
            .into_iter()
            .map(|w| (w.get_word(), w))
            .collect();
    let mut keys: Vec<String> = expanded.keys().cloned().collect();
    sort_utf16(&mut keys);
    assert_list_equals(None, &expected, &keys);

    check_affixed_word(&expanded["created"], "create", &[], &["D"]);
    check_affixed_word(&expanded["recreated"], "create", &["A"], &["D"]);

    let generator = WordFormGenerator::new(Arc::clone(&h.dictionary))?;
    let override_flag = generator.get_all_word_forms_with_flags("create", "U", &noop())?;
    assert_list_equals(
        None,
        &list(&["create", "uncreate"]),
        &word_set(&override_flag),
    );

    let non_existent_root = generator.get_all_word_forms_with_flags("form", "S", &noop())?;
    assert_list_equals(
        None,
        &list(&["form", "forms"]),
        &word_set(&non_existent_root),
    );
    Ok(())
}

fn check_compression(h: &Hunspell, expected: &str, words: &[&str]) -> Result<()> {
    let suggestion = h.compress(words)?.ok_or_else(LuceneError::null_pointer)?;
    eq_str(expected, &suggestion.internals_to_string());
    Ok(())
}

fn h_test_compressing_api() -> Result<()> {
    let h = load_no_timeout("base")?;
    let create_query = ["create", "created", "creates", "creating", "creation"];
    check_compression(
        &h,
        "toEdit=[create/DGNS], toAdd=[], extra=[]",
        &create_query,
    )?;
    check_compression(
        &h,
        "toEdit=[create/DS], toAdd=[], extra=[]",
        &["creates", "created"],
    )?;
    check_compression(
        &h,
        "toEdit=[], toAdd=[creation/S], extra=[]",
        &["creation", "creations"],
    )?;
    check_compression(&h, "toEdit=[], toAdd=[abc, def], extra=[]", &["abc", "def"])?;
    check_compression(
        &h,
        "toEdit=[], toAdd=[form/S], extra=[]",
        &["form", "forms"],
    )?;

    check_compression(
        &load_no_timeout("compress")?,
        "toEdit=[], toAdd=[form/X], extra=[forms]",
        &["form", "formx"],
    )?;
    Ok(())
}

fn string_set(items: &[&str]) -> HashSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn h_test_compressing_is_minimal() -> Result<()> {
    let h = load_no_timeout("compress")?;
    check_compression(
        &h,
        "toEdit=[], toAdd=[form/GS], extra=[]",
        &["formings", "forming", "form", "forms"],
    )?;

    check_compression(
        &h,
        "toEdit=[], toAdd=[f/def], extra=[]",
        &["f", "fd", "fe", "ff"],
    )?;

    let gen = WordFormGenerator::new(Arc::clone(&h.dictionary))?;
    let f_abc = gen
        .compress(
            &["f", "fa", "fb", "fc"],
            &string_set(&["fyy", "fxx"]),
            &noop(),
        )?
        .ok_or_else(LuceneError::null_pointer)?;
    eq_str(
        "toEdit=[], toAdd=[f/abc], extra=[]",
        &f_abc.internals_to_string(),
    );
    Ok(())
}

fn h_test_compressing_is_fast_on_large_unrelated_word_sets() -> Result<()> {
    let h = load_no_timeout("compress")?;
    let letters = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l"];
    check_compression(
        &h,
        "toEdit=[], toAdd=[a, b, c, d, e, f, g, h, i, j, k, l], extra=[]",
        &letters,
    )
}

fn h_test_compressing_with_prohibition() -> Result<()> {
    let gen = WordFormGenerator::new(Arc::clone(&load_no_timeout("compress")?.dictionary))?;
    eq_str(
        "toEdit=[], toAdd=[form/S], extra=[]",
        &gen.compress(&["form", "forms"], &string_set(&["formx"]), &noop())?
            .ok_or_else(LuceneError::null_pointer)?
            .internals_to_string(),
    );
    eq_str(
        "toEdit=[], toAdd=[form, formx], extra=[]",
        &gen.compress(&["form", "formx"], &string_set(&["forms"]), &noop())?
            .ok_or_else(LuceneError::null_pointer)?
            .internals_to_string(),
    );
    Ok(())
}

fn h_test_suggestion_order_stability_on_dictionary_editing() -> Result<()> {
    let original = "some_word";

    let mut words: Vec<String> = Vec::new();
    for c in 0u16..65535 {
        if is_letter(i32::from(c)) {
            // a letter is never a surrogate unit
            words.push(format!("{original}{}", to_rust_string(&[c])));
        }
    }

    let small_dict = format!("1\n{}", words[..words.len() / 4].join("\n"));
    let larger_dict = format!("1\n{}", words.join("\n"));
    let small = Arc::new(load_dictionary_streams(
        false,
        &mut Cursor::new(Vec::new()),
        vec![&mut Cursor::new(small_dict.into_bytes())],
    )?);
    let larger = Arc::new(load_dictionary_streams(
        false,
        &mut Cursor::new(Vec::new()),
        vec![&mut Cursor::new(larger_dict.into_bytes())],
    )?);

    Assert::assert_false(Hunspell::new(Arc::clone(&small)).spell(original)?);

    let small_sug = load_no_timeout_dictionary(small).suggest(original)?;
    let larger_sug = load_no_timeout_dictionary(larger).suggest(original)?;
    Assert::assert_equals_msg(Some(&java_list(&small_sug)), 4, small_sug.len());
    assert_list_equals(None, &small_sug, &larger_sug);
    Ok(())
}

fn hunspell_class() -> Arc<Class> {
    let test = |name: &str, body: fn() -> Result<()>| t(name, body).annotate(Test::new());
    lucene_class(&format!("{PACKAGE}.TestHunspell"))
        .method(t("testCheckCanceled", h_test_check_canceled))
        .method(t(
            "testCustomCheckCanceledGivesPartialResult",
            h_test_custom_check_canceled_gives_partial_result,
        ))
        .method(t("testSuggestionTimeLimit", h_test_suggestion_time_limit))
        .method(test("testStemmingApi", h_test_stemming_api))
        .method(test("testAnalysisApi", h_test_analysis_api))
        .method(test(
            "testAnalysisSeveralSuffixes",
            h_test_analysis_several_suffixes,
        ))
        .method(test("testAnalysisFlagLong", h_test_analysis_flag_long))
        .method(test("testAnalysisFlagNum", h_test_analysis_flag_num))
        .method(test("testAnalysisMorphData", h_test_analysis_morph_data))
        .method(test("testExpandRootApi", h_test_expand_root_api))
        .method(test("testCompressingApi", h_test_compressing_api))
        .method(test(
            "testCompressingIsMinimal",
            h_test_compressing_is_minimal,
        ))
        .method(test(
            "testCompressingIsFastOnLargeUnrelatedWordSets",
            h_test_compressing_is_fast_on_large_unrelated_word_sets,
        ))
        .method(test(
            "testCompressingWithProhibition",
            h_test_compressing_with_prohibition,
        ))
        .method(test(
            "testSuggestionOrderStabilityOnDictionaryEditing",
            h_test_suggestion_order_stability_on_dictionary_editing,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestSpellChecking
// -----------------------------------------------------------------------------

/// `doTest(name)`.
fn do_test(name: &str) -> Result<()> {
    let base = resource_path(name);
    check_spell_checker_expectations(&base)
}

/// `checkSpellCheckerExpectations(basePath)`.
fn check_spell_checker_expectations(base_path: &Path) -> Result<()> {
    check_spell_checker_expectations_with(
        base_path,
        SortingStrategy::offline(Arc::new(ByteBuffersDirectory::new()), "dictionary"),
    )?;
    check_spell_checker_expectations_with(base_path, SortingStrategy::in_memory())
}

/// `basePath + suffix`.
fn with_suffix(base_path: &Path, suffix: &str) -> PathBuf {
    let mut s = base_path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

fn check_spell_checker_expectations_with(
    base_path: &Path,
    strategy: SortingStrategy,
) -> Result<()> {
    let aff_file = with_suffix(base_path, ".aff");
    let dic_file = with_suffix(base_path, ".dic");
    let mut affix_stream = std::fs::File::open(&aff_file)?;
    let mut dict_stream = std::fs::File::open(&dic_file)?;

    let dictionary = Arc::new(Dictionary::with_sorting_strategy(
        &mut affix_stream,
        vec![&mut dict_stream],
        false,
        strategy,
    )?);
    let speller = Hunspell::with_policy(Arc::clone(&dictionary), TimeoutPolicy::NoTimeout, noop());
    let mut suggesters: Vec<(String, Suggester)> = Vec::new();
    let suggester = Suggester::new(Arc::clone(&dictionary));
    suggesters.push((
        "caching".to_string(),
        suggester.with_suggestible_entry_cache()?,
    ));
    suggesters.insert(0, ("default".to_string(), suggester));
    if dictionary.compound_rules.is_none()
        && dictionary.compound_begin == FLAG_UNSET
        && dictionary.compound_flag == FLAG_UNSET
    {
        for n in 2..=4 {
            let checker =
                NGramFragmentChecker::from_all_simple_words(n, Arc::clone(&dictionary), &noop())?;
            let with = suggesters[0].1.with_fragment_checker(Arc::new(checker));
            suggesters.push((format!("ngram{n}"), with));
        }
    }

    let good = with_suffix(base_path, ".good");
    if good.exists() {
        for word in read_all_lines(&good)? {
            Assert::assert_true_msg(
                &format!("Unexpectedly considered misspelled: {word}"),
                speller.spell(&java_trim(&word))?,
            );
        }
    }

    let wrong = with_suffix(base_path, ".wrong");
    let sug = with_suffix(base_path, ".sug");
    if wrong.exists() {
        let wrong_words = read_all_lines(&wrong)?;
        for word in &wrong_words {
            Assert::assert_false_msg(
                &format!("Unexpectedly considered correct: {word}"),
                speller.spell(&java_trim(word))?,
            );
        }
        if sug.exists() {
            // `Files.readString(sug).trim()`
            let sug_lines = java_trim(&String::from_utf8(std::fs::read(&sug)?).map_err(|e| {
                LuceneError::unchecked("java.nio.charset.MalformedInputException", e.to_string())
            })?);
            for (key, suggester) in &suggesters {
                eq_str_msg(
                    &format!("Suggester={key}"),
                    &sug_lines,
                    &suggest(suggester, &wrong_words)?,
                );
            }
        }
    } else {
        Assert::assert_false_msg(".sug file without .wrong file!", sug.exists());
    }

    let everything_generated = expand_whole_dictionary(&dic_file, &speller)?;
    if let Some(everything_generated) = everything_generated {
        if !speller.dictionary.may_need_input_cleaning() {
            check_good_sug_words_are_generated(&speller, &good, &sug, &everything_generated)?;
        }
    }
    Ok(())
}

fn suggest(suggester: &Suggester, wrong_words: &[String]) -> Result<String> {
    let mut lines = Vec::new();
    for s in wrong_words {
        let joined = suggester.suggest_no_timeout(s, noop())?.join(", ");
        if !joined.is_empty() {
            lines.push(joined);
        }
    }
    Ok(lines.join("\n"))
}

/// `s.toLowerCase(Locale.ROOT)`.
fn lower_root(s: &str) -> String {
    use rucene_analysis_common::jdk::locale::Locale;
    let root = Locale::builder_from_language_tag("und").expect("the root locale");
    to_rust_string(&rucene_analysis_common::jdk::string::to_lower_case(
        &units(s),
        &root,
    ))
}

fn expand_whole_dictionary(dic: &Path, speller: &Hunspell) -> Result<Option<HashSet<String>>> {
    let mut everything_generated = HashSet::new();
    let mut generated_everything = true;
    let lines = read_lines_with(dic, speller.dictionary.decoder.charset())?;
    for line in lines.iter().skip(1) {
        let u = units(line);
        let len = u
            .iter()
            .take_while(|&&c| !is_whitespace(i32::from(c)) && c != u16::from(b'/'))
            .count();
        let word = java_trim(&to_rust_string(&u[..len]));
        if word.is_empty() || word.contains('\\') {
            generated_everything = false;
            continue;
        }

        let expanded =
            check_expansion_generates_correct_words(speller, &word, &dic.to_string_lossy())?;
        for w in expanded {
            everything_generated.insert(lower_root(&w.get_word()));
        }
    }
    Ok(if generated_everything {
        Some(everything_generated)
    } else {
        None
    })
}

fn check_good_sug_words_are_generated(
    speller: &Hunspell,
    good: &Path,
    sug: &Path,
    everything_generated: &HashSet<String>,
) -> Result<()> {
    let mut good_words: HashSet<String> = HashSet::new();
    if good.exists() {
        for line in read_all_lines(good)? {
            good_words.insert(java_trim(&line));
        }
    }
    if sug.exists() {
        for line in read_all_lines(sug)? {
            for s in line.split(", ") {
                let s = java_trim(s);
                if !s.contains(' ') {
                    good_words.insert(s);
                }
            }
        }
    }

    good_words.retain(|s| !everything_generated.contains(s));
    good_words.retain(|s| *s == lower_root(s));
    let mut kept = HashSet::new();
    for s in good_words {
        if !speller.analyze_simple_word(&s)?.is_empty() {
            kept.insert(s);
        }
    }

    Assert::assert_true_msg(
        &format!(
            "Some *.good/sug words weren't generated: {}",
            java_list(&sorted(&kept))
        ),
        kept.is_empty(),
    );
    Ok(())
}

/// `checkExpansionGeneratesCorrectWords(hunspell, stem, baseName)`.
fn check_expansion_generates_correct_words(
    hunspell: &Hunspell,
    stem: &str,
    base_name: &str,
) -> Result<Vec<AffixedWord>> {
    let expanded = hunspell.get_all_word_forms(stem)?;
    let mut misspelled = Vec::new();
    for word in &expanded {
        if (!hunspell.spell(&word.get_word())?
            || hunspell.analyze_simple_word(&word.get_word())?.is_empty())
            && !misspelled.contains(word)
        {
            misspelled.push(word.clone());
        }
    }
    if !misspelled.is_empty() {
        Assert::fail(&format!(
            "Misspelled words generated in {base_name}: {}",
            java_list(&misspelled)
        ));
    }

    if expanded.iter().any(|e| e.get_word() == stem) {
        let words: Vec<String> = expanded.iter().map(AffixedWord::get_word).collect();
        let refs: Vec<&str> = words.iter().map(String::as_str).collect();
        if let Some(suggestion) = hunspell.compress(&refs)? {
            let message = format!(
                "Compression suggests a different stem from the original {stem} in {base_name}:{suggestion}"
            );
            Assert::assert_true_msg(
                &message,
                suggestion
                    .get_entries_to_edit()
                    .iter()
                    .any(|e| e.get_stem() == stem),
            );
        }
    }

    Ok(expanded)
}

fn sc_test_rep_suggestions() -> Result<()> {
    do_test("rep")?;

    let aff = resource_path("rep.aff");
    let dictionary = Arc::new(all_dictionaries_load_dictionary(&aff)?);
    let suggester = Suggester::new(dictionary);
    assert_list_equals(
        None,
        &list(&["auto's"]),
        &suggester.suggest_no_timeout("autos", noop())?,
    );
    assert_list_equals(
        None,
        &list(&["auto's", "auto"]),
        &suggester
            .proceed_past_rep()
            .suggest_no_timeout("autos", noop())?,
    );
    Ok(())
}

/// The `doTest(name)` methods of `TestSpellChecking`.
static SPELL_CHECKING_TESTS: &[(&str, &str)] = &[
    ("testEmpty", "empty"),
    ("testBase", "base"),
    ("testBaseUtf", "base_utf"),
    ("testKeepcase", "keepcase"),
    ("testAllcaps", "allcaps"),
    ("testPhSuggestions", "ph"),
    ("testPhSuggestions2", "ph2"),
    ("testForceUCase", "forceucase"),
    ("testCheckSharpS", "checksharps"),
    ("testIJ", "IJ"),
    ("testI53643_numbersWithSeparators", "i53643"),
    ("testCheckCompoundPattern", "checkcompoundpattern"),
    ("testCheckCompoundPattern2", "checkcompoundpattern2"),
    ("testCheckCompoundPattern3", "checkcompoundpattern3"),
    ("testDotless_i", "dotless_i"),
    ("testNeedAffixOnAffixes", "needaffix5"),
    ("testCompoundFlag", "compoundflag"),
    ("testFlagUtf8", "flagutf8"),
    ("testCheckCompoundCase", "checkcompoundcase"),
    ("testCheckCompoundDup", "checkcompounddup"),
    ("testCheckCompoundTriple", "checkcompoundtriple"),
    ("testSimplifiedTriple", "simplifiedtriple"),
    ("testCompoundForbid", "compoundforbid"),
    ("testBreak", "break"),
    ("testBreakDefault", "breakdefault"),
    ("testBreakOff", "breakoff"),
    ("testCheckCompoundRep", "checkcompoundrep"),
    (
        "testDisallowCompoundsWhenDictionaryContainsSeparatedWordPair",
        "wordpair",
    ),
    ("testCompoundrule", "compoundrule"),
    ("testCompoundrule2", "compoundrule2"),
    ("testCompoundrule3", "compoundrule3"),
    ("testCompoundrule4", "compoundrule4"),
    ("testCompoundrule5", "compoundrule5"),
    ("testCompoundrule6", "compoundrule6"),
    ("testCompoundrule7", "compoundrule7"),
    ("testCompoundrule8", "compoundrule8"),
    (
        "testDisallowCompoundOnlySuffixesAtTheVeryEnd",
        "onlyincompound2",
    ),
    ("testForbiddenWord", "forbiddenword"),
    ("testForbiddenWord1", "opentaal_forbiddenword1"),
    ("testForbiddenWord2", "opentaal_forbiddenword2"),
    ("testGermanCompounding", "germancompounding"),
    ("testGermanManualCase", "germanManualCase"),
    ("testApplyOconvToSuggestions", "oconv"),
    ("testModifyingSuggestions", "sug"),
    ("testModifyingSuggestions2", "sug2"),
    ("testGeneratedSuggestions", "ngram"),
    (
        "testMaxNGramSugsDefaultIsNotUnlimited",
        "maxNGramSugsDefault",
    ),
    ("testMixedCaseSuggestionHeuristics", "i58202"),
    ("testMapSuggestions", "map"),
    ("testNoSuggest", "nosuggest"),
    ("testUseUSKeyboardAsDefaultKeyValue", "keyDefault"),
    (
        "testDoNotFailWhenExpandingRootWithLongerStrippingAffix",
        "longStrip",
    ),
];

/// `Method` bodies are plain `fn`s: one per `doTest` call, generated.
macro_rules! spell_checking_fns {
    ($($f:ident => $name:literal),* $(,)?) => {
        $(fn $f() -> Result<()> { do_test($name) })*
        fn spell_checking_body(name: &str) -> fn() -> Result<()> {
            match name {
                $($name => $f,)*
                _ => panic!("no doTest body for {name}"),
            }
        }
    };
}

spell_checking_fns! {
    sc_empty => "empty", sc_base => "base", sc_base_utf => "base_utf",
    sc_keepcase => "keepcase", sc_allcaps => "allcaps", sc_ph => "ph", sc_ph2 => "ph2",
    sc_forceucase => "forceucase", sc_checksharps => "checksharps", sc_ij => "IJ",
    sc_i53643 => "i53643", sc_ccp => "checkcompoundpattern",
    sc_ccp2 => "checkcompoundpattern2", sc_ccp3 => "checkcompoundpattern3",
    sc_dotless_i => "dotless_i", sc_needaffix5 => "needaffix5",
    sc_compoundflag => "compoundflag", sc_flagutf8 => "flagutf8",
    sc_ccc => "checkcompoundcase", sc_ccd => "checkcompounddup",
    sc_cct => "checkcompoundtriple", sc_simplifiedtriple => "simplifiedtriple",
    sc_compoundforbid => "compoundforbid", sc_break => "break",
    sc_breakdefault => "breakdefault", sc_breakoff => "breakoff",
    sc_ccr => "checkcompoundrep", sc_wordpair => "wordpair",
    sc_cr => "compoundrule", sc_cr2 => "compoundrule2", sc_cr3 => "compoundrule3",
    sc_cr4 => "compoundrule4", sc_cr5 => "compoundrule5", sc_cr6 => "compoundrule6",
    sc_cr7 => "compoundrule7", sc_cr8 => "compoundrule8",
    sc_onlyincompound2 => "onlyincompound2", sc_forbiddenword => "forbiddenword",
    sc_fw1 => "opentaal_forbiddenword1", sc_fw2 => "opentaal_forbiddenword2",
    sc_germancompounding => "germancompounding", sc_germanmanualcase => "germanManualCase",
    sc_oconv => "oconv", sc_sug => "sug", sc_sug2 => "sug2", sc_ngram => "ngram",
    sc_maxngram => "maxNGramSugsDefault", sc_i58202 => "i58202", sc_map => "map",
    sc_nosuggest => "nosuggest", sc_keydefault => "keyDefault", sc_longstrip => "longStrip",
}

fn spell_checking_class() -> Arc<Class> {
    let mut b = lucene_class(&format!("{PACKAGE}.TestSpellChecking"));
    for (method, name) in SPELL_CHECKING_TESTS {
        b = b.method(t(method, spell_checking_body(name)));
        if *method == "testAllcaps" {
            b = b.method(t("testRepSuggestions", sc_test_rep_suggestions));
        }
    }
    b.build()
}

// -----------------------------------------------------------------------------
// TestHunspellStemFilter
// -----------------------------------------------------------------------------

/// `TestHunspellStemFilter.dictionary`, the static field.
static FILTER_DICTIONARY: Mutex<Option<Arc<Dictionary>>> = Mutex::new(None);

fn filter_dictionary() -> Arc<Dictionary> {
    Arc::clone(
        FILTER_DICTIONARY
            .lock()
            .unwrap()
            .as_ref()
            .expect("beforeClass loaded the dictionary"),
    )
}

/// `@BeforeClass beforeClass()`.
fn hsf_before_class() -> Result<()> {
    let mut affix_stream = resource_stream("simple.aff").ok_or_else(LuceneError::null_pointer)?;
    let mut dict_stream = resource_stream("simple.dic").ok_or_else(LuceneError::null_pointer)?;
    let temp_dir = get_directory();
    let dictionary = Dictionary::new(
        temp_dir.as_directory(),
        "dictionary",
        &mut affix_stream,
        &mut dict_stream,
    )?;
    *FILTER_DICTIONARY.lock().unwrap() = Some(Arc::new(dictionary));
    temp_dir.close()?;
    Ok(())
}

/// Simple test for KeywordAttribute
fn hsf_test_keyword_attribute() -> Result<()> {
    let mut tokenizer = B::whitespace_mock_tokenizer_str("lucene is awesome")?;
    tokenizer.set_enable_checks(true);
    let mut filter = new_hunspell_stem_filter(boxed(tokenizer), filter_dictionary())?;
    B::assert_token_stream_contents_pos_increments(
        &mut filter,
        &["lucene", "lucen", "is", "awesome"],
        &[1, 0, 1, 1],
    )?;

    // assert with keyword marker
    let tokenizer = B::whitespace_mock_tokenizer_str("lucene is awesome")?;
    let set = CharArraySet::from_iter(["Lucene"], true);
    let mut filter = new_hunspell_stem_filter(
        boxed(new_set_keyword_marker_filter(boxed(tokenizer), set)?),
        filter_dictionary(),
    )?;
    B::assert_token_stream_contents_pos_increments(
        &mut filter,
        &["lucene", "is", "awesome"],
        &[1, 1, 1],
    )?;
    Ok(())
}

/// simple test for longestOnly option
fn hsf_test_longest_only() -> Result<()> {
    let mut tokenizer = B::whitespace_mock_tokenizer_str("lucene is awesome")?;
    tokenizer.set_enable_checks(true);
    let mut filter =
        new_hunspell_stem_filter_with(boxed(tokenizer), filter_dictionary(), true, true)?;
    B::assert_token_stream_contents_pos_increments(
        &mut filter,
        &["lucene", "is", "awesome"],
        &[1, 1, 1],
    )?;
    Ok(())
}

/// blast some random strings through the analyzer
fn hsf_test_random_strings() -> Result<()> {
    let dictionary = filter_dictionary();
    let analyzer = tokenizer_analyzer(
        "org.apache.lucene.analysis.hunspell.TestHunspellStemFilter$1",
        || {
            MockTokenizer::new(
                Arc::clone(&rucene_test_framework::analysis::mock_tokenizer::WHITESPACE),
                false,
            )
        },
        move |t| Ok(boxed(new_hunspell_stem_filter(t, Arc::clone(&dictionary))?)),
    );
    B::check_random_data(
        &*LuceneTestCase::random(),
        &analyzer,
        200 * *RANDOM_MULTIPLIER,
    )?;
    analyzer.close();
    Ok(())
}

/// `new Analyzer() { KeywordTokenizer -> HunspellStemFilter(d) }`.
fn keyword_stem_analyzer(name: &'static str, d: Arc<Dictionary>) -> Arc<dyn Analyzer> {
    support::keyword_tokenizer_analyzer(name, move |t| {
        Ok(boxed(new_hunspell_stem_filter(t, Arc::clone(&d))?))
    })
}

fn hsf_test_empty_term() -> Result<()> {
    let a = keyword_stem_analyzer(
        "org.apache.lucene.analysis.hunspell.TestHunspellStemFilter$2",
        filter_dictionary(),
    );
    B::check_one_term(&*a, "", "")?;
    a.close();
    Ok(())
}

fn hsf_test_ignore_case_no_side_effects() -> Result<()> {
    let mut affix_stream = resource_stream("simple.aff").ok_or_else(LuceneError::null_pointer)?;
    let mut dict_stream = resource_stream("simple.dic").ok_or_else(LuceneError::null_pointer)?;
    let temp_dir = get_directory();
    let d = Arc::new(Dictionary::with_dictionaries(
        temp_dir.as_directory(),
        "dictionary",
        &mut affix_stream,
        vec![&mut dict_stream],
        true,
    )?);
    let a = keyword_stem_analyzer(
        "org.apache.lucene.analysis.hunspell.TestHunspellStemFilter$3",
        d,
    );
    B::check_one_term(&*a, "NoChAnGy", "NoChAnGy")?;
    a.close();
    temp_dir.close()?;
    Ok(())
}

fn hunspell_stem_filter_class() -> Arc<Class> {
    btstc_class(&format!("{PACKAGE}.TestHunspellStemFilter"))
        .method(
            Method::new("beforeClass")
                .modifiers(modifier::PUBLIC | modifier::STATIC)
                .throws(&["java.lang.Exception"])
                .annotate(BeforeClass)
                .static_body(|| hsf_before_class().unwrap_or_else(|e| rethrow_java(e))),
        )
        .method(
            Method::new("afterClass")
                .modifiers(modifier::PUBLIC | modifier::STATIC)
                .annotate(AfterClass)
                .static_body(|| *FILTER_DICTIONARY.lock().unwrap() = None),
        )
        .method(t("testKeywordAttribute", hsf_test_keyword_attribute))
        .method(t("testLongestOnly", hsf_test_longest_only))
        .method(t("testRandomStrings", hsf_test_random_strings))
        .method(t("testEmptyTerm", hsf_test_empty_term))
        .method(t(
            "testIgnoreCaseNoSideEffects",
            hsf_test_ignore_case_no_side_effects,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestHunspellStemFilterFactory
// -----------------------------------------------------------------------------

fn hsff_test_stemming() -> Result<()> {
    let stream = B::whitespace_mock_tokenizer_str("abc")?;
    let mut stream = token_filter_factory(
        PACKAGE,
        "HunspellStem",
        &["dictionary", "simple.dic", "affix", "simple.aff"],
    )?
    .create(boxed(stream))?;
    B::assert_token_stream_contents(&mut *stream, &["ab"])?;
    Ok(())
}

/// Test that bogus arguments result in exception
fn hsff_test_bogus_arguments() -> Result<()> {
    let expected = expect_throws("java.lang.IllegalArgumentException", || {
        token_filter_factory(
            PACKAGE,
            "HunspellStem",
            &["dictionary", "simple.dic", "bogusArg", "bogusValue"],
        )
    });
    Assert::assert_true(
        expected
            .message()
            .unwrap_or("")
            .contains("Unknown parameters"),
    );
    Ok(())
}

fn hunspell_stem_filter_factory_class() -> Arc<Class> {
    factory_class(&format!("{PACKAGE}.TestHunspellStemFilterFactory"))
        .method(t("testStemming", hsff_test_stemming))
        .method(t("testBogusArguments", hsff_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestTrigramAutomaton
// -----------------------------------------------------------------------------

fn check_scores(s1: &str, s2: &str) -> Result<()> {
    let message = format!("Fails: checkScores(\"{s1}\", \"{s2}\")");
    let (u1, u2) = (units(s1), units(s2));
    // `new CharsRef(s)`: the units of `s`
    Assert::assert_equals_msg(
        Some(&message),
        ngram_score(3, &u1, &u2, false),
        TrigramAutomaton::new(&u1)?.ngram_score(&u2),
    );
    Assert::assert_equals_msg(
        Some(&message),
        ngram_score(3, &u2, &u1, false),
        TrigramAutomaton::new(&u2)?.ngram_score(&u1),
    );
    Ok(())
}

fn ta_test_same_score() -> Result<()> {
    check_scores("look", "looked")?;
    check_scores("look", "cool")?;
    check_scores("abracadabra", "abraham")
}

fn random_concatenation(alphabet: &[&str]) -> String {
    let random = LuceneTestCase::random();
    // `random().ints(0, n).limit(random().nextInt(20) + 1)`: the limit is
    // drawn first, the stream lazily
    let limit = random.next_int_bound(20) + 1;
    let mut s = String::new();
    for _ in 0..limit {
        s.push_str(alphabet[random.next_int_range(0, alphabet.len() as i32) as usize]);
    }
    s
}

fn ta_test_randomized() -> Result<()> {
    let alphabet = [
        "a",
        "b",
        "c",
        "aa",
        "ab",
        "abc",
        "ccc",
        "\u{103D1}",
        "\u{103D2}",
        "\u{103D2}\u{103D1}",
        "\u{103D2}\u{103D2}",
    ];
    for _ in 0..100 {
        check_scores(
            &random_concatenation(&alphabet),
            &random_concatenation(&alphabet),
        )?;
    }
    Ok(())
}

fn trigram_automaton_class() -> Arc<Class> {
    lucene_class(&format!("{PACKAGE}.TestTrigramAutomaton"))
        .method(t("testSameScore", ta_test_same_score))
        .method(t("testRandomized", ta_test_randomized))
        .build()
}

// -----------------------------------------------------------------------------
// TestAllDictionaries
// -----------------------------------------------------------------------------

/// `findAllAffixFiles()`: every `*.aff` under `hunspell.dictionaries`,
/// sorted; assumed away when the property is unset.
fn find_all_affix_files() -> Vec<PathBuf> {
    let dic_dir = system::get_property("hunspell.dictionaries");
    Assume::assume_true_msg(
        "Requires Hunspell dictionaries at -Dhunspell.dictionaries=...",
        dic_dir.is_some(),
    );
    let mut files = Vec::new();
    walk(Path::new(&dic_dir.unwrap()), &mut files);
    files.retain(|f| f.to_string_lossy().ends_with(".aff"));
    files.sort();
    files
}

/// `Files.walk(path)`.
fn walk(path: &Path, out: &mut Vec<PathBuf>) {
    out.push(path.to_path_buf());
    if path.is_dir() {
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                walk(&entry.path(), out);
            }
        }
    }
}

/// The anonymous subclass of `TestAllDictionaries.loadDictionary`.
struct Tolerant;

impl DictionarySubclass for Tolerant {
    fn tolerate_affix_rule_count_mismatches(&self) -> bool {
        true
    }

    fn tolerate_rep_rule_count_mismatches(&self) -> bool {
        true
    }
}

/// `TestAllDictionaries.loadDictionary(aff)`.
fn all_dictionaries_load_dictionary(aff: &Path) -> Result<Dictionary> {
    let aff_path = aff.to_string_lossy();
    let dic = PathBuf::from(format!("{}.dic", &aff_path[..aff_path.len() - 4]));
    debug_assert!(dic.exists(), "{}", dic.display());
    let mut dictionary = std::fs::File::open(&dic)?;
    let mut affix = std::fs::File::open(aff)?;
    Dictionary::with_subclass(
        &mut affix,
        vec![&mut dictionary],
        false,
        SortingStrategy::in_memory(),
        Arc::new(Tolerant),
    )
}

/// `@Ignore testMaxPrologueNeeded()`.
fn ad_test_max_prologue_needed() -> Result<()> {
    let mut fail_test = false;

    let mut global: Vec<(String, Vec<u64>)> = Vec::new();
    for aff in find_all_affix_files() {
        let mut local: Vec<(String, Vec<u64>)> = Vec::new();
        let bytes = std::fs::read(&aff)?;
        let mut line = Vec::new();
        for (pos, &chr) in bytes.iter().enumerate() {
            line.push(chr);
            if chr == b'\n' {
                // `baos.toString(ISO_8859_1)`
                let text: String = line.iter().map(|&b| char::from(b)).collect();
                if !text.trim().is_empty() {
                    let first_word = text.split(char::is_whitespace).next().unwrap_or("");
                    if first_word == "SET" || first_word == "FLAG" {
                        let position = pos as u64 + 1;
                        push_position(&mut local, first_word, position);
                        push_position(&mut global, first_word, position);
                    }
                }
                line.clear();
            }
        }

        for (flag, positions) in &local {
            if positions.len() > 1 {
                println!(
                    "Flag {flag} at more than one position in {}: {}",
                    aff.display(),
                    java_list(positions)
                );
                fail_test = true;
            }
        }
    }

    for (flag, positions) in &global {
        let max = positions.iter().copied().max().unwrap_or(0);
        println!("Flag {flag} at maximum offset {max}");
        Assert::assert_true_msg(
            &format!("Flags beyond max prologue scan window: {max}"),
            max < MAX_PROLOGUE_SCAN_WINDOW as u64,
        );
    }

    if fail_test {
        Assert::fail("Duplicate flags were present in at least one .aff file.");
    }
    Ok(())
}

fn push_position(map: &mut Vec<(String, Vec<u64>)>, key: &str, position: u64) {
    match map.iter_mut().find(|(k, _)| k == key) {
        Some((_, v)) => v.push(position),
        None => map.push((key.to_string(), vec![position])),
    }
}

fn ad_test_dictionaries_load_successfully() -> Result<()> {
    let failures: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    let process = |aff: &Path| {
        let r: Result<()> = (|| {
            let dic = Arc::new(all_dictionaries_load_dictionary(aff)?);
            Hunspell::new(Arc::clone(&dic)).spell("aaaa")?;
            let suggester = Suggester::new(Arc::clone(&dic)).with_suggestible_entry_cache()?;
            match suggester.suggest_with_timeout("aaaaaaaaaa", SUGGEST_TIME_LIMIT, noop()) {
                Ok(_) => {}
                Err(e) if SuggestionTimeoutException::of(&e).is_some() => {}
                Err(e) => return Err(e),
            }
            println!("{}", aff.display());
            Ok(())
        })();
        if let Err(e) = r {
            failures.lock().unwrap().push(aff.to_path_buf());
            eprintln!("While checking {}:\n{e:?}", aff.display());
        }
    };

    let tasks = find_all_affix_files();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    std::thread::scope(|scope| {
        for chunk in tasks.chunks(tasks.len().div_ceil(threads).max(1)) {
            let process = &process;
            scope.spawn(move || {
                for aff in chunk {
                    process(aff);
                }
            });
        }
    });

    let failures = failures.into_inner().unwrap();
    if !failures.is_empty() {
        let list: Vec<String> = failures
            .iter()
            .map(|p| {
                std::fs::canonicalize(p)
                    .unwrap_or_else(|_| p.clone())
                    .display()
                    .to_string()
            })
            .collect();
        Assert::fail(&format!(
            "Certain dictionaries failed to parse:\n  - {}",
            list.join("\n  - ")
        ));
    }

    println!("Total dictionaries loaded: {}", tasks.len());
    Ok(())
}

fn all_dictionaries_class() -> Arc<Class> {
    lucene_class(&format!("{PACKAGE}.TestAllDictionaries"))
        .method(t("testMaxPrologueNeeded", ad_test_max_prologue_needed).annotate(Ignore::new()))
        .method(t(
            "testDictionariesLoadSuccessfully",
            ad_test_dictionaries_load_successfully,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestPerformance
// -----------------------------------------------------------------------------

/// `TestPerformance.corporaDir`.
static CORPORA_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// `@BeforeClass resolveCorpora()`.
fn resolve_corpora() {
    let dir = system::get_property("hunspell.corpora");
    Assume::assume_true_msg(
        "Requires test word corpora at -Dhunspell.corpora=...",
        dir.is_some(),
    );
    *CORPORA_DIR.lock().unwrap() = dir.map(PathBuf::from);
}

fn perf_load_dictionary(code: &str) -> Result<Arc<Dictionary>> {
    let start = std::time::Instant::now();
    let aff = find_aff_file(code);
    let dictionary = all_dictionaries_load_dictionary(&aff)?;
    println!(
        "Loaded {} in {}ms",
        aff.display(),
        start.elapsed().as_millis()
    );
    Ok(Arc::new(dictionary))
}

fn find_aff_file(code: &str) -> PathBuf {
    find_all_affix_files()
        .into_iter()
        .find(|path| {
            let parent_name = path
                .parent()
                .and_then(Path::file_name)
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            to_rust_string(extract_language_code(&units(&parent_name))) == code
        })
        .unwrap_or_else(|| Assume::violated(&format!("Ignored, cannot find aff/dic for: {code}")))
}

fn load_words(code: &str, word_count: usize, dictionary: &Dictionary) -> Result<Vec<String>> {
    let corpora = CORPORA_DIR.lock().unwrap().clone().expect("corpora dir");
    let data_path = corpora.join(format!("{code}.txt"));
    if std::fs::File::open(&data_path).is_err() {
        Assume::violated(&format!("Missing text corpora at: {}", data_path.display()));
    }

    let word_chars = to_rust_string(&dictionary.word_chars);
    let mut words = Vec::new();
    for line in read_all_lines(&data_path)? {
        // `line.split("[^\\p{IsLetter}" + Pattern.quote(wordChars) + "]+")`
        let mut token = String::new();
        let mut tokens = Vec::new();
        for c in line.chars() {
            if is_letter(c as i32) || word_chars.contains(c) {
                token.push(c);
            } else if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            } else if tokens.is_empty() {
                tokens.push(String::new());
            }
        }
        if !token.is_empty() {
            tokens.push(token);
        }
        for token in tokens {
            if let Some(word) = strip_punctuation(&token) {
                words.push(word);
                if words.len() == word_count {
                    return Ok(words);
                }
            }
        }
    }
    Ok(words)
}

fn measure(what: &str, word_count: usize, iteration: &dyn Fn()) {
    // warmup
    for _ in 0..2 {
        iteration();
    }

    let mut times = Vec::new();
    for _ in 0..7 {
        let start = std::time::Instant::now();
        iteration();
        times.push(start.elapsed().as_millis() as i64);
    }
    let average = times.iter().sum::<i64>() as f64 / times.len() as f64;
    println!(
        "{what}: average {average} ({} per word), all times = {}",
        average / word_count as f64,
        java_list(&times)
    );
}

fn check_analysis_performance(code: &str, word_count: usize) -> Result<()> {
    let dictionary = perf_load_dictionary(code)?;

    let words = load_words(code, word_count, &dictionary)?;
    let half_words = &words[..words.len() / 2];

    let stemmer = Stemmer::new(Arc::clone(&dictionary));
    let speller = Hunspell::with_policy(Arc::clone(&dictionary), TimeoutPolicy::NoTimeout, noop());
    let cpus = std::thread::available_parallelism().map_or(1, |n| n.get());

    let stem_words = |words: &[String], stemmer: &Stemmer| {
        for word in words {
            let u = units(word);
            stemmer
                .stem(&u, u.len())
                .unwrap_or_else(|e| rethrow_java(e));
        }
    };

    measure(&format!("Stemming {code}"), words.len(), &|| {
        stem_words(&words, &stemmer)
    });

    measure(
        &format!("Multi-threaded stemming {code}"),
        words.len(),
        &|| {
            std::thread::scope(|scope| {
                for _ in 0..cpus {
                    let local_stemmer = Stemmer::new(Arc::clone(&dictionary));
                    let stem_words = &stem_words;
                    scope.spawn(move || stem_words(half_words, &local_stemmer));
                }
            });
        },
    );

    measure(&format!("Spellchecking {code}"), words.len(), &|| {
        for word in &words {
            speller.spell(word).unwrap_or_else(|e| rethrow_java(e));
        }
    });

    println!();
    Ok(())
}

/// The `de` fragment checker of `fragmentChecker(dictionary, "de")`.
struct GermanChecker(NGramFragmentChecker);

impl FragmentChecker for GermanChecker {
    fn has_impossible_fragment_around(&self, word: &[u16], start: usize, end: usize) -> bool {
        word[0] != u16::from(b'-')
            && word[word.len() - 1] != u16::from(b'-')
            && self.0.has_impossible_fragment_around(word, start, end)
    }
}

fn fragment_checker(
    dictionary: &Arc<Dictionary>,
    lang_code: &str,
) -> Result<Arc<dyn FragmentChecker>> {
    let started = std::time::Instant::now();
    let trigram = NGramFragmentChecker::from_all_simple_words(3, Arc::clone(dictionary), &noop())?;
    println!(
        "Populated {} trigram hashes in {}ms",
        trigram.hash_count(),
        started.elapsed().as_millis()
    );
    if lang_code == "de" {
        return Ok(Arc::new(GermanChecker(trigram)));
    }
    Ok(Arc::new(trigram))
}

fn has_quick_suggestions(
    speller: &Hunspell,
    base: &Suggester,
    optimized: &Suggester,
    word: &str,
) -> Result<bool> {
    if speller.spell(word)? {
        return Ok(false);
    }

    let from_optimized = match optimized.suggest_with_timeout(word, SUGGEST_TIME_LIMIT, noop()) {
        Ok(s) => s,
        Err(e) if SuggestionTimeoutException::of(&e).is_some() => {
            println!("Timeout happened for {word}, skipping");
            return Ok(false);
        }
        Err(e) => return Err(e),
    };

    let from_base = base.suggest_no_timeout(word, noop())?;
    if from_base != from_optimized {
        Assert::fail(&format!(
            "Optimization breaks suggestions: for '{word}', base={}, optimized={}",
            java_list(&from_base),
            java_list(&from_optimized)
        ));
    }

    Ok(true)
}

fn check_suggestion_performance(code: &str, word_count: usize) -> Result<()> {
    let dictionary = perf_load_dictionary(code)?;
    let base = Suggester::new(Arc::clone(&dictionary));
    let optimized = base
        .with_suggestible_entry_cache()?
        .with_fragment_checker(fragment_checker(&dictionary, code)?);
    let speller = Hunspell::with_policy(
        Arc::clone(&dictionary),
        TimeoutPolicy::ThrowException,
        noop(),
    );
    let mut seen = HashSet::new();
    let mut words = Vec::new();
    for w in load_words(code, word_count, &dictionary)? {
        if seen.insert(w.clone()) && has_quick_suggestions(&speller, &base, &optimized, &w)? {
            words.push(w);
        }
    }
    println!("Checking {} misspelled words", words.len());

    measure(&format!("Suggestions for {code}"), words.len(), &|| {
        for word in &words {
            optimized
                .suggest_no_timeout(word, noop())
                .unwrap_or_else(|e| rethrow_java(e));
        }
    });
    println!();
    Ok(())
}

/// `stripPunctuation(token)`.
fn strip_punctuation(token: &str) -> Option<String> {
    let u = units(token);
    let mut start = 0;
    let mut end = u.len();
    while start < end && is_punctuation(u[start]) {
        start += 1;
    }
    while start + 1 < end && is_punctuation(u[end - 1]) {
        end -= 1;
    }
    (start < end).then(|| to_rust_string(&u[start..end]))
}

fn is_punctuation(c: u16) -> bool {
    units(".!?,\"'\u{2019}\u{2018}").contains(&c)
}

macro_rules! perf_fns {
    ($($f:ident => $kind:ident($code:literal, $n:literal)),* $(,)?) => {
        $(fn $f() -> Result<()> { $kind($code, $n) })*
    };
}

perf_fns! {
    perf_en => check_analysis_performance("en", 1_200_000),
    perf_en_suggest => check_suggestion_performance("en", 3_500),
    perf_ru => check_analysis_performance("ru", 400_000),
    perf_ru_suggest => check_suggestion_performance("ru", 1000),
    perf_de => check_analysis_performance("de", 300_000),
    perf_de_suggest => check_suggestion_performance("de", 250),
    perf_fr => check_analysis_performance("fr", 100_000),
    perf_fr_suggest => check_suggestion_performance("fr", 1_000),
    perf_uk => check_analysis_performance("uk", 200_000),
    perf_uk_suggest => check_suggestion_performance("uk", 800),
}

fn performance_class() -> Arc<Class> {
    let test = |name: &str, body: fn() -> Result<()>| t(name, body).annotate(Test::new());
    lucene_class(&format!("{PACKAGE}.TestPerformance"))
        .method(
            Method::new("resolveCorpora")
                .modifiers(modifier::PUBLIC | modifier::STATIC)
                .annotate(BeforeClass)
                .static_body(resolve_corpora),
        )
        .method(test("en", perf_en))
        .method(test("en_suggest", perf_en_suggest))
        .method(test("ru", perf_ru))
        .method(test("ru_suggest", perf_ru_suggest))
        .method(test("de", perf_de))
        .method(test("de_suggest", perf_de_suggest))
        .method(test("fr", perf_fr))
        .method(test("fr_suggest", perf_fr_suggest))
        .method(test("uk", perf_uk))
        .method(test("uk_suggest", perf_uk_suggest))
        .build()
}

// -----------------------------------------------------------------------------
// TestHunspellRepositoryTestCases
// -----------------------------------------------------------------------------

/// `EXPECTED_FAILURES`.
const EXPECTED_FAILURES: [&str; 6] = [
    "hu",    // Hungarian is hard: a lot of its rules are hardcoded in Hunspell code, not aff/dic
    "morph", // we don't do morphological analysis yet
    "opentaal_keepcase", // Hunspell bug: https://github.com/hunspell/hunspell/issues/712
    "nepali", // not supported yet
    "utf8_nonbmp", // code points not supported yet
    "phone", // not supported yet, used only for suggestions in en_ZA
];

/// `@Parameterized.Parameters data()`: `None` for its
/// `AssumptionViolatedException` (property unset).
fn repository_data() -> Option<Vec<(String, PathBuf)>> {
    let hunspell_repo = system::get_property("hunspell.repo.path")?;

    let tests = Path::new(&hunspell_repo).join("tests");
    let mut names: Vec<String> = std::fs::read_dir(&tests)
        .expect("Files.newDirectoryStream(tests)")
        .flatten()
        .filter_map(|file| {
            let name = file.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".aff").map(str::to_string)
        })
        .collect();
    names.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    names.dedup();

    Some(
        names
            .into_iter()
            .map(|s| {
                let path = tests.join(&s);
                (s, path)
            })
            .collect(),
    )
}

/// `test()` of one parameter.
fn repository_test(test_name: &str, path_prefix: &Path) {
    let test = || check_spell_checker_expectations(path_prefix);
    if EXPECTED_FAILURES.contains(&test_name) {
        // `Assert.assertThrows(Throwable.class, test)`: an error or a failed
        // assertion
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(test));
        if matches!(outcome, Ok(Ok(()))) {
            Assert::fail("expected java.lang.Throwable to be thrown, but nothing was thrown");
        }
    } else {
        test().unwrap_or_else(|e| rethrow_java(e));
    }
}

#[test]
fn test_hunspell_repository_test_cases() {
    let Some(data) = repository_data() else {
        println!(
            "TestHunspellRepositoryTestCases: assumption violated: hunspell.repo.path property \
             not specified."
        );
        return;
    };
    let mut failures = Vec::new();
    for (name, path) in &data {
        let r = std::panic::catch_unwind(|| repository_test(name, path));
        println!("test[{name}]: {}", if r.is_ok() { "ok" } else { "FAILED" });
        if r.is_err() {
            failures.push(name.clone());
        }
    }
    assert!(failures.is_empty(), "failed: {failures:?}");
}

// -----------------------------------------------------------------------------
// Suites
// -----------------------------------------------------------------------------

macro_rules! stemmer_suite_tests {
    ($($f:ident => $name:literal),* $(,)?) => {
        $(
            #[test]
            fn $f() {
                support::run(|| stemmer_suite($name));
            }
        )*
    };
}

stemmer_suite_tests! {
    test_all_caps => "TestAllCaps",
    test_alternate_casing => "TestAlternateCasing",
    test_case_insensitive => "TestCaseInsensitive",
    test_case_sensitive => "TestCaseSensitive",
    test_check_sharp_s => "TestCheckSharpS",
    test_circumfix => "TestCircumfix",
    test_complex_prefix => "TestComplexPrefix",
    test_compressed => "TestCompressed",
    test_condition => "TestCondition",
    test_condition2 => "TestCondition2",
    test_conv => "TestConv",
    test_dependencies => "TestDependencies",
    test_double_escape => "TestDoubleEscape",
    test_dutch_ij => "TestDutchIJ",
    test_escaped => "TestEscaped",
    test_flag_long => "TestFlagLong",
    test_flag_num => "TestFlagNum",
    test_full_strip => "TestFullStrip",
    test_homonyms => "TestHomonyms",
    test_ignore => "TestIgnore",
    test_keep_case => "TestKeepCase",
    test_morph => "TestMorph",
    test_morph_alias => "TestMorphAlias",
    test_morph_data => "TestMorphData",
    test_need_affix => "TestNeedAffix",
    test_only_in_compound => "TestOnlyInCompound",
    test_optional_condition => "TestOptionalCondition",
    test_spaces => "TestSpaces",
    test_stemmer => "TestStemmer",
    test_strange_overgeneration => "TestStrangeOvergeneration",
    test_two_fold => "TestTwoFold",
    test_two_suffixes => "TestTwoSuffixes",
    test_zero_affix => "TestZeroAffix",
    test_zero_affix2 => "TestZeroAffix2",
}

#[test]
fn test_base_utf() {
    support::run(base_utf_class);
}

#[test]
fn test_affix_condition() {
    support::run(affix_condition_class);
}

#[test]
fn test_64k_affixes() {
    support::run(k64_class);
}

#[test]
fn test_dictionary() {
    support::run(dictionary_class);
}

#[test]
fn test_hunspell() {
    support::run(hunspell_class);
}

#[test]
fn test_spell_checking() {
    support::run(spell_checking_class);
}

#[test]
fn test_hunspell_stem_filter() {
    support::run(hunspell_stem_filter_class);
}

#[test]
fn test_hunspell_stem_filter_factory() {
    support::run(hunspell_stem_filter_factory_class);
}

#[test]
fn test_trigram_automaton() {
    support::run(trigram_automaton_class);
}

/// Lucene ignores `testMaxPrologueNeeded` and assumes the suite away
/// without `hunspell.dictionaries`.
#[test]
fn test_all_dictionaries() {
    support::run_allowing(all_dictionaries_class, 1, true);
}

/// Lucene assumes the suite away without `hunspell.corpora` (the runner
/// then reports its ten methods ignored).
#[test]
fn test_performance() {
    support::run_allowing(performance_class, 10, true);
}
