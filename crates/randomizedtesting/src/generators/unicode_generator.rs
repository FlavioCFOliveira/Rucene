//! Port of `generators/UnicodeGenerator.java` (randomizedtesting 2.8.1).

use super::{array_length, string_from_code_points, RandomNumbers, StringGenerator};
use crate::jdk::random::Random;

/// `Character.MAX_SURROGATE - Character.MIN_SURROGATE + 1`.
const SURROGATE_RANGE: i32 = 0xdfff - 0xd800 + 1;
/// `Character.MAX_CODE_POINT - SURROGATE_RANGE`.
const CODEPOINT_RANGE: i32 = 0x10ffff - SURROGATE_RANGE;

/// Any valid Unicode code point: `UnicodeGenerator extends
/// StringGenerator`.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnicodeGenerator;

impl UnicodeGenerator {
    /// `new UnicodeGenerator()`.
    pub fn new() -> UnicodeGenerator {
        UnicodeGenerator
    }

    /// `ofUtf8Length(r, minUtf8Length, maxUtf8Length)`: a string whose UTF-8
    /// form is `[min, max]` bytes long.
    pub fn of_utf8_length(
        &self,
        r: &dyn Random,
        min_utf8_length: i32,
        max_utf8_length: i32,
    ) -> String {
        let length = RandomNumbers::random_int_between(r, min_utf8_length, max_utf8_length);
        let capacity = array_length(length.wrapping_mul(3));
        let mut buffer: Vec<u16> = Vec::with_capacity(capacity);
        let mut bytes = length;
        while buffer.len() < capacity && bytes != 0 {
            let t = if bytes >= 4 {
                r.next_int_bound(5)
            } else if bytes >= 3 {
                r.next_int_bound(4)
            } else if bytes >= 2 {
                r.next_int_bound(2)
            } else {
                0
            };
            match t {
                0 => {
                    buffer.push(RandomNumbers::random_int_between(r, 0, 0x7f) as u16);
                    bytes -= 1;
                }
                1 => {
                    buffer.push(RandomNumbers::random_int_between(r, 0x80, 0x7ff) as u16);
                    bytes -= 2;
                }
                2 => {
                    buffer.push(RandomNumbers::random_int_between(r, 0x800, 0xd7ff) as u16);
                    bytes -= 3;
                }
                3 => {
                    buffer.push(RandomNumbers::random_int_between(r, 0xe000, 0xffff) as u16);
                    bytes -= 3;
                }
                4 => {
                    // A surrogate pair.
                    buffer.push(RandomNumbers::random_int_between(r, 0xd800, 0xdbff) as u16);
                    buffer.push(RandomNumbers::random_int_between(r, 0xdc00, 0xdfff) as u16);
                    bytes -= 4;
                }
                _ => {}
            }
        }
        String::from_utf16(&buffer).expect("paired surrogates")
    }
}

impl StringGenerator for UnicodeGenerator {
    fn of_code_units_length(
        &self,
        r: &dyn Random,
        min_code_units: i32,
        max_code_units: i32,
    ) -> String {
        let length = RandomNumbers::random_int_between(r, min_code_units, max_code_units);
        let mut chars: Vec<u16> = Vec::with_capacity(array_length(length));
        while (chars.len() as i32) < length {
            let i = chars.len() as i32;
            let t = RandomNumbers::random_int_between(r, 0, 4);
            if t == 0 && i < length - 1 {
                // A surrogate pair.
                chars.push(RandomNumbers::random_int_between(r, 0xd800, 0xdbff) as u16);
                chars.push(RandomNumbers::random_int_between(r, 0xdc00, 0xdfff) as u16);
            } else if t <= 1 {
                chars.push(RandomNumbers::random_int_between(r, 0, 0x007f) as u16);
            } else if t == 2 {
                chars.push(RandomNumbers::random_int_between(r, 0x80, 0x07ff) as u16);
            } else if t == 3 {
                chars.push(RandomNumbers::random_int_between(r, 0x800, 0xd7ff) as u16);
            } else if t == 4 {
                chars.push(RandomNumbers::random_int_between(r, 0xe000, 0xffff) as u16);
            }
        }
        String::from_utf16(&chars).expect("paired surrogates")
    }

    fn of_code_points_length(
        &self,
        r: &dyn Random,
        min_code_points: i32,
        max_code_points: i32,
    ) -> String {
        let length = RandomNumbers::random_int_between(r, min_code_points, max_code_points);
        let chars: Vec<i32> = (0..array_length(length) as i32)
            .map(|_| {
                let v = RandomNumbers::random_int_between(r, 0, CODEPOINT_RANGE);
                if v >= 0xd800 {
                    v + SURROGATE_RANGE
                } else {
                    v
                }
            })
            .collect();
        string_from_code_points(&chars)
    }
}
