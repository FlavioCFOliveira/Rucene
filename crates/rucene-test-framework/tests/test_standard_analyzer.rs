//! Port of `lucene/core/src/test/org/apache/lucene/analysis/standard/TestStandardAnalyzer.java`:
//! every test method, including the Unicode test data `testUnicodeWordBreaks`
//! (`WordBreakTestUnicode_12_1_0.test(a)`) and `testUnicodeEmojiTests`
//! (`EmojiTokenizationTestUnicode_12_1.test(a)`), with the `setUp`/`tearDown`
//! that build and close the analyzer `a`, run as a suite extending
//! `BaseTokenStreamTestCase` by the ported runner against core
//! `StandardTokenizer` and `StandardAnalyzer`.

use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::cell::RefCell;
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::{Assert, JUnitCore, Request};
use rucene::analysis::standard::DEFAULT_MAX_TOKEN_LENGTH;
use rucene::analysis::{
    new_standard_tokenizer, new_standard_tokenizer_with_factory, Analyzer, AnalyzerState,
    CharReader, GlobalReuseStrategy, StandardAnalyzer, StringCharReader, TokenStream,
    TokenStreamComponents,
};
use rucene::error::Result;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::attributes::{char_term_id, Att};
use rucene_test_framework::analysis::base_token_stream_test_case::java_class_and_message;
use rucene_test_framework::analysis::mock_graph_token_filter::MockGraphTokenFilter;
use rucene_test_framework::analysis::standard::{
    EmojiTokenizationTestUnicode_12_1, WordBreakTestUnicode_12_1_0,
};
use rucene_test_framework::analysis::{
    components, shared, tokenizer_components, BaseTokenStreamTestCase,
};
use rucene_test_framework::util::lucene_test_case::RANDOM_MULTIPLIER;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

/// `TestStandardAnalyzer.a`: the anonymous `Analyzer` of `setUp`, whose
/// components are a `StandardTokenizer(newAttributeFactory())`.
#[derive(Debug)]
struct StandardTokenizerAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for StandardTokenizerAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.standard.TestStandardAnalyzer$1"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let tokenizer =
            new_standard_tokenizer_with_factory(BaseTokenStreamTestCase::new_attribute_factory())?;
        Ok(tokenizer_components(std::sync::Arc::new(
            rucene::util::atomic_ref_cell::AtomicRefCell::new(tokenizer),
        )))
    }
}

thread_local! {
    /// `TestStandardAnalyzer.a`, the instance field `setUp` sets and
    /// `tearDown` closes; `setUp`, the test and `tearDown` run on the test's
    /// thread.
    static A: RefCell<Option<StandardTokenizerAnalyzer>> = const { RefCell::new(None) };
}

/// `setUp()`: `super.setUp()`, then the analyzer.
fn set_up() {
    LuceneTestCase::super_set_up();
    A.with(|a| {
        *a.borrow_mut() = Some(StandardTokenizerAnalyzer {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        })
    });
}

/// `tearDown()`: `a.close()`, then `super.tearDown()`.
fn tear_down() {
    if let Some(a) = A.with(|a| a.borrow_mut().take()) {
        a.close();
    }
    LuceneTestCase::super_tear_down();
}

/// The body of a test over `a`.
fn with_analyzer(body: impl FnOnce(&dyn Analyzer) -> Result<()>) -> Result<()> {
    A.with(|a| body(a.borrow().as_ref().expect("setUp created the analyzer")))
}

/// `testUnicodeWordBreaks`.
fn test_unicode_word_breaks() -> Result<()> {
    with_analyzer(WordBreakTestUnicode_12_1_0::test)
}

/// `testUnicodeEmojiTests`.
fn test_unicode_emoji_tests() -> Result<()> {
    with_analyzer(EmojiTokenizationTestUnicode_12_1::test)
}

// LUCENE-5897: slow tokenization of strings of the form
// (\p{WB:ExtendNumLet}[\p{WB:Format}\p{WB:Extend}]*)+
/// Port of `TestStandardAnalyzer.testLargePartiallyMatchingToken`.
fn test_large_partially_matching_token() -> Result<()> {
    // TODO: get these lists of chars matching a property from ICU4J
    // http://www.unicode.org/Public/6.3.0/ucd/auxiliary/WordBreakProperty.txt
    let word_break_extend_num_let_chars: Vec<u16> =
        "_\u{203f}\u{2040}\u{2054}\u{fe33}\u{fe34}\u{fe4d}\u{fe4e}\u{fe4f}\u{ff3f}"
            .encode_utf16()
            .collect();

    // http://www.unicode.org/Public/6.3.0/ucd/auxiliary/WordBreakProperty.txt
    // only the first char in ranges
    let word_break_format_chars: [u32; 16] = [
        0xad, 0x600, 0x61c, 0x6dd, 0x70f, 0x180e, 0x200e, 0x202a, 0x2060, 0x2066, 0xfeff, 0xfff9,
        0x110bd, 0x1d173, 0xe0001, 0xe0020,
    ];

    // http://www.unicode.org/Public/6.3.0/ucd/auxiliary/WordBreakProperty.txt
    // only the first char in ranges
    let word_break_extend_chars: [u32; 219] = [
        0x300, 0x483, 0x591, 0x5bf, 0x5c1, 0x5c4, 0x5c7, 0x610, 0x64b, 0x670, 0x6d6, 0x6df, 0x6e7,
        0x6ea, 0x711, 0x730, 0x7a6, 0x7eb, 0x816, 0x81b, 0x825, 0x829, 0x859, 0x8e4, 0x900, 0x93a,
        0x93e, 0x951, 0x962, 0x981, 0x9bc, 0x9be, 0x9c7, 0x9cb, 0x9d7, 0x9e2, 0xa01, 0xa3c, 0xa3e,
        0xa47, 0xa4b, 0xa51, 0xa70, 0xa75, 0xa81, 0xabc, 0xabe, 0xac7, 0xacb, 0xae2, 0xb01, 0xb3c,
        0xb3e, 0xb47, 0xb4b, 0xb56, 0xb62, 0xb82, 0xbbe, 0xbc6, 0xbca, 0xbd7, 0xc01, 0xc3e, 0xc46,
        0xc4a, 0xc55, 0xc62, 0xc82, 0xcbc, 0xcbe, 0xcc6, 0xcca, 0xcd5, 0xce2, 0xd02, 0xd3e, 0xd46,
        0xd4a, 0xd57, 0xd62, 0xd82, 0xdca, 0xdcf, 0xdd6, 0xdd8, 0xdf2, 0xe31, 0xe34, 0xe47, 0xeb1,
        0xeb4, 0xebb, 0xec8, 0xf18, 0xf35, 0xf37, 0xf39, 0xf3e, 0xf71, 0xf86, 0xf8d, 0xf99, 0xfc6,
        0x102b, 0x1056, 0x105e, 0x1062, 0x1067, 0x1071, 0x1082, 0x108f, 0x109a, 0x135d, 0x1712,
        0x1732, 0x1752, 0x1772, 0x17b4, 0x17dd, 0x180b, 0x18a9, 0x1920, 0x1930, 0x19b0, 0x19c8,
        0x1a17, 0x1a55, 0x1a60, 0x1a7f, 0x1b00, 0x1b34, 0x1b6b, 0x1b80, 0x1ba1, 0x1be6, 0x1c24,
        0x1cd0, 0x1cd4, 0x1ced, 0x1cf2, 0x1dc0, 0x1dfc, 0x200c, 0x20d0, 0x2cef, 0x2d7f, 0x2de0,
        0x302a, 0x3099, 0xa66f, 0xa674, 0xa69f, 0xa6f0, 0xa802, 0xa806, 0xa80b, 0xa823, 0xa880,
        0xa8b4, 0xa8e0, 0xa926, 0xa947, 0xa980, 0xa9b3, 0xaa29, 0xaa43, 0xaa4c, 0xaa7b, 0xaab0,
        0xaab2, 0xaab7, 0xaabe, 0xaac1, 0xaaeb, 0xaaf5, 0xabe3, 0xabec, 0xfb1e, 0xfe00, 0xfe20,
        0xff9e, 0x101fd, 0x10a01, 0x10a05, 0x10a0c, 0x10a38, 0x10a3f, 0x11000, 0x11001, 0x11038,
        0x11080, 0x11082, 0x110b0, 0x110b3, 0x110b7, 0x110b9, 0x11100, 0x11127, 0x1112c, 0x11180,
        0x11182, 0x111b3, 0x111b6, 0x111bf, 0x116ab, 0x116ac, 0x116b0, 0x116b6, 0x16f51, 0x16f8f,
        0x1d165, 0x1d167, 0x1d16d, 0x1d17b, 0x1d185, 0x1d1aa, 0x1d242, 0xe0100,
    ];

    let random = LuceneTestCase::random();
    let mut builder = String::new();
    let num_chars = TestUtil::next_int(&*random, 100 * 1024, 1024 * 1024);
    let mut i = 0;
    while i < num_chars {
        builder.push(
            char::from_u32(u32::from(
                word_break_extend_num_let_chars
                    [random.next_int_bound(word_break_extend_num_let_chars.len() as i32) as usize],
            ))
            .expect("a BMP non-surrogate char"),
        );
        i += 1;
        if random.next_boolean() {
            let num_format_extend_chars = TestUtil::next_int(&*random, 1, 8);
            for _ in 0..num_format_extend_chars {
                let codepoint = if random.next_boolean() {
                    word_break_format_chars
                        [random.next_int_bound(word_break_format_chars.len() as i32) as usize]
                } else {
                    word_break_extend_chars
                        [random.next_int_bound(word_break_extend_chars.len() as i32) as usize]
                };
                // Character.toChars(codepoint)
                let c = char::from_u32(codepoint).expect("a valid code point");
                builder.push(c);
                i += c.len_utf16() as i32;
            }
        }
    }
    let mut ts = new_standard_tokenizer();
    ts.set_reader(Box::new(StringCharReader::new(builder.as_str())))?;
    ts.reset()?;
    while ts.increment_token()? {}
    ts.end()?;
    ts.close()?;

    let new_buffer_size = TestUtil::next_int(&*random, 200, 8192);
    ts.logic_mut().set_max_token_length(new_buffer_size)?; // try a different buffer size
    ts.set_reader(Box::new(StringCharReader::new(builder.as_str())))?;
    ts.reset()?;
    while ts.increment_token()? {}
    ts.end()?;
    ts.close()?;
    Ok(())
}

/// Port of `TestStandardAnalyzer.testHugeDoc`.
fn test_huge_doc() -> Result<()> {
    let mut sb = String::new();
    let whitespace = " ".repeat(4094);
    sb.push_str(&whitespace);
    sb.push_str("testing 1234");
    let input = sb;
    let mut tokenizer = new_standard_tokenizer();
    tokenizer.set_reader(Box::new(StringCharReader::new(input)))?;
    BaseTokenStreamTestCase::assert_token_stream_contents(&mut tokenizer, &["testing", "1234"])
}

/// Port of `TestStandardAnalyzer.testArmenian`.
fn test_armenian() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to(a, "Վիքիպեդիայի 13 միլիոն հոդվածները (4,600` հայերեն վիքիպեդիայում) գրվել են կամավորների կողմից ու համարյա բոլոր հոդվածները կարող է խմբագրել ցանկաց մարդ ով կարող է բացել Վիքիպեդիայի կայքը։", &["Վիքիպեդիայի", "13", "միլիոն", "հոդվածները", "4,600", "հայերեն", "վիքիպեդիայում", "գրվել", "են", "կամավորների", "կողմից", "ու", "համարյա", "բոլոր", "հոդվածները", "կարող", "է", "խմբագրել", "ցանկաց", "մարդ", "ով", "կարող", "է", "բացել", "Վիքիպեդիայի", "կայքը"])?;
        Ok(())
    })
}

/// Port of `TestStandardAnalyzer.testAmharic`.
fn test_amharic() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to(
            a,
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
    })
}

/// Port of `TestStandardAnalyzer.testArabic`.
fn test_arabic() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to(a, "الفيلم الوثائقي الأول عن ويكيبيديا يسمى \"الحقيقة بالأرقام: قصة ويكيبيديا\" (بالإنجليزية: Truth in Numbers: The Wikipedia Story)، سيتم إطلاقه في 2008.", &["الفيلم", "الوثائقي", "الأول", "عن", "ويكيبيديا", "يسمى", "الحقيقة", "بالأرقام", "قصة", "ويكيبيديا", "بالإنجليزية", "Truth", "in", "Numbers", "The", "Wikipedia", "Story", "سيتم", "إطلاقه", "في", "2008"])?;
        Ok(())
    })
}

/// Port of `TestStandardAnalyzer.testAramaic`.
fn test_aramaic() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to(a, "ܘܝܩܝܦܕܝܐ (ܐܢܓܠܝܐ: Wikipedia) ܗܘ ܐܝܢܣܩܠܘܦܕܝܐ ܚܐܪܬܐ ܕܐܢܛܪܢܛ ܒܠܫܢ̈ܐ ܣܓܝܐ̈ܐ܂ ܫܡܗ ܐܬܐ ܡܢ ܡ̈ܠܬܐ ܕ\"ܘܝܩܝ\" ܘ\"ܐܝܢܣܩܠܘܦܕܝܐ\"܀", &["ܘܝܩܝܦܕܝܐ", "ܐܢܓܠܝܐ", "Wikipedia", "ܗܘ", "ܐܝܢܣܩܠܘܦܕܝܐ", "ܚܐܪܬܐ", "ܕܐܢܛܪܢܛ", "ܒܠܫܢ̈ܐ", "ܣܓܝܐ̈ܐ", "ܫܡܗ", "ܐܬܐ", "ܡܢ", "ܡ̈ܠܬܐ", "ܕ", "ܘܝܩܝ", "ܘ", "ܐܝܢܣܩܠܘܦܕܝܐ"])?;
        Ok(())
    })
}

/// Port of `TestStandardAnalyzer.testBengali`.
fn test_bengali() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to(a, "এই বিশ্বকোষ পরিচালনা করে উইকিমিডিয়া ফাউন্ডেশন (একটি অলাভজনক সংস্থা)। উইকিপিডিয়ার শুরু ১৫ জানুয়ারি, ২০০১ সালে। এখন পর্যন্ত ২০০টিরও বেশী ভাষায় উইকিপিডিয়া রয়েছে।", &["এই", "বিশ্বকোষ", "পরিচালনা", "করে", "উইকিমিডিয়া", "ফাউন্ডেশন", "একটি", "অলাভজনক", "সংস্থা", "উইকিপিডিয়ার", "শুরু", "১৫", "জানুয়ারি", "২০০১", "সালে", "এখন", "পর্যন্ত", "২০০টিরও", "বেশী", "ভাষায়", "উইকিপিডিয়া", "রয়েছে"])?;
        Ok(())
    })
}

/// Port of `TestStandardAnalyzer.testFarsi`.
fn test_farsi() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to(a, "ویکی پدیای انگلیسی در تاریخ ۲۵ دی ۱۳۷۹ به صورت مکملی برای دانشنامهٔ تخصصی نوپدیا نوشته شد.", &["ویکی", "پدیای", "انگلیسی", "در", "تاریخ", "۲۵", "دی", "۱۳۷۹", "به", "صورت", "مکملی", "برای", "دانشنامهٔ", "تخصصی", "نوپدیا", "نوشته", "شد"])?;
        Ok(())
    })
}

/// Port of `TestStandardAnalyzer.testGreek`.
fn test_greek() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to(a, "Γράφεται σε συνεργασία από εθελοντές με το λογισμικό wiki, κάτι που σημαίνει ότι άρθρα μπορεί να προστεθούν ή να αλλάξουν από τον καθένα.", &["Γράφεται", "σε", "συνεργασία", "από", "εθελοντές", "με", "το", "λογισμικό", "wiki", "κάτι", "που", "σημαίνει", "ότι", "άρθρα", "μπορεί", "να", "προστεθούν", "ή", "να", "αλλάξουν", "από", "τον", "καθένα"])?;
        Ok(())
    })
}

/// Port of `TestStandardAnalyzer.testThai`.
fn test_thai() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to(
            a,
            "การที่ได้ต้องแสดงว่างานดี. แล้วเธอจะไปไหน? ๑๒๓๔",
            &["การที่ได้ต้องแสดงว่างานดี", "แล้วเธอจะไปไหน", "๑๒๓๔"],
        )?;
        Ok(())
    })
}

/// Port of `TestStandardAnalyzer.testLao`.
fn test_lao() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to(
            a,
            "ສາທາລະນະລັດ ປະຊາທິປະໄຕ ປະຊາຊົນລາວ",
            &["ສາທາລະນະລັດ", "ປະຊາທິປະໄຕ", "ປະຊາຊົນລາວ"],
        )?;
        Ok(())
    })
}

/// Port of `TestStandardAnalyzer.testTibetan`.
fn test_tibetan() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to(
            a,
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
    })
}

/// For chinese, tokenize as char (these can later form bigrams or whatever)
///
/// Port of `TestStandardAnalyzer.testChinese`.
fn test_chinese() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to(
            a,
            "我是中国人。 １２３４ Ｔｅｓｔｓ ",
            &["我", "是", "中", "国", "人", "１２３４", "Ｔｅｓｔｓ"],
        )?;
        Ok(())
    })
}

/// Port of `TestStandardAnalyzer.testEmpty`.
fn test_empty() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to(a, "", &[])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, ".", &[])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, " ", &[])?;
        Ok(())
    })
}

/// test various jira issues this analyzer is related to
///
/// Port of `TestStandardAnalyzer.testLUCENE1545`.
fn test_lucene1545() -> Result<()> {
    with_analyzer(|a| {
        // Standard analyzer does not correctly tokenize combining character U+0364 COMBINING
        // LATIN SMALL LETTRE E. The word "moͤchte" is incorrectly tokenized into "mo" "chte", the
        // combining character is lost. Expected result is only on token "moͤchte".
        BaseTokenStreamTestCase::assert_analyzes_to(a, "moͤchte", &["moͤchte"])?;
        Ok(())
    })
}

/// Tests from StandardAnalyzer, just to show behavior is similar
///
/// Port of `TestStandardAnalyzer.testAlphanumericSA`.
fn test_alphanumeric_sa() -> Result<()> {
    with_analyzer(|a| {
        // alphanumeric tokens
        BaseTokenStreamTestCase::assert_analyzes_to(a, "B2B", &["B2B"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "2B", &["2B"])?;
        Ok(())
    })
}

/// Port of `TestStandardAnalyzer.testDelimitersSA`.
fn test_delimiters_sa() -> Result<()> {
    with_analyzer(|a| {
        // other delimiters: "-", "/", ","
        BaseTokenStreamTestCase::assert_analyzes_to(
            a,
            "some-dashed-phrase",
            &["some", "dashed", "phrase"],
        )?;
        BaseTokenStreamTestCase::assert_analyzes_to(
            a,
            "dogs,chase,cats",
            &["dogs", "chase", "cats"],
        )?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "ac/dc", &["ac", "dc"])?;
        Ok(())
    })
}

/// Port of `TestStandardAnalyzer.testApostrophesSA`.
fn test_apostrophes_sa() -> Result<()> {
    with_analyzer(|a| {
        // internal apostrophes: O'Reilly, you're, O'Reilly's
        BaseTokenStreamTestCase::assert_analyzes_to(a, "O'Reilly", &["O'Reilly"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "you're", &["you're"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "she's", &["she's"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "Jim's", &["Jim's"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "don't", &["don't"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "O'Reilly's", &["O'Reilly's"])?;
        Ok(())
    })
}

/// Port of `TestStandardAnalyzer.testNumericSA`.
fn test_numeric_sa() -> Result<()> {
    with_analyzer(|a| {
        // floating point, serial, model numbers, ip addresses, etc.
        BaseTokenStreamTestCase::assert_analyzes_to(a, "21.35", &["21.35"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "R2D2 C3PO", &["R2D2", "C3PO"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "216.239.63.104", &["216.239.63.104"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "216.239.63.104", &["216.239.63.104"])?;
        Ok(())
    })
}

/// Port of `TestStandardAnalyzer.testTextWithNumbersSA`.
fn test_text_with_numbers_sa() -> Result<()> {
    with_analyzer(|a| {
        // numbers
        BaseTokenStreamTestCase::assert_analyzes_to(
            a,
            "David has 5000 bones",
            &["David", "has", "5000", "bones"],
        )?;
        Ok(())
    })
}

/// Port of `TestStandardAnalyzer.testVariousTextSA`.
fn test_various_text_sa() -> Result<()> {
    with_analyzer(|a| {
        // various
        BaseTokenStreamTestCase::assert_analyzes_to(
            a,
            "C embedded developers wanted",
            &["C", "embedded", "developers", "wanted"],
        )?;
        BaseTokenStreamTestCase::assert_analyzes_to(
            a,
            "foo bar FOO BAR",
            &["foo", "bar", "FOO", "BAR"],
        )?;
        BaseTokenStreamTestCase::assert_analyzes_to(
            a,
            "foo      bar .  FOO <> BAR",
            &["foo", "bar", "FOO", "BAR"],
        )?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "\"QUOTED\" word", &["QUOTED", "word"])?;
        Ok(())
    })
}

/// Port of `TestStandardAnalyzer.testKoreanSA`.
fn test_korean_sa() -> Result<()> {
    with_analyzer(|a| {
        // Korean words
        BaseTokenStreamTestCase::assert_analyzes_to(
            a,
            "안녕하세요 한글입니다",
            &["안녕하세요", "한글입니다"],
        )?;
        Ok(())
    })
}

/// Port of `TestStandardAnalyzer.testOffsets`.
fn test_offsets() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to_offsets(
            a,
            "David has 5000 bones",
            &["David", "has", "5000", "bones"],
            &[0, 6, 10, 15],
            &[5, 9, 14, 20],
        )?;
        Ok(())
    })
}

/// Port of `TestStandardAnalyzer.testTypes`.
fn test_types() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to_types_only(
            a,
            "David has 5000 bones",
            &["David", "has", "5000", "bones"],
            &["<ALPHANUM>", "<ALPHANUM>", "<NUM>", "<ALPHANUM>"],
        )?;
        Ok(())
    })
}

/// Port of `TestStandardAnalyzer.testSupplementary`.
fn test_supplementary() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to_types_only(
            a,
            "𩬅艱鍟䇹愯瀛",
            &["𩬅", "艱", "鍟", "䇹", "愯", "瀛"],
            &[
                "<IDEOGRAPHIC>",
                "<IDEOGRAPHIC>",
                "<IDEOGRAPHIC>",
                "<IDEOGRAPHIC>",
                "<IDEOGRAPHIC>",
                "<IDEOGRAPHIC>",
            ],
        )?;
        Ok(())
    })
}

/// Port of `TestStandardAnalyzer.testKorean`.
fn test_korean() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to_types_only(
            a,
            "훈민정음",
            &["훈민정음"],
            &["<HANGUL>"],
        )?;
        Ok(())
    })
}

/// Port of `TestStandardAnalyzer.testJapanese`.
fn test_japanese() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to_types_only(
            a,
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
    })
}

/// Port of `TestStandardAnalyzer.testCombiningMarks`.
fn test_combining_marks() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::check_one_term(a, "ざ", "ざ")?; // hiragana
        BaseTokenStreamTestCase::check_one_term(a, "ザ", "ザ")?; // katakana
        BaseTokenStreamTestCase::check_one_term(a, "壹゙", "壹゙")?; // ideographic
        BaseTokenStreamTestCase::check_one_term(a, "아゙", "아゙")?; // hangul
        Ok(())
    })
}

/// Multiple consecutive chars in \p{WB:MidLetter}, \p{WB:MidNumLet}, and/or \p{MidNum} should
/// trigger a token split.
///
/// Port of `TestStandardAnalyzer.testMid`.
fn test_mid() -> Result<()> {
    with_analyzer(|a| {
        // ':' is in \p{WB:MidLetter}, which should trigger a split unless there is a Letter char on
        // both sides
        BaseTokenStreamTestCase::assert_analyzes_to(a, "A:B", &["A:B"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "A::B", &["A", "B"])?;
        // '.' is in \p{WB:MidNumLet}, which should trigger a split unless there is a Letter or Numeric
        // char on both sides
        BaseTokenStreamTestCase::assert_analyzes_to(a, "1.2", &["1.2"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "A.B", &["A.B"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "1..2", &["1", "2"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "A..B", &["A", "B"])?;
        // ',' is in \p{WB:MidNum}, which should trigger a split unless there is a Numeric char on both
        // sides
        BaseTokenStreamTestCase::assert_analyzes_to(a, "1,2", &["1,2"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "1,,2", &["1", "2"])?;
        // Mixed consecutive \p{WB:MidLetter} and \p{WB:MidNumLet} should trigger a split
        BaseTokenStreamTestCase::assert_analyzes_to(a, "A.:B", &["A", "B"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "A:.B", &["A", "B"])?;
        // Mixed consecutive \p{WB:MidNum} and \p{WB:MidNumLet} should trigger a split
        BaseTokenStreamTestCase::assert_analyzes_to(a, "1,.2", &["1", "2"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "1.,2", &["1", "2"])?;
        // '_' is in \p{WB:ExtendNumLet}
        BaseTokenStreamTestCase::assert_analyzes_to(a, "A:B_A:B", &["A:B_A:B"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "A:B_A::B", &["A:B_A", "B"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "1.2_1.2", &["1.2_1.2"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "A.B_A.B", &["A.B_A.B"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "1.2_1..2", &["1.2_1", "2"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "A.B_A..B", &["A.B_A", "B"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "1,2_1,2", &["1,2_1,2"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "1,2_1,,2", &["1,2_1", "2"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "C_A.:B", &["C_A", "B"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "C_A:.B", &["C_A", "B"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "3_1,.2", &["3_1", "2"])?;
        BaseTokenStreamTestCase::assert_analyzes_to(a, "3_1.,2", &["3_1", "2"])?;
        Ok(())
    })
}

/// simple emoji
///
/// Port of `TestStandardAnalyzer.testEmoji`.
fn test_emoji() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to_types_only(
            a,
            "💩 💩💩",
            &["💩", "💩", "💩"],
            &["<EMOJI>", "<EMOJI>", "<EMOJI>"],
        )?;
        Ok(())
    })
}

/// emoji zwj sequence
///
/// Port of `TestStandardAnalyzer.testEmojiSequence`.
fn test_emoji_sequence() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to_types_only(a, "👩‍❤️‍👩", &["👩‍❤️‍👩"], &["<EMOJI>"])?;
        Ok(())
    })
}

/// emoji zwj sequence with fitzpatrick modifier
///
/// Port of `TestStandardAnalyzer.testEmojiSequenceWithModifier`.
fn test_emoji_sequence_with_modifier() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to_types_only(a, "👨🏼‍⚕️", &["👨🏼‍⚕️"], &["<EMOJI>"])?;
        Ok(())
    })
}

/// regional indicator
///
/// Port of `TestStandardAnalyzer.testEmojiRegionalIndicator`.
fn test_emoji_regional_indicator() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to_types_only(
            a,
            "🇺🇸🇺🇸",
            &["🇺🇸", "🇺🇸"],
            &["<EMOJI>", "<EMOJI>"],
        )?;
        Ok(())
    })
}

/// variation sequence
///
/// Port of `TestStandardAnalyzer.testEmojiVariationSequence`.
fn test_emoji_variation_sequence() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to_types_only(a, "#️⃣", &["#️⃣"], &["<EMOJI>"])?;
        BaseTokenStreamTestCase::assert_analyzes_to_types_only(a, "3️⃣", &["3️⃣"], &["<EMOJI>"])?;
        // text presentation sequences
        BaseTokenStreamTestCase::assert_analyzes_to_types_only(a, "#\u{FE0E}", &[], &[])?;
        // \u{FE0E} is included in \p{WB:Extend}
        BaseTokenStreamTestCase::assert_analyzes_to_types_only(
            a,
            "3\u{FE0E}",
            &["3\u{FE0E}"],
            &["<NUM>"],
        )?;
        // \u{2B55} = HEAVY BLACK CIRCLE
        BaseTokenStreamTestCase::assert_analyzes_to_types_only(
            a,
            "\u{2B55}\u{FE0E}",
            &["\u{2B55}"],
            &["<EMOJI>"],
        )?;
        BaseTokenStreamTestCase::assert_analyzes_to_types_only(
            a,
            "\u{2B55}\u{FE0E}\u{200D}\u{2B55}\u{FE0E}",
            &["\u{2B55}", "\u{200D}\u{2B55}"],
            &["<EMOJI>", "<EMOJI>"],
        )?;
        Ok(())
    })
}

/// Port of `TestStandardAnalyzer.testEmojiTagSequence`.
fn test_emoji_tag_sequence() -> Result<()> {
    with_analyzer(|a| {
        BaseTokenStreamTestCase::assert_analyzes_to_types_only(a, "🏴󠁧󠁢󠁥󠁮󠁧󠁿", &["🏴󠁧󠁢󠁥󠁮󠁧󠁿"], &["<EMOJI>"])?;
        Ok(())
    })
}

/// Port of `TestStandardAnalyzer.testEmojiTokenization`.
fn test_emoji_tokenization() -> Result<()> {
    with_analyzer(|a| {
        // simple emoji around latin
        BaseTokenStreamTestCase::assert_analyzes_to_types_only(
            a,
            "poo💩poo",
            &["poo", "💩", "poo"],
            &["<ALPHANUM>", "<EMOJI>", "<ALPHANUM>"],
        )?;
        // simple emoji around non-latin
        BaseTokenStreamTestCase::assert_analyzes_to_types_only(
            a,
            "💩中國💩",
            &["💩", "中", "國", "💩"],
            &["<EMOJI>", "<IDEOGRAPHIC>", "<IDEOGRAPHIC>", "<EMOJI>"],
        )?;
        Ok(())
    })
}
/// blast some random strings through the analyzer
///
/// Port of `TestStandardAnalyzer.testRandomStrings`.
fn test_random_strings() -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(StandardAnalyzer::new());
    BaseTokenStreamTestCase::check_random_data(
        &*LuceneTestCase::random(),
        &analyzer,
        200 * *RANDOM_MULTIPLIER,
    )?;
    analyzer.close();
    Ok(())
}

/// blast some random large strings through the analyzer
///
/// Port of `TestStandardAnalyzer.testRandomHugeStrings`.
fn test_random_huge_strings() -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(StandardAnalyzer::new());
    BaseTokenStreamTestCase::check_random_data_max_word_length(
        &*LuceneTestCase::random(),
        &analyzer,
        20 * *RANDOM_MULTIPLIER,
        8192,
    )?;
    analyzer.close();
    Ok(())
}

/// The anonymous `Analyzer` of `testRandomHugeStringsGraphAfter`: a
/// `StandardTokenizer(newAttributeFactory())` followed by a
/// `MockGraphTokenFilter(random(), tokenizer)`.
#[derive(Debug)]
struct GraphAfterAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for GraphAfterAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.standard.TestStandardAnalyzer$2"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let tokenizer = Arc::new(AtomicRefCell::new(new_standard_tokenizer_with_factory(
            BaseTokenStreamTestCase::new_attribute_factory(),
        )?));
        let token_stream =
            MockGraphTokenFilter::new(&*LuceneTestCase::random(), shared(&tokenizer))?;
        let sink: Arc<AtomicRefCell<dyn TokenStream>> = Arc::new(AtomicRefCell::new(token_stream));
        Ok(components(tokenizer, sink))
    }
}

// Adds random graph after:
/// Port of `TestStandardAnalyzer.testRandomHugeStringsGraphAfter`.
fn test_random_huge_strings_graph_after() -> Result<()> {
    let random = LuceneTestCase::random();
    let analyzer: Arc<dyn Analyzer> = Arc::new(GraphAfterAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
    });
    BaseTokenStreamTestCase::check_random_data_max_word_length(
        &*random,
        &analyzer,
        20 * *RANDOM_MULTIPLIER,
        8192,
    )?;
    analyzer.close();
    Ok(())
}

/// Port of `TestStandardAnalyzer.testNormalize`.
fn test_normalize() -> Result<()> {
    let a = StandardAnalyzer::new();
    Assert::assert_equals(
        BytesRef::from_char_sequence("\"\\à3[]()! cz@"),
        a.normalize("dummy", "\"\\À3[]()! Cz@")?,
    );
    Ok(())
}

/// Port of `TestStandardAnalyzer.testMaxTokenLengthDefault`.
fn test_max_token_length_default() -> Result<()> {
    let a = StandardAnalyzer::new();

    // exact max length:
    let b_string = "b".repeat(DEFAULT_MAX_TOKEN_LENGTH);
    // first bString is exact max default length; next one is 1 too long
    let input = format!("x {b_string} {b_string}b");
    BaseTokenStreamTestCase::assert_analyzes_to(&a, &input, &["x", &b_string, &b_string, "b"])?;
    a.close();
    Ok(())
}

/// Port of `TestStandardAnalyzer.testMaxTokenLengthNonDefault`.
fn test_max_token_length_non_default() -> Result<()> {
    let mut a = StandardAnalyzer::new();
    a.set_max_token_length(5);
    BaseTokenStreamTestCase::assert_analyzes_to(
        &a,
        "ab cd toolong xy z",
        &["ab", "cd", "toolo", "ng", "xy", "z"],
    )?;
    a.close();
    Ok(())
}

/// Port of `TestStandardAnalyzer.testSplitSurrogatePairWithSpoonFeedReader`.
fn test_split_surrogate_pair_with_spoon_feed_reader() -> Result<()> {
    let text = "12345678\u{10300}"; // U+D800 U+DF00 = U+10300 = 𐌀 (OLD ITALIC LETTER A)

    // Collect tokens with normal reader
    let a = StandardAnalyzer::new();
    let ts = a.token_stream_from_str("dummy", text)?;
    let mut tokens: Vec<String> = Vec::new();
    {
        let mut ts = ts.borrow_mut();
        let term_att = Att::add(ts.attribute_source_mut(), char_term_id())?;
        ts.reset()?;
        while ts.increment_token()? {
            tokens.push(term_att.term_string());
        }
        ts.end()?;
        ts.close()?;
    }

    // Tokens from a spoon-feed reader should be the same as from a normal reader
    // The 9th char is a high surrogate, so the 9-max-chars spoon-feed reader will split the
    // surrogate pair at a read boundary
    let reader = Box::new(SpoonFeedMaxCharsReaderWrapper::new(
        9,
        Box::new(StringCharReader::new(text)),
    ));
    let ts = a.token_stream("dummy", reader)?;
    let mut ts = ts.borrow_mut();
    let term_att = Att::add(ts.attribute_source_mut(), char_term_id())?;
    ts.reset()?;
    let mut token_num = 0;
    while ts.increment_token()? {
        // `tokens.get(tokenNum)` throws `IndexOutOfBoundsException` past the end.
        let expected = tokens.get(token_num).unwrap_or_else(|| {
            Throwable::new(
                "java.lang.IndexOutOfBoundsException",
                format!(
                    "Index {token_num} out of bounds for length {}",
                    tokens.len()
                ),
            )
            .throw()
        });
        Assert::assert_equals_str(
            Some(format!("token #{token_num} mismatch: ").as_str()),
            Some(term_att.term_string().as_str()),
            Some(expected.as_str()),
        );
        token_num += 1;
    }
    ts.end()?;
    ts.close()?;
    Ok(())
}

/// `SpoonFeedMaxCharsReaderWrapper` (package-private class of
/// `TestStandardAnalyzer.java`): returns at most the configured number of
/// chars per read.
#[derive(Debug)]
struct SpoonFeedMaxCharsReaderWrapper {
    input: Box<dyn CharReader>,
    max_chars: usize,
}

impl SpoonFeedMaxCharsReaderWrapper {
    fn new(max_chars: usize, input: Box<dyn CharReader>) -> Self {
        Self { input, max_chars }
    }
}

impl CharReader for SpoonFeedMaxCharsReaderWrapper {
    /// Returns the configured number of chars if available
    fn read(&mut self, buf: &mut [u16]) -> Result<usize> {
        let len = self.max_chars.min(buf.len());
        self.input.read(&mut buf[..len])
    }

    fn close(&mut self) -> Result<()> {
        self.input.close()
    }
}

type TestFn = fn() -> Result<()>;

/// `TestStandardAnalyzer extends BaseTokenStreamTestCase`, its test methods
/// found as `LuceneTestCase`'s JUnit 3 provider finds them, with
/// its `setUp`/`tearDown` overrides, under the full `LuceneTestCase` rule
/// chain.
#[test]
fn test_standard_analyzer() {
    let methods: [(&'static str, TestFn); 45] = [
        (
            "testLargePartiallyMatchingToken",
            test_large_partially_matching_token,
        ),
        ("testHugeDoc", test_huge_doc),
        ("testArmenian", test_armenian),
        ("testAmharic", test_amharic),
        ("testArabic", test_arabic),
        ("testAramaic", test_aramaic),
        ("testBengali", test_bengali),
        ("testFarsi", test_farsi),
        ("testGreek", test_greek),
        ("testThai", test_thai),
        ("testLao", test_lao),
        ("testTibetan", test_tibetan),
        ("testChinese", test_chinese),
        ("testEmpty", test_empty),
        ("testLUCENE1545", test_lucene1545),
        ("testAlphanumericSA", test_alphanumeric_sa),
        ("testDelimitersSA", test_delimiters_sa),
        ("testApostrophesSA", test_apostrophes_sa),
        ("testNumericSA", test_numeric_sa),
        ("testTextWithNumbersSA", test_text_with_numbers_sa),
        ("testVariousTextSA", test_various_text_sa),
        ("testKoreanSA", test_korean_sa),
        ("testOffsets", test_offsets),
        ("testTypes", test_types),
        ("testUnicodeWordBreaks", test_unicode_word_breaks),
        ("testSupplementary", test_supplementary),
        ("testKorean", test_korean),
        ("testJapanese", test_japanese),
        ("testCombiningMarks", test_combining_marks),
        ("testMid", test_mid),
        ("testEmoji", test_emoji),
        ("testEmojiSequence", test_emoji_sequence),
        (
            "testEmojiSequenceWithModifier",
            test_emoji_sequence_with_modifier,
        ),
        ("testEmojiRegionalIndicator", test_emoji_regional_indicator),
        ("testEmojiVariationSequence", test_emoji_variation_sequence),
        ("testEmojiTagSequence", test_emoji_tag_sequence),
        ("testEmojiTokenization", test_emoji_tokenization),
        ("testUnicodeEmojiTests", test_unicode_emoji_tests),
        ("testRandomStrings", test_random_strings),
        ("testRandomHugeStrings", test_random_huge_strings),
        (
            "testRandomHugeStringsGraphAfter",
            test_random_huge_strings_graph_after,
        ),
        ("testNormalize", test_normalize),
        ("testMaxTokenLengthDefault", test_max_token_length_default),
        (
            "testMaxTokenLengthNonDefault",
            test_max_token_length_non_default,
        ),
        (
            "testSplitSurrogatePairWithSpoonFeedReader",
            test_split_surrogate_pair_with_spoon_feed_reader,
        ),
    ];
    let mut builder = Class::builder("org.apache.lucene.analysis.standard.TestStandardAnalyzer")
        .extends(&BaseTokenStreamTestCase::class())
        .constructor(Constructor::no_args(|| ()))
        // As in Java, the overrides carry no `@Before`/`@After`: the runner
        // invokes `LuceneTestCase.setUp`/`tearDown`, which dispatch to them.
        .method(
            Method::new("setUp")
                .throws(&["java.lang.Exception"])
                .body_any(set_up),
        )
        .method(
            Method::new("tearDown")
                .throws(&["java.lang.Exception"])
                .body_any(tear_down),
        );
    for (method, body) in methods {
        builder = builder.method(Method::new(method).body_any(move || {
            if let Err(e) = body() {
                let (class, message) = java_class_and_message(&e);
                Throwable::new(class, message).throw();
            }
        }));
    }
    let class = builder.build();
    let result = JUnitCore::new().run(&Request::a_class(&class));
    let failures: Vec<String> = result
        .failures()
        .iter()
        .map(|f| format!("{}: {}", f.description(), f.exception()))
        .collect();
    assert_eq!(result.run_count(), 45, "every test method ran");
    assert!(
        failures.is_empty(),
        "TestStandardAnalyzer failures:\n{}",
        failures.join("\n")
    );
}
