//! Port of `com.carrotsearch.randomizedtesting.generators.TestCodepointSetGenerator`
//! (randomized-runner 2.8.1 test sources, Apache-2.0):
//! `@RunWith(Suite.class)` over three `StringGeneratorTestBase` suites.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use randomizedtesting::generators::{CodepointSetGenerator, StringGenerator};
use randomizedtesting::jdk::exception::ILLEGAL_ARGUMENT_EXCEPTION;
use randomizedtesting::jdk::random::JdkRandom;
use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Test;
use randomizedtesting::RandomizedTest;

use super::string_generator_test_base::{constructor, identity, Inst, CLASS as BASE};
use crate::support::{assert_true, get_random, nested, test};

const O: &str = "generators.TestCodepointSetGenerator";

/// `codepoints`.
const CODEPOINTS: [u32; 9] = [
    'a' as u32, 'b' as u32, 'c' as u32, 'd' as u32, 0xd7ff, 0xffff, 0x10000, 0x1D11E, 0x10FFFD,
];
/// `surrogates`.
const SURROGATES: [u32; 3] = [0x10000, 0x1D11E, 0x10FFFD];

fn string_of(cps: &[u32]) -> String {
    cps.iter().filter_map(|c| char::from_u32(*c)).collect()
}

/// `withSurrogates`.
fn with_surrogates() -> String {
    string_of(&CODEPOINTS)
}

/// `testAllCharactersUsed` over a generator built from `"abcdefABCDEF"`.
fn all_characters_used(gen: CodepointSetGenerator) {
    let domain: Vec<u16> = "abcdefABCDEF".encode_utf16().collect();
    let mut chars: HashSet<u16> = domain.iter().copied().collect();
    let r = JdkRandom::new(RandomizedTest::random_long());
    let mut i = 0;
    while i < 1_000_000 && !chars.is_empty() {
        for ch in gen.of_code_units_length(&r, 100, 100).encode_utf16() {
            chars.remove(&ch);
        }
        i += 1;
    }
    assert_true(chars.is_empty());
}

static CODEPOINT_SET_ON_CHARS: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "CodepointSetOnChars")
        .extends(&BASE)
        .constructor(constructor(
            || {
                Arc::new(CodepointSetGenerator::from_chars(&[
                    'a' as u16, 'b' as u16, 'c' as u16, 'd' as u16, 0x100, 0xd7ff, 0xffff,
                ]))
            },
            identity,
        ))
        .method(test("testAllCharactersUsed", || {
            let domain: Vec<u16> = "abcdefABCDEF".encode_utf16().collect();
            all_characters_used(CodepointSetGenerator::from_chars(&domain));
        }))
        .method(
            Method::new("testSurrogatesInConstructor")
                .annotate(Test::expected(ILLEGAL_ARGUMENT_EXCEPTION))
                .body_any(|| {
                    let units: Vec<u16> = with_surrogates().encode_utf16().collect();
                    CodepointSetGenerator::from_chars(&units);
                }),
        )
        .build()
});

static CODEPOINT_SET_ON_CODE_POINTS: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "CodepointSetOnCodePoints")
        .extends(&BASE)
        .constructor(constructor(
            || Arc::new(CodepointSetGenerator::from_string(&with_surrogates())),
            identity,
        ))
        .method(test("testAllCharactersUsed", || {
            all_characters_used(CodepointSetGenerator::from_string("abcdefABCDEF"));
        }))
        .build()
});

/// `iterationFix(i)`: even only.
fn even_only(i: i32) -> i32 {
    i & !1
}

static CODEPOINT_SET_ON_SURROGATES_ONLY: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "CodepointSetOnSurrogatesOnly")
        .extends(&BASE)
        .constructor(constructor(
            || Arc::new(CodepointSetGenerator::from_string(&string_of(&SURROGATES))),
            even_only,
        ))
        .method(
            Method::new("testOddCodePoints")
                .annotate(Test::expected(ILLEGAL_ARGUMENT_EXCEPTION))
                .body(|t: &mut Inst| {
                    t.generator.of_code_units_length(&*get_random(), 3, 3);
                }),
        )
        .build()
});

/// The `@SuiteClasses` of `TestCodepointSetGenerator`, in order.
pub fn classes() -> Vec<Arc<Class>> {
    vec![
        CODEPOINT_SET_ON_CHARS.clone(),
        CODEPOINT_SET_ON_CODE_POINTS.clone(),
        CODEPOINT_SET_ON_SURROGATES_ONLY.clone(),
    ]
}
