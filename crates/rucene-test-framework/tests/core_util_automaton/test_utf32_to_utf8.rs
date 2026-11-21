//! Port of `lucene/core/src/test/org/apache/lucene/util/automaton/TestUTF32ToUTF8.java`.

use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::automaton::{
    Automata, Automaton, ByteRunAutomaton, ByteRunnable, CharacterRunAutomaton, Operations, RegExp,
    UTF32ToUTF8, DEFAULT_DETERMINIZE_WORK_LIMIT,
};
use rucene::util::fst::Util;
use rucene::util::unicode_util::{
    UNI_SUR_HIGH_END, UNI_SUR_HIGH_START, UNI_SUR_LOW_END, UNI_SUR_LOW_START,
};
use rucene::util::{BytesRef, IntsRefBuilder, UnicodeUtil};
use rucene_test_framework::util::automaton::automaton_test_util::{
    AutomatonTestUtil, RandomAcceptedStrings,
};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, det, random, test, try_catch, JavaHashSet};
use crate::test_operations;

/// `TestUTF32ToUTF8`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        // `setUp()` only calls `super.setUp()`.
        test("testRandomRanges", test_random_ranges),
        test("testSpecialCase", test_special_case),
        test("testSpecialCase2", test_special_case2),
        test("testSpecialCase3", test_special_case3),
        test("testRandomRegexes", test_random_regexes),
        test("testSingleton", test_singleton),
    ];
    class("org.apache.lucene.util.automaton.TestUTF32ToUTF8", methods)
}

const MAX_UNICODE: i32 = 0x10FFFF;
const SUR_HIGH_START: i32 = UNI_SUR_HIGH_START as i32;
const SUR_HIGH_END: i32 = UNI_SUR_HIGH_END as i32;
const SUR_LOW_START: i32 = UNI_SUR_LOW_START as i32;
const SUR_LOW_END: i32 = UNI_SUR_LOW_END as i32;

/// `matches(a, code)`.
fn matches(a: &ByteRunAutomaton, code: i32) -> bool {
    let mut units = [0u16; 2];
    let chars = char::from_u32(code as u32)
        .expect("a non-surrogate code point")
        .encode_utf16(&mut units);
    let mut b = vec![0u8; UnicodeUtil::max_utf8_length(chars.len())];
    let len = UnicodeUtil::utf16_to_utf8(chars, 0, chars.len(), &mut b);
    a.run_range(&b, 0, len)
}

/// `testOne(r, a, startCode, endCode, iters)`.
fn test_one(r: &dyn Random, a: &ByteRunAutomaton, start_code: i32, end_code: i32, iters: i32) {
    // Verify correct ints are accepted
    let non_surrogate_count;
    let ov_sur_start;
    if end_code < SUR_HIGH_START || start_code > SUR_LOW_END {
        // no overlap w/ surrogates
        non_surrogate_count = end_code - start_code + 1;
        ov_sur_start = false;
    } else if is_surrogate(start_code) {
        // start of range overlaps surrogates
        non_surrogate_count = end_code - start_code + 1 - (SUR_LOW_END - start_code + 1);
        ov_sur_start = false;
    } else if is_surrogate(end_code) {
        // end of range overlaps surrogates
        ov_sur_start = true;
        non_surrogate_count = end_code - start_code + 1 - (end_code - SUR_HIGH_START + 1);
    } else {
        // range completely subsumes surrogates
        ov_sur_start = true;
        non_surrogate_count = end_code - start_code + 1 - (SUR_LOW_END - SUR_HIGH_START + 1);
    }

    assert!(non_surrogate_count > 0);

    for _ in 0..iters {
        // pick random code point in-range

        let mut code = start_code + r.next_int_bound(non_surrogate_count);
        if is_surrogate(code) {
            if ov_sur_start {
                code = SUR_LOW_END + 1 + (code - SUR_HIGH_START);
            } else {
                code = SUR_LOW_END + 1 + (code - start_code);
            }
        }

        assert!(
            code >= start_code && code <= end_code,
            "code={code} start={start_code} end={end_code}"
        );
        assert!(!is_surrogate(code));

        Assert::assert_true_msg(
            &format!("DFA for range {start_code}-{end_code} failed to match code={code}"),
            matches(a, code),
        );
    }

    // Verify invalid ints are not accepted
    let invalid_range = MAX_UNICODE - (end_code - start_code + 1);
    if invalid_range > 0 {
        let mut iter = 0;
        while iter < iters {
            let x = TestUtil::next_int(r, 0, invalid_range - 1);
            let code = if x >= start_code {
                end_code + 1 + x - start_code
            } else {
                x
            };
            if (SUR_HIGH_START..=SUR_HIGH_END).contains(&code)
                || (SUR_LOW_START..=SUR_LOW_END).contains(&code)
            {
                // iter--; continue;
                continue;
            }
            Assert::assert_false_msg(
                &format!("DFA for range {start_code}-{end_code} matched invalid code={code}"),
                matches(a, code),
            );
            iter += 1;
        }
    }
}

/// `getCodeStart(r)`: evenly picks random code point from the 4 "buckets"
/// (bucket = same #bytes when encoded to utf8).
fn get_code_start(r: &dyn Random) -> i32 {
    match r.next_int_bound(4) {
        0 => TestUtil::next_int(r, 0, 128),
        1 => TestUtil::next_int(r, 128, 2048),
        2 => TestUtil::next_int(r, 2048, 65536),
        _ => TestUtil::next_int(r, 65536, 1 + MAX_UNICODE),
    }
}

fn is_surrogate(code: i32) -> bool {
    (SUR_HIGH_START..=SUR_LOW_END).contains(&code)
}

fn test_random_ranges() -> Result<()> {
    let r = random();
    let iters = LuceneTestCase::at_least(10);
    let iters_per_dfa = LuceneTestCase::at_least(100);
    let mut iter = 0;
    while iter < iters {
        let x1 = get_code_start(&*r);
        let x2 = get_code_start(&*r);
        let (start_code, end_code) = if x1 < x2 { (x1, x2) } else { (x2, x1) };

        if is_surrogate(start_code) && is_surrogate(end_code) {
            // iter--; continue;
            continue;
        }

        let a = Automata::make_char_range(start_code, end_code);
        test_one(
            &*r,
            &ByteRunAutomaton::from_utf32(a)?,
            start_code,
            end_code,
            iters_per_dfa,
        );
        iter += 1;
    }
    Ok(())
}

fn test_special_case() -> Result<()> {
    let re = RegExp::new(".?")?;
    let automaton = re.to_automaton()?;
    let cra = CharacterRunAutomaton::new(automaton.clone())?;
    let bra = ByteRunAutomaton::from_utf32(automaton)?;
    // make sure character dfa accepts empty string
    Assert::assert_true(cra.is_accept(0));
    Assert::assert_true(cra.run(""));
    Assert::assert_true(cra.run_chars(&[], 0, 0));

    // make sure byte dfa accepts empty string
    Assert::assert_true(ByteRunnable::is_accept(&bra, 0));
    Assert::assert_true(bra.run_range(&[], 0, 0));
    Ok(())
}

/// The UTF-16 `input` of `testSpecialCase2`/`testSpecialCase3`, whose
/// surrogates all pair up.
fn from_utf16(units: &[u16]) -> String {
    String::from_utf16(units).expect("valid UTF-16")
}

fn test_special_case2() -> Result<()> {
    let re = RegExp::new(".+\u{0775}")?;
    let input = from_utf16(&[
        0xfadc, 0xfffd, 0xb80b, 0xda5a, 0xdc68, 0xf234, 0x0056, 0xda5b, 0xdcc1, 0xfffd, 0xfffd,
        0x0775,
    ]);
    let automaton = re.to_automaton()?;
    let automaton = det(Operations::determinize(
        &automaton,
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    ));
    let cra = CharacterRunAutomaton::new(automaton.clone())?;
    let bra = ByteRunAutomaton::from_utf32(automaton)?;

    Assert::assert_true(cra.run(&input));

    let bytes = input.as_bytes();
    Assert::assert_true(bra.run_range(bytes, 0, bytes.len())); // this one fails!
    Ok(())
}

fn test_special_case3() -> Result<()> {
    let re = RegExp::new("(\\鯺)*(.)*\\Ӕ")?;
    let input = from_utf16(&[
        0x5cfd, 0xfffd, 0xb2f7, 0x0033, 0xe304, 0x51d7, 0x3692, 0xdb50, 0xdfb3, 0x0576, 0xdae2,
        0xdc62, 0x0053, 0x0449, 0x04d4,
    ]);
    let automaton = re.to_automaton()?;
    let automaton = det(Operations::determinize(
        &automaton,
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    ));
    let cra = CharacterRunAutomaton::new(automaton.clone())?;
    let bra = ByteRunAutomaton::from_utf32(automaton)?;

    Assert::assert_true(cra.run(&input));
    let bytes = input.as_bytes();
    Assert::assert_true(bra.run_range(bytes, 0, bytes.len()));
    Ok(())
}

fn test_random_regexes() -> Result<()> {
    let num = LuceneTestCase::at_least(50);
    for _ in 0..num {
        let automaton =
            RegExp::with_flags(&AutomatonTestUtil::random_regexp(&*random()), RegExp::NONE)?
                .to_automaton()?;
        let automaton = det(Operations::determinize(
            &automaton,
            DEFAULT_DETERMINIZE_WORK_LIMIT,
        ));
        assert_automaton(&automaton)?;
    }
    Ok(())
}

fn test_singleton() -> Result<()> {
    let iters = LuceneTestCase::at_least(100);
    for _ in 0..iters {
        let s = TestUtil::random_realistic_unicode_string(&*random());
        let a = Automata::make_string(&s);
        let utf8 = UTF32ToUTF8::new().convert(&a);
        let mut ints = IntsRefBuilder::new();
        Util::to_ints_ref(&BytesRef::from_char_sequence(&s), &mut ints);
        let mut set = JavaHashSet::new();
        set.add(ints.get());
        assert_equals(&set, &test_operations::get_finite_strings(&utf8));
    }
    Ok(())
}

/// `assertAutomaton(automaton)`.
fn assert_automaton(automaton: &Automaton) -> Result<()> {
    let cra = CharacterRunAutomaton::new(automaton.clone())?;
    let bra = ByteRunAutomaton::from_utf32(automaton.clone())?;
    let ras = RandomAcceptedStrings::new(automaton);

    let num = LuceneTestCase::at_least(1000);
    for _ in 0..num {
        let string = if random().next_boolean() {
            // likely not accepted
            TestUtil::random_unicode_string(&*random())
        } else {
            // will be accepted
            let codepoints = ras.get_random_accepted_string(&*random());
            let converted = try_catch("java.lang.Exception", || {
                crate::core_support::unchecked(UnicodeUtil::new_string(
                    &codepoints,
                    0,
                    codepoints.len(),
                ))
            });
            match converted {
                Ok(s) => s,
                Err(e) => {
                    println!("{} codepoints:", codepoints.len());
                    for cp in &codepoints {
                        println!("  {cp:x}");
                    }
                    e.throw()
                }
            }
        };
        let bytes = string.as_bytes();
        assert_equals(&cra.run(&string), &bra.run_range(bytes, 0, bytes.len()));
    }
    Ok(())
}
