// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.PatternProps`.

/// Implements the immutable Unicode properties `Pattern_Syntax` and
/// `Pattern_White_Space`, and helpers for parsing patterns with them.
///
/// Equivalent to `com.ibm.icu.impl.PatternProps`.
pub struct PatternProps;

/// `latin1`: Latin-1 values; 1 = `Pattern_White_Space` only (unused),
/// 3 = `Pattern_Syntax`, 5 = `Pattern_White_Space`.
const LATIN1: [u8; 256] = [
    // 160 Latin-1 ... C0 controls
    0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 5, 5, 5, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    // ' '-'/'
    5, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, // '0'-'?'
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 3, 3, 3, 3, 3, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    // 'P'-'_'
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 3, 3, 3, 0, // '`'-'o'
    3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 'p'-DEL
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 3, 3, 3, 0, // C1 controls
    0, 0, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    // NBSP-'¯'
    0, 3, 3, 3, 3, 3, 3, 3, 0, 3, 0, 3, 3, 0, 3, 0, // '°'-'¿'
    3, 3, 0, 0, 0, 0, 3, 0, 0, 0, 0, 3, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    // 'Ð'-'ß'
    0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    // 'ð'-'ÿ'
    0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// `index2000`: one byte per 32 code points in `U+2000..U+3040`, selecting
/// a bit set of `syntax2000` / `syntaxOrWhiteSpace2000`.
const INDEX2000: [u8; 130] = [
    2, 3, 4, 0, 0, 0, 0, 0, // 20xx
    0, 0, 0, 0, 5, 1, 1, 1, // 21xx
    1, 1, 1, 1, 1, 1, 1, 1, // 22xx
    1, 1, 1, 1, 1, 1, 1, 1, // 23xx
    1, 1, 1, 0, 0, 0, 0, 0, // 24xx
    1, 1, 1, 1, 1, 1, 1, 1, // 25xx
    1, 1, 1, 1, 1, 1, 1, 1, // 26xx
    1, 1, 1, 6, 7, 1, 1, 1, // 27xx
    1, 1, 1, 1, 1, 1, 1, 1, // 28xx
    1, 1, 1, 1, 1, 1, 1, 1, // 29xx
    1, 1, 1, 1, 1, 1, 1, 1, // 2Axx
    1, 1, 1, 1, 1, 1, 1, 1, // 2Bxx
    0, 0, 0, 0, 0, 0, 0, 0, // 2Cxx
    0, 0, 0, 0, 0, 0, 0, 0, // 2Dxx
    1, 1, 1, 1, 0, 0, 0, 0, // 2Exx
    0, 0, 0, 0, 0, 0, 0, 0, // 2Fxx
    8, 9, // 3000..303F
];

/// `syntax2000`: the 32-bit `Pattern_Syntax` sets `index2000` selects.
const SYNTAX2000: [i32; 10] = [
    0,
    -1,
    0xffff0000_u32 as i32, // 2: 2010..201F
    0x7fff00ff,            // 3: 2020..202F, 2030..203E
    0x7feffffe,            // 4: 2040..204F, 2041..205E
    0xffff0000_u32 as i32, // 5: 2190..219F
    0x003fffff,            // 6: 2760..2775
    0xfff00000_u32 as i32, // 7: 2794..279F
    0xffffff0e_u32 as i32, // 8: 3001..3003, 3008..301F
    0x00010001,            // 9: 3020, 3030
];

/// `syntaxOrWhiteSpace2000`: `Pattern_Syntax` or `Pattern_White_Space`.
const SYNTAX_OR_WHITE_SPACE2000: [i32; 10] = [
    0,
    -1,
    0xffffc000_u32 as i32, // 2: 200E..201F
    0x7fff03ff,            // 3: 2020..2029, 2030..203E
    0x7feffffe,            // 4: 2040..204F, 2041..205E
    0xffff0000_u32 as i32, // 5: 2190..219F
    0x003fffff,            // 6: 2760..2775
    0xfff00000_u32 as i32, // 7: 2794..279F
    0xffffff0e_u32 as i32, // 8: 3001..3003, 3008..301F
    0x00010001,            // 9: 3020, 3030
];

impl PatternProps {
    /// `isSyntax(int c)`: true if `c` is a `Pattern_Syntax` code point.
    pub fn is_syntax(c: i32) -> bool {
        if c < 0 {
            false
        } else if c <= 0xff {
            LATIN1[c as usize] == 3
        } else if c < 0x2010 {
            false
        } else if c <= 0x3030 {
            let bits = SYNTAX2000[INDEX2000[((c - 0x2000) >> 5) as usize] as usize];
            ((bits >> (c & 0x1f)) & 1) != 0
        } else if (0xfd3e..=0xfe46).contains(&c) {
            c <= 0xfd3f || 0xfe45 <= c
        } else {
            false
        }
    }

    /// `isSyntaxOrWhiteSpace(int c)`: true if `c` is a `Pattern_Syntax` or
    /// `Pattern_White_Space` code point.
    pub fn is_syntax_or_white_space(c: i32) -> bool {
        if c < 0 {
            false
        } else if c <= 0xff {
            LATIN1[c as usize] != 0
        } else if c < 0x200e {
            false
        } else if c <= 0x3030 {
            let bits = SYNTAX_OR_WHITE_SPACE2000[INDEX2000[((c - 0x2000) >> 5) as usize] as usize];
            ((bits >> (c & 0x1f)) & 1) != 0
        } else if (0xfd3e..=0xfe46).contains(&c) {
            c <= 0xfd3f || 0xfe45 <= c
        } else {
            false
        }
    }

    /// `isWhiteSpace(int c)`: true if `c` is a `Pattern_White_Space` code
    /// point.
    pub fn is_white_space(c: i32) -> bool {
        if c < 0 {
            false
        } else if c <= 0xff {
            LATIN1[c as usize] == 5
        } else if (0x200e..=0x2029).contains(&c) {
            c <= 0x200f || 0x2028 <= c
        } else {
            false
        }
    }

    /// `skipWhiteSpace(CharSequence s, int i)`: the index after the
    /// `Pattern_White_Space` units from `i`.
    pub fn skip_white_space(s: &[u16], mut i: i32) -> i32 {
        while (i as usize) < s.len() && Self::is_white_space(s[i as usize] as i32) {
            i += 1;
        }
        i
    }

    /// `trimWhiteSpace(String s)`: `s` without leading and trailing
    /// `Pattern_White_Space`.
    pub fn trim_white_space(s: &[u16]) -> Vec<u16> {
        if s.is_empty()
            || (!Self::is_white_space(s[0] as i32) && !Self::is_white_space(s[s.len() - 1] as i32))
        {
            return s.to_vec();
        }
        let mut start = 0usize;
        let mut limit = s.len();
        while start < limit && Self::is_white_space(s[start] as i32) {
            start += 1;
        }
        if start < limit {
            // There is non-white space at start; we will not move limit below that,
            // so we need not test start<limit in the loop.
            while Self::is_white_space(s[limit - 1] as i32) {
                limit -= 1;
            }
        }
        s[start..limit].to_vec()
    }

    /// `trimSpaceChar(String s)`: `s` without leading
    /// `Character.isSpaceChar` units and, after them, trailing
    /// `Pattern_White_Space` units (the trailing loop tests
    /// `isWhiteSpace`, as ICU does).
    pub fn trim_space_char(s: &[u16]) -> Vec<u16> {
        use crate::jdk::is_space_char;
        if s.is_empty() || (!is_space_char(s[0]) && !is_space_char(s[s.len() - 1])) {
            return s.to_vec();
        }
        let mut start = 0usize;
        let mut limit = s.len();
        while start < limit && is_space_char(s[start]) {
            start += 1;
        }
        if start < limit {
            // There is non-white space at start; we will not move limit below that,
            // so we need not test start<limit in the loop.
            while Self::is_white_space(s[limit - 1] as i32) {
                limit -= 1;
            }
        }
        s[start..limit].to_vec()
    }

    /// `isIdentifier(CharSequence s)`: true if `s` is not empty and has no
    /// `Pattern_Syntax` or `Pattern_White_Space` unit.
    pub fn is_identifier(s: &[u16]) -> bool {
        let limit = s.len() as i32;
        if limit == 0 {
            return false;
        }
        Self::is_identifier_range(s, 0, limit)
    }

    /// `isIdentifier(CharSequence s, int start, int limit)`.
    pub fn is_identifier_range(s: &[u16], mut start: i32, limit: i32) -> bool {
        if start >= limit {
            return false;
        }
        loop {
            let c = s[start as usize];
            start += 1;
            if Self::is_syntax_or_white_space(c as i32) {
                return false;
            }
            if start >= limit {
                break;
            }
        }
        true
    }

    /// `skipIdentifier(CharSequence s, int i)`: the index after the units
    /// from `i` that are neither `Pattern_Syntax` nor `Pattern_White_Space`.
    pub fn skip_identifier(s: &[u16], mut i: i32) -> i32 {
        while (i as usize) < s.len() && !Self::is_syntax_or_white_space(s[i as usize] as i32) {
            i += 1;
        }
        i
    }
}
