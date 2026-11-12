//! Java number rendering, reproduced for the strings Lucene's `toString()`
//! bodies build by concatenation, and the JDK's lenient UTF-8 decoding,
//! reproduced for the strings Lucene builds from bytes
//! ([`java_utf8_to_string`]).
//!
//! Lucene's `toString()` implementations are plain string concatenation, so the
//! exact text they produce depends on how Java renders the values that go into
//! them — `Double.toString`, `Integer.toHexString` and `java.util.Formatter`'s
//! `%f` and `%,d` conversions. Rust's own rendering differs from all four, and
//! those strings are observable (`CheckIndex` output, `IndexWriter` info-stream
//! messages, exception text, Luke), so the renderings are reproduced here
//! rather than approximated:
//!
//! * Rust's `Display` prints `800` where Java prints `800.0`, and prints
//!   `0.0000000001` where Java prints `1.0E-10`.
//! * Rust's `{:.2}` rounds ties to even where Java's `%f` rounds ties away from
//!   zero (`java.math.RoundingMode.HALF_UP`).
//! * Rust has no equivalent of `%,d`, which groups the integer digits in threes
//!   under `Locale.ROOT`.
//! * Rust's `{:x}` on a negative `i32` prints a `-` sign, where Java's
//!   `Integer.toHexString` renders the two's-complement bit pattern unsigned.
//!
//! # Relationship to `search::similarities::java_fmt`
//!
//! `crate::search::similarities::java_fmt` already reproduces `Float.toString`,
//! `Double.toString` and `%f` for the explanation tree. That module sits under
//! `search`, and Lucene's `util` package does not depend on `search`, so this
//! module does not call into it; consolidating the two would mean moving the
//! `search` copy, which is outside the scope of the task that introduced this
//! file.

#![deny(unsafe_code)]

/// Renders an `f64` the way `java.lang.Double.toString(double)` does.
///
/// JDK 19 and later (Raffaello Giulietti's algorithm, the `Double.toString`
/// javadoc) render the shortest decimal that rounds to the value, with at
/// least two significant digits, the one closest to the value and, of two
/// equally close, the one with the even significand; it switches to
/// "computerized scientific notation" when the magnitude leaves
/// `[10^-3, 10^7)`. Rust's shortest form breaks the tie upwards and has no
/// two-digit minimum, so [`giulietti_decimal`] corrects it: `Double.MIN_VALUE`
/// renders `4.9E-324` (Rust's shortest form is `5e-324`) and
/// `9.4832877835713125E12`, a halfway value, `9.483287783571312E12`.
///
/// The algorithm is the one of the workspace crate `randomizedtesting`
/// (`src/jdk/lang.rs`), pinned against JDK 25 by its oracle grid.
pub fn double_to_string(d: f64) -> String {
    if d.is_nan() {
        return "NaN".to_string();
    }
    if d.is_infinite() {
        return if d > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if d == 0.0 {
        return if d.is_sign_negative() { "-0.0" } else { "0.0" }.to_string();
    }
    let a = d.abs();
    let decimal = giulietti_decimal(
        format!("{a:e}"),
        format!("{a:.1e}"),
        // A double's exact decimal expansion has at most 767 significant
        // digits, so 800 fraction digits render it exactly.
        |digits| format!("{a:.*e}", digits.min(800)),
        |s| s.parse::<f64>().is_ok_and(|v| v.to_bits() == a.to_bits()),
    );
    java_float_format(&decimal, d < 0.0)
}

/// Renders an `f32` the way `java.lang.Float.toString(float)` does.
///
/// The rules of [`double_to_string`], applied to the decimals that round to
/// the value as a `float`: `Float.MIN_VALUE` renders `1.4E-45` and
/// `390.015625f`, a halfway value, `390.01562`.
pub fn float_to_string(f: f32) -> String {
    if f.is_nan() {
        return "NaN".to_string();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0" } else { "0.0" }.to_string();
    }
    let a = f.abs();
    let decimal = giulietti_decimal(
        format!("{a:e}"),
        format!("{a:.1e}"),
        // A float's exact decimal expansion has at most 112 significant digits.
        |digits| format!("{a:.*e}", digits.min(150)),
        |s| s.parse::<f32>().is_ok_and(|v| v.to_bits() == a.to_bits()),
    );
    java_float_format(&decimal, f < 0.0)
}

/// The decimal `Double.toString` / `Float.toString` render for the positive
/// finite value whose shortest round-trip form (`{:e}`) is `shortest` and
/// whose form rounded to two significant digits (`{:.1e}`) is `two`.
///
/// JDK 19 and later (the `Double.toString(double)` javadoc, the algorithm of
/// Raffaello Giulietti) take the set R of decimals that round to the value,
/// and the minimal length p over R: when p >= 2 the result is the decimal of
/// length p in R closest to the value, which is the shortest form; when p is
/// 1, the result is the decimal of length 1 or 2 in R closest to the value,
/// which is the value rounded to two significant digits (that decimal is
/// closer than the one-digit one, so it is in R too). Before, this
/// emulation always took the shortest form and padded `d` to `d.0`, so
/// `Double.MIN_VALUE` (shortest `5e-324`) rendered `5.0E-324` where JDK 25
/// renders `4.9E-324`, and `Float.MIN_VALUE` `1.0E-45` for `1.4E-45`.
///
/// Among the decimals of length p, JDK 25 takes the one closest to the value
/// and, of two equally close, the one with the even significand. Rust's
/// shortest form breaks that tie upwards: when the value lies exactly halfway
/// between two p-digit decimals (its exact expansion, `exact(n)` with `n`
/// fraction digits, continues
/// after the p-th digit with `5` and zeros only), the neighbour with the even
/// last digit is taken if it rounds to the value too (`round_trips`). Measured on JDK 25:
/// `Float.toString(2234752.25f)` is `2234752.2`, where the shortest form is
/// `2234752.3`.
fn giulietti_decimal(
    shortest: String,
    two: String,
    exact: impl Fn(usize) -> String,
    round_trips: impl Fn(&str) -> bool,
) -> String {
    let split = |sci: &str| -> (String, i32) {
        let (m, e) = sci.split_once('e').expect("scientific notation");
        (
            m.chars().filter(char::is_ascii_digit).collect(),
            e.parse().expect("exponent"),
        )
    };
    let (digits, exp) = split(&shortest);
    if digits.len() == 1 {
        return two;
    }
    // The exact value's digits at the same exponent: a halfway point is
    // `digits - 1` followed by `5000…`.
    let p = digits.len();
    let halfway = |sci: &str| -> Option<String> {
        let (exact_digits, exact_exp) = split(sci);
        let tail = &exact_digits[p..];
        (exact_exp == exp && tail.starts_with('5') && tail[1..].bytes().all(|b| b == b'0'))
            .then_some(exact_digits)
    };
    // A cheap rendering with 20 more digits rules out almost every value; a
    // possible halfway point is confirmed on the exact expansion.
    if halfway(&exact(p + 20)).is_none() {
        return shortest;
    }
    let Some(exact_digits) = halfway(&exact(usize::MAX)) else {
        return shortest;
    };
    // The two neighbours: `exact_digits[..p]` and the next p-digit decimal.
    let lower = exact_digits[..p].to_string();
    let mut upper = lower.clone().into_bytes();
    let mut i = p;
    loop {
        if i == 0 {
            // `99…9` + 1 has more digits: not a neighbour of length p.
            return shortest;
        }
        i -= 1;
        if upper[i] == b'9' {
            upper[i] = b'0';
        } else {
            upper[i] += 1;
            break;
        }
    }
    let upper = String::from_utf8(upper).expect("ASCII digits");
    let even = if (lower.as_bytes()[p - 1] - b'0').is_multiple_of(2) {
        lower
    } else {
        upper
    };
    if even == digits {
        return shortest;
    }
    let candidate = format!("{}.{}e{exp}", &even[..1], &even[1..]);
    if round_trips(&candidate) {
        candidate
    } else {
        shortest
    }
}

/// Formats decimal digits as `Double.toString` / `Float.toString` do:
/// plain notation with at least one fraction digit for magnitudes in
/// `[10^-3, 10^7)`, otherwise `d.dddE[-]n`.
fn java_float_format(sci: &str, negative: bool) -> String {
    // `sci` is the decimal of `giulietti_decimal`, `d[.ddd]e[-]n`.
    let (mantissa, exp) = sci.split_once('e').expect("scientific notation");
    let exp: i32 = exp.parse().expect("exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    // `{:.1e}` keeps a second digit that may be a zero; the significand
    // carries no trailing zero (`d.0` is rendered by the branches below).
    let digits = match digits.trim_end_matches('0') {
        "" => "0".to_string(),
        trimmed => trimmed.to_string(),
    };
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    if (-3..7).contains(&exp) {
        if exp < 0 {
            out.push_str("0.");
            for _ in 0..(-exp - 1) {
                out.push('0');
            }
            out.push_str(&digits);
        } else {
            let int_len = (exp + 1) as usize;
            if digits.len() <= int_len {
                out.push_str(&digits);
                for _ in digits.len()..int_len {
                    out.push('0');
                }
                out.push_str(".0");
            } else {
                out.push_str(&digits[..int_len]);
                out.push('.');
                out.push_str(&digits[int_len..]);
            }
        }
    } else {
        out.push_str(&digits[..1]);
        out.push('.');
        if digits.len() > 1 {
            out.push_str(&digits[1..]);
        } else {
            out.push('0');
        }
        out.push('E');
        out.push_str(&exp.to_string());
    }
    out
}

/// Renders `value` the way `String.format(Locale.ROOT, "%.<digits>f", value)`
/// does: exactly `digits` fraction digits, no grouping separators, ties rounded
/// away from zero.
///
/// Rust's `{:.n}` would round ties to even; Java's `Formatter` rounds them away
/// from zero (`RoundingMode.HALF_UP`). The two disagree on values whose exact
/// decimal expansion stops on a `5` in the first dropped position, so the digits
/// are taken from the exact expansion and rounded here instead.
pub(crate) fn format_fixed(value: f64, digits: usize) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    // A finite `f64` has an exact decimal expansion of at most 1074 fraction
    // digits (the smallest subnormal), so this rendering is exact, never
    // rounded, and the rounding below is therefore a single rounding step.
    let exact = format!("{:.*}", 1074, value.abs());
    let (integer, fraction) = match exact.split_once('.') {
        Some(split) => split,
        None => (exact.as_str(), ""),
    };

    let mut rendered: Vec<u8> = integer
        .bytes()
        .chain(fraction.bytes().take(digits))
        .collect();
    // Pad a fraction shorter than `digits`, which cannot happen for the exact
    // expansion above but keeps the indexing below independent of it.
    while rendered.len() < integer.len() + digits {
        rendered.push(b'0');
    }
    let round_up = fraction
        .as_bytes()
        .get(digits)
        .is_some_and(|digit| *digit >= b'5');
    if round_up {
        let mut carry = true;
        for digit in rendered.iter_mut().rev() {
            if !carry {
                break;
            }
            if *digit == b'9' {
                *digit = b'0';
            } else {
                *digit += 1;
                carry = false;
            }
        }
        if carry {
            rendered.insert(0, b'1');
        }
    }

    let rendered = String::from_utf8(rendered).unwrap_or_default();
    let split = rendered.len() - digits;
    let sign = if value.is_sign_negative() { "-" } else { "" };
    if digits == 0 {
        format!("{sign}{rendered}")
    } else {
        format!("{sign}{}.{}", &rendered[..split], &rendered[split..])
    }
}

/// Renders `value` the way `String.format(Locale.ROOT, "%,d", value)` does:
/// the decimal digits grouped in threes by a comma, which is `Locale.ROOT`'s
/// grouping separator and grouping size.
pub(crate) fn format_grouped(value: i64) -> String {
    let negative = value < 0;
    // `i64::MIN` has no positive counterpart, so group the digits of the
    // rendered magnitude rather than negating the value.
    let digits = value.unsigned_abs().to_string();
    let bytes = digits.as_bytes();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if negative {
        grouped.push('-');
    }
    for (i, byte) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(*byte as char);
    }
    grouped
}

/// Renders `value` the way `java.lang.Integer.toHexString(int)` does: the
/// two's-complement bit pattern as unsigned lowercase hexadecimal, with no
/// leading zeros and no sign.
pub(crate) fn to_hex_string(value: i32) -> String {
    format!("{:x}", value as u32)
}

/// Decodes `bytes` as `new String(bytes, StandardCharsets.UTF_8)` does.
///
/// **This has no counterpart in Lucene 10.5.0**: Java delegates to the JDK,
/// whose decoder replaces every malformed sequence with U+FFFD and never
/// fails. Its notion of a malformed sequence is not Unicode's "maximal
/// subpart" that [`String::from_utf8_lossy`] follows — an encoded surrogate
/// such as `ED A0 80` is one replacement in Java and three in Rust — so the
/// JDK's rules (`java.lang.String.decodeUTF8_UTF16` and its `malformed3` /
/// `malformed4` helpers in OpenJDK 21-25) are reproduced here, and checked
/// against JDK 25 by the codec-header portability test.
///
/// Used by
/// [`check_index_header_suffix`](crate::codecs::codec_util::check_index_header_suffix)
/// and by [`DataInput::read_string`](crate::store::DataInput::read_string), whose
/// Java counterparts both build the string with that constructor
/// (`CodecUtil.java:383`, `DataInput.java:234`).
pub fn java_utf8_to_string(bytes: &[u8]) -> String {
    const REPL: u16 = 0xFFFD;
    fn is_not_continuation(b: u8) -> bool {
        b & 0xC0 != 0x80
    }
    fn is_malformed3(b1: u8, b2: u8, b3: u8) -> bool {
        (b1 == 0xE0 && b2 & 0xE0 == 0x80) || is_not_continuation(b2) || is_not_continuation(b3)
    }
    fn is_malformed3_2(b1: u8, b2: u8) -> bool {
        (b1 == 0xE0 && b2 & 0xE0 == 0x80) || is_not_continuation(b2)
    }
    fn is_malformed4(b2: u8, b3: u8, b4: u8) -> bool {
        is_not_continuation(b2) || is_not_continuation(b3) || is_not_continuation(b4)
    }
    fn is_malformed4_2(b1: u8, b2: u8) -> bool {
        (b1 == 0xF0 && !(0x90..=0xBF).contains(&b2))
            || (b1 == 0xF4 && b2 & 0xF0 != 0x80)
            || is_not_continuation(b2)
    }
    /// How many bytes of a malformed three-byte sequence are skipped.
    fn malformed3(src: &[u8], sp: usize) -> usize {
        let (b1, b2) = (src[sp], src[sp + 1]);
        if (b1 == 0xE0 && b2 & 0xE0 == 0x80) || is_not_continuation(b2) {
            1
        } else {
            2
        }
    }
    /// How many bytes of a malformed four-byte sequence are skipped.
    fn malformed4(src: &[u8], sp: usize) -> usize {
        let (b1, b2) = (src[sp], src[sp + 1]);
        if b1 > 0xF4
            || (b1 == 0xF0 && !(0x90..=0xBF).contains(&b2))
            || (b1 == 0xF4 && b2 & 0xF0 != 0x80)
            || is_not_continuation(b2)
        {
            1
        } else if is_not_continuation(src[sp + 2]) {
            2
        } else {
            3
        }
    }

    let mut out: Vec<u16> = Vec::with_capacity(bytes.len());
    let (mut sp, sl) = (0usize, bytes.len());
    while sp < sl {
        let b1 = bytes[sp];
        sp += 1;
        if b1 < 0x80 {
            out.push(u16::from(b1));
        } else if b1 >> 5 == 0b110 && b1 & 0x1E != 0 {
            if sp < sl {
                let b2 = bytes[sp];
                sp += 1;
                if is_not_continuation(b2) {
                    out.push(REPL);
                    sp -= 1;
                } else {
                    out.push((u16::from(b1 & 0x1F) << 6) | u16::from(b2 & 0x3F));
                }
                continue;
            }
            out.push(REPL);
            break;
        } else if b1 >> 4 == 0b1110 {
            if sp + 1 < sl {
                let (b2, b3) = (bytes[sp], bytes[sp + 1]);
                sp += 2;
                if is_malformed3(b1, b2, b3) {
                    out.push(REPL);
                    sp -= 3;
                    sp += malformed3(bytes, sp);
                } else {
                    let c = (u16::from(b1 & 0x0F) << 12)
                        | (u16::from(b2 & 0x3F) << 6)
                        | u16::from(b3 & 0x3F);
                    // An encoded surrogate is one replacement.
                    out.push(if (0xD800..=0xDFFF).contains(&c) {
                        REPL
                    } else {
                        c
                    });
                }
                continue;
            }
            if sp < sl && is_malformed3_2(b1, bytes[sp]) {
                out.push(REPL);
                continue;
            }
            out.push(REPL);
            break;
        } else if b1 >> 3 == 0b11110 {
            if sp + 2 < sl {
                let (b2, b3, b4) = (bytes[sp], bytes[sp + 1], bytes[sp + 2]);
                sp += 3;
                let uc = (u32::from(b1 & 0x07) << 18)
                    | (u32::from(b2 & 0x3F) << 12)
                    | (u32::from(b3 & 0x3F) << 6)
                    | u32::from(b4 & 0x3F);
                if is_malformed4(b2, b3, b4) || !(0x10000..=0x10FFFF).contains(&uc) {
                    out.push(REPL);
                    sp -= 4;
                    sp += malformed4(bytes, sp);
                } else {
                    let v = uc - 0x10000;
                    out.push(0xD800 | (v >> 10) as u16);
                    out.push(0xDC00 | (v & 0x3FF) as u16);
                }
                continue;
            }
            if b1 > 0xF4 || (sp < sl && is_malformed4_2(b1, bytes[sp])) {
                out.push(REPL);
                continue;
            }
            sp += 1;
            out.push(REPL);
            if sp < sl && is_not_continuation(bytes[sp]) {
                continue;
            }
            break;
        } else {
            out.push(REPL);
        }
    }
    // Every surrogate pushed above is half of a well-formed pair.
    String::from_utf16_lossy(&out)
}

// ---------------------------------------------------------------------------
// Double.parseDouble / Float.parseFloat
// ---------------------------------------------------------------------------
//
// Copied from the workspace crate `randomizedtesting` (`src/jdk/lang.rs`),
// whose parsers are pinned against JDK 25 by its oracle grid
// (`tests/fixtures/java_lang.tsv`). `lucene/codecs`' `SimpleText` formats read
// back what `Float.toString` and `Double.toString` wrote with these two JDK
// parsers (`SimpleTextStoredFieldsReader`, `SimpleTextKnnVectorsReader`).

/// The binary format a floating-point string is rounded to.
#[derive(Clone, Copy)]
struct BinaryFormat {
    /// Explicit significand bits (52 or 23).
    mant_bits: u32,
    /// Exponent bias (1023 or 127).
    bias: i64,
}

const F64_FORMAT: BinaryFormat = BinaryFormat {
    mant_bits: 52,
    bias: 1023,
};
const F32_FORMAT: BinaryFormat = BinaryFormat {
    mant_bits: 23,
    bias: 127,
};

/// Rounds `m * 2^e` (plus a nonzero remainder below `m`'s last bit when
/// `sticky`) to the nearest value of `format`, ties to even, and returns its
/// unsigned bit pattern (`+inf` on overflow).
fn round_binary(m: u64, e: i64, sticky: bool, format: BinaryFormat) -> u64 {
    if m == 0 {
        return 0;
    }
    let mb = i64::from(format.mant_bits);
    let emin = 1 - format.bias;
    let emax = format.bias;
    let inf_bits = ((2 * format.bias + 1) as u64) << format.mant_bits;
    let n = 64 - i64::from(m.leading_zeros());
    let top = e + n - 1;
    if top > emax + 1 {
        return inf_bits;
    }
    // The weight of the last bit kept.
    let unit = top.max(emin) - mb;
    let shift = unit - e;
    let q: u64 = if shift <= 0 {
        // Exact: fewer significant bits than the format keeps.
        m << (-shift)
    } else if shift > 64 {
        // The value is below half the smallest subnormal: it rounds to zero.
        0
    } else {
        let shift = shift as u32;
        let wide = u128::from(m);
        let kept = (wide >> shift) as u64;
        let rem = wide & ((1u128 << shift) - 1);
        let half = 1u128 << (shift - 1);
        let round_up = rem > half || (rem == half && (sticky || kept & 1 == 1));
        kept + u64::from(round_up)
    };
    let base = ((top.max(emin) + format.bias - 1) as u64) << format.mant_bits;
    let bits = base + q;
    if bits >= inf_bits {
        inf_bits
    } else {
        bits
    }
}

/// Parses the hexadecimal significand and binary exponent of a Java hex
/// floating-point literal (after the sign and `0x`), rounding to `format`.
/// `None` when the text does not match
/// `(HexDigits\.?|HexDigits?\.HexDigits)[pP][+-]?Digits[fFdD]?`.
fn parse_hex(body: &[u8], format: BinaryFormat) -> Option<u64> {
    let mut i = 0;
    let mut m: u64 = 0;
    let mut e: i64 = 0;
    let mut sticky = false;
    let mut digits = 0;
    let accumulate = |d: u64, fraction: bool, m: &mut u64, e: &mut i64, sticky: &mut bool| {
        if *m >> 60 == 0 {
            *m = (*m << 4) | d;
            if fraction {
                *e -= 4;
            }
        } else {
            if d != 0 {
                *sticky = true;
            }
            if !fraction {
                *e += 4;
            }
        }
    };
    while i < body.len() && body[i].is_ascii_hexdigit() {
        let d = u64::from((body[i] as char).to_digit(16)?);
        accumulate(d, false, &mut m, &mut e, &mut sticky);
        i += 1;
        digits += 1;
    }
    if i < body.len() && body[i] == b'.' {
        i += 1;
        while i < body.len() && body[i].is_ascii_hexdigit() {
            let d = u64::from((body[i] as char).to_digit(16)?);
            accumulate(d, true, &mut m, &mut e, &mut sticky);
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if i >= body.len() || (body[i] != b'p' && body[i] != b'P') {
        return None;
    }
    i += 1;
    let mut exp_negative = false;
    if i < body.len() && (body[i] == b'+' || body[i] == b'-') {
        exp_negative = body[i] == b'-';
        i += 1;
    }
    let exp_start = i;
    let mut exp: i64 = 0;
    while i < body.len() && body[i].is_ascii_digit() {
        // Saturate: any exponent this large over- or underflows.
        exp = (exp * 10 + i64::from(body[i] - b'0')).min(1 << 40);
        i += 1;
    }
    if i == exp_start {
        return None;
    }
    if i < body.len() && matches!(body[i], b'f' | b'F' | b'd' | b'D') {
        i += 1;
    }
    if i != body.len() {
        return None;
    }
    let exp = if exp_negative { -exp } else { exp };
    Some(round_binary(m, e + exp, sticky, format))
}

/// Whether `body` (after the sign) is a Java decimal floating-point literal:
/// `Digits.Digits?Exponent?|.Digits Exponent?|Digits Exponent?`, where
/// `Exponent` is `[eE][+-]?Digits`. Returns the length without a trailing
/// `[fFdD]` suffix.
fn decimal_literal_len(body: &[u8]) -> Option<usize> {
    let mut i = 0;
    let mut digits = 0;
    while i < body.len() && body[i].is_ascii_digit() {
        i += 1;
        digits += 1;
    }
    if i < body.len() && body[i] == b'.' {
        i += 1;
        while i < body.len() && body[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if i < body.len() && (body[i] == b'e' || body[i] == b'E') {
        i += 1;
        if i < body.len() && (body[i] == b'+' || body[i] == b'-') {
            i += 1;
        }
        let start = i;
        while i < body.len() && body[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return None;
        }
    }
    let len = i;
    if i < body.len() && matches!(body[i], b'f' | b'F' | b'd' | b'D') {
        i += 1;
    }
    (i == body.len()).then_some(len)
}

/// What a Java floating-point string denotes, before rounding.
enum JavaFloat<'a> {
    NaN,
    Infinity(bool),
    /// A decimal literal (sign, then the literal without its suffix).
    Decimal(bool, &'a str),
    /// A hexadecimal literal (sign, then the text after `0x`).
    Hex(bool, &'a [u8]),
}

/// `FloatingDecimal.readJavaFormatString`'s grammar: `String.trim()`, an
/// optional sign, then `NaN`, `Infinity`, a hexadecimal literal or a decimal
/// literal with an optional `[fFdD]` suffix.
fn classify(s: &str) -> Option<JavaFloat<'_>> {
    let s = s.trim_matches(|c: char| c <= '\u{20}');
    let bytes = s.as_bytes();
    if bytes.is_empty() || !s.is_ascii() {
        return None;
    }
    let (negative, rest) = match bytes[0] {
        b'-' => (true, &s[1..]),
        b'+' => (false, &s[1..]),
        _ => (false, s),
    };
    if rest == "NaN" {
        return Some(JavaFloat::NaN);
    }
    if rest == "Infinity" {
        return Some(JavaFloat::Infinity(negative));
    }
    let rb = rest.as_bytes();
    if rb.len() >= 2 && rb[0] == b'0' && (rb[1] == b'x' || rb[1] == b'X') {
        return Some(JavaFloat::Hex(negative, &rb[2..]));
    }
    let len = decimal_literal_len(rb)?;
    Some(JavaFloat::Decimal(negative, &rest[..len]))
}

/// `Double.parseDouble(s)`; `None` where Java throws
/// `NumberFormatException`.
pub fn parse_double(s: &str) -> Option<f64> {
    match classify(s)? {
        JavaFloat::NaN => Some(f64::NAN),
        JavaFloat::Infinity(neg) => Some(if neg {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        }),
        JavaFloat::Decimal(neg, lit) => {
            // The literal matches Java's grammar, which `f64::from_str`
            // accepts and rounds correctly, as Java does.
            let v: f64 = lit.parse().ok()?;
            Some(if neg { -v } else { v })
        }
        JavaFloat::Hex(neg, body) => {
            let v = f64::from_bits(parse_hex(body, F64_FORMAT)?);
            Some(if neg { -v } else { v })
        }
    }
}

/// `Float.parseFloat(s)`; `None` where Java throws `NumberFormatException`.
/// The string is rounded to `float` directly, not through `double`.
pub fn parse_float(s: &str) -> Option<f32> {
    match classify(s)? {
        JavaFloat::NaN => Some(f32::NAN),
        JavaFloat::Infinity(neg) => Some(if neg {
            f32::NEG_INFINITY
        } else {
            f32::INFINITY
        }),
        JavaFloat::Decimal(neg, lit) => {
            let v: f32 = lit.parse().ok()?;
            Some(if neg { -v } else { v })
        }
        JavaFloat::Hex(neg, body) => {
            let v = f32::from_bits(parse_hex(body, F32_FORMAT)? as u32);
            Some(if neg { -v } else { v })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_to_string_keeps_a_fraction_digit_and_switches_to_scientific() {
        assert_eq!(double_to_string(800.0), "800.0");
        assert_eq!(double_to_string(0.0), "0.0");
        assert_eq!(double_to_string(-0.0), "-0.0");
        assert_eq!(double_to_string(1e-10), "1.0E-10");
        assert_eq!(double_to_string(1e7), "1.0E7");
        assert_eq!(double_to_string(9_999_999.0), "9999999.0");
        assert_eq!(double_to_string(0.001), "0.001");
        assert_eq!(double_to_string(f64::NAN), "NaN");
        assert_eq!(double_to_string(f64::INFINITY), "Infinity");
    }

    /// JDK 25 (`java P.java`, the values printed by `Float.toString` and
    /// `Double.toString`): the two-digit minimum of the smallest subnormals
    /// and the even neighbour of a halfway value, where Rust's shortest form
    /// prints `1e-45`, `5e-324`, `390.01563`, `2234752.3` and
    /// `9.483287783571313e12`.
    #[test]
    fn float_and_double_to_string_match_jdk_25() {
        assert_eq!(float_to_string(f32::from_bits(1)), "1.4E-45");
        assert_eq!(float_to_string(f32::from_bits(0x43c3_0200)), "390.01562");
        assert_eq!(float_to_string(f32::from_bits(0x4a08_6601)), "2234752.2");
        assert_eq!(float_to_string(-0.0), "-0.0");
        assert_eq!(float_to_string(1.0E-30), "1.0E-30");
        assert_eq!(float_to_string(9.999999E-4), "9.999999E-4");
        assert_eq!(double_to_string(f64::from_bits(1)), "4.9E-324");
        assert_eq!(
            double_to_string(f64::from_bits(0x42a1_3fff_ffd1_26a0)),
            "9.483287783571312E12"
        );
        assert_eq!(double_to_string(1e23), "1.0E23");
        assert_eq!(double_to_string(9.99999999999E-4), "9.99999999999E-4");
    }

    /// JDK 25 `Float.parseFloat` and `Double.parseDouble`: surrounding
    /// characters up to U+0020 are trimmed, a `[fFdD]` suffix and hexadecimal
    /// literals are accepted, an underflow is zero.
    #[test]
    fn parse_float_and_double_match_jdk_25() {
        assert_eq!(parse_float(" 1.5f "), Some(1.5));
        assert_eq!(parse_float("0x1.8p1"), Some(3.0));
        assert_eq!(parse_double("-Infinity"), Some(f64::NEG_INFINITY));
        assert_eq!(parse_double("1e-400"), Some(0.0));
        assert_eq!(parse_double("NaN").map(f64::is_nan), Some(true));
        assert_eq!(parse_double("1.0.0"), None);
        assert_eq!(parse_float(""), None);
    }

    #[test]
    fn format_fixed_rounds_ties_away_from_zero() {
        // Rust's own `{:.2}` renders 0.125 as "0.12" (ties to even); Java's
        // "%.2f" renders it as "0.13".
        assert_eq!(format!("{:.2}", 0.125_f64), "0.12");
        assert_eq!(format_fixed(0.125, 2), "0.13");
        assert_eq!(format_fixed(0.0, 2), "0.00");
        assert_eq!(format_fixed(9.999, 2), "10.00");
        assert_eq!(format_fixed(-0.125, 2), "-0.13");
        assert_eq!(format_fixed(1.5, 0), "2");
    }

    #[test]
    fn format_grouped_groups_in_threes() {
        assert_eq!(format_grouped(0), "0");
        assert_eq!(format_grouped(999), "999");
        assert_eq!(format_grouped(1000), "1,000");
        assert_eq!(format_grouped(1_234_567), "1,234,567");
        assert_eq!(format_grouped(-1_234_567), "-1,234,567");
        assert_eq!(format_grouped(i64::MIN), "-9,223,372,036,854,775,808");
    }

    #[test]
    fn to_hex_string_is_unsigned() {
        assert_eq!(to_hex_string(0), "0");
        assert_eq!(to_hex_string(255), "ff");
        assert_eq!(to_hex_string(-1), "ffffffff");
    }
}

/// A `java.text.DecimalFormat` built with
/// `DecimalFormatSymbols.getInstance(Locale.ROOT)`, implemented from the
/// `java.text.DecimalFormat` specification (its class javadoc: pattern
/// syntax, special pattern characters, scientific notation, rounding and
/// special values) for formatting `double` values.
///
/// Equivalent to `java.text.DecimalFormat` restricted to what
/// `NumberFormat.format(double)` observes: the pattern's prefixes and
/// suffixes (quoting, `%`, `\u{2030}`, `\u{a4}`, `-`), the integer, fraction
/// and exponent digit counts, grouping, the decimal separator shown always,
/// and `RoundingMode.HALF_EVEN` rounding. The `Locale.ROOT` symbols are
/// `0`, `,`, `.`, `%`, `\u{2030}`, `-`, `E`, `\u{221e}` (infinity) and `NaN`,
/// with currency symbol `\u{a4}` and international currency symbol `XXX`.
///
/// Digits follow `DigitList.set(boolean, double, int, boolean)`: the digits
/// of `FloatingDecimal.getBinaryToASCIIConverter(double)` (see `Converted`),
/// rounded half-even only when the pattern cannot show all of them, with the
/// converter's `digitsRoundedUp()` and `decimalDigitsExact()` flags deciding
/// a tie on its last digit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecimalFormat {
    /// The pattern this format was built from.
    pattern: String,
    positive_prefix: String,
    positive_suffix: String,
    negative_prefix: String,
    negative_suffix: String,
    /// 1, or 100 for `%`, or 1000 for `\u{2030}`.
    multiplier: i32,
    grouping_used: bool,
    grouping_size: i32,
    min_int: i32,
    max_int: i32,
    min_frac: i32,
    max_frac: i32,
    decimal_separator_always_shown: bool,
    use_exponential: bool,
    min_exponent_digits: i32,
}

/// `NumberFormat`'s `DOUBLE_INTEGER_DIGITS`: the most integer digits a
/// `double` is formatted with.
const DOUBLE_INTEGER_DIGITS: i32 = 309;
/// `NumberFormat`'s `DOUBLE_FRACTION_DIGITS`.
const DOUBLE_FRACTION_DIGITS: i32 = 340;
/// `DecimalFormat.MAXIMUM_INTEGER_DIGITS` / `MAXIMUM_FRACTION_DIGITS`.
const MAXIMUM_DIGITS: i32 = i32::MAX;

impl DecimalFormat {
    /// Creates a format from `pattern` with
    /// `DecimalFormatSymbols.getInstance(Locale.ROOT)`.
    ///
    /// Equivalent to `new DecimalFormat(String, DecimalFormatSymbols)`, which
    /// applies the (non-localized) pattern.
    ///
    /// # Errors
    ///
    /// `IllegalArgument` for a malformed pattern, as `applyPattern` throws.
    pub fn new(pattern: &str) -> crate::error::Result<Self> {
        let malformed = |what: &str| {
            crate::error::LuceneError::illegal_argument(format!("{what} \"{pattern}\""))
        };
        let mut df = Self {
            pattern: pattern.to_string(),
            positive_prefix: String::new(),
            positive_suffix: String::new(),
            negative_prefix: String::new(),
            negative_suffix: String::new(),
            multiplier: 1,
            grouping_used: false,
            grouping_size: 0,
            min_int: 0,
            max_int: MAXIMUM_DIGITS,
            min_frac: 0,
            max_frac: MAXIMUM_DIGITS,
            decimal_separator_always_shown: false,
            use_exponential: false,
            min_exponent_digits: 0,
        };
        if pattern.is_empty() {
            // No subpattern is read: the grouping keeps the defaults of the
            // fields, `NumberFormat.groupingUsed = true` and
            // `DecimalFormat.groupingSize = 3`.
            df.grouping_used = true;
            df.grouping_size = 3;
            df.negative_prefix = "-".to_string();
            return Ok(df);
        }
        let chars: Vec<char> = pattern.chars().collect();
        // The affixes as the pattern spells them (quotes resolved), compared
        // to decide whether the negative subpattern differs from the positive.
        let mut pos_affix_patterns = (String::new(), String::new());
        let mut start = 0usize;
        for j in [1, 0] {
            if j == 0 && start >= chars.len() {
                break;
            }
            let mut in_quote = false;
            let mut prefix = String::new();
            let mut suffix = String::new();
            let mut prefix_pattern = String::new();
            let mut suffix_pattern = String::new();
            let mut decimal_pos: i32 = -1;
            let mut multiplier = 1;
            let mut digit_left_count: i32 = 0;
            let mut zero_digit_count: i32 = 0;
            let mut digit_right_count: i32 = 0;
            // A Java `byte`, which wraps.
            let mut grouping_count: i8 = -1;
            let mut use_exponential = false;
            let mut min_exponent_digits = 0;
            // 0: prefix, 1: number, 2: suffix.
            let mut phase = 0;
            let mut pos = start;
            let mut next_start = chars.len();
            while pos < chars.len() {
                let ch = chars[pos];
                match phase {
                    0 | 2 => {
                        if in_quote {
                            if ch == '\'' {
                                if pos + 1 < chars.len() && chars[pos + 1] == '\'' {
                                    pos += 1;
                                    push_affix(phase, &mut prefix, &mut suffix, "'");
                                    push_affix(
                                        phase,
                                        &mut prefix_pattern,
                                        &mut suffix_pattern,
                                        "''",
                                    );
                                } else {
                                    in_quote = false;
                                }
                            } else {
                                let s = ch.to_string();
                                push_affix(phase, &mut prefix, &mut suffix, &s);
                                push_affix(phase, &mut prefix_pattern, &mut suffix_pattern, &s);
                            }
                            pos += 1;
                            continue;
                        }
                        // A digit character starts (or, after a suffix
                        // literal, resumes) the number part.
                        if matches!(ch, '#' | '0' | ',' | '.') {
                            phase = 1;
                            continue;
                        }
                        match ch {
                            '\u{a4}' => {
                                let doubled = pos + 1 < chars.len() && chars[pos + 1] == '\u{a4}';
                                if doubled {
                                    pos += 1;
                                }
                                let symbol = if doubled { "XXX" } else { "\u{a4}" };
                                push_affix(phase, &mut prefix, &mut suffix, symbol);
                                let spelled = if doubled { "'\u{a4}\u{a4}" } else { "'\u{a4}" };
                                push_affix(
                                    phase,
                                    &mut prefix_pattern,
                                    &mut suffix_pattern,
                                    spelled,
                                );
                            }
                            '\'' => {
                                if pos + 1 < chars.len() && chars[pos + 1] == '\'' {
                                    pos += 1;
                                    push_affix(phase, &mut prefix, &mut suffix, "'");
                                    push_affix(
                                        phase,
                                        &mut prefix_pattern,
                                        &mut suffix_pattern,
                                        "''",
                                    );
                                } else {
                                    in_quote = true;
                                }
                            }
                            ';' => {
                                if phase == 0 || j == 0 {
                                    return Err(malformed(&format!(
                                        "Unquoted special character '{ch}' in pattern"
                                    )));
                                }
                                next_start = pos + 1;
                                pos = chars.len();
                                continue;
                            }
                            '%' | '\u{2030}' => {
                                if multiplier != 1 {
                                    return Err(malformed(
                                        "Too many percent/per mille characters in pattern",
                                    ));
                                }
                                multiplier = if ch == '%' { 100 } else { 1000 };
                                let s = ch.to_string();
                                push_affix(phase, &mut prefix, &mut suffix, &s);
                                push_affix(
                                    phase,
                                    &mut prefix_pattern,
                                    &mut suffix_pattern,
                                    &format!("'{s}"),
                                );
                            }
                            '-' => {
                                push_affix(phase, &mut prefix, &mut suffix, "-");
                                push_affix(phase, &mut prefix_pattern, &mut suffix_pattern, "'-");
                            }
                            _ => {
                                let s = ch.to_string();
                                push_affix(phase, &mut prefix, &mut suffix, &s);
                                push_affix(phase, &mut prefix_pattern, &mut suffix_pattern, &s);
                            }
                        }
                        pos += 1;
                    }
                    _ if j == 0 => {
                        // The number part of the negative subpattern: only
                        // its extent matters, the digit counts come from the
                        // positive one.
                        if matches!(ch, '#' | '0' | ',' | '.' | 'E') {
                            pos += 1;
                        } else {
                            phase = 2;
                        }
                    }
                    _ => match ch {
                        '#' => {
                            if zero_digit_count > 0 {
                                digit_right_count += 1;
                            } else {
                                digit_left_count += 1;
                            }
                            if grouping_count >= 0 && decimal_pos < 0 {
                                grouping_count = grouping_count.wrapping_add(1);
                            }
                            pos += 1;
                        }
                        '0' => {
                            if digit_right_count > 0 {
                                return Err(malformed("Unexpected '0' in pattern"));
                            }
                            zero_digit_count += 1;
                            if grouping_count >= 0 && decimal_pos < 0 {
                                grouping_count = grouping_count.wrapping_add(1);
                            }
                            pos += 1;
                        }
                        ',' => {
                            grouping_count = 0;
                            pos += 1;
                        }
                        '.' => {
                            if decimal_pos >= 0 {
                                return Err(malformed("Multiple decimal separators in pattern"));
                            }
                            decimal_pos = digit_left_count + zero_digit_count + digit_right_count;
                            pos += 1;
                        }
                        'E' => {
                            if use_exponential {
                                return Err(malformed("Multiple exponential symbols in pattern"));
                            }
                            use_exponential = true;
                            min_exponent_digits = 0;
                            pos += 1;
                            while pos < chars.len() && chars[pos] == '0' {
                                min_exponent_digits += 1;
                                pos += 1;
                            }
                            if digit_left_count + zero_digit_count < 1 || min_exponent_digits < 1 {
                                return Err(malformed("Malformed exponential pattern"));
                            }
                            phase = 2;
                        }
                        _ => {
                            phase = 2;
                        }
                    },
                }
            }

            // Handle patterns with no '0' pattern character: "##.###" is
            // "#0.###", ".###" is ".0##".
            if zero_digit_count == 0 && digit_left_count > 0 && decimal_pos >= 0 {
                let mut n = decimal_pos;
                if n == 0 {
                    n += 1;
                }
                digit_right_count = digit_left_count - n;
                digit_left_count = n - 1;
                zero_digit_count = 1;
            }

            if (decimal_pos < 0 && digit_right_count > 0)
                || (decimal_pos >= 0
                    && (decimal_pos < digit_left_count
                        || decimal_pos > digit_left_count + zero_digit_count))
                || grouping_count == 0
                || in_quote
            {
                return Err(malformed("Malformed pattern"));
            }

            if j == 1 {
                df.positive_prefix = prefix;
                df.positive_suffix = suffix;
                pos_affix_patterns = (prefix_pattern, suffix_pattern);
                // The default negative affixes, replaced below by an explicit
                // negative subpattern.
                df.negative_prefix = format!("-{}", df.positive_prefix);
                df.negative_suffix = df.positive_suffix.clone();
                let digit_total_count = digit_left_count + zero_digit_count + digit_right_count;
                // The effective decimal position is the position the decimal
                // is at or would be at if there is no decimal.
                let effective_decimal_pos = if decimal_pos >= 0 {
                    decimal_pos
                } else {
                    digit_total_count
                };
                df.set_minimum_integer_digits(effective_decimal_pos - digit_left_count);
                df.set_maximum_integer_digits(if use_exponential {
                    digit_left_count + df.min_int
                } else {
                    MAXIMUM_DIGITS
                });
                df.set_maximum_fraction_digits(if decimal_pos >= 0 {
                    digit_total_count - decimal_pos
                } else {
                    0
                });
                df.set_minimum_fraction_digits(if decimal_pos >= 0 {
                    digit_left_count + zero_digit_count - decimal_pos
                } else {
                    0
                });
                df.grouping_used = grouping_count > 0;
                df.grouping_size = if grouping_count > 0 {
                    i32::from(grouping_count)
                } else {
                    0
                };
                df.multiplier = multiplier;
                df.decimal_separator_always_shown =
                    decimal_pos == 0 || decimal_pos == digit_total_count;
                df.use_exponential = use_exponential;
                df.min_exponent_digits = min_exponent_digits;
            } else {
                // An explicit negative subpattern whose affixes equal the
                // positive ones behaves as if there were none.
                if (prefix_pattern.as_str(), suffix_pattern.as_str())
                    != (pos_affix_patterns.0.as_str(), pos_affix_patterns.1.as_str())
                {
                    df.negative_prefix = prefix;
                    df.negative_suffix = suffix;
                }
            }
            start = next_start;
        }
        Ok(df)
    }

    /// `NumberFormat.setMinimumIntegerDigits(int)`.
    fn set_minimum_integer_digits(&mut self, value: i32) {
        self.min_int = value.max(0);
        if self.min_int > self.max_int {
            self.max_int = self.min_int;
        }
    }

    /// `NumberFormat.setMaximumIntegerDigits(int)`.
    fn set_maximum_integer_digits(&mut self, value: i32) {
        self.max_int = value.max(0);
        if self.min_int > self.max_int {
            self.min_int = self.max_int;
        }
    }

    /// `NumberFormat.setMinimumFractionDigits(int)`.
    fn set_minimum_fraction_digits(&mut self, value: i32) {
        self.min_frac = value.max(0);
        if self.max_frac < self.min_frac {
            self.max_frac = self.min_frac;
        }
    }

    /// `NumberFormat.setMaximumFractionDigits(int)`.
    fn set_maximum_fraction_digits(&mut self, value: i32) {
        self.max_frac = value.max(0);
        if self.min_frac > self.max_frac {
            self.min_frac = self.max_frac;
        }
    }

    /// The pattern this format was built from.
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// Formats `value`.
    ///
    /// Equivalent to `NumberFormat.format(double)`.
    pub fn format(&self, value: f64) -> String {
        if value.is_nan() {
            return "NaN".to_string();
        }
        let is_negative = value < 0.0 || (value == 0.0 && value.is_sign_negative());
        let mut number = value;
        if self.multiplier != 1 {
            number *= f64::from(self.multiplier);
        }
        let (prefix, suffix) = if is_negative {
            (&self.negative_prefix, &self.negative_suffix)
        } else {
            (&self.positive_prefix, &self.positive_suffix)
        };
        let mut result = String::new();
        result.push_str(prefix);
        if number.is_infinite() {
            result.push('\u{221e}');
            result.push_str(suffix);
            return result;
        }
        let number = number.abs();
        let max_int = self.max_int.min(DOUBLE_INTEGER_DIGITS);
        let min_int = self.min_int.min(DOUBLE_INTEGER_DIGITS);
        let max_frac = self.max_frac.min(DOUBLE_FRACTION_DIGITS);
        let min_frac = self.min_frac.min(DOUBLE_FRACTION_DIGITS);
        if self.use_exponential {
            let digits = DigitList::new(number, max_int.saturating_add(max_frac), false);
            self.subformat_exponential(&mut result, &digits, max_int, min_int, min_frac);
        } else {
            let digits = DigitList::new(number, max_frac, true);
            self.subformat_fixed(&mut result, &digits, max_int, min_int, max_frac, min_frac);
        }
        result.push_str(suffix);
        result
    }

    fn subformat_fixed(
        &self,
        result: &mut String,
        digits: &DigitList,
        max_int: i32,
        min_int: i32,
        max_frac: i32,
        min_frac: i32,
    ) {
        let decimal_at = digits.decimal_at;
        let count_digits = digits.digits.len() as i32;
        // Output the integer portion: at least `min_int` digits, all of the
        // digits before the decimal point, at most `max_int` (the highest
        // ones are dropped).
        let mut count = min_int;
        let mut digit_index: i32 = 0;
        if decimal_at > 0 && count < decimal_at {
            count = decimal_at;
        }
        if count > max_int {
            count = max_int;
            digit_index = decimal_at - count;
        }
        let size_before_integer_part = result.len();
        let mut i = count - 1;
        while i >= 0 {
            if i < decimal_at && digit_index < count_digits {
                result.push(digits.digits[digit_index as usize] as char);
                digit_index += 1;
            } else {
                result.push('0');
            }
            if self.grouping_used && i > 0 && self.grouping_size != 0 && i % self.grouping_size == 0
            {
                result.push(',');
            }
            i -= 1;
        }
        let fraction_present = min_frac > 0 || digit_index < count_digits;
        // No fraction and no integer digit printed: print a zero.
        if !fraction_present && result.len() == size_before_integer_part {
            result.push('0');
        }
        if self.decimal_separator_always_shown || fraction_present {
            result.push('.');
        }
        for i in 0..max_frac {
            if i >= min_frac && digit_index >= count_digits {
                break;
            }
            // Zeros between the decimal point and the first digit.
            if -1 - i > decimal_at - 1 {
                result.push('0');
                continue;
            }
            if digit_index < count_digits {
                result.push(digits.digits[digit_index as usize] as char);
                digit_index += 1;
            } else {
                result.push('0');
            }
        }
    }

    fn subformat_exponential(
        &self,
        result: &mut String,
        digits: &DigitList,
        max_int: i32,
        min_int: i32,
        min_frac: i32,
    ) {
        let is_zero = digits.digits.is_empty();
        let mut exponent = digits.decimal_at;
        let repeat = max_int;
        let mut minimum_integer_digits = min_int;
        if repeat > 1 && repeat > min_int {
            // A repeating range: the exponent is a multiple of `repeat`
            // (6,5,4 => 3; 3,2,1 => 0; 0,-1,-2 => -3; ...), the decimal
            // exponent here being that of the form 0.MMMMM x 10^n.
            exponent = if exponent >= 1 {
                ((exponent - 1) / repeat) * repeat
            } else {
                ((exponent - repeat) / repeat) * repeat
            };
            minimum_integer_digits = 1;
        } else {
            // No repeating range: the minimum integer digits are reached by
            // adjusting the exponent.
            exponent -= minimum_integer_digits;
        }
        let mut minimum_digits = min_int + min_frac;
        if minimum_digits < 0 {
            minimum_digits = i32::MAX;
        }
        let integer_digits = if is_zero {
            minimum_integer_digits
        } else {
            digits.decimal_at - exponent
        };
        if minimum_digits < integer_digits {
            minimum_digits = integer_digits;
        }
        let mut total_digits = digits.digits.len() as i32;
        if minimum_digits > total_digits {
            total_digits = minimum_digits;
        }
        for i in 0..total_digits {
            if i == integer_digits {
                result.push('.');
            }
            result.push(if (i as usize) < digits.digits.len() {
                digits.digits[i as usize] as char
            } else {
                '0'
            });
        }
        if self.decimal_separator_always_shown && total_digits == integer_digits {
            result.push('.');
        }
        // The exponent, with the minus sign but not the affixes.
        result.push('E');
        if is_zero {
            exponent = 0;
        }
        let negative_exponent = exponent < 0;
        if negative_exponent {
            exponent = -exponent;
            result.push('-');
        }
        let exponent_digits = exponent.to_string();
        for _ in exponent_digits.len() as i32..self.min_exponent_digits {
            result.push('0');
        }
        result.push_str(&exponent_digits);
    }
}

/// Appends `s` to the prefix in phase 0 and to the suffix otherwise.
fn push_affix(phase: i32, prefix: &mut String, suffix: &mut String, s: &str) {
    if phase == 0 {
        prefix.push_str(s);
    } else {
        suffix.push_str(s);
    }
}

/// The decimal digits of a non-negative finite `double`, as
/// `java.text.DigitList` holds them: `0.d1d2...dn x 10^decimal_at`, with no
/// trailing zero; no digits at all is zero.
struct DigitList {
    digits: Vec<u8>,
    decimal_at: i32,
}

impl DigitList {
    /// `DigitList.set(boolean, double, int, boolean)`
    /// (`DigitList.java:295-392`, read as specification): the decimal
    /// digits `FloatingDecimal`'s binary-to-ASCII converter produces for
    /// `value`, with its `digitsRoundedUp()` and `decimalDigitsExact()`
    /// flags, rounded half-even to `maximum_digits` fraction digits
    /// (`fixed_point`) or significant digits.
    fn new(value: f64, maximum_digits: i32, fixed_point: bool) -> Self {
        if value == 0.0 {
            return Self {
                digits: Vec::new(),
                decimal_at: 0,
            };
        }
        let converted = Converted::of(value);
        let mut list = Self {
            digits: converted.digits.clone(),
            decimal_at: converted.decimal_at,
        };
        if fixed_point {
            // Underflow of the printed representation: rounding something
            // like 0.0009 to 2 fraction digits.
            if -list.decimal_at > maximum_digits {
                list.digits.clear();
                return list;
            } else if -list.decimal_at == maximum_digits {
                // Rounding 0.0009 to 3 fraction digits creates a new digit.
                if list.should_round_up(0, &converted) {
                    list.digits = vec![b'1'];
                    list.decimal_at += 1;
                } else {
                    list.digits.clear();
                }
                return list;
            }
        }
        let limit = if fixed_point {
            maximum_digits + list.decimal_at
        } else {
            maximum_digits
        };
        list.round(limit, &converted);
        list
    }

    /// `DigitList.round(int, boolean, boolean)`: keeps at most
    /// `maximum_digits` digits, rounding up when
    /// [`should_round_up`](Self::should_round_up) says so.
    fn round(&mut self, maximum_digits: i32, converted: &Converted) {
        if maximum_digits >= 0 && (maximum_digits as usize) < self.digits.len() {
            let mut maximum_digits = maximum_digits;
            if self.should_round_up(maximum_digits as usize, converted) {
                // Increment from the least significant digit; all nines
                // become a single one with the decimal point moved.
                loop {
                    maximum_digits -= 1;
                    if maximum_digits < 0 {
                        self.digits[0] = b'1';
                        self.decimal_at += 1;
                        maximum_digits = 0;
                        break;
                    }
                    self.digits[maximum_digits as usize] += 1;
                    if self.digits[maximum_digits as usize] <= b'9' {
                        break;
                    }
                }
                maximum_digits += 1;
            }
            self.digits.truncate(maximum_digits as usize);
            while self.digits.len() > 1 && self.digits.last() == Some(&b'0') {
                self.digits.pop();
            }
        }
    }

    /// `DigitList.shouldRoundUp(int, boolean, boolean)` for
    /// `RoundingMode.HALF_EVEN`: a tie on the converter's last digit is
    /// decided by whether that digit was rounded up (then down), was not exact
    /// (then up), or is exact (then to the even neighbour).
    fn should_round_up(&self, maximum_digits: usize, converted: &Converted) -> bool {
        let count = self.digits.len();
        if maximum_digits >= count {
            return false;
        }
        let digit = self.digits[maximum_digits];
        if digit > b'5' {
            return true;
        }
        if digit == b'5' {
            if maximum_digits == count - 1 {
                if converted.rounded_up() {
                    return false;
                }
                if !converted.exact() {
                    return true;
                }
                return maximum_digits > 0
                    && !(self.digits[maximum_digits - 1] - b'0').is_multiple_of(2);
            }
            return self.digits[maximum_digits + 1..].iter().any(|&d| d != b'0');
        }
        false
    }
}

/// The output of `FloatingDecimal.getBinaryToASCIIConverter(double)` for a
/// positive finite value (`jdk.internal.math.FloatingDecimal`, JDK 21): its
/// digits and decimal position, and the two flags `DigitList` rounds with.
///
/// An integral value below 2^63 takes the converter's integer path
/// (`developLongDigits`): the integer's digits, its low digits beyond the 53
/// significant bits rounded half up away, and both flags false. Every other
/// value takes the general path of `BinaryToASCIIBuffer.dtoa`, the free-format
/// digit generation of Steele and White as the JDK implements it — which is
/// not always the shortest decimal (`1.0E23` renders `9.999999999999999E22`).
///
/// This is an independent implementation of that general path, written from
/// the JDK source read as a specification only: it reproduces the JDK's
/// decimal-exponent estimate, its choice between `int`, `long` and big-integer
/// arithmetic (whose stopping tests differ: `b + m > 10s` for the machine
/// integers, `B + M >= 10S` for the big integers), the forced second digit of
/// E-form values, the halfway rounding of the last digit, and the two flags:
/// `decimalDigitsExact()` (the remainder is zero when generation stops) and
/// `digitsRoundedUp()` (the last digit was incremented without a carry out of
/// the first digit).
struct Converted {
    digits: Vec<u8>,
    decimal_at: i32,
    exact: bool,
    rounded_up: bool,
}

/// `ceil(log2(5^i))` for `i >= 1`, `0` for `i == 0`, as the JDK's `N_5_BITS`
/// table holds it for `i < 27`, and its `i * 3` fallback beyond.
fn n_5_bits(i: i32) -> i32 {
    if i <= 0 {
        0
    } else if i < 27 {
        64 - 5u64.pow(i as u32).leading_zeros() as i32
    } else {
        i * 3
    }
}

/// `FloatingDecimal.estimateDecExp(long, int)`: `floor(log10(d))`, estimated
/// in `double` arithmetic from the significand and the binary exponent, as the
/// JDK computes it (the estimate may be one too high; the digit loop then
/// drops a leading zero).
// The JDK's own constants: `0.301029995663981` is its truncated `log10(2)`,
// not `std::f64::consts::LOG10_2`.
#[allow(clippy::approx_constant, clippy::excessive_precision)]
fn estimate_dec_exp(fract_bits: u64, bin_exp: i32) -> i32 {
    const SIGNIF_BIT_MASK: u64 = (1 << 52) - 1;
    let d2 = f64::from_bits(0x3FF0_0000_0000_0000 | (fract_bits & SIGNIF_BIT_MASK));
    let d = (d2 - 1.5) * 0.289529654 + 0.176091259 + f64::from(bin_exp) * 0.301029995663981;
    let d_bits = d.to_bits();
    let exponent = ((d_bits >> 52) & 0x7FF) as i32 - 1023;
    let is_negative = (d_bits >> 63) != 0;
    if (0..52).contains(&exponent) {
        let mask = SIGNIF_BIT_MASK >> exponent;
        let r = (((d_bits & SIGNIF_BIT_MASK) | (1 << 52)) >> (52 - exponent)) as i32;
        if is_negative {
            if mask & d_bits == 0 {
                -r
            } else {
                -r - 1
            }
        } else {
            r
        }
    } else if exponent < 0 {
        if d_bits & !(1u64 << 63) == 0 {
            0
        } else if is_negative {
            -1
        } else {
            0
        }
    } else {
        d as i32
    }
}

/// A minimal non-negative big integer (little-endian base-2^32 limbs) for the
/// big-integer branch of the digit generation.
#[derive(Clone, PartialEq, Eq)]
struct BigNat(Vec<u32>);

impl BigNat {
    fn from_u64(v: u64) -> Self {
        let mut n = BigNat(vec![v as u32, (v >> 32) as u32]);
        n.trim();
        n
    }

    fn trim(&mut self) {
        while self.0.last() == Some(&0) {
            self.0.pop();
        }
    }

    fn is_zero(&self) -> bool {
        self.0.is_empty()
    }

    fn mul_small(&mut self, m: u32) {
        let mut carry = 0u64;
        for limb in self.0.iter_mut() {
            let v = u64::from(*limb) * u64::from(m) + carry;
            *limb = v as u32;
            carry = v >> 32;
        }
        if carry != 0 {
            self.0.push(carry as u32);
        }
    }

    fn shl(&mut self, bits: u32) {
        if self.is_zero() || bits == 0 {
            return;
        }
        let words = (bits / 32) as usize;
        let rem = bits % 32;
        if rem != 0 {
            let mut carry = 0u32;
            for limb in self.0.iter_mut() {
                let v = (*limb << rem) | carry;
                carry = *limb >> (32 - rem);
                *limb = v;
            }
            if carry != 0 {
                self.0.push(carry);
            }
        }
        self.0.splice(0..0, std::iter::repeat_n(0, words));
    }

    /// `v * 5^p5 * 2^p2`.
    fn mul_pow52(v: u64, p5: i32, p2: i32) -> Self {
        let mut n = BigNat::from_u64(v);
        for _ in 0..p5 {
            n.mul_small(5);
        }
        n.shl(p2 as u32);
        n
    }

    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0
            .len()
            .cmp(&other.0.len())
            .then_with(|| self.0.iter().rev().cmp(other.0.iter().rev()))
    }

    fn add(&self, other: &Self) -> Self {
        let (long, short) = if self.0.len() >= other.0.len() {
            (self, other)
        } else {
            (other, self)
        };
        let mut out = Vec::with_capacity(long.0.len() + 1);
        let mut carry = 0u64;
        for (k, &limb) in long.0.iter().enumerate() {
            let v = u64::from(limb) + u64::from(short.0.get(k).copied().unwrap_or(0)) + carry;
            out.push(v as u32);
            carry = v >> 32;
        }
        if carry != 0 {
            out.push(carry as u32);
        }
        BigNat(out)
    }

    fn sub_assign(&mut self, other: &Self) {
        let mut borrow = 0i64;
        for k in 0..self.0.len() {
            let v = i64::from(self.0[k]) - i64::from(other.0.get(k).copied().unwrap_or(0)) - borrow;
            if v < 0 {
                self.0[k] = (v + (1i64 << 32)) as u32;
                borrow = 1;
            } else {
                self.0[k] = v as u32;
                borrow = 0;
            }
        }
        self.trim();
    }

    /// `FDBigInteger.quoRemIteration(S)`: returns `q = this / s` (a single
    /// decimal digit) and replaces `this` with `10 * (this % s)`.
    fn quo_rem_iteration(&mut self, s: &Self) -> u32 {
        let mut q = 0u32;
        while self.cmp(s) != std::cmp::Ordering::Less {
            self.sub_assign(s);
            q += 1;
        }
        self.mul_small(10);
        self.trim();
        q
    }
}

impl Converted {
    fn of(value: f64) -> Self {
        if value.fract() == 0.0 && (1.0..9_223_372_036_854_775_808.0).contains(&value) {
            let mut lvalue = value as u64;
            let bin_exp = 63 - lvalue.leading_zeros() as i32;
            let mut insignificant = 0u32;
            // `insignificantDigitsForPow2(binExp - nSignificantBits - 1)`.
            if bin_exp > 53 {
                let p2 = bin_exp - 54;
                if p2 > 1 {
                    let mut v = 1u64 << p2;
                    while v >= 10 {
                        v /= 10;
                        insignificant += 1;
                    }
                }
            }
            if insignificant != 0 {
                let pow10 = 10u64.pow(insignificant);
                let residue = lvalue % pow10;
                lvalue /= pow10;
                if residue >= (pow10 >> 1) {
                    lvalue += 1;
                }
            }
            let text = lvalue.to_string();
            let mut digits = text.into_bytes();
            let decimal_at = digits.len() as i32 + insignificant as i32;
            while digits.len() > 1 && digits.last() == Some(&b'0') {
                digits.pop();
            }
            return Self {
                digits,
                decimal_at,
                exact: false,
                rounded_up: false,
            };
        }
        Self::dtoa(value)
    }

    /// The general path of `BinaryToASCIIBuffer.dtoa(int, long, int,
    /// boolean)` with `isCompatibleFormat == true`, as
    /// `getBinaryToASCIIConverter(double)` calls it, for a positive finite
    /// non-integral (or out-of-`long`-range) value.
    fn dtoa(value: f64) -> Self {
        const EXP_SHIFT: i32 = 52;
        let bits = value.to_bits();
        let mut fract_bits = bits & ((1u64 << 52) - 1);
        let mut bin_exp = ((bits >> 52) & 0x7FF) as i32;
        let n_significant_bits;
        if bin_exp == 0 {
            // Normalize a subnormal.
            let leading_zeros = fract_bits.leading_zeros() as i32;
            let shift = leading_zeros - (63 - EXP_SHIFT);
            fract_bits <<= shift;
            bin_exp = 1 - shift;
            n_significant_bits = 64 - leading_zeros;
        } else {
            fract_bits |= 1u64 << EXP_SHIFT;
            n_significant_bits = EXP_SHIFT + 1;
        }
        bin_exp -= 1023;

        let tail_zeros = fract_bits.trailing_zeros() as i32;
        let n_fract_bits = EXP_SHIFT + 1 - tail_zeros;
        let n_tiny_bits = (n_fract_bits - bin_exp - 1).max(0);

        let mut dec_exp = estimate_dec_exp(fract_bits, bin_exp);
        let b5 = (-dec_exp).max(0);
        let mut b2 = b5 + n_tiny_bits + bin_exp;
        let s5 = dec_exp.max(0);
        let mut s2 = s5 + n_tiny_bits;
        let m5 = b5;
        let mut m2 = b2 - n_significant_bits;

        fract_bits >>= tail_zeros;
        b2 -= n_fract_bits - 1;
        let common2factor = b2.min(s2);
        b2 -= common2factor;
        s2 -= common2factor;
        m2 -= common2factor;
        // For an exact power of two the next smaller value is half as far.
        if n_fract_bits == 1 {
            m2 -= 1;
        }
        if m2 < 0 {
            b2 -= m2;
            s2 -= m2;
            m2 = 0;
        }

        let e_form = |dec_exp: i32| !(-3..8).contains(&dec_exp);
        let mut digits: Vec<u8> = Vec::with_capacity(20);
        let low;
        let high;
        let low_digit_difference: i64;
        let exact;

        let b_bits = n_fract_bits + b2 + n_5_bits(b5);
        let ten_s_bits = s2 + 1 + n_5_bits(s5 + 1);
        if b_bits < 64 && ten_s_bits < 64 {
            if b_bits < 32 && ten_s_bits < 32 {
                // `int` arithmetic, with Java's wrapping and shift masking.
                let pow5 = |p: i32| 5i32.wrapping_pow(p as u32);
                let mut b = (fract_bits as i32)
                    .wrapping_mul(pow5(b5))
                    .wrapping_shl(b2 as u32);
                let s = pow5(s5).wrapping_shl(s2 as u32);
                let mut m = pow5(m5).wrapping_shl(m2 as u32);
                let tens = s.wrapping_mul(10);
                let mut q = b / s;
                b = (b % s).wrapping_mul(10);
                m = m.wrapping_mul(10);
                let mut lo = b < m;
                let mut hi = b.wrapping_add(m) > tens;
                if q == 0 && !hi {
                    dec_exp -= 1;
                } else {
                    digits.push(b'0' + q as u8);
                }
                if e_form(dec_exp) {
                    hi = false;
                    lo = false;
                }
                while !lo && !hi {
                    q = b / s;
                    b = (b % s).wrapping_mul(10);
                    m = m.wrapping_mul(10);
                    if m > 0 {
                        lo = b < m;
                        hi = b.wrapping_add(m) > tens;
                    } else {
                        lo = true;
                        hi = true;
                    }
                    digits.push(b'0' + q as u8);
                }
                low = lo;
                high = hi;
                low_digit_difference = i64::from(b.wrapping_shl(1).wrapping_sub(tens));
                exact = b == 0;
            } else {
                // `long` arithmetic.
                let pow5 = |p: i32| 5i64.wrapping_pow(p as u32);
                let mut b = (fract_bits as i64)
                    .wrapping_mul(pow5(b5))
                    .wrapping_shl(b2 as u32);
                let s = pow5(s5).wrapping_shl(s2 as u32);
                let mut m = pow5(m5).wrapping_shl(m2 as u32);
                let tens = s.wrapping_mul(10);
                let mut q = b / s;
                b = (b % s).wrapping_mul(10);
                m = m.wrapping_mul(10);
                let mut lo = b < m;
                let mut hi = b.wrapping_add(m) > tens;
                if q == 0 && !hi {
                    dec_exp -= 1;
                } else {
                    digits.push(b'0' + q as u8);
                }
                if e_form(dec_exp) {
                    hi = false;
                    lo = false;
                }
                while !lo && !hi {
                    q = b / s;
                    b = (b % s).wrapping_mul(10);
                    m = m.wrapping_mul(10);
                    if m > 0 {
                        lo = b < m;
                        hi = b.wrapping_add(m) > tens;
                    } else {
                        lo = true;
                        hi = true;
                    }
                    digits.push(b'0' + q as u8);
                }
                low = lo;
                high = hi;
                low_digit_difference = b.wrapping_shl(1).wrapping_sub(tens);
                exact = b == 0;
            }
        } else {
            // Big-integer arithmetic. The JDK normalizes `S` (and scales the
            // others alike) only to speed its division up; the comparisons and
            // quotients are scale-invariant.
            let s_val = BigNat::mul_pow52(1, s5, s2);
            let mut b_val = BigNat::mul_pow52(fract_bits, b5, b2);
            let mut m_val = BigNat::mul_pow52(1, m5 + 1, m2 + 1);
            let ten_s_val = BigNat::mul_pow52(1, s5 + 1, s2 + 1);
            let mut q = b_val.quo_rem_iteration(&s_val);
            let mut lo = b_val.cmp(&m_val).is_lt();
            let mut hi = ten_s_val.cmp(&b_val.add(&m_val)).is_le();
            if q == 0 && !hi {
                dec_exp -= 1;
            } else {
                digits.push(b'0' + q as u8);
            }
            if e_form(dec_exp) {
                hi = false;
                lo = false;
            }
            while !lo && !hi {
                q = b_val.quo_rem_iteration(&s_val);
                m_val.mul_small(10);
                lo = b_val.cmp(&m_val).is_lt();
                hi = ten_s_val.cmp(&b_val.add(&m_val)).is_le();
                digits.push(b'0' + q as u8);
            }
            low = lo;
            high = hi;
            low_digit_difference = if hi && lo {
                let mut twice = b_val.clone();
                twice.shl(1);
                match twice.cmp(&ten_s_val) {
                    std::cmp::Ordering::Less => -1,
                    std::cmp::Ordering::Equal => 0,
                    std::cmp::Ordering::Greater => 1,
                }
            } else {
                0
            };
            exact = b_val.is_zero();
        }

        let mut dec_exponent = dec_exp + 1;
        let mut rounded_up = false;
        // The last digit is rounded on the stopping condition.
        let round_up = high
            && (!low
                || low_digit_difference > 0
                || (low_digit_difference == 0 && (digits[digits.len() - 1] & 1) != 0));
        if round_up {
            // `roundup()`: a carry out of the first digit makes it `1` with a
            // larger exponent and leaves `digitsRoundedUp()` false.
            let mut i = digits.len() - 1;
            let mut q = digits[i];
            let mut carried_out = false;
            if q == b'9' {
                while q == b'9' && i > 0 {
                    digits[i] = b'0';
                    i -= 1;
                    q = digits[i];
                }
                if q == b'9' {
                    dec_exponent += 1;
                    digits[0] = b'1';
                    carried_out = true;
                }
            }
            if !carried_out {
                digits[i] = q + 1;
                rounded_up = true;
            }
        }
        // `DigitList.set` reads the digits back from `toJavaFormatString()`,
        // skipping a leading zero (a first quotient of zero kept because the
        // stopping test already held) into the decimal position, and drops
        // trailing zeros.
        while digits.len() > 1 && digits[0] == b'0' {
            digits.remove(0);
            dec_exponent -= 1;
        }
        while digits.len() > 1 && digits.last() == Some(&b'0') {
            digits.pop();
        }
        Self {
            digits,
            decimal_at: dec_exponent,
            exact,
            rounded_up,
        }
    }

    /// `decimalDigitsExact()`.
    fn exact(&self) -> bool {
        self.exact
    }

    /// `digitsRoundedUp()`.
    fn rounded_up(&self) -> bool {
        self.rounded_up
    }
}
