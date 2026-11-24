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

//! Port of the `org.apache.lucene.analysis.icu.segmentation` test classes of
//! Lucene 10.5.0 `lucene/analysis/icu/src/test`, each run as a suite by the
//! ported runner: `TestCharArrayIterator`, `TestICUTokenizer`,
//! `TestICUTokenizerCJK`, `TestICUTokenizerFactory`, `TestMyanmarSyllable`
//! and `TestWithCJKBigramFilter`.
//!
//! `TestICUTokenizerCJK` carries Lucene's class-level
//! `@AwaitsFix(bugUrl = "https://issues.apache.org/jira/browse/LUCENE-8222")`:
//! the runner ignores it unless `tests.awaitsfix` is set, as Lucene's does.

mod support;

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Barrier};

use icu4j::lang::u_script::UScript;
use icu4j::text::unicode_set::UnicodeSet;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::factories::TokenizerFactory;
use rucene::analysis::stop_filter::new_stop_filter;
use rucene::analysis::{Analyzer, CharArraySet, StringCharReader, TokenStream};
use rucene::error::Result;
use rucene::util::resource_loader::ResourceLoaderAware;
use rucene_analysis_common::analysis::cjk::cjk_bigram_filter::new_cjk_bigram_filter;
use rucene_analysis_common::jdk::character_iterator::{CharacterIterator, DONE};
use rucene_analysis_icu::analysis::icu::icu_normalizer2_filter::new_icu_normalizer2_filter;
use rucene_analysis_icu::analysis::icu::segmentation::char_array_iterator::CharArrayIterator;
use rucene_analysis_icu::analysis::icu::segmentation::default_icu_tokenizer_config::DefaultICUTokenizerConfig;
use rucene_analysis_icu::analysis::icu::segmentation::icu_tokenizer::{
    new_icu_tokenizer, new_icu_tokenizer_with_factory, ICUTokenizer,
};
use rucene_analysis_icu::analysis::icu::segmentation::icu_tokenizer_factory::ICUTokenizerFactory;
use rucene_analysis_icu::analysis::icu::tokenattributes::script_attribute::ScriptAttribute;
use rucene_test_framework::analysis::{tokenizer_components, BaseTokenStreamTestCase};
use rucene_test_framework::util::lucene_test_case::{AwaitsFix, RANDOM_MULTIPLIER, VERBOSE};
use rucene_test_framework::util::LuceneTestCase;
use support::{
    boxed, btstc_class, hook, iae_message, lucene_class, own, run, run_allowing, t,
    test_class_loader, tokenizer_analyzer, FnAnalyzer,
};

type B = BaseTokenStreamTestCase;

/// The test package of the segmentation classes.
const SEGMENTATION: &str = "org.apache.lucene.analysis.icu.segmentation";

/// `random()`.
fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// `RANDOM_MULTIPLIER`.
fn mult() -> i32 {
    *RANDOM_MULTIPLIER
}

/// A Java `String` as UTF-16 code units.
fn u(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

thread_local! {
    /// The analyzer fields (`a`, or `analyzer` and `analyzer2`) of the suite
    /// that runs.
    static A: RefCell<Vec<Arc<dyn Analyzer>>> = const { RefCell::new(Vec::new()) };
}

fn set_analyzers(analyzers: Vec<Arc<dyn Analyzer>>) {
    A.with(|cell| *cell.borrow_mut() = analyzers);
}

/// The first analyzer field (`a` or `analyzer`).
fn analyzer() -> Arc<dyn Analyzer> {
    A.with(|cell| Arc::clone(cell.borrow().first().expect("setUp")))
}

/// `a.close()` (`IOUtils.close(analyzer, analyzer2)`), then
/// `super.tearDown()`.
fn close_analyzers_tear_down() -> Result<()> {
    for a in A.with(|cell| std::mem::take(&mut *cell.borrow_mut())) {
        a.close();
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}

/// `new Analyzer() { createComponents: return new TokenStreamComponents(new
/// ICUTokenizer(newAttributeFactory(), new DefaultICUTokenizerConfig(cjkAsWords,
/// myanmarAsWords))); }`.
fn icu_tokenizer_analyzer(
    name: &'static str,
    cjk_as_words: bool,
    myanmar_as_words: bool,
) -> Arc<dyn Analyzer> {
    FnAnalyzer::new(name, move |_| {
        let tokenizer = new_icu_tokenizer_with_factory(
            B::new_attribute_factory(),
            Arc::new(DefaultICUTokenizerConfig::new(
                cjk_as_words,
                myanmar_as_words,
            )),
        )?;
        Ok(tokenizer_components(own(tokenizer)))
    })
    .arc()
}

// -----------------------------------------------------------------------------
// TestCharArrayIterator
// -----------------------------------------------------------------------------

/// `ci.setText(text.toCharArray(), 0, text.length())`.
fn set_text(ci: &mut CharArrayIterator, text: &str) {
    let units = u(text);
    let length = units.len() as i32;
    ci.set_text(Arc::from(units), 0, length);
}

/// `assertEquals(char, char)`.
fn eq_char(expected: char, actual: u16) {
    Assert::assert_equals(expected as u16, actual);
}

fn cai_test_basic_usage() -> Result<()> {
    let mut ci = CharArrayIterator::new();
    set_text(&mut ci, "testing");
    Assert::assert_equals(0, ci.get_begin_index());
    Assert::assert_equals(7, ci.get_end_index());
    Assert::assert_equals(0, ci.get_index());
    eq_char('t', ci.current());
    eq_char('e', ci.next());
    eq_char('g', ci.last());
    eq_char('n', ci.previous());
    eq_char('t', ci.first());
    Assert::assert_equals(DONE, ci.previous());
    Ok(())
}

fn cai_test_first() -> Result<()> {
    let mut ci = CharArrayIterator::new();
    set_text(&mut ci, "testing");
    ci.next();
    // Sets the position to getBeginIndex() and returns the character at that position.
    eq_char('t', ci.first());
    Assert::assert_equals(ci.get_begin_index(), ci.get_index());
    // or DONE if the text is empty
    ci.set_text(Arc::from(Vec::<u16>::new()), 0, 0);
    Assert::assert_equals(DONE, ci.first());
    Ok(())
}

fn cai_test_last() -> Result<()> {
    let mut ci = CharArrayIterator::new();
    set_text(&mut ci, "testing");
    // Sets the position to getEndIndex()-1 (getEndIndex() if the text is empty)
    // and returns the character at that position.
    eq_char('g', ci.last());
    Assert::assert_equals(ci.get_index(), ci.get_end_index() - 1);
    // or DONE if the text is empty
    ci.set_text(Arc::from(Vec::<u16>::new()), 0, 0);
    Assert::assert_equals(DONE, ci.last());
    Assert::assert_equals(ci.get_end_index(), ci.get_index());
    Ok(())
}

fn cai_test_current() -> Result<()> {
    let mut ci = CharArrayIterator::new();
    // Gets the character at the current position (as returned by getIndex()).
    set_text(&mut ci, "testing");
    eq_char('t', ci.current());
    ci.last();
    ci.next();
    // or DONE if the current position is off the end of the text.
    Assert::assert_equals(DONE, ci.current());
    Ok(())
}

fn cai_test_next() -> Result<()> {
    let mut ci = CharArrayIterator::new();
    ci.set_text(Arc::from(u("te")), 0, 2);
    // Increments the iterator's index by one and returns the character at the new index.
    eq_char('e', ci.next());
    Assert::assert_equals(1, ci.get_index());
    // or DONE if the new position is off the end of the text range.
    Assert::assert_equals(DONE, ci.next());
    Assert::assert_equals(ci.get_end_index(), ci.get_index());
    Ok(())
}

fn cai_test_set_index() -> Result<()> {
    let mut ci = CharArrayIterator::new();
    set_text(&mut ci, "test");
    rucene_test_framework::util::java_exceptions::expect_throws(
        "java.lang.IllegalArgumentException",
        || ci.set_index(5),
    );
    Ok(())
}

fn cai_test_clone() -> Result<()> {
    let text = u("testing");
    let length = text.len() as i32;
    let mut ci = CharArrayIterator::new();
    ci.set_text(Arc::from(text), 0, length);
    ci.next();
    let mut ci2 = ci.clone();
    Assert::assert_equals(ci.get_index(), ci2.get_index());
    Assert::assert_equals(ci.next(), ci2.next());
    Assert::assert_equals(ci.last(), ci2.last());
    Ok(())
}

fn char_array_iterator_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.icu.segmentation.TestCharArrayIterator")
        .method(t("testBasicUsage", cai_test_basic_usage))
        .method(t("testFirst", cai_test_first))
        .method(t("testLast", cai_test_last))
        .method(t("testCurrent", cai_test_current))
        .method(t("testNext", cai_test_next))
        .method(t("testSetIndex", cai_test_set_index))
        .method(t("testClone", cai_test_clone))
        .build()
}

// -----------------------------------------------------------------------------
// TestICUTokenizer
// -----------------------------------------------------------------------------

/// `new ICUTokenizer(newAttributeFactory(), new DefaultICUTokenizerConfig(false, true))`
/// over `input`.
fn tokenizer_over(input: &str) -> Result<ICUTokenizer> {
    let mut tokenizer = new_icu_tokenizer_with_factory(
        B::new_attribute_factory(),
        Arc::new(DefaultICUTokenizerConfig::new(false, true)),
    )?;
    tokenizer.set_reader(Box::new(StringCharReader::new(input)))?;
    Ok(tokenizer)
}

fn it_test_huge_doc() -> Result<()> {
    let mut sb = " ".repeat(4094);
    sb.push_str("testing 1234");
    let input = sb;
    let mut tokenizer = tokenizer_over(&input)?;
    B::assert_token_stream_contents(&mut tokenizer, &["testing", "1234"])
}

fn it_test_huge_term2() -> Result<()> {
    let input = "a".repeat(40960);
    let mut tokenizer = tokenizer_over(&input)?;
    let expected_token = "a".repeat(4096);
    let expected: Vec<&str> = vec![expected_token.as_str(); 10];
    B::assert_token_stream_contents(&mut tokenizer, &expected)
}

fn it_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    set_analyzers(vec![icu_tokenizer_analyzer(
        "org.apache.lucene.analysis.icu.segmentation.TestICUTokenizer$1",
        false,
        true,
    )]);
    Ok(())
}

/* test various jira issues this analyzer is related to */

fn it_test_lucene1545() -> Result<()> {
    let a = analyzer();
    /*
     * Standard analyzer does not correctly tokenize combining character U+0364 COMBINING LATIN SMALL LETTRE E.
     * The word "moͤchte" is incorrectly tokenized into "mo" "chte", the combining character is lost.
     * Expected result is only on token "moͤchte".
     */
    B::assert_analyzes_to(&*a, "moͤchte", &["moͤchte"])
}

fn it_test_emoji_from_the_future() -> Result<()> {
    let a = analyzer();
    // pick an unassigned character with extended_pictographic
    let ch = UnicodeSet::from_pattern(&u("[[:Extended_Pictographic:]&[:Unassigned:]]"))?
        .get_range_start(0);
    let value = char::from_u32(ch as u32).expect("a code point").to_string();
    // should analyze to emoji type
    B::assert_analyzes_to_types_only(&*a, &value, &[value.as_str()], &["<EMOJI>"])?;
    // shouldn't break in a sequence
    let sequence = format!("{value}\u{200D}{value}");
    B::assert_analyzes_to_types_only(&*a, &sequence, &[sequence.as_str()], &["<EMOJI>"])
}

/// blast some random strings through the analyzer
fn it_test_random_strings() -> Result<()> {
    B::check_random_data(&*random(), &analyzer(), 200 * mult())
}

/// blast some random large strings through the analyzer
fn it_test_random_huge_strings() -> Result<()> {
    let random = random();
    B::check_random_data_max_word_length(&*random, &analyzer(), 10 * mult(), 8192)
}

fn it_test_token_attributes() -> Result<()> {
    let a = analyzer();
    let ts = a.token_stream("dummy", Box::new(StringCharReader::new("This is a test")))?;
    let mut ts = ts.borrow_mut();
    ts.attribute_source_mut()
        .add_attribute_interface::<dyn ScriptAttribute>()?;
    ts.reset()?;
    while ts.increment_token()? {
        let (code, name, short_name) = {
            let script_att = ts
                .attribute_source()
                .get_attribute_interface::<dyn ScriptAttribute>()
                .expect("addAttribute(ScriptAttribute.class)");
            (
                script_att.get_code(),
                script_att.get_name(),
                script_att.get_short_name(),
            )
        };
        Assert::assert_equals(UScript::LATIN, code);
        Assert::assert_equals_str(
            None,
            UScript::get_name(UScript::LATIN)?.as_deref(),
            name.as_deref(),
        );
        Assert::assert_equals_str(
            None,
            UScript::get_short_name(UScript::LATIN)?.as_deref(),
            short_name.as_deref(),
        );
        Assert::assert_true(
            ts.attribute_source()
                .reflect_as_string(false)
                .contains("script=Latin"),
        );
    }
    ts.end()?;
    ts.close()
}

/// test for bugs like http://bugs.icu-project.org/trac/ticket/10767
fn it_test_icu_concurrency() -> Result<()> {
    let num_threads = 8;
    let starting_gun = Barrier::new(num_threads + 1);
    std::thread::scope(|scope| {
        let mut threads = Vec::with_capacity(num_threads);
        for _ in 0..num_threads {
            threads.push(scope.spawn(|| -> Result<()> {
                starting_gun.wait();
                let mut token_count: i64 = 0;
                let contents = "英 เบียร์ ビール ເບຍ abc";
                for _ in 0..1000 {
                    let mut tokenizer = new_icu_tokenizer()?;
                    tokenizer.set_reader(Box::new(StringCharReader::new(contents)))?;
                    tokenizer.reset()?;
                    while tokenizer.increment_token()? {
                        token_count += 1;
                    }
                    tokenizer.end()?;
                    tokenizer.close()?;
                }
                if *VERBOSE {
                    println!("{token_count}");
                }
                Ok(())
            }));
        }
        starting_gun.wait();
        for thread in threads {
            thread
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))?;
        }
        Ok(())
    })
}

fn it_test_armenian() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "Վիքիպեդիայի 13 միլիոն հոդվածները (4,600` հայերեն վիքիպեդիայում) գրվել են կամավորների կողմից ու համարյա բոլոր հոդվածները կարող է խմբագրել ցանկաց մարդ ով կարող է բացել Վիքիպեդիայի կայքը։", &["Վիքիպեդիայի", "13", "միլիոն", "հոդվածները", "4,600", "հայերեն", "վիքիպեդիայում", "գրվել", "են", "կամավորների", "կողմից", "ու", "համարյա", "բոլոր", "հոդվածները", "կարող", "է", "խմբագրել", "ցանկաց", "մարդ", "ով", "կարող", "է", "բացել", "Վիքիպեդիայի", "կայքը"])?;
    Ok(())
}

fn it_test_amharic() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(
        &*a,
        "ዊኪፔድያ የባለ ብዙ ቋንቋ የተሟላ ትክክለኛና ነጻ መዝገበ ዕውቀት (ኢንሳይክሎፒዲያ) ነው። ማንኛውም",
        &[
            "ዊኪፔድያ",
            "የባለ",
            "ብዙ",
            "ቋንቋ",
            "የተሟላ",
            "ትክክለኛና",
            "ነጻ",
            "መዝገበ",
            "ዕውቀት",
            "ኢንሳይክሎፒዲያ",
            "ነው",
            "ማንኛውም",
        ],
    )?;
    Ok(())
}

fn it_test_arabic() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "الفيلم الوثائقي الأول عن ويكيبيديا يسمى \"الحقيقة بالأرقام: قصة ويكيبيديا\" (بالإنجليزية: Truth in Numbers: The Wikipedia Story)، سيتم إطلاقه في 2008.", &["الفيلم", "الوثائقي", "الأول", "عن", "ويكيبيديا", "يسمى", "الحقيقة", "بالأرقام", "قصة", "ويكيبيديا", "بالإنجليزية", "Truth", "in", "Numbers", "The", "Wikipedia", "Story", "سيتم", "إطلاقه", "في", "2008"])?;
    Ok(())
}

fn it_test_aramaic() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ܘܝܩܝܦܕܝܐ (ܐܢܓܠܝܐ: Wikipedia) ܗܘ ܐܝܢܣܩܠܘܦܕܝܐ ܚܐܪܬܐ ܕܐܢܛܪܢܛ ܒܠܫܢ̈ܐ ܣܓܝܐ̈ܐ܂ ܫܡܗ ܐܬܐ ܡܢ ܡ̈ܠܬܐ ܕ\"ܘܝܩܝ\" ܘ\"ܐܝܢܣܩܠܘܦܕܝܐ\"܀", &["ܘܝܩܝܦܕܝܐ", "ܐܢܓܠܝܐ", "Wikipedia", "ܗܘ", "ܐܝܢܣܩܠܘܦܕܝܐ", "ܚܐܪܬܐ", "ܕܐܢܛܪܢܛ", "ܒܠܫܢ̈ܐ", "ܣܓܝܐ̈ܐ", "ܫܡܗ", "ܐܬܐ", "ܡܢ", "ܡ̈ܠܬܐ", "ܕ", "ܘܝܩܝ", "ܘ", "ܐܝܢܣܩܠܘܦܕܝܐ"])?;
    Ok(())
}

fn it_test_bengali() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "এই বিশ্বকোষ পরিচালনা করে উইকিমিডিয়া ফাউন্ডেশন (একটি অলাভজনক সংস্থা)। উইকিপিডিয়ার শুরু ১৫ জানুয়ারি, ২০০১ সালে। এখন পর্যন্ত ২০০টিরও বেশী ভাষায় উইকিপিডিয়া রয়েছে।", &["এই", "বিশ্বকোষ", "পরিচালনা", "করে", "উইকিমিডিয়া", "ফাউন্ডেশন", "একটি", "অলাভজনক", "সংস্থা", "উইকিপিডিয়ার", "শুরু", "১৫", "জানুয়ারি", "২০০১", "সালে", "এখন", "পর্যন্ত", "২০০টিরও", "বেশী", "ভাষায়", "উইকিপিডিয়া", "রয়েছে"])?;
    Ok(())
}

fn it_test_farsi() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(
        &*a,
        "ویکی پدیای انگلیسی در تاریخ ۲۵ دی ۱۳۷۹ به صورت مکملی برای دانشنامهٔ تخصصی نوپدیا نوشته شد.",
        &[
            "ویکی",
            "پدیای",
            "انگلیسی",
            "در",
            "تاریخ",
            "۲۵",
            "دی",
            "۱۳۷۹",
            "به",
            "صورت",
            "مکملی",
            "برای",
            "دانشنامهٔ",
            "تخصصی",
            "نوپدیا",
            "نوشته",
            "شد",
        ],
    )?;
    Ok(())
}

fn it_test_greek() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "Γράφεται σε συνεργασία από εθελοντές με το λογισμικό wiki, κάτι που σημαίνει ότι άρθρα μπορεί να προστεθούν ή να αλλάξουν από τον καθένα.", &["Γράφεται", "σε", "συνεργασία", "από", "εθελοντές", "με", "το", "λογισμικό", "wiki", "κάτι", "που", "σημαίνει", "ότι", "άρθρα", "μπορεί", "να", "προστεθούν", "ή", "να", "αλλάξουν", "από", "τον", "καθένα"])?;
    Ok(())
}

fn it_test_khmer() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(
        &*a,
        "ផ្ទះស្កឹមស្កៃបីបួនខ្នងនេះ",
        &["ផ្ទះ", "ស្កឹមស្កៃ", "បី", "បួន", "ខ្នង", "នេះ"],
    )?;
    Ok(())
}

fn it_test_lao() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ກວ່າດອກ", &["ກວ່າ", "ດອກ"])?;
    B::assert_analyzes_to_types_only(
        &*a,
        "ພາສາລາວ",
        &["ພາສາ", "ລາວ"],
        &["<ALPHANUM>", "<ALPHANUM>"],
    )?;
    Ok(())
}

fn it_test_myanmar() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "သက်ဝင်လှုပ်ရှားစေပြီး", &["သက်ဝင်", "လှုပ်ရှား", "စေ", "ပြီး"])?;
    Ok(())
}

fn it_test_thai() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(
        &*a,
        "การที่ได้ต้องแสดงว่างานดี. แล้วเธอจะไปไหน? ๑๒๓๔",
        &[
            "การ",
            "ที่",
            "ได้",
            "ต้อง",
            "แสดง",
            "ว่า",
            "งาน",
            "ดี",
            "แล้ว",
            "เธอ",
            "จะ",
            "ไป",
            "ไหน",
            "๑๒๓๔",
        ],
    )?;
    Ok(())
}

fn it_test_tibetan() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(
        &*a,
        "སྣོན་མཛོད་དང་ལས་འདིས་བོད་ཡིག་མི་ཉམས་གོང་འཕེལ་དུ་གཏོང་བར་ཧ་ཅང་དགེ་མཚན་མཆིས་སོ། །",
        &[
            "སྣོན",
            "མཛོད",
            "དང",
            "ལས",
            "འདིས",
            "བོད",
            "ཡིག",
            "མི",
            "ཉམས",
            "གོང",
            "འཕེལ",
            "དུ",
            "གཏོང",
            "བར",
            "ཧ",
            "ཅང",
            "དགེ",
            "མཚན",
            "མཆིས",
            "སོ",
        ],
    )?;
    Ok(())
}

fn it_test_chinese() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(
        &*a,
        "我是中国人。 １２３４ Ｔｅｓｔｓ ",
        &["我", "是", "中", "国", "人", "１２３４", "Ｔｅｓｔｓ"],
    )?;
    Ok(())
}

fn it_test_hebrew() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(
        &*a,
        "דנקנר תקף את הדו\"ח",
        &["דנקנר", "תקף", "את", "הדו\"ח"],
    )?;
    B::assert_analyzes_to(&*a, "חברת בת של מודי'ס", &["חברת", "בת", "של", "מודי'ס"])?;
    Ok(())
}

fn it_test_empty() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "", &[])?;
    B::assert_analyzes_to(&*a, ".", &[])?;
    B::assert_analyzes_to(&*a, " ", &[])?;
    Ok(())
}

fn it_test_alphanumeric_sa() -> Result<()> {
    let a = analyzer();
    // alphanumeric tokens
    B::assert_analyzes_to(&*a, "B2B", &["B2B"])?;
    B::assert_analyzes_to(&*a, "2B", &["2B"])?;
    Ok(())
}

fn it_test_delimiters_sa() -> Result<()> {
    let a = analyzer();
    // other delimiters: "-", "/", ","
    B::assert_analyzes_to(&*a, "some-dashed-phrase", &["some", "dashed", "phrase"])?;
    B::assert_analyzes_to(&*a, "dogs,chase,cats", &["dogs", "chase", "cats"])?;
    B::assert_analyzes_to(&*a, "ac/dc", &["ac", "dc"])?;
    Ok(())
}

fn it_test_apostrophes_sa() -> Result<()> {
    let a = analyzer();
    // internal apostrophes: O'Reilly, you're, O'Reilly's
    B::assert_analyzes_to(&*a, "O'Reilly", &["O'Reilly"])?;
    B::assert_analyzes_to(&*a, "you're", &["you're"])?;
    B::assert_analyzes_to(&*a, "she's", &["she's"])?;
    B::assert_analyzes_to(&*a, "Jim's", &["Jim's"])?;
    B::assert_analyzes_to(&*a, "don't", &["don't"])?;
    B::assert_analyzes_to(&*a, "O'Reilly's", &["O'Reilly's"])?;
    Ok(())
}

fn it_test_numeric_sa() -> Result<()> {
    let a = analyzer();
    // floating point, serial, model numbers, ip addresses, etc.
    // every other segment must have at least one digit
    B::assert_analyzes_to(&*a, "21.35", &["21.35"])?;
    B::assert_analyzes_to(&*a, "R2D2 C3PO", &["R2D2", "C3PO"])?;
    B::assert_analyzes_to(&*a, "216.239.63.104", &["216.239.63.104"])?;
    B::assert_analyzes_to(&*a, "216.239.63.104", &["216.239.63.104"])?;
    Ok(())
}

fn it_test_text_with_numbers_sa() -> Result<()> {
    let a = analyzer();
    // numbers
    B::assert_analyzes_to(
        &*a,
        "David has 5000 bones",
        &["David", "has", "5000", "bones"],
    )?;
    Ok(())
}

fn it_test_various_text_sa() -> Result<()> {
    let a = analyzer();
    // various
    B::assert_analyzes_to(
        &*a,
        "C embedded developers wanted",
        &["C", "embedded", "developers", "wanted"],
    )?;
    B::assert_analyzes_to(&*a, "foo bar FOO BAR", &["foo", "bar", "FOO", "BAR"])?;
    B::assert_analyzes_to(
        &*a,
        "foo      bar .  FOO <> BAR",
        &["foo", "bar", "FOO", "BAR"],
    )?;
    B::assert_analyzes_to(&*a, "\"QUOTED\" word", &["QUOTED", "word"])?;
    Ok(())
}

fn it_test_korean_sa() -> Result<()> {
    let a = analyzer();
    // Korean words
    B::assert_analyzes_to(&*a, "안녕하세요 한글입니다", &["안녕하세요", "한글입니다"])?;
    Ok(())
}

fn it_test_reusable_token_stream() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(
        &*a,
        "སྣོན་མཛོད་དང་ལས་འདིས་བོད་ཡིག་མི་ཉམས་གོང་འཕེལ་དུ་གཏོང་བར་ཧ་ཅང་དགེ་མཚན་མཆིས་སོ། །",
        &[
            "སྣོན",
            "མཛོད",
            "དང",
            "ལས",
            "འདིས",
            "བོད",
            "ཡིག",
            "མི",
            "ཉམས",
            "གོང",
            "འཕེལ",
            "དུ",
            "གཏོང",
            "བར",
            "ཧ",
            "ཅང",
            "དགེ",
            "མཚན",
            "མཆིས",
            "སོ",
        ],
    )?;
    Ok(())
}

fn it_test_offsets() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to_offsets(
        &*a,
        "David has 5000 bones",
        &["David", "has", "5000", "bones"],
        &[0, 6, 10, 15],
        &[5, 9, 14, 20],
    )?;
    Ok(())
}

fn it_test_types() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to_types_only(
        &*a,
        "David has 5000 bones",
        &["David", "has", "5000", "bones"],
        &["<ALPHANUM>", "<ALPHANUM>", "<NUM>", "<ALPHANUM>"],
    )?;
    Ok(())
}

fn it_test_korean() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to_types_only(&*a, "훈민정음", &["훈민정음"], &["<HANGUL>"])?;
    Ok(())
}

fn it_test_japanese() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to_types_only(
        &*a,
        "仮名遣い カタカナ",
        &["仮", "名", "遣", "い", "カタカナ"],
        &[
            "<IDEOGRAPHIC>",
            "<IDEOGRAPHIC>",
            "<IDEOGRAPHIC>",
            "<HIRAGANA>",
            "<KATAKANA>",
        ],
    )?;
    Ok(())
}

fn it_test_emoji() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to_types_only(
        &*a,
        "💩 💩💩",
        &["💩", "💩", "💩"],
        &["<EMOJI>", "<EMOJI>", "<EMOJI>"],
    )?;
    Ok(())
}

fn it_test_emoji_sequence() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to_types_only(&*a, "👩‍❤️‍👩", &["👩‍❤️‍👩"], &["<EMOJI>"])?;
    Ok(())
}

fn it_test_emoji_sequence_with_modifier() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to_types_only(&*a, "👨🏼‍⚕️", &["👨🏼‍⚕️"], &["<EMOJI>"])?;
    Ok(())
}

fn it_test_emoji_regional_indicator() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to_types_only(&*a, "🇺🇸🇺🇸", &["🇺🇸", "🇺🇸"], &["<EMOJI>", "<EMOJI>"])?;
    Ok(())
}

fn it_test_emoji_variation_sequence() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to_types_only(&*a, "#️⃣", &["#️⃣"], &["<EMOJI>"])?;
    B::assert_analyzes_to_types_only(&*a, "3️⃣", &["3️⃣"], &["<EMOJI>"])?;
    Ok(())
}

fn it_test_emoji_tag_sequence() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to_types_only(&*a, "🏴󠁧󠁢󠁥󠁮󠁧󠁿", &["🏴󠁧󠁢󠁥󠁮󠁧󠁿"], &["<EMOJI>"])?;
    Ok(())
}

fn it_test_emoji_tokenization() -> Result<()> {
    let a = analyzer();
    // simple emoji around latin
    B::assert_analyzes_to_types_only(
        &*a,
        "poo💩poo",
        &["poo", "💩", "poo"],
        &["<ALPHANUM>", "<EMOJI>", "<ALPHANUM>"],
    )?;
    // simple emoji around non-latin
    B::assert_analyzes_to_types_only(
        &*a,
        "💩中國💩",
        &["💩", "中", "國", "💩"],
        &["<EMOJI>", "<IDEOGRAPHIC>", "<IDEOGRAPHIC>", "<EMOJI>"],
    )?;
    Ok(())
}

fn it_test_script_extensions() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "𑅗०", &["𑅗०"])?;
    Ok(())
}

fn it_test_script_spacing_mark() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "𑅗ा", &["𑅗ा"])?;
    Ok(())
}

fn it_test_script_enclosing_mark() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "𑅗᪾", &["𑅗᪾"])?;
    Ok(())
}

fn icu_tokenizer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.icu.segmentation.TestICUTokenizer")
        .method(hook("setUp", it_set_up))
        .method(hook("tearDown", close_analyzers_tear_down))
        .method(t("testHugeDoc", it_test_huge_doc))
        .method(t("testHugeTerm2", it_test_huge_term2))
        .method(t("testArmenian", it_test_armenian))
        .method(t("testAmharic", it_test_amharic))
        .method(t("testArabic", it_test_arabic))
        .method(t("testAramaic", it_test_aramaic))
        .method(t("testBengali", it_test_bengali))
        .method(t("testFarsi", it_test_farsi))
        .method(t("testGreek", it_test_greek))
        .method(t("testKhmer", it_test_khmer))
        .method(t("testLao", it_test_lao))
        .method(t("testMyanmar", it_test_myanmar))
        .method(t("testThai", it_test_thai))
        .method(t("testTibetan", it_test_tibetan))
        .method(t("testChinese", it_test_chinese))
        .method(t("testHebrew", it_test_hebrew))
        .method(t("testEmpty", it_test_empty))
        .method(t("testLUCENE1545", it_test_lucene1545))
        .method(t("testAlphanumericSA", it_test_alphanumeric_sa))
        .method(t("testDelimitersSA", it_test_delimiters_sa))
        .method(t("testApostrophesSA", it_test_apostrophes_sa))
        .method(t("testNumericSA", it_test_numeric_sa))
        .method(t("testTextWithNumbersSA", it_test_text_with_numbers_sa))
        .method(t("testVariousTextSA", it_test_various_text_sa))
        .method(t("testKoreanSA", it_test_korean_sa))
        .method(t("testReusableTokenStream", it_test_reusable_token_stream))
        .method(t("testOffsets", it_test_offsets))
        .method(t("testTypes", it_test_types))
        .method(t("testKorean", it_test_korean))
        .method(t("testJapanese", it_test_japanese))
        .method(t("testEmoji", it_test_emoji))
        .method(t("testEmojiSequence", it_test_emoji_sequence))
        .method(t(
            "testEmojiSequenceWithModifier",
            it_test_emoji_sequence_with_modifier,
        ))
        .method(t(
            "testEmojiRegionalIndicator",
            it_test_emoji_regional_indicator,
        ))
        .method(t(
            "testEmojiVariationSequence",
            it_test_emoji_variation_sequence,
        ))
        .method(t("testEmojiTagSequence", it_test_emoji_tag_sequence))
        .method(t("testEmojiTokenization", it_test_emoji_tokenization))
        .method(t("testEmojiFromTheFuture", it_test_emoji_from_the_future))
        .method(t("testRandomStrings", it_test_random_strings))
        .method(t("testRandomHugeStrings", it_test_random_huge_strings))
        .method(t("testTokenAttributes", it_test_token_attributes))
        .method(t("testICUConcurrency", it_test_icu_concurrency))
        .method(t("testScriptExtensions", it_test_script_extensions))
        .method(t("testScriptSpacingMark", it_test_script_spacing_mark))
        .method(t("testScriptEnclosingMark", it_test_script_enclosing_mark))
        .build()
}

// -----------------------------------------------------------------------------
// TestICUTokenizerCJK
// -----------------------------------------------------------------------------

fn cjk_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    set_analyzers(vec![icu_tokenizer_analyzer(
        "org.apache.lucene.analysis.icu.segmentation.TestICUTokenizerCJK$1",
        true,
        true,
    )]);
    Ok(())
}

/// blast some random strings through the analyzer
fn cjk_test_random_strings() -> Result<()> {
    B::check_random_data(&*random(), &analyzer(), 10000 * mult())
}

/// blast some random large strings through the analyzer
fn cjk_test_random_huge_strings() -> Result<()> {
    let random = random();
    B::check_random_data_max_word_length(&*random, &analyzer(), 10 * mult(), 8192)
}

fn cjk_test_simple_chinese() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(
        &*a,
        "我购买了道具和服装。",
        &["我", "购买", "了", "道具", "和", "服装"],
    )?;
    Ok(())
}

fn cjk_test_traditional_chinese() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(
        &*a,
        "我購買了道具和服裝。",
        &["我", "購買", "了", "道具", "和", "服裝"],
    )?;
    B::assert_analyzes_to(
        &*a,
        "定義切分字串的基本單位是訂定分詞標準的首要工作",
        &[
            "定義", "切", "分", "字串", "的", "基本", "單位", "是", "訂定", "分詞", "標準", "的",
            "首要", "工作",
        ],
    )?;
    Ok(())
}

fn cjk_test_chinese_numerics() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "９４８３", &["９４８３"])?;
    B::assert_analyzes_to(&*a, "院內分機９４８３。", &["院", "內", "分機", "９４８３"])?;
    B::assert_analyzes_to(&*a, "院內分機9483。", &["院", "內", "分機", "9483"])?;
    Ok(())
}

fn cjk_test_simple_japanese() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(
        &*a,
        "それはまだ実験段階にあります",
        &["それ", "は", "まだ", "実験", "段階", "に", "あり", "ます"],
    )?;
    Ok(())
}

fn cjk_test_simple_japanese_with_emoji() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(
        &*a,
        "それはまだ実験段階にあります💩",
        &[
            "それ", "は", "まだ", "実験", "段階", "に", "あり", "ます", "💩",
        ],
    )?;
    Ok(())
}

fn cjk_test_japanese_types() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to_types_only(
        &*a,
        "仮名遣い カタカナ",
        &["仮名遣い", "カタカナ"],
        &["<IDEOGRAPHIC>", "<IDEOGRAPHIC>"],
    )?;
    Ok(())
}

fn cjk_test_korean() -> Result<()> {
    let a = analyzer();
    // Korean words
    B::assert_analyzes_to(&*a, "안녕하세요 한글입니다", &["안녕하세요", "한글입니다"])?;
    Ok(())
}

fn cjk_test_korean_types() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to_types_only(&*a, "훈민정음", &["훈민정음"], &["<HANGUL>"])?;
    Ok(())
}

fn icu_tokenizer_cjk_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.icu.segmentation.TestICUTokenizerCJK")
        .annotate(AwaitsFix {
            bug_url: "https://issues.apache.org/jira/browse/LUCENE-8222".to_string(),
        })
        .method(hook("setUp", cjk_set_up))
        .method(hook("tearDown", close_analyzers_tear_down))
        .method(t("testSimpleChinese", cjk_test_simple_chinese))
        .method(t("testTraditionalChinese", cjk_test_traditional_chinese))
        .method(t("testChineseNumerics", cjk_test_chinese_numerics))
        .method(t("testSimpleJapanese", cjk_test_simple_japanese))
        .method(t(
            "testSimpleJapaneseWithEmoji",
            cjk_test_simple_japanese_with_emoji,
        ))
        .method(t("testJapaneseTypes", cjk_test_japanese_types))
        .method(t("testKorean", cjk_test_korean))
        .method(t("testKoreanTypes", cjk_test_korean_types))
        .method(t("testRandomStrings", cjk_test_random_strings))
        .method(t("testRandomHugeStrings", cjk_test_random_huge_strings))
        .build()
}

// -----------------------------------------------------------------------------
// TestICUTokenizerFactory
// -----------------------------------------------------------------------------

/// `new HashMap<String, String>()` filled with `pairs`.
fn args(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

/// `factory.inform(new ClasspathResourceLoader(getClass()));
/// Tokenizer stream = factory.create(newAttributeFactory()); stream.setReader(reader);`
fn informed_stream(mut factory: ICUTokenizerFactory, text: &str) -> Result<Box<dyn TokenStream>> {
    factory.inform(&test_class_loader(SEGMENTATION))?;
    let mut stream = factory.create_with_factory(B::new_attribute_factory())?;
    stream.set_reader(Box::new(StringCharReader::new(text)))?;
    Ok(boxed(stream))
}

/// `ICUTokenizerFactory.RULEFILES` (package-private in Java, read by the test
/// of the same package).
const RULEFILES: &str = "rulefiles";

fn itf_test_mixed_text() -> Result<()> {
    let factory = ICUTokenizerFactory::new(&mut HashMap::new())?;
    let mut stream = informed_stream(factory, "การที่ได้ต้องแสดงว่างานดี  This is a test ກວ່າດອກ")?;
    B::assert_token_stream_contents(
        &mut *stream,
        &[
            "การ",
            "ที่",
            "ได้",
            "ต้อง",
            "แสดง",
            "ว่า",
            "งาน",
            "ดี",
            "This",
            "is",
            "a",
            "test",
            "ກວ່າ",
            "ດອກ",
        ],
    )
}

fn itf_test_tokenize_latin_on_whitespace_only() -> Result<()> {
    // “ U+201C LEFT DOUBLE QUOTATION MARK; ” U+201D RIGHT DOUBLE QUOTATION MARK
    let factory = ICUTokenizerFactory::new(&mut args(&[(
        RULEFILES,
        "Latn:Latin-break-only-on-whitespace.rbbi",
    )]))?;
    let mut stream = informed_stream(
        factory,
        "  Don't,break.at?/(punct)!  \u{201C}nice\u{201D}\r\n\r\n85_At:all; `really\" +2=3$5,&813 !@#%$^)(*@#$   ",
    )?;
    B::assert_token_stream_contents_types_only(
        &mut *stream,
        &[
            "Don't,break.at?/(punct)!",
            "\u{201C}nice\u{201D}",
            "85_At:all;",
            "`really\"",
            "+2=3$5,&813",
            "!@#%$^)(*@#$",
        ],
        &[
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<NUM>",
            "<OTHER>",
        ],
    )
}

fn itf_test_tokenize_latin_dont_break_on_hyphens() -> Result<()> {
    let factory = ICUTokenizerFactory::new(&mut args(&[(
        RULEFILES,
        "Latn:Latin-dont-break-on-hyphens.rbbi",
    )]))?;
    let mut stream = informed_stream(
        factory,
        "One-two punch.  Brang-, not brung-it.  This one--not that one--is the right one, -ish.",
    )?;
    B::assert_token_stream_contents(
        &mut *stream,
        &[
            "One-two", "punch", "Brang", "not", "brung-it", "This", "one", "not", "that", "one",
            "is", "the", "right", "one", "ish",
        ],
    )
}

/// Specify more than one script/rule file pair. Override default DefaultICUTokenizerConfig Thai
/// script tokenization. Use the same rule file for both scripts.
fn itf_test_keyword_tokenize_cyrillic_and_thai() -> Result<()> {
    let factory = ICUTokenizerFactory::new(&mut args(&[(
        RULEFILES,
        "Cyrl:KeywordTokenizer.rbbi,Thai:KeywordTokenizer.rbbi",
    )]))?;
    let mut stream = informed_stream(
        factory,
        "Some English.  Немного русский.  ข้อความภาษาไทยเล็ก ๆ น้อย ๆ  More English.",
    )?;
    B::assert_token_stream_contents(
        &mut *stream,
        &[
            "Some",
            "English",
            "Немного русский.  ",
            "ข้อความภาษาไทยเล็ก ๆ น้อย ๆ  ",
            "More",
            "English",
        ],
    )
}

/// Test that bogus arguments result in exception
fn itf_test_bogus_arguments() -> Result<()> {
    let expected =
        iae_message(|| ICUTokenizerFactory::new(&mut args(&[("bogusArg", "bogusValue")])));
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

fn icu_tokenizer_factory_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.icu.segmentation.TestICUTokenizerFactory")
        .method(t("testMixedText", itf_test_mixed_text))
        .method(t(
            "testTokenizeLatinOnWhitespaceOnly",
            itf_test_tokenize_latin_on_whitespace_only,
        ))
        .method(t(
            "testTokenizeLatinDontBreakOnHyphens",
            itf_test_tokenize_latin_dont_break_on_hyphens,
        ))
        .method(t(
            "testKeywordTokenizeCyrillicAndThai",
            itf_test_keyword_tokenize_cyrillic_and_thai,
        ))
        .method(t("testBogusArguments", itf_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestMyanmarSyllable
// -----------------------------------------------------------------------------

fn ms_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    set_analyzers(vec![icu_tokenizer_analyzer(
        "org.apache.lucene.analysis.icu.segmentation.TestMyanmarSyllable$1",
        false,
        false,
    )]);
    Ok(())
}

fn ms_test_basics() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(
        &*a,
        "သက်ဝင်လှုပ်ရှားစေပြီး",
        &["သက်", "ဝင်", "လှုပ်", "ရှား", "စေ", "ပြီး"],
    )?;
    Ok(())
}

fn ms_test_c() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ကက", &["က", "က"])?;
    Ok(())
}

fn ms_test_cf() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ကံကံ", &["ကံ", "ကံ"])?;
    Ok(())
}

fn ms_test_cca() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ကင်ကင်", &["ကင်", "ကင်"])?;
    Ok(())
}

fn ms_test_ccaf() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ကင်းကင်း", &["ကင်း", "ကင်း"])?;
    Ok(())
}

fn ms_test_cv() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ကာကာ", &["ကာ", "ကာ"])?;
    Ok(())
}

fn ms_test_cvf() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ကားကား", &["ကား", "ကား"])?;
    Ok(())
}

fn ms_test_cvva() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ကော်ကော်", &["ကော်", "ကော်"])?;
    Ok(())
}

fn ms_test_cvvca() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ကောင်ကောင်", &["ကောင်", "ကောင်"])?;
    Ok(())
}

fn ms_test_cvvcaf() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ကောင်းကောင်း", &["ကောင်း", "ကောင်း"])?;
    Ok(())
}

fn ms_test_cm() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ကျကျ", &["ကျ", "ကျ"])?;
    Ok(())
}

fn ms_test_cmf() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ကျံကျံ", &["ကျံ", "ကျံ"])?;
    Ok(())
}

fn ms_test_cmca() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ကျင်ကျင်", &["ကျင်", "ကျင်"])?;
    Ok(())
}

fn ms_test_cmcaf() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ကျင်းကျင်း", &["ကျင်း", "ကျင်း"])?;
    Ok(())
}

fn ms_test_cmv() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ကျာကျာ", &["ကျာ", "ကျာ"])?;
    Ok(())
}

fn ms_test_cmvf() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ကျားကျား", &["ကျား", "ကျား"])?;
    Ok(())
}

fn ms_test_cmvva() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ကျော်ကျော်", &["ကျော်", "ကျော်"])?;
    Ok(())
}

fn ms_test_cmvvca() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ကြောင်ကြောင်", &["ကြောင်", "ကြောင်"])?;
    Ok(())
}

fn ms_test_cmvvcaf() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ကြောင်းကြောင်း", &["ကြောင်း", "ကြောင်း"])?;
    Ok(())
}

fn ms_test_i() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ဪဪ", &["ဪ", "ဪ"])?;
    Ok(())
}

fn ms_test_e() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ဣဣ", &["ဣ", "ဣ"])?;
    Ok(())
}

fn myanmar_syllable_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.icu.segmentation.TestMyanmarSyllable")
        .method(hook("setUp", ms_set_up))
        .method(hook("tearDown", close_analyzers_tear_down))
        .method(t("testBasics", ms_test_basics))
        .method(t("testC", ms_test_c))
        .method(t("testCF", ms_test_cf))
        .method(t("testCCA", ms_test_cca))
        .method(t("testCCAF", ms_test_ccaf))
        .method(t("testCV", ms_test_cv))
        .method(t("testCVF", ms_test_cvf))
        .method(t("testCVVA", ms_test_cvva))
        .method(t("testCVVCA", ms_test_cvvca))
        .method(t("testCVVCAF", ms_test_cvvcaf))
        .method(t("testCM", ms_test_cm))
        .method(t("testCMF", ms_test_cmf))
        .method(t("testCMCA", ms_test_cmca))
        .method(t("testCMCAF", ms_test_cmcaf))
        .method(t("testCMV", ms_test_cmv))
        .method(t("testCMVF", ms_test_cmvf))
        .method(t("testCMVVA", ms_test_cmvva))
        .method(t("testCMVVCA", ms_test_cmvvca))
        .method(t("testCMVVCAF", ms_test_cmvvcaf))
        .method(t("testI", ms_test_i))
        .method(t("testE", ms_test_e))
        .build()
}

// -----------------------------------------------------------------------------
// TestWithCJKBigramFilter
// -----------------------------------------------------------------------------

/// `new ICUTokenizer(newAttributeFactory(), new DefaultICUTokenizerConfig(false, true))`.
fn cjk_off_tokenizer() -> Result<ICUTokenizer> {
    new_icu_tokenizer_with_factory(
        B::new_attribute_factory(),
        Arc::new(DefaultICUTokenizerConfig::new(false, true)),
    )
}

fn wcb_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    /*
     * ICUTokenizer+CJKBigramFilter
     */
    let analyzer = tokenizer_analyzer(
        "org.apache.lucene.analysis.icu.segmentation.TestWithCJKBigramFilter$1",
        cjk_off_tokenizer,
        |source| {
            let result = new_cjk_bigram_filter(source)?;
            Ok(boxed(new_stop_filter(
                boxed(result),
                CharArraySet::empty_set(),
            )?))
        },
    );
    /*
     * ICUTokenizer+ICUNormalizer2Filter+CJKBigramFilter.
     *
     * ICUNormalizer2Filter uses nfkc_casefold by default, so this is a language-independent
     * superset of CJKWidthFilter's foldings.
     */
    let analyzer2 = tokenizer_analyzer(
        "org.apache.lucene.analysis.icu.segmentation.TestWithCJKBigramFilter$2",
        cjk_off_tokenizer,
        |source| {
            // we put this before the CJKBigramFilter, because the normalization might combine
            // some halfwidth katakana forms, which will affect the bigramming.
            let result = new_icu_normalizer2_filter(source)?;
            let result = new_cjk_bigram_filter(boxed(result))?;
            Ok(boxed(new_stop_filter(
                boxed(result),
                CharArraySet::empty_set(),
            )?))
        },
    );
    set_analyzers(vec![analyzer, analyzer2]);
    Ok(())
}

fn wcb_test_ja1() -> Result<()> {
    let analyzer = analyzer();
    B::assert_analyzes_to_types(
        &*analyzer,
        "一二三四五六七八九十",
        &[
            "一二", "二三", "三四", "四五", "五六", "六七", "七八", "八九", "九十",
        ],
        Some(&[0, 1, 2, 3, 4, 5, 6, 7, 8]),
        Some(&[2, 3, 4, 5, 6, 7, 8, 9, 10]),
        Some(&[
            "<DOUBLE>", "<DOUBLE>", "<DOUBLE>", "<DOUBLE>", "<DOUBLE>", "<DOUBLE>", "<DOUBLE>",
            "<DOUBLE>", "<DOUBLE>",
        ]),
        Some(&[1, 1, 1, 1, 1, 1, 1, 1, 1]),
    )?;
    Ok(())
}

fn wcb_test_ja2() -> Result<()> {
    let analyzer = analyzer();
    B::assert_analyzes_to_types(
        &*analyzer,
        "一 二三四 五六七八九 十",
        &["一", "二三", "三四", "五六", "六七", "七八", "八九", "十"],
        Some(&[0, 2, 3, 6, 7, 8, 9, 12]),
        Some(&[1, 4, 5, 8, 9, 10, 11, 13]),
        Some(&[
            "<SINGLE>", "<DOUBLE>", "<DOUBLE>", "<DOUBLE>", "<DOUBLE>", "<DOUBLE>", "<DOUBLE>",
            "<SINGLE>",
        ]),
        Some(&[1, 1, 1, 1, 1, 1, 1, 1]),
    )?;
    Ok(())
}

fn wcb_test_c() -> Result<()> {
    let analyzer = analyzer();
    B::assert_analyzes_to_types(
        &*analyzer,
        "abc defgh ijklmn opqrstu vwxy z",
        &["abc", "defgh", "ijklmn", "opqrstu", "vwxy", "z"],
        Some(&[0, 4, 10, 17, 25, 30]),
        Some(&[3, 9, 16, 24, 29, 31]),
        Some(&[
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
        ]),
        Some(&[1, 1, 1, 1, 1, 1]),
    )?;
    Ok(())
}

fn wcb_test_final_offset() -> Result<()> {
    let analyzer = analyzer();
    B::assert_analyzes_to_types(
        &*analyzer,
        "あい",
        &["あい"],
        Some(&[0]),
        Some(&[2]),
        Some(&["<DOUBLE>"]),
        Some(&[1]),
    )?;
    B::assert_analyzes_to_types(
        &*analyzer,
        "あい   ",
        &["あい"],
        Some(&[0]),
        Some(&[2]),
        Some(&["<DOUBLE>"]),
        Some(&[1]),
    )?;
    B::assert_analyzes_to_types(
        &*analyzer,
        "test",
        &["test"],
        Some(&[0]),
        Some(&[4]),
        Some(&["<ALPHANUM>"]),
        Some(&[1]),
    )?;
    B::assert_analyzes_to_types(
        &*analyzer,
        "test   ",
        &["test"],
        Some(&[0]),
        Some(&[4]),
        Some(&["<ALPHANUM>"]),
        Some(&[1]),
    )?;
    B::assert_analyzes_to_types(
        &*analyzer,
        "あいtest",
        &["あい", "test"],
        Some(&[0, 2]),
        Some(&[2, 6]),
        Some(&["<DOUBLE>", "<ALPHANUM>"]),
        Some(&[1, 1]),
    )?;
    B::assert_analyzes_to_types(
        &*analyzer,
        "testあい    ",
        &["test", "あい"],
        Some(&[0, 4]),
        Some(&[4, 6]),
        Some(&["<ALPHANUM>", "<DOUBLE>"]),
        Some(&[1, 1]),
    )?;
    Ok(())
}

fn wcb_test_mix() -> Result<()> {
    let analyzer = analyzer();
    B::assert_analyzes_to_types(
        &*analyzer,
        "あいうえおabcかきくけこ",
        &[
            "あい", "いう", "うえ", "えお", "abc", "かき", "きく", "くけ", "けこ",
        ],
        Some(&[0, 1, 2, 3, 5, 8, 9, 10, 11]),
        Some(&[2, 3, 4, 5, 8, 10, 11, 12, 13]),
        Some(&[
            "<DOUBLE>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<ALPHANUM>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<DOUBLE>",
        ]),
        Some(&[1, 1, 1, 1, 1, 1, 1, 1, 1]),
    )?;
    Ok(())
}

fn wcb_test_mix2() -> Result<()> {
    let analyzer = analyzer();
    B::assert_analyzes_to_types(
        &*analyzer,
        "あいうえおabんcかきくけ こ",
        &[
            "あい", "いう", "うえ", "えお", "ab", "ん", "c", "かき", "きく", "くけ", "こ",
        ],
        Some(&[0, 1, 2, 3, 5, 7, 8, 9, 10, 11, 14]),
        Some(&[2, 3, 4, 5, 7, 8, 9, 11, 12, 13, 15]),
        Some(&[
            "<DOUBLE>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<ALPHANUM>",
            "<SINGLE>",
            "<ALPHANUM>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<SINGLE>",
        ]),
        Some(&[1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]),
    )?;
    Ok(())
}

fn wcb_test_non_ideographic() -> Result<()> {
    let analyzer = analyzer();
    B::assert_analyzes_to_types(
        &*analyzer,
        "一 روبرت موير",
        &["一", "روبرت", "موير"],
        Some(&[0, 2, 8]),
        Some(&[1, 7, 12]),
        Some(&["<SINGLE>", "<ALPHANUM>", "<ALPHANUM>"]),
        Some(&[1, 1, 1]),
    )?;
    Ok(())
}

fn wcb_test_non_ideographic_non_letter() -> Result<()> {
    let analyzer = analyzer();
    B::assert_analyzes_to_types(
        &*analyzer,
        "一 رُوبرت موير",
        &["一", "رُوبرت", "موير"],
        Some(&[0, 2, 9]),
        Some(&[1, 8, 13]),
        Some(&["<SINGLE>", "<ALPHANUM>", "<ALPHANUM>"]),
        Some(&[1, 1, 1]),
    )?;
    Ok(())
}

fn wcb_test_surrogates() -> Result<()> {
    let analyzer = analyzer();
    B::assert_analyzes_to_types(
        &*analyzer,
        "𩬅艱鍟䇹愯瀛",
        &["𩬅艱", "艱鍟", "鍟䇹", "䇹愯", "愯瀛"],
        Some(&[0, 2, 3, 4, 5]),
        Some(&[3, 4, 5, 6, 7]),
        Some(&["<DOUBLE>", "<DOUBLE>", "<DOUBLE>", "<DOUBLE>", "<DOUBLE>"]),
        Some(&[1, 1, 1, 1, 1]),
    )?;
    Ok(())
}

fn wcb_test_reusable_token_stream() -> Result<()> {
    let analyzer = analyzer();
    B::assert_analyzes_to_types(
        &*analyzer,
        "あいうえおabcかきくけこ",
        &[
            "あい", "いう", "うえ", "えお", "abc", "かき", "きく", "くけ", "けこ",
        ],
        Some(&[0, 1, 2, 3, 5, 8, 9, 10, 11]),
        Some(&[2, 3, 4, 5, 8, 10, 11, 12, 13]),
        Some(&[
            "<DOUBLE>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<ALPHANUM>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<DOUBLE>",
        ]),
        Some(&[1, 1, 1, 1, 1, 1, 1, 1, 1]),
    )?;
    B::assert_analyzes_to_types(
        &*analyzer,
        "あいうえおabんcかきくけ こ",
        &[
            "あい", "いう", "うえ", "えお", "ab", "ん", "c", "かき", "きく", "くけ", "こ",
        ],
        Some(&[0, 1, 2, 3, 5, 7, 8, 9, 10, 11, 14]),
        Some(&[2, 3, 4, 5, 7, 8, 9, 11, 12, 13, 15]),
        Some(&[
            "<DOUBLE>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<ALPHANUM>",
            "<SINGLE>",
            "<ALPHANUM>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<SINGLE>",
        ]),
        Some(&[1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]),
    )?;
    Ok(())
}

fn wcb_test_single_char() -> Result<()> {
    let analyzer = analyzer();
    B::assert_analyzes_to_types(
        &*analyzer,
        "一",
        &["一"],
        Some(&[0]),
        Some(&[1]),
        Some(&["<SINGLE>"]),
        Some(&[1]),
    )?;
    Ok(())
}

fn wcb_test_token_stream() -> Result<()> {
    let analyzer = analyzer();
    B::assert_analyzes_to_types(
        &*analyzer,
        "一丁丂",
        &["一丁", "丁丂"],
        Some(&[0, 1]),
        Some(&[2, 3]),
        Some(&["<DOUBLE>", "<DOUBLE>"]),
        Some(&[1, 1]),
    )?;
    Ok(())
}

fn with_cjk_bigram_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.icu.segmentation.TestWithCJKBigramFilter")
        .method(hook("setUp", wcb_set_up))
        .method(hook("tearDown", close_analyzers_tear_down))
        .method(t("testJa1", wcb_test_ja1))
        .method(t("testJa2", wcb_test_ja2))
        .method(t("testC", wcb_test_c))
        .method(t("testFinalOffset", wcb_test_final_offset))
        .method(t("testMix", wcb_test_mix))
        .method(t("testMix2", wcb_test_mix2))
        .method(t("testNonIdeographic", wcb_test_non_ideographic))
        .method(t(
            "testNonIdeographicNonLetter",
            wcb_test_non_ideographic_non_letter,
        ))
        .method(t("testSurrogates", wcb_test_surrogates))
        .method(t("testReusableTokenStream", wcb_test_reusable_token_stream))
        .method(t("testSingleChar", wcb_test_single_char))
        .method(t("testTokenStream", wcb_test_token_stream))
        .build()
}

// -----------------------------------------------------------------------------
// Suites
// -----------------------------------------------------------------------------

#[test]
fn test_char_array_iterator() {
    run(char_array_iterator_class);
}

#[test]
fn test_icu_tokenizer() {
    run(icu_tokenizer_class);
}

/// Lucene skips the whole class (`@AwaitsFix`, `tests.awaitsfix` unset): the
/// ported runner reports its 10 methods as assumption failures, none fails.
#[test]
fn test_icu_tokenizer_cjk() {
    run_allowing(icu_tokenizer_cjk_class, 0, true);
}

#[test]
fn test_icu_tokenizer_factory() {
    run(icu_tokenizer_factory_class);
}

#[test]
fn test_myanmar_syllable() {
    run(myanmar_syllable_class);
}

#[test]
fn test_with_cjk_bigram_filter() {
    run(with_cjk_bigram_filter_class);
}
