//! `generators/` against randomizedtesting 2.8.1: 20 seeds × 500 calls of
//! every generator and parameter (`generators.tsv`) and their errors
//! (`generator_errors.tsv`).

#![allow(deprecated)]

use std::collections::BTreeMap;

use super::{assert_same_lines, catch, java_error, rows, seeds, Rec};
use crate::generators as statics;
use crate::generators::{
    AsciiGenerator, BiasedNumbers, CodepointSetGenerator, RandomBytes, RandomNumbers, RandomPicks,
    RandomStrings, StringGenerator,
};
use crate::jdk::random::Random;
use crate::Xoroshiro128PlusRandom;

const GEN_DRAWS: usize = 500;

type Draw = Box<dyn Fn(&dyn Random, &mut Rec)>;

fn hex_f64(v: f64) -> String {
    format!("{:x}", v.to_bits())
}

fn generator_methods() -> Vec<(String, Draw)> {
    let mut m: Vec<(String, Draw)> = Vec::new();
    let ints = [
        (0, 0),
        (0, 10),
        (-10, 10),
        (i32::MIN, i32::MAX),
        (i32::MIN, 0),
        (0, i32::MAX),
        (-1, i32::MAX),
        (i32::MIN + 1, i32::MAX),
        (5, 5),
        (i32::MAX - 1, i32::MAX),
        (-1, i32::MAX - 1),
    ];
    for (a, b) in ints {
        m.push((
            format!("randomIntBetween({a},{b})"),
            Box::new(move |r, rec| rec.i(RandomNumbers::random_int_between(r, a, b))),
        ));
    }
    let longs = [
        (0, 0),
        (0, 10),
        (-10, 10),
        (i64::MIN, i64::MAX),
        (i64::MIN, 0),
        (i64::MIN, 1),
        (-1, i64::MAX),
        (0, i64::MAX),
        (i64::MIN + 1, i64::MAX),
        (i64::MIN, -1),
        (-2, i64::MAX),
    ];
    for (a, b) in longs {
        m.push((
            format!("randomLongBetween({a},{b})"),
            Box::new(move |r, rec| rec.l(RandomNumbers::random_long_between(r, a, b))),
        ));
    }
    for n in [1, 2, 3, 1000, 1i64 << 40, i64::MAX] {
        m.push((
            format!("nextLong({n})"),
            Box::new(move |r, rec| rec.l(RandomNumbers::next_long(r, n))),
        ));
    }
    let inf = f64::INFINITY;
    let ds = [
        (0.0, 1.0),
        (-1.0, 1.0),
        (-inf, inf),
        (0.0, inf),
        (-inf, 0.0),
        (1.0, 1.0),
        (-f64::MAX, f64::MAX),
        (f64::from_bits(1), 1.0),
        (-5.0, -4.0),
        (0.0, 0.0),
        (1e-310, 1e-300),
        (-inf, -inf),
        (inf, inf),
        (-0.0, 0.0),
    ];
    for (a, b) in ds {
        let lbl = format!("{},{}", hex_f64(a), hex_f64(b));
        m.push((
            format!("randomDoubleBetween({lbl})"),
            Box::new(move |r, rec| rec.d(BiasedNumbers::random_double_between(r, a, b))),
        ));
        m.push((
            format!("randomFloatBetween({lbl})"),
            Box::new(move |r, rec| {
                rec.f(BiasedNumbers::random_float_between(r, a as f32, b as f32))
            }),
        ));
    }
    let fuzz = [
        (0.0, 1.0),
        (1.0, 1.0),
        (-1.0, 0.0),
        (0.0, f64::from_bits(1)),
        (f64::MAX, inf),
        (-inf, 0.0),
    ];
    for (a, b) in fuzz {
        let lbl = format!("{},{}", hex_f64(a), hex_f64(b));
        m.push((
            format!("fuzzUp({lbl})"),
            Box::new(move |r, rec| rec.d(BiasedNumbers::fuzz_up(r, a, b))),
        ));
        m.push((
            format!("fuzzDown({lbl})"),
            Box::new(move |r, rec| rec.d(BiasedNumbers::fuzz_down(r, b, a))),
        ));
        m.push((
            format!("fuzzUpF({lbl})"),
            Box::new(move |r, rec| rec.f(BiasedNumbers::fuzz_up_f32(r, a as f32, b as f32))),
        ));
        m.push((
            format!("fuzzDownF({lbl})"),
            Box::new(move |r, rec| rec.f(BiasedNumbers::fuzz_down_f32(r, b as f32, a as f32))),
        ));
    }
    for n in [0, 1, 5, 100] {
        m.push((
            format!("randomBytesOfLength({n})"),
            Box::new(move |r, rec| rec.bytes(&RandomBytes::random_bytes_of_length(r, n))),
        ));
    }
    for (a, b) in [(0, 10), (3, 3), (0, 1000)] {
        m.push((
            format!("randomBytesOfLengthBetween({a},{b})"),
            Box::new(move |r, rec| {
                rec.bytes(&RandomBytes::random_bytes_of_length_between(r, a, b))
            }),
        ));
    }
    for size in [1usize, 7, 100] {
        let objs: Vec<i32> = (0..size as i32).map(|k| k * 3).collect();
        let is: Vec<i32> = (0..size as i32).map(|k| k * 5).collect();
        let ls: Vec<i64> = (0..size as i64).map(|k| k * 7).collect();
        let bs: Vec<i8> = (0..size).map(|k| k as i8).collect();
        let ss: Vec<i16> = (0..size).map(|k| (k * 11) as i16).collect();
        let cs: Vec<u16> = (0..size).map(|k| b'a' as u16 + k as u16).collect();
        let fs: Vec<f32> = (0..size).map(|k| k as f32 / 2.0).collect();
        let dd: Vec<f64> = (0..size).map(|k| k as f64 / 4.0).collect();
        let (o1, o2, o3) = (objs.clone(), objs.clone(), objs);
        m.push((
            format!("randomFrom(Object[{size}])"),
            Box::new(move |r, rec| rec.i(*RandomPicks::random_from(r, &o1))),
        ));
        m.push((
            format!("randomFrom(List[{size}])"),
            Box::new(move |r, rec| rec.i(*RandomPicks::random_from_list(r, &o2))),
        ));
        m.push((
            format!("randomFrom(Collection[{size}])"),
            Box::new(move |r, rec| rec.i(*RandomPicks::random_from_collection(r, o3.iter()))),
        ));
        m.push((
            format!("randomFrom(int[{size}])"),
            Box::new(move |r, rec| rec.i(*RandomPicks::random_from(r, &is))),
        ));
        m.push((
            format!("randomFrom(long[{size}])"),
            Box::new(move |r, rec| rec.l(*RandomPicks::random_from(r, &ls))),
        ));
        m.push((
            format!("randomFrom(byte[{size}])"),
            Box::new(move |r, rec| rec.i(i32::from(*RandomPicks::random_from(r, &bs)))),
        ));
        m.push((
            format!("randomFrom(short[{size}])"),
            Box::new(move |r, rec| rec.i(i32::from(*RandomPicks::random_from(r, &ss)))),
        ));
        m.push((
            format!("randomFrom(char[{size}])"),
            Box::new(move |r, rec| rec.i(i32::from(*RandomPicks::random_from(r, &cs)))),
        ));
        m.push((
            format!("randomFrom(float[{size}])"),
            Box::new(move |r, rec| rec.f(*RandomPicks::random_from(r, &fs))),
        ));
        m.push((
            format!("randomFrom(double[{size}])"),
            Box::new(move |r, rec| rec.d(*RandomPicks::random_from(r, &dd))),
        ));
    }
    for (a, b) in [(0, 0), (0, 10), (5, 5), (1, 20), (0, 100)] {
        let l = format!("{a},{b}");
        m.push((
            format!("randomAsciiOfLengthBetween({l})"),
            Box::new(move |r, rec| rec.s(&RandomStrings::random_ascii_of_length_between(r, a, b))),
        ));
        m.push((
            format!("randomAsciiLettersOfLengthBetween({l})"),
            Box::new(move |r, rec| {
                rec.s(&RandomStrings::random_ascii_letters_of_length_between(
                    r, a, b,
                ))
            }),
        ));
        m.push((
            format!("randomAsciiAlphanumOfLengthBetween({l})"),
            Box::new(move |r, rec| {
                rec.s(&RandomStrings::random_ascii_alphanum_of_length_between(
                    r, a, b,
                ))
            }),
        ));
        m.push((
            format!("randomUnicodeOfLengthBetween({l})"),
            Box::new(move |r, rec| {
                rec.s(&RandomStrings::random_unicode_of_length_between(r, a, b))
            }),
        ));
        m.push((
            format!("randomUnicodeOfCodepointLengthBetween({l})"),
            Box::new(move |r, rec| {
                rec.s(&RandomStrings::random_unicode_of_codepoint_length_between(
                    r, a, b,
                ))
            }),
        ));
        m.push((
            format!("randomRealisticUnicodeOfLengthBetween({l})"),
            Box::new(move |r, rec| {
                rec.s(&RandomStrings::random_realistic_unicode_of_length_between(
                    r, a, b,
                ))
            }),
        ));
        m.push((
            format!("randomRealisticUnicodeOfCodepointLengthBetween({l})"),
            Box::new(move |r, rec| {
                rec.s(&RandomStrings::random_realistic_unicode_of_codepoint_length_between(r, a, b))
            }),
        ));
        m.push((
            format!("ofUtf8Length({l})"),
            Box::new(move |r, rec| rec.s(&statics::UNICODE_GENERATOR.of_utf8_length(r, a, b))),
        ));
        m.push((
            format!("ofStringLength({l})"),
            Box::new(move |r, rec| {
                rec.s(&statics::REALISTIC_UNICODE_GENERATOR.of_string_length(r, a, b))
            }),
        ));
        m.push((
            format!("ASCIIGenerator({l})"),
            Box::new(move |r, rec| rec.s(&AsciiGenerator::new().of_code_units_length(r, a, b))),
        ));
        m.push((
            format!("asciiGenerator.ofCodePointsLength({l})"),
            Box::new(move |r, rec| rec.s(&statics::ASCII_GENERATOR.of_code_points_length(r, a, b))),
        ));
    }
    for n in [0, 1, 7, 30] {
        m.push((
            format!("randomAsciiOfLength({n})"),
            Box::new(move |r, rec| rec.s(&RandomStrings::random_ascii_of_length(r, n))),
        ));
        m.push((
            format!("randomAsciiLettersOfLength({n})"),
            Box::new(move |r, rec| rec.s(&RandomStrings::random_ascii_letters_of_length(r, n))),
        ));
        m.push((
            format!("randomAsciiAlphanumOfLength({n})"),
            Box::new(move |r, rec| rec.s(&RandomStrings::random_ascii_alphanum_of_length(r, n))),
        ));
        m.push((
            format!("randomUnicodeOfLength({n})"),
            Box::new(move |r, rec| rec.s(&RandomStrings::random_unicode_of_length(r, n))),
        ));
        m.push((
            format!("randomUnicodeOfCodepointLength({n})"),
            Box::new(move |r, rec| rec.s(&RandomStrings::random_unicode_of_codepoint_length(r, n))),
        ));
        m.push((
            format!("randomRealisticUnicodeOfLength({n})"),
            Box::new(move |r, rec| rec.s(&RandomStrings::random_realistic_unicode_of_length(r, n))),
        ));
        m.push((
            format!("randomRealisticUnicodeOfCodepointLength({n})"),
            Box::new(move |r, rec| {
                rec.s(&RandomStrings::random_realistic_unicode_of_codepoint_length(r, n))
            }),
        ));
    }
    let from_chars = std::sync::Arc::new(CodepointSetGenerator::from_chars(&[
        b'a' as u16,
        b'b' as u16,
        b'c' as u16,
        0xe9,
        0xffff,
    ]));
    let mixed = std::sync::Arc::new(CodepointSetGenerator::from_string(
        "ab\u{1F600}\u{1D11E}c\u{4e2d}",
    ));
    let supp_only = std::sync::Arc::new(CodepointSetGenerator::from_string("\u{1F600}\u{1D11E}"));
    for (a, b) in [(0, 0), (0, 10), (3, 3), (1, 20), (3, 4), (4, 4), (3, 9)] {
        let l = format!("{a},{b}");
        let (g1, g2) = (from_chars.clone(), from_chars.clone());
        m.push((
            format!("fromChars.units({l})"),
            Box::new(move |r, rec| rec.s(&g1.of_code_units_length(r, a, b))),
        ));
        m.push((
            format!("fromChars.points({l})"),
            Box::new(move |r, rec| rec.s(&g2.of_code_points_length(r, a, b))),
        ));
        let (g1, g2) = (mixed.clone(), mixed.clone());
        m.push((
            format!("mixed.units({l})"),
            Box::new(move |r, rec| rec.s(&g1.of_code_units_length(r, a, b))),
        ));
        m.push((
            format!("mixed.points({l})"),
            Box::new(move |r, rec| rec.s(&g2.of_code_points_length(r, a, b))),
        ));
        if a != b || a & 1 == 0 {
            let g = supp_only.clone();
            m.push((
                format!("suppOnly.units({l})"),
                Box::new(move |r, rec| rec.s(&g.of_code_units_length(r, a, b))),
            ));
        }
        let g = supp_only.clone();
        m.push((
            format!("suppOnly.points({l})"),
            Box::new(move |r, rec| rec.s(&g.of_code_points_length(r, a, b))),
        ));
    }
    m
}

#[test]
fn generators_equal_randomizedtesting() {
    let expected: BTreeMap<String, String> = rows("generators.tsv")
        .into_iter()
        .map(|r| (format!("{}\t{}", r[0], r[1]), format!("{}\t{}", r[2], r[3])))
        .collect();
    let methods = generator_methods();
    let mut actual = BTreeMap::new();
    for seed in seeds() {
        for (label, draw) in &methods {
            let r = Xoroshiro128PlusRandom::new(seed);
            let mut rec = Rec::default();
            for _ in 0..GEN_DRAWS {
                draw(&r, &mut rec);
            }
            actual.insert(format!("{seed:x}\t{label}"), rec.line());
        }
    }
    assert_same_lines("generators", &expected, &actual);
}

type Call = Box<dyn Fn(&dyn Random)>;

#[test]
fn generator_errors_equal_randomizedtesting() {
    let calls: Vec<(&str, Call)> = vec![
        (
            "nextLong(0)",
            Box::new(|r| {
                RandomNumbers::next_long(r, 0);
            }),
        ),
        (
            "nextLong(-5)",
            Box::new(|r| {
                RandomNumbers::next_long(r, -5);
            }),
        ),
        (
            "randomIntBetween(5,4)",
            Box::new(|r| {
                RandomNumbers::random_int_between(r, 5, 4);
            }),
        ),
        (
            "randomLongBetween(5,4)",
            Box::new(|r| {
                RandomNumbers::random_long_between(r, 5, 4);
            }),
        ),
        (
            "randomDoubleBetween(1,0.5)",
            Box::new(|r| {
                BiasedNumbers::random_double_between(r, 1.0, 0.5);
            }),
        ),
        (
            "randomDoubleBetween(0,NaN)",
            Box::new(|r| {
                BiasedNumbers::random_double_between(r, 0.0, f64::NAN);
            }),
        ),
        (
            "randomDoubleBetween(NaN,NaN)",
            Box::new(|r| {
                BiasedNumbers::random_double_between(r, f64::NAN, f64::NAN);
            }),
        ),
        (
            "randomDoubleBetween(1e-5,1e7)",
            Box::new(|r| {
                BiasedNumbers::random_double_between(r, 1e7, 1e-5);
            }),
        ),
        (
            "randomFloatBetween(1,0.5)",
            Box::new(|r| {
                BiasedNumbers::random_float_between(r, 1.0, 0.5);
            }),
        ),
        (
            "randomFloatBetween(3e10,1.25e-4)",
            Box::new(|r| {
                BiasedNumbers::random_float_between(r, 3e10, 1.25e-4);
            }),
        ),
        (
            "randomFloatBetween(0,NaN)",
            Box::new(|r| {
                BiasedNumbers::random_float_between(r, 0.0, f32::NAN);
            }),
        ),
        (
            "fuzzUp(2,1)",
            Box::new(|r| {
                BiasedNumbers::fuzz_up(r, 2.0, 1.0);
            }),
        ),
        (
            "fuzzDown(1,2)",
            Box::new(|r| {
                BiasedNumbers::fuzz_down(r, 1.0, 2.0);
            }),
        ),
        (
            "fuzzUpF(2,1)",
            Box::new(|r| {
                BiasedNumbers::fuzz_up_f32(r, 2.0, 1.0);
            }),
        ),
        (
            "fuzzDownF(1,2)",
            Box::new(|r| {
                BiasedNumbers::fuzz_down_f32(r, 1.0, 2.0);
            }),
        ),
        (
            "randomFrom(Object[0])",
            Box::new(|r| {
                RandomPicks::random_from::<_, i32>(r, &[]);
            }),
        ),
        (
            "randomFrom(int[0])",
            Box::new(|r| {
                RandomPicks::random_from::<_, i32>(r, &[]);
            }),
        ),
        (
            "randomFrom(List[0])",
            Box::new(|r| {
                RandomPicks::random_from_list::<_, i32>(r, &[]);
            }),
        ),
        (
            "randomFrom(Collection[0])",
            Box::new(|r| {
                RandomPicks::random_from_collection(r, Vec::<i32>::new());
            }),
        ),
        (
            "CodepointSetGenerator(char[0])",
            Box::new(|_| {
                CodepointSetGenerator::from_chars(&[]);
            }),
        ),
        (
            "CodepointSetGenerator(\"\")",
            Box::new(|_| {
                CodepointSetGenerator::from_string("");
            }),
        ),
        (
            "CodepointSetGenerator(surrogate)",
            Box::new(|_| {
                CodepointSetGenerator::from_chars(&[b'a' as u16, 0xd800]);
            }),
        ),
        (
            "CodepointSetGenerator(low surrogate)",
            Box::new(|_| {
                CodepointSetGenerator::from_chars(&[0xdfff]);
            }),
        ),
        (
            "suppOnly.units(3,3)",
            Box::new(|r| {
                CodepointSetGenerator::from_string("\u{1F600}").of_code_units_length(r, 3, 3);
            }),
        ),
        (
            "randomBytesOfLength(-1)",
            Box::new(|r| {
                RandomBytes::random_bytes_of_length(r, -1);
            }),
        ),
        (
            "ofCodeUnitsLength(5,4)",
            Box::new(|r| {
                RandomStrings::random_unicode_of_length_between(r, 5, 4);
            }),
        ),
    ];
    let expected: BTreeMap<String, String> = rows("generator_errors.tsv")
        .into_iter()
        .map(|r| (r[0].clone(), format!("{}\t{}", r[1], r[2])))
        .collect();
    let mut actual = BTreeMap::new();
    for (label, call) in calls {
        let r = Xoroshiro128PlusRandom::new(3);
        let got = match catch(|| call(&r)) {
            Ok(()) => "OK\t-".to_string(),
            Err(p) => java_error(&p),
        };
        actual.insert(label.to_string(), got);
    }
    assert_same_lines("generator errors", &expected, &actual);
}
