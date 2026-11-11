//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexWriterUnicode.java`.
//!
//! * Java `char[]` buffers are `u16` slices; `new BytesRef(CharSequence)`
//!   over them is its body, `UnicodeUtil.UTF16toUTF8` into a buffer of
//!   `maxUTF8Length` bytes ([`bytes_ref_of_utf16`]).
//! * **Divergence.** Java builds field values and terms from `String`s that
//!   hold unpaired surrogates (`utf8Data`, `TestIndexWriterUnicode.java:43-71`);
//!   a Rust `String` is valid UTF-8 by construction and cannot hold one, so
//!   the field values are built with `String::from_utf16_lossy`, which
//!   replaces each unpaired surrogate with U+FFFD, the replacement Lucene's
//!   `UnicodeUtil.UTF16toUTF8` applies when the value is indexed and stored.
//!   The expected values (`utf8Data[2 * i + 1]`) are valid and unchanged.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::terms::{SeekStatus, Term};
use rucene::index::IndexReader;
use rucene::util::unicode_util::{
    UnicodeUtil, UNI_SUR_HIGH_END, UNI_SUR_HIGH_START, UNI_SUR_LOW_END, UNI_SUR_LOW_START,
};
use rucene::util::{BytesRef, BytesRefBuilder, CharsRefBuilder};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_equals_msg, class, random, test};
use crate::support::field_mut;
use crate::test_index_writer::StringSplitAnalyzer;

/// `TestIndexWriterUnicode`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestIndexWriterUnicode",
        vec![
            test("testRandomUnicodeStrings", test_random_unicode_strings),
            test("testAllUnicodeChars", test_all_unicode_chars),
            test("testEmbeddedFFFF", test_embedded_ffff),
            test("testInvalidUTF16", test_invalid_utf16),
            test("testTermUTF16SortOrder", test_term_utf16_sort_order),
        ],
    )
}

/// `utf8Data`, as UTF-16 code units.
const UTF8_DATA: &[&[u16]] = &[
    // unpaired low surrogate
    &[0x61, 0x62, 0xdc17, 0x63, 0x64],
    &[0x61, 0x62, 0xfffd, 0x63, 0x64],
    &[0xdc17, 0x61, 0x62, 0x63, 0x64],
    &[0xfffd, 0x61, 0x62, 0x63, 0x64],
    &[0xdc17],
    &[0xfffd],
    &[0x61, 0x62, 0xdc17, 0xdc17, 0x63, 0x64],
    &[0x61, 0x62, 0xfffd, 0xfffd, 0x63, 0x64],
    &[0xdc17, 0xdc17, 0x61, 0x62, 0x63, 0x64],
    &[0xfffd, 0xfffd, 0x61, 0x62, 0x63, 0x64],
    &[0xdc17, 0xdc17],
    &[0xfffd, 0xfffd],
    // unpaired high surrogate
    &[0x61, 0x62, 0xd917, 0x63, 0x64],
    &[0x61, 0x62, 0xfffd, 0x63, 0x64],
    &[0xd917, 0x61, 0x62, 0x63, 0x64],
    &[0xfffd, 0x61, 0x62, 0x63, 0x64],
    &[0xd917],
    &[0xfffd],
    &[0x61, 0x62, 0xd917, 0xd917, 0x63, 0x64],
    &[0x61, 0x62, 0xfffd, 0xfffd, 0x63, 0x64],
    &[0xd917, 0xd917, 0x61, 0x62, 0x63, 0x64],
    &[0xfffd, 0xfffd, 0x61, 0x62, 0x63, 0x64],
    &[0xd917, 0xd917],
    &[0xfffd, 0xfffd],
    // backwards surrogates
    &[0x61, 0x62, 0xdc17, 0xd917, 0x63, 0x64],
    &[0x61, 0x62, 0xfffd, 0xfffd, 0x63, 0x64],
    &[0xdc17, 0xd917, 0x61, 0x62, 0x63, 0x64],
    &[0xfffd, 0xfffd, 0x61, 0x62, 0x63, 0x64],
    &[0xdc17, 0xd917],
    &[0xfffd, 0xfffd],
    &[0x61, 0x62, 0xdc17, 0xd917, 0xdc17, 0xd917, 0x63, 0x64],
    &[0x61, 0x62, 0xfffd, 0xd917, 0xdc17, 0xfffd, 0x63, 0x64],
    &[0xdc17, 0xd917, 0xdc17, 0xd917, 0x61, 0x62, 0x63, 0x64],
    &[0xfffd, 0xd917, 0xdc17, 0xfffd, 0x61, 0x62, 0x63, 0x64],
    &[0xdc17, 0xd917, 0xdc17, 0xd917],
    &[0xfffd, 0xd917, 0xdc17, 0xfffd],
];

/// `utf8Data[i]` as a field value (see the module's divergence).
fn utf8_data(i: usize) -> String {
    String::from_utf16_lossy(UTF8_DATA[i])
}

/// The private `nextInt(int lim)`.
fn next_int(lim: i32) -> i32 {
    random().next_int_bound(lim)
}

/// The private `nextInt(int start, int end)`.
fn next_int_range(start: i32, end: i32) -> i32 {
    start + next_int(end - start)
}

/// The private `fillUnicode(char[], char[], int, int)`.
fn fill_unicode(buffer: &mut [u16], expected: &mut [u16], mut offset: usize, count: usize) -> bool {
    let len = offset + count;
    let mut has_illegal = false;

    if offset > 0 && buffer[offset] >= 0xdc00 && buffer[offset] < 0xe000 {
        // Don't start in the middle of a valid surrogate pair
        offset -= 1;
    }

    let mut i = offset;
    while i < len {
        let t = next_int(6);
        if 0 == t && i < len - 1 {
            // Make a surrogate pair
            // High surrogate
            buffer[i] = next_int_range(0xd800, 0xdc00) as u16;
            expected[i] = buffer[i];
            i += 1;
            // Low surrogate
            buffer[i] = next_int_range(0xdc00, 0xe000) as u16;
            expected[i] = buffer[i];
        } else if t <= 1 {
            buffer[i] = next_int(0x80) as u16;
            expected[i] = buffer[i];
        } else if 2 == t {
            buffer[i] = next_int_range(0x80, 0x800) as u16;
            expected[i] = buffer[i];
        } else if 3 == t {
            buffer[i] = next_int_range(0x800, 0xd800) as u16;
            expected[i] = buffer[i];
        } else if 4 == t {
            buffer[i] = next_int_range(0xe000, 0xffff) as u16;
            expected[i] = buffer[i];
        } else if 5 == t && i < len - 1 {
            // Illegal unpaired surrogate
            if next_int(10) == 7 {
                if random().next_boolean() {
                    buffer[i] = next_int_range(0xd800, 0xdc00) as u16;
                } else {
                    buffer[i] = next_int_range(0xdc00, 0xe000) as u16;
                }
                expected[i] = 0xfffd;
                i += 1;
                buffer[i] = next_int_range(0x800, 0xd800) as u16;
                expected[i] = buffer[i];
                has_illegal = true;
            } else {
                buffer[i] = next_int_range(0x800, 0xd800) as u16;
                expected[i] = buffer[i];
            }
        } else {
            buffer[i] = b' ' as u16;
            expected[i] = buffer[i];
        }
        i += 1;
    }

    has_illegal
}

/// both start & end are inclusive
///
/// The private `getInt(Random, int, int)`.
fn get_int(r: &dyn Random, start: i32, end: i32) -> i32 {
    start + r.next_int_bound(1 + end - start)
}

/// The private `asUnicodeChar(char)`.
fn as_unicode_char(c: u16) -> String {
    format!("U+{c:x}")
}

/// The private `termDesc(String)`.
fn term_desc(s: &str) -> String {
    let units: Vec<u16> = s.encode_utf16().collect();
    Assert::assert_true(units.len() <= 2);
    if units.len() == 1 {
        as_unicode_char(units[0])
    } else {
        format!(
            "{},{}",
            as_unicode_char(units[0]),
            as_unicode_char(units[1])
        )
    }
}

/// `new BytesRef(CharSequence text)` over UTF-16 units: `this(new
/// byte[UnicodeUtil.maxUTF8Length(text.length())]); length =
/// UnicodeUtil.UTF16toUTF8(text, 0, text.length(), bytes);`.
fn bytes_ref_of_utf16(text: &[u16]) -> BytesRef {
    let mut bytes = vec![0u8; UnicodeUtil::max_utf8_length(text.len())];
    let length = UnicodeUtil::utf16_to_utf8(text, 0, text.len(), &mut bytes);
    let mut r = BytesRef::new(bytes);
    r.length = length;
    r
}

/// The private `checkTermsOrder(IndexReader, Set<String>, boolean)`.
fn check_terms_order(
    r: &Arc<dyn IndexReader>,
    all_terms: &HashSet<String>,
    is_top: bool,
) -> Result<()> {
    let mut terms = MultiTerms::get_terms(r, "f")?
        .unwrap_or_else(|| crate::support::npe("terms"))
        .iterator()?;

    let mut last = BytesRefBuilder::new();

    let mut seen_terms: HashSet<String> = HashSet::new();

    loop {
        let Some(term) = terms.next()? else {
            break;
        };

        Assert::assert_true(last.get() < term);
        last.copy_bytes(&term.bytes[term.offset..], 0, term.length);

        let s = term.utf8_to_string()?;
        Assert::assert_true_msg(
            &format!(
                "term {} was not added to index (count={})",
                term_desc(&s),
                all_terms.len()
            ),
            all_terms.contains(&s),
        );
        seen_terms.insert(s);
    }

    if is_top {
        Assert::assert_true(*all_terms == seen_terms);
    }

    // Test seeking:
    for s in &seen_terms {
        let tr = BytesRef::from_char_sequence(s);
        assert_equals_msg(
            Some(&format!(
                "seek failed for term={}",
                term_desc(&tr.utf8_to_string()?)
            )),
            &SeekStatus::FOUND,
            &terms.seek_ceil(&tr)?,
        );
    }
    Ok(())
}

/// LUCENE-510
///
/// Port of `TestIndexWriterUnicode.testRandomUnicodeStrings`.
fn test_random_unicode_strings() -> Result<()> {
    let mut buffer = [0u16; 20];
    let mut expected = [0u16; 20];

    let mut utf16 = CharsRefBuilder::new();

    let num = LuceneTestCase::at_least(10000);
    for _ in 0..num {
        let has_illegal = fill_unicode(&mut buffer, &mut expected, 0, 20);

        let utf8 = bytes_ref_of_utf16(&buffer[0..20]);
        if !has_illegal {
            let b = String::from_utf16_lossy(&buffer[0..20]).into_bytes();
            assert_equals(&b.len(), &utf8.length);
            for (i, byte) in b.iter().enumerate() {
                assert_equals(byte, &utf8.bytes[i]);
            }
        }

        utf16.copy_utf8_bytes(&utf8.bytes, 0, utf8.length);
        assert_equals(&utf16.length(), &20);
        for (i, e) in expected.iter().enumerate() {
            assert_equals(e, &utf16.char_at(i));
        }
    }
    Ok(())
}

/// LUCENE-510
///
/// Port of `TestIndexWriterUnicode.testAllUnicodeChars`.
fn test_all_unicode_chars() -> Result<()> {
    let mut utf16 = CharsRefBuilder::new();
    let mut chars = [0u16; 2];

    let mut ch: u32 = 0;
    while ch < 0x0010FFFF {
        if ch == 0xd800 {
            // Skip invalid code points
            ch = 0xe000;
        }

        let mut len = 0;
        if ch <= 0xffff {
            chars[len] = ch as u16;
            len += 1;
        } else {
            chars[len] = (((ch - 0x0010000) >> 10) + UNI_SUR_HIGH_START) as u16;
            len += 1;
            chars[len] = (((ch - 0x0010000) & 0x3FF) + UNI_SUR_LOW_START) as u16;
            len += 1;
        }

        let utf8 = bytes_ref_of_utf16(&chars[0..len]);

        let s1 = String::from_utf16_lossy(&chars[0..len]);
        let s2 = String::from_utf8_lossy(&utf8.bytes[0..utf8.length]).into_owned();
        assert_equals_msg(Some(&format!("codepoint {ch}")), &s1, &s2);

        utf16.copy_utf8_bytes(&utf8.bytes, 0, utf8.length);
        assert_equals_msg(Some(&format!("codepoint {ch}")), &s1, &utf16.to_string());

        let b = s1.as_bytes();
        assert_equals(&utf8.length, &b.len());
        for j in 0..utf8.length {
            assert_equals(&utf8.bytes[j], &b[j]);
        }
        ch += 1;
    }
    Ok(())
}

/// Port of `TestIndexWriterUnicode.testEmbeddedFFFF`.
fn test_embedded_ffff() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        d.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*random(),
        )))),
    )?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "a a\u{ffff}b",
        Store::NO,
    )));
    w.add_document(&doc)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "a",
        Store::NO,
    )));
    w.add_document(&doc)?;
    let r = directory_reader::open_with_writer(&*w)?;
    assert_equals(&1, &r.doc_freq(&Term::from_text("field", "a\u{ffff}b"))?);
    r.close()?;
    w.close()?;
    drop(w);
    d.close()
}

/// LUCENE-510
///
/// Port of `TestIndexWriterUnicode.testInvalidUTF16`.
fn test_invalid_utf16() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(
            Arc::new(StringSplitAnalyzer::new()),
        )),
    )?;
    let mut doc = Document::new();

    let count = UTF8_DATA.len() / 2;
    for i in 0..count {
        doc.add(Box::new(LuceneTestCase::new_text_field(
            &format!("f{i}"),
            &utf8_data(2 * i),
            Store::YES,
        )));
    }
    w.add_document(&doc)?;
    w.close()?;

    let ir = directory_reader::open(dir.as_directory())?;
    let doc2 = ir.stored_fields()?.document(0)?;
    for i in 0..count {
        assert_equals_msg(
            Some(&format!("field {i} was not indexed correctly")),
            &1,
            &ir.doc_freq(&Term::from_text(format!("f{i}"), &utf8_data(2 * i + 1)))?,
        );
        assert_equals_msg(
            Some(&format!("field {i} is incorrect")),
            &Some(utf8_data(2 * i + 1)),
            &doc2
                .get_field(&format!("f{i}"))
                .unwrap_or_else(|| crate::support::npe("field"))
                .string_value(),
        );
    }
    ir.close()?;
    drop(w);
    dir.close()
}

/// Make sure terms, including ones with surrogate pairs,
/// sort in codepoint sort order by default
///
/// Port of `TestIndexWriterUnicode.testTermUTF16SortOrder`.
fn test_term_utf16_sort_order() -> Result<()> {
    let rnd = random();
    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*rnd, dir.as_directory())?;
    let mut d = Document::new();
    // Single segment
    d.add(Box::new(LuceneTestCase::new_string_field(
        "f",
        "",
        Store::NO,
    )));
    let mut chars = [0u16; 2];
    let mut all_terms: HashSet<String> = HashSet::new();

    let num = LuceneTestCase::at_least(200);
    for i in 0..num {
        #[allow(clippy::needless_late_init)] // Java's definitely-assigned local, set in each branch
        let s;
        if rnd.next_boolean() {
            // Single char
            if rnd.next_boolean() {
                // Above surrogates
                chars[0] = get_int(&*rnd, 1 + UNI_SUR_LOW_END as i32, 0xffff) as u16;
            } else {
                // Below surrogates
                chars[0] = get_int(&*rnd, 0, UNI_SUR_HIGH_START as i32 - 1) as u16;
            }
            s = String::from_utf16_lossy(&chars[0..1]);
        } else {
            // Surrogate pair
            chars[0] = get_int(&*rnd, UNI_SUR_HIGH_START as i32, UNI_SUR_HIGH_END as i32) as u16;
            Assert::assert_true(
                u32::from(chars[0]) >= UNI_SUR_HIGH_START
                    && u32::from(chars[0]) <= UNI_SUR_HIGH_END,
            );
            chars[1] = get_int(&*rnd, UNI_SUR_LOW_START as i32, UNI_SUR_LOW_END as i32) as u16;
            s = String::from_utf16_lossy(&chars[0..2]);
        }
        all_terms.insert(s.clone());
        field_mut(&mut d, "f").set_string_value(s)?;

        writer.add_document(&d)?;

        if (1 + i) % 42 == 0 {
            writer.commit()?;
        }
    }

    let r = writer.get_reader()?;
    let ir = Arc::clone(&r) as Arc<dyn IndexReader>;

    // Test each sub-segment
    for ctx in Arc::clone(&ir).leaves()? {
        check_terms_order(&ctx.leaf_reader().as_index_reader(), &all_terms, false)?;
    }
    check_terms_order(&ir, &all_terms, true)?;

    // Test multi segment
    r.close()?;

    writer.force_merge(1)?;

    // Test single segment
    let r = writer.get_reader()?;
    check_terms_order(&(Arc::clone(&r) as Arc<dyn IndexReader>), &all_terms, true)?;
    r.close()?;

    writer.close()?;
    drop(writer);
    dir.close()
}
