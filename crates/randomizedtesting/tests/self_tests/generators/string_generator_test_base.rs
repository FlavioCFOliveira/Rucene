//! Port of `com.carrotsearch.randomizedtesting.generators.StringGeneratorTestBase`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): the base of the
//! `StringGenerator` tests.

use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::Repeat;
use randomizedtesting::generators::StringGenerator;
use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use randomizedtesting::junit::Test;
use randomizedtesting::RandomizedTest;

use super::cls;
use crate::support::{
    assert_equals, assert_equals_msg, assert_true_msg, get_random, RANDOMIZED_TEST,
};

/// The instance: `protected final StringGenerator generator` and the
/// subclass's `iterationFix` override.
pub struct Inst {
    /// `generator`.
    pub generator: Arc<dyn StringGenerator>,
    /// `iterationFix(i)`.
    pub iteration_fix: fn(i32) -> i32,
}

/// `iterationFix(i)`: `i` (the base implementation).
pub fn identity(i: i32) -> i32 {
    i
}

/// A subclass constructor `super(generator)`.
pub fn constructor(
    make: fn() -> Arc<dyn StringGenerator>,
    iteration_fix: fn(i32) -> i32,
) -> Constructor {
    Constructor::no_args(move || Inst {
        generator: make(),
        iteration_fix,
    })
}

fn code_points(s: &str) -> i32 {
    s.chars().count() as i32
}

fn code_units(s: &str) -> i32 {
    s.encode_utf16().count() as i32
}

const TEN: Repeat = Repeat {
    iterations: 10,
    use_constant_seed: false,
};

fn repeated(name: &str, body: fn(&mut Inst)) -> Method {
    Method::new(name)
        .annotate(Test::new())
        .annotate(TEN)
        .body(body)
}

/// `public abstract class StringGeneratorTestBase extends RandomizedTest`.
pub static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("StringGeneratorTestBase")
        .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
        .extends(&RANDOMIZED_TEST)
        .constructor(Constructor::no_args(|| ()).modifiers(modifier::PROTECTED))
        .method(repeated("checkFixedCodePointLength", |t| {
            let codepoints = (t.iteration_fix)(RandomizedTest::random_int_between(1, 100));
            let s = t
                .generator
                .of_code_points_length(&*get_random(), codepoints, codepoints);
            assert_equals_msg(&s, codepoints, code_points(&s));
        }))
        .method(repeated("checkRandomCodePointLength", |t| {
            let from = (t.iteration_fix)(RandomizedTest::random_int_between(1, 100));
            let to = from + RandomizedTest::random_int_between(0, 100);
            let s = t.generator.of_code_points_length(&*get_random(), from, to);
            let codepoints = code_points(&s);
            assert_true_msg(
                &format!("{codepoints} not within {from}-{to}"),
                from <= codepoints && codepoints <= to,
            );
        }))
        .method(repeated("checkFixedCodeUnitLength", |t| {
            let codeunits = (t.iteration_fix)(RandomizedTest::random_int_between(1, 100));
            let s = t
                .generator
                .of_code_units_length(&*get_random(), codeunits, codeunits);
            assert_equals_msg(&s, codeunits, code_units(&s));
            assert_equals_msg(&s, codeunits, code_units(&s));
        }))
        .method(repeated("checkRandomCodeUnitLength", |t| {
            let from = (t.iteration_fix)(RandomizedTest::random_int_between(1, 100));
            let to = from + RandomizedTest::random_int_between(0, 100);
            let s = t.generator.of_code_units_length(&*get_random(), from, to);
            let codeunits = code_units(&s);
            assert_true_msg(
                &format!("{codeunits} not within {from}-{to}"),
                from <= codeunits && codeunits <= to,
            );
        }))
        .method(
            Method::new("checkZeroLength")
                .annotate(Test::new())
                .body(|t: &mut Inst| {
                    assert_equals(
                        String::new(),
                        t.generator.of_code_points_length(&*get_random(), 0, 0),
                    );
                    assert_equals(
                        String::new(),
                        t.generator.of_code_units_length(&*get_random(), 0, 0),
                    );
                }),
        )
        .build()
});
