//! Port of `com.carrotsearch.randomizedtesting.generators` (randomizedtesting
//! 2.8.1, `generators/`): numbers, strings, bytes and picks drawn from a
//! `Random`.
//!
//! Every function takes the generator as `&R` with `R: Random + ?Sized`, so
//! a `&Xoroshiro128PlusRandom`, a `&JdkRandom` and an `&Arc<dyn Random>`
//! all work.

mod ascii_alphanum_generator;
#[allow(deprecated)]
mod ascii_generator;
mod ascii_letters_generator;
mod biased_numbers;
mod codepoint_set_generator;
mod random_bytes;
mod random_numbers;
mod random_picks;
mod random_strings;
mod realistic_unicode_generator;
mod string_generator;
mod unicode_generator;

pub use ascii_alphanum_generator::AsciiAlphanumGenerator;
#[allow(deprecated)]
pub use ascii_generator::AsciiGenerator;
pub use ascii_letters_generator::AsciiLettersGenerator;
pub use biased_numbers::BiasedNumbers;
pub use codepoint_set_generator::CodepointSetGenerator;
pub use random_bytes::RandomBytes;
pub use random_numbers::RandomNumbers;
pub use random_picks::RandomPicks;
#[allow(deprecated)]
pub use random_strings::{
    RandomStrings, ASCII_ALPHANUM_GENERATOR, ASCII_GENERATOR, ASCII_LETTERS_GENERATOR,
    REALISTIC_UNICODE_GENERATOR, UNICODE_GENERATOR,
};
pub use realistic_unicode_generator::RealisticUnicodeGenerator;
pub use string_generator::StringGenerator;
pub use unicode_generator::UnicodeGenerator;

/// `new String(int[] codePoints, 0, count)` for code points that are valid
/// Unicode scalar values (every generator here only produces those).
pub(crate) fn string_from_code_points(code_points: &[i32]) -> String {
    code_points
        .iter()
        .map(|&cp| char::from_u32(cp as u32).expect("a Unicode scalar value"))
        .collect()
}

/// `new T[length]`: the length as `usize`.
///
/// # Panics
///
/// `NegativeArraySizeException` (`<length>`) for a negative length.
pub(crate) fn array_length(length: i32) -> usize {
    if length < 0 {
        crate::jdk::exception::throw(crate::jdk::exception::NEGATIVE_ARRAY_SIZE_EXCEPTION, length)
    }
    length as usize
}
