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

/// A `java.text.DecimalFormat` under `Locale.ROOT` symbols: the subset of its
/// pattern syntax and rounding (`RoundingMode.HALF_EVEN`) that Lucene's callers
/// pass, implemented from the JDK specification.
///
/// Equivalent to `java.text.DecimalFormat`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecimalFormat {
    /// The pattern, as `DecimalFormat.toPattern()` returns it.
    pattern: String,
}

impl DecimalFormat {
    /// Creates a format from `pattern` with `DecimalFormatSymbols.getInstance(Locale.ROOT)`.
    ///
    /// Equivalent to `new DecimalFormat(String, DecimalFormatSymbols)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgument` for a malformed pattern, as `applyPattern` throws.
    pub fn new(pattern: &str) -> crate::error::Result<Self> {
        let _ = pattern;
        todo!("PORT G-108")
    }

    /// Formats `value`.
    ///
    /// Equivalent to `NumberFormat.format(double)`.
    pub fn format(&self, value: f64) -> String {
        let _ = value;
        todo!("PORT G-108")
    }
}
