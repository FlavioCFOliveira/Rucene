// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! Reproductions of `java.math` (JDK 25): [`BigInteger`], [`BigDecimal`],
//! [`MathContext`] and [`RoundingMode`], limited to the members the ICU4J
//! number-formatting classes of this crate call (37 members of
//! `BigDecimal`, 16 of `BigInteger`, 6 of `MathContext`, 11 of
//! `RoundingMode`, read from the bytecode of the closure, rmp task #651),
//! plus the members those members are defined through.
//!
//! The JDK 25 sources (`java/math/BigDecimal.java`, `BigInteger.java`,
//! `MathContext.java`, `RoundingMode.java`) are the specification; the code
//! is written for this crate. Every result, scale, exception class and
//! exception message is the JDK's.
//!
//! **Divergence.** The JDK keeps a `BigDecimal` either as a `long`
//! (`intCompact`) or as a `BigInteger` (`intVal`) and has a separate fast
//! path for each representation; each fast path computes the same value as
//! the `BigInteger` path (the JDK relies on that). This port keeps only the
//! `BigInteger` representation and follows the `BigInteger` paths
//! (`BigDecimal.java` `divide(BigInteger, int, BigInteger, int, long,
//! MathContext)`, `divideAndRound(BigInteger, BigInteger, int, int, int)`,
//! `doRound(BigInteger, int, MathContext)`, `add(BigInteger, int,
//! BigInteger, int)`), so results are identical; the cached `precision`
//! and `stringCache` fields are recomputed on demand. `BigInteger`
//! multiplication and division use the schoolbook and Knuth algorithms
//! instead of the JDK's Karatsuba, Toom-Cook and Burnikel-Ziegler
//! thresholds, which change only the running time: the quotient,
//! remainder and product of two integers are unique.

use std::cmp::Ordering;
use std::fmt;

use rucene::error::{LuceneError, Result};

use crate::jdk::{arithmetic, character_digit, to_rust_string};

// ---------------------------------------------------------------------------
// RoundingMode
// ---------------------------------------------------------------------------

/// The rounding behaviour of the operations that can discard precision.
///
/// Equivalent to the enum `java.math.RoundingMode`; the declaration order
/// (the `ordinal()`) is the JDK's, and so is the legacy `BigDecimal.ROUND_*`
/// constant each one maps to (`oldMode`, equal to the ordinal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RoundingMode {
    /// `UP`: away from zero.
    Up,
    /// `DOWN`: towards zero.
    Down,
    /// `CEILING`: towards positive infinity.
    Ceiling,
    /// `FLOOR`: towards negative infinity.
    Floor,
    /// `HALF_UP`: to the nearest neighbour, ties away from zero.
    HalfUp,
    /// `HALF_DOWN`: to the nearest neighbour, ties towards zero.
    HalfDown,
    /// `HALF_EVEN`: to the nearest neighbour, ties to the even neighbour.
    HalfEven,
    /// `UNNECESSARY`: the result is exact, or `ArithmeticException`.
    Unnecessary,
}

impl RoundingMode {
    const VALUES: [RoundingMode; 8] = [
        RoundingMode::Up,
        RoundingMode::Down,
        RoundingMode::Ceiling,
        RoundingMode::Floor,
        RoundingMode::HalfUp,
        RoundingMode::HalfDown,
        RoundingMode::HalfEven,
        RoundingMode::Unnecessary,
    ];

    /// `RoundingMode.values()`: the constants in declaration order.
    pub fn values() -> [RoundingMode; 8] {
        Self::VALUES
    }

    /// `ordinal()`.
    pub fn ordinal(self) -> i32 {
        self as i32
    }

    /// The `BigDecimal.ROUND_*` constant of this mode (`oldMode`).
    fn old_mode(self) -> i32 {
        self as i32
    }

    /// `RoundingMode.valueOf(int rm)`: the mode of a legacy
    /// `BigDecimal.ROUND_*` constant.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("argument out of range")` when `rm` is
    /// outside `0..=7`.
    pub fn value_of(rm: i32) -> Result<RoundingMode> {
        if (0..8).contains(&rm) {
            Ok(Self::VALUES[rm as usize])
        } else {
            Err(LuceneError::illegal_argument("argument out of range"))
        }
    }

    /// `name()` (and `toString()`): the constant's name.
    pub fn name(self) -> &'static str {
        match self {
            RoundingMode::Up => "UP",
            RoundingMode::Down => "DOWN",
            RoundingMode::Ceiling => "CEILING",
            RoundingMode::Floor => "FLOOR",
            RoundingMode::HalfUp => "HALF_UP",
            RoundingMode::HalfDown => "HALF_DOWN",
            RoundingMode::HalfEven => "HALF_EVEN",
            RoundingMode::Unnecessary => "UNNECESSARY",
        }
    }
}

impl fmt::Display for RoundingMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

const ROUND_UP: i32 = 0;
const ROUND_DOWN: i32 = 1;
const ROUND_CEILING: i32 = 2;
const ROUND_FLOOR: i32 = 3;
const ROUND_HALF_UP: i32 = 4;
const ROUND_HALF_DOWN: i32 = 5;
const ROUND_HALF_EVEN: i32 = 6;
const ROUND_UNNECESSARY: i32 = 7;

// ---------------------------------------------------------------------------
// MathContext
// ---------------------------------------------------------------------------

/// A precision (number of significant digits, 0 for unlimited) and a
/// rounding mode.
///
/// Equivalent to `java.math.MathContext` (immutable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MathContext {
    precision: i32,
    rounding_mode: RoundingMode,
}

impl MathContext {
    /// `MathContext.UNLIMITED`: precision 0, `HALF_UP`.
    pub const UNLIMITED: MathContext = MathContext { precision: 0, rounding_mode: RoundingMode::HalfUp };

    /// `MathContext.DECIMAL32`: 7 digits, `HALF_EVEN`.
    pub const DECIMAL32: MathContext = MathContext { precision: 7, rounding_mode: RoundingMode::HalfEven };

    /// `MathContext.DECIMAL64`: 16 digits, `HALF_EVEN`.
    pub const DECIMAL64: MathContext = MathContext { precision: 16, rounding_mode: RoundingMode::HalfEven };

    /// `MathContext.DECIMAL128`: 34 digits, `HALF_EVEN`.
    pub const DECIMAL128: MathContext = MathContext { precision: 34, rounding_mode: RoundingMode::HalfEven };

    /// `new MathContext(int setPrecision)`: `HALF_UP` rounding.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Digits < 0")` when `set_precision` is
    /// negative.
    pub fn new(set_precision: i32) -> Result<Self> {
        Self::with_rounding_mode(set_precision, RoundingMode::HalfUp)
    }

    /// `new MathContext(int setPrecision, RoundingMode setRoundingMode)`
    /// (the `null` rounding mode's `NullPointerException` has no
    /// counterpart: the Rust enum cannot be null).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Digits < 0")` when `set_precision` is
    /// negative.
    pub fn with_rounding_mode(set_precision: i32, set_rounding_mode: RoundingMode) -> Result<Self> {
        if set_precision < 0 {
            return Err(LuceneError::illegal_argument("Digits < 0"));
        }
        Ok(Self { precision: set_precision, rounding_mode: set_rounding_mode })
    }

    /// `getPrecision()`.
    pub fn get_precision(&self) -> i32 {
        self.precision
    }

    /// `getRoundingMode()`.
    pub fn get_rounding_mode(&self) -> RoundingMode {
        self.rounding_mode
    }

    /// `equals(Object x)`: same precision and rounding mode (the derived
    /// `PartialEq`).
    pub fn java_equals(&self, other: &MathContext) -> bool {
        self == other
    }
}

impl fmt::Display for MathContext {
    /// `toString()`: `precision=<p> roundingMode=<mode>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "precision={} roundingMode={}", self.precision, self.rounding_mode)
    }
}

// ---------------------------------------------------------------------------
// Magnitude arithmetic (little-endian 32-bit words, no high zero words)
// ---------------------------------------------------------------------------

fn trim(v: &mut Vec<u32>) {
    while v.last() == Some(&0) {
        v.pop();
    }
}

fn mag_cmp(a: &[u32], b: &[u32]) -> Ordering {
    if a.len() != b.len() {
        return a.len().cmp(&b.len());
    }
    for i in (0..a.len()).rev() {
        if a[i] != b[i] {
            return a[i].cmp(&b[i]);
        }
    }
    Ordering::Equal
}

fn mag_add(a: &[u32], b: &[u32]) -> Vec<u32> {
    let (long, short) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    let mut r = Vec::with_capacity(long.len() + 1);
    let mut carry = 0u64;
    for i in 0..long.len() {
        let s = long[i] as u64 + if i < short.len() { short[i] as u64 } else { 0 } + carry;
        r.push(s as u32);
        carry = s >> 32;
    }
    if carry != 0 {
        r.push(carry as u32);
    }
    r
}

/// `a - b`, where `a >= b`.
fn mag_sub(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut r = Vec::with_capacity(a.len());
    let mut borrow = 0i64;
    for i in 0..a.len() {
        let mut d = a[i] as i64 - borrow - if i < b.len() { b[i] as i64 } else { 0 };
        if d < 0 {
            d += 1 << 32;
            borrow = 1;
        } else {
            borrow = 0;
        }
        r.push(d as u32);
    }
    trim(&mut r);
    r
}

fn mag_mul(a: &[u32], b: &[u32]) -> Vec<u32> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut r = vec![0u32; a.len() + b.len()];
    for (i, &x) in a.iter().enumerate() {
        let mut carry = 0u64;
        for (j, &y) in b.iter().enumerate() {
            let t = x as u64 * y as u64 + r[i + j] as u64 + carry;
            r[i + j] = t as u32;
            carry = t >> 32;
        }
        let mut k = i + b.len();
        while carry != 0 {
            let t = r[k] as u64 + carry;
            r[k] = t as u32;
            carry = t >> 32;
            k += 1;
        }
    }
    trim(&mut r);
    r
}

fn mag_mul_add_small(a: &mut Vec<u32>, mul: u32, add: u32) {
    let mut carry = add as u64;
    for w in a.iter_mut() {
        let t = *w as u64 * mul as u64 + carry;
        *w = t as u32;
        carry = t >> 32;
    }
    if carry != 0 {
        a.push(carry as u32);
    }
    trim(a);
}

/// `(a / d, a % d)` for a one-word divisor.
fn mag_divmod_small(a: &[u32], d: u32) -> (Vec<u32>, u32) {
    let mut q = vec![0u32; a.len()];
    let mut rem = 0u64;
    for i in (0..a.len()).rev() {
        let cur = (rem << 32) | a[i] as u64;
        q[i] = (cur / d as u64) as u32;
        rem = cur % d as u64;
    }
    trim(&mut q);
    (q, rem as u32)
}

fn mag_shl(a: &[u32], n: u32) -> Vec<u32> {
    if a.is_empty() {
        return Vec::new();
    }
    let words = (n / 32) as usize;
    let bits = n % 32;
    let mut r = vec![0u32; words];
    if bits == 0 {
        r.extend_from_slice(a);
    } else {
        let mut carry = 0u32;
        for &w in a {
            r.push((w << bits) | carry);
            carry = w >> (32 - bits);
        }
        if carry != 0 {
            r.push(carry);
        }
    }
    trim(&mut r);
    r
}

fn mag_shr(a: &[u32], n: u32) -> Vec<u32> {
    let words = (n / 32) as usize;
    if words >= a.len() {
        return Vec::new();
    }
    let bits = n % 32;
    let mut r: Vec<u32> = a[words..].to_vec();
    if bits != 0 {
        for i in 0..r.len() {
            let hi = if i + 1 < r.len() { r[i + 1] << (32 - bits) } else { 0 };
            r[i] = (r[i] >> bits) | hi;
        }
    }
    trim(&mut r);
    r
}

/// `(a / b, a % b)`, `b` non-zero (Knuth, TAOCP vol. 2, 4.3.1, algorithm D).
fn mag_divmod(a: &[u32], b: &[u32]) -> (Vec<u32>, Vec<u32>) {
    if mag_cmp(a, b) == Ordering::Less {
        return (Vec::new(), a.to_vec());
    }
    if b.len() == 1 {
        let (q, r) = mag_divmod_small(a, b[0]);
        return (q, if r == 0 { Vec::new() } else { vec![r] });
    }
    let s = b[b.len() - 1].leading_zeros();
    let bn = mag_shl(b, s);
    let mut an = mag_shl(a, s);
    while an.len() < a.len() + 1 {
        an.push(0);
    }
    let n = bn.len();
    let m = an.len() - n;
    let mut q = vec![0u32; m];
    let b1 = bn[n - 1] as u64;
    let b2 = bn[n - 2] as u64;
    for j in (0..m).rev() {
        let num = ((an[j + n] as u64) << 32) | an[j + n - 1] as u64;
        let mut qhat = num / b1;
        let mut rhat = num % b1;
        while qhat >= (1 << 32) || qhat * b2 > ((rhat << 32) | an[j + n - 2] as u64) {
            qhat -= 1;
            rhat += b1;
            if rhat >= (1 << 32) {
                break;
            }
        }
        let mut borrow = 0i64;
        let mut carry = 0u64;
        for i in 0..n {
            let p = qhat * bn[i] as u64 + carry;
            carry = p >> 32;
            let t = an[i + j] as i64 - borrow - (p & 0xFFFF_FFFF) as i64;
            an[i + j] = t as u32;
            borrow = if t < 0 { 1 } else { 0 };
        }
        let t = an[j + n] as i64 - borrow - carry as i64;
        an[j + n] = t as u32;
        if t < 0 {
            qhat -= 1;
            let mut c = 0u64;
            for i in 0..n {
                let s2 = an[i + j] as u64 + bn[i] as u64 + c;
                an[i + j] = s2 as u32;
                c = s2 >> 32;
            }
            an[j + n] = an[j + n].wrapping_add(c as u32);
        }
        q[j] = qhat as u32;
    }
    trim(&mut q);
    an.truncate(n);
    trim(&mut an);
    (q, mag_shr(&an, s))
}

fn mag_from_u64(v: u64) -> Vec<u32> {
    let mut r = vec![v as u32, (v >> 32) as u32];
    trim(&mut r);
    r
}

fn mag_bit_length(a: &[u32]) -> i32 {
    match a.last() {
        None => 0,
        Some(&top) => ((a.len() - 1) * 32) as i32 + (32 - top.leading_zeros() as i32),
    }
}

// ---------------------------------------------------------------------------
// BigInteger
// ---------------------------------------------------------------------------

/// An immutable arbitrary-precision integer.
///
/// Equivalent to `java.math.BigInteger`: a sign (`signum`, -1, 0 or 1) and
/// a magnitude. The derived equality is `equals(Object)` (same sign and
/// magnitude).
///
/// **Divergence.** The JDK stores the magnitude as a big-endian `int[]`;
/// this port stores it little-endian (least significant word first), which
/// is an internal layout only: [`hash_code`](Self::hash_code) walks it in
/// the JDK's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BigInteger {
    signum: i32,
    mag: Vec<u32>,
}

impl BigInteger {
    /// `BigInteger.ZERO`.
    pub fn zero() -> Self {
        Self { signum: 0, mag: Vec::new() }
    }

    /// `BigInteger.ONE`.
    pub fn one() -> Self {
        Self::value_of(1)
    }

    /// `BigInteger.TEN`.
    pub fn ten() -> Self {
        Self::value_of(10)
    }

    fn from_parts(signum: i32, mut mag: Vec<u32>) -> Self {
        trim(&mut mag);
        if mag.is_empty() {
            Self::zero()
        } else {
            Self { signum, mag }
        }
    }

    /// `BigInteger.valueOf(long val)`.
    pub fn value_of(val: i64) -> Self {
        match val.cmp(&0) {
            Ordering::Equal => Self::zero(),
            Ordering::Greater => Self::from_parts(1, mag_from_u64(val as u64)),
            Ordering::Less => Self::from_parts(-1, mag_from_u64(val.unsigned_abs())),
        }
    }

    /// `new BigInteger(String val)`: the decimal string representation,
    /// with an optional leading sign.
    ///
    /// # Errors
    ///
    /// `NumberFormatException` with the JDK's messages: `Zero length
    /// BigInteger`, `Illegal embedded sign character`, `Illegal digit`, or
    /// `Integer.parseInt`'s `For input string: "<group>"` for a group of
    /// digits holding a character that is not a digit.
    pub fn from_string(val: &[u16]) -> Result<Self> {
        Self::from_string_radix(val, 10)
    }

    /// `new BigInteger(String val, int radix)`.
    ///
    /// # Errors
    ///
    /// As [`from_string`](Self::from_string), plus `NumberFormatException("Radix
    /// out of range")` when `radix` is outside `2..=36`.
    pub fn from_string_radix(val: &[u16], radix: i32) -> Result<Self> {
        let len = val.len();
        if !(2..=36).contains(&radix) {
            return Err(LuceneError::number_format("Radix out of range"));
        }
        if len == 0 {
            return Err(LuceneError::number_format("Zero length BigInteger"));
        }
        let mut cursor = 0usize;
        let mut sign = 1;
        let index1 = val.iter().rposition(|&c| c == b'-' as u16);
        let index2 = val.iter().rposition(|&c| c == b'+' as u16);
        if let Some(i1) = index1 {
            if i1 != 0 || index2.is_some() {
                return Err(LuceneError::number_format("Illegal embedded sign character"));
            }
            sign = -1;
            cursor = 1;
        } else if let Some(i2) = index2 {
            if i2 != 0 {
                return Err(LuceneError::number_format("Illegal embedded sign character"));
            }
            cursor = 1;
        }
        if cursor == len {
            return Err(LuceneError::number_format("Zero length BigInteger"));
        }
        while cursor < len && character_digit(val[cursor] as i32, radix) == 0 {
            cursor += 1;
        }
        if cursor == len {
            return Ok(Self::zero());
        }
        let num_digits = len - cursor;
        let digits_per_int = DIGITS_PER_INT[radix as usize] as usize;
        let mut first_group_len = num_digits % digits_per_int;
        if first_group_len == 0 {
            first_group_len = digits_per_int;
        }
        let group = &val[cursor..cursor + first_group_len];
        cursor += first_group_len;
        let first = crate::jdk::parse_int(group, radix)?;
        if first < 0 {
            return Err(LuceneError::number_format("Illegal digit"));
        }
        let mut mag = mag_from_u64(first as u64);
        let super_radix = INT_RADIX[radix as usize];
        while cursor < len {
            let group = &val[cursor..cursor + digits_per_int];
            cursor += digits_per_int;
            let group_val = crate::jdk::parse_int(group, radix)?;
            if group_val < 0 {
                return Err(LuceneError::number_format("Illegal digit"));
            }
            mag_mul_add_small(&mut mag, super_radix, group_val as u32);
        }
        Ok(Self::from_parts(sign, mag))
    }

    /// `signum()`.
    pub fn signum(&self) -> i32 {
        self.signum
    }

    /// `negate()`.
    pub fn negate(&self) -> Self {
        Self { signum: -self.signum, mag: self.mag.clone() }
    }

    /// `abs()`.
    pub fn abs(&self) -> Self {
        if self.signum >= 0 {
            self.clone()
        } else {
            self.negate()
        }
    }

    /// `add(BigInteger val)`.
    pub fn add(&self, val: &BigInteger) -> Self {
        if val.signum == 0 {
            return self.clone();
        }
        if self.signum == 0 {
            return val.clone();
        }
        if val.signum == self.signum {
            return Self::from_parts(self.signum, mag_add(&self.mag, &val.mag));
        }
        match mag_cmp(&self.mag, &val.mag) {
            Ordering::Equal => Self::zero(),
            Ordering::Greater => Self::from_parts(self.signum, mag_sub(&self.mag, &val.mag)),
            Ordering::Less => Self::from_parts(-self.signum, mag_sub(&val.mag, &self.mag)),
        }
    }

    /// `subtract(BigInteger val)`.
    pub fn subtract(&self, val: &BigInteger) -> Self {
        self.add(&val.negate())
    }

    /// `multiply(BigInteger val)`.
    pub fn multiply(&self, val: &BigInteger) -> Self {
        if self.signum == 0 || val.signum == 0 {
            return Self::zero();
        }
        Self::from_parts(self.signum * val.signum, mag_mul(&self.mag, &val.mag))
    }

    fn multiply_long(&self, v: i64) -> Self {
        self.multiply(&BigInteger::value_of(v))
    }

    /// `divideAndRemainder(BigInteger val)`: `[this / val, this % val]`,
    /// the quotient truncated towards zero and the remainder with the sign
    /// of `this`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException("BigInteger divide by zero")` when `val` is
    /// zero.
    pub fn divide_and_remainder(&self, val: &BigInteger) -> Result<[BigInteger; 2]> {
        if val.signum == 0 {
            return Err(arithmetic("BigInteger divide by zero"));
        }
        let (q, r) = mag_divmod(&self.mag, &val.mag);
        Ok([Self::from_parts(self.signum * val.signum, q), Self::from_parts(self.signum, r)])
    }

    /// `pow(int exponent)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException("Negative exponent")` when `exponent` is
    /// negative.
    pub fn pow(&self, exponent: i32) -> Result<Self> {
        if exponent < 0 {
            return Err(arithmetic("Negative exponent"));
        }
        let mut result = BigInteger::one();
        let mut base = self.clone();
        let mut e = exponent as u32;
        while e != 0 {
            if e & 1 != 0 {
                result = result.multiply(&base);
            }
            e >>= 1;
            if e != 0 {
                base = base.multiply(&base);
            }
        }
        Ok(result)
    }

    /// `compareTo(BigInteger val)`.
    pub fn compare_to(&self, val: &BigInteger) -> i32 {
        if self.signum != val.signum {
            return if self.signum > val.signum { 1 } else { -1 };
        }
        let c = match self.signum {
            1 => mag_cmp(&self.mag, &val.mag),
            -1 => mag_cmp(&val.mag, &self.mag),
            _ => Ordering::Equal,
        };
        c as i32
    }

    /// `compareMagnitude(BigInteger val)`.
    fn compare_magnitude(&self, val: &BigInteger) -> i32 {
        mag_cmp(&self.mag, &val.mag) as i32
    }

    /// `bitLength()`: the number of bits of the minimal two's-complement
    /// representation, excluding the sign bit.
    pub fn bit_length(&self) -> i32 {
        let n = mag_bit_length(&self.mag);
        if self.signum < 0 {
            // A negative power of two needs one bit fewer.
            let pow2 = self.mag.last().is_some_and(|t| t.is_power_of_two())
                && self.mag[..self.mag.len() - 1].iter().all(|&w| w == 0);
            if pow2 {
                return n - 1;
            }
        }
        n
    }

    /// The low 64 bits of the two's-complement representation.
    fn low_bits(&self) -> u64 {
        let lo = self.mag.first().copied().unwrap_or(0) as u64 | (self.mag.get(1).copied().unwrap_or(0) as u64) << 32;
        if self.signum < 0 {
            lo.wrapping_neg()
        } else {
            lo
        }
    }

    /// `intValue()`: the low 32 bits of the two's-complement
    /// representation.
    pub fn int_value(&self) -> i32 {
        self.low_bits() as i32
    }

    /// `longValue()`: the low 64 bits of the two's-complement
    /// representation.
    pub fn long_value(&self) -> i64 {
        self.low_bits() as i64
    }

    /// `byteValue()` (`Number.byteValue()`: `(byte) intValue()`).
    pub fn byte_value(&self) -> i8 {
        self.int_value() as i8
    }

    /// `doubleValue()`: the correctly rounded `double` of this integer
    /// (infinite when out of range).
    pub fn double_value(&self) -> f64 {
        self.to_string_radix(10).parse::<f64>().unwrap_or(0.0)
    }

    /// `hashCode()`: `ArraysSupport.hashCode` over the big-endian
    /// magnitude words, times the sign.
    pub fn hash_code(&self) -> i32 {
        let mut h: i32 = 0;
        for &w in self.mag.iter().rev() {
            h = (h.wrapping_mul(31) as i64 + w as i64) as i32;
        }
        h.wrapping_mul(self.signum)
    }

    /// `equals(Object x)` (the derived `PartialEq`).
    pub fn java_equals(&self, x: &BigInteger) -> bool {
        self == x
    }

    /// `toString(int radix)`: lowercase digits, a leading `-` when
    /// negative; a radix outside `2..=36` is replaced by 10.
    pub fn to_string_radix(&self, radix: i32) -> String {
        if self.signum == 0 {
            return "0".to_string();
        }
        let radix = if (2..=36).contains(&radix) { radix } else { 10 };
        let mut digits = Vec::new();
        let mut mag = self.mag.clone();
        let per = DIGITS_PER_INT[radix as usize] as usize;
        let chunk = INT_RADIX[radix as usize];
        while !mag.is_empty() {
            let (q, mut r) = mag_divmod_small(&mag, chunk);
            mag = q;
            for _ in 0..per {
                digits.push(r % radix as u32);
                r /= radix as u32;
                if mag.is_empty() && r == 0 {
                    break;
                }
            }
        }
        while digits.len() > 1 && digits.last() == Some(&0) {
            digits.pop();
        }
        let mut s = String::with_capacity(digits.len() + 1);
        if self.signum < 0 {
            s.push('-');
        }
        for &d in digits.iter().rev() {
            s.push(char::from_digit(d, radix as u32).unwrap_or('?'));
        }
        s
    }
}

impl fmt::Display for BigInteger {
    /// `toString()`: radix 10.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_radix(10))
    }
}

/// `BigInteger.digitsPerInt`: the digits of a radix that fit in an `int`.
const DIGITS_PER_INT: [i32; 37] =
    [0, 0, 30, 19, 15, 13, 11, 11, 10, 9, 9, 8, 8, 8, 8, 7, 7, 7, 7, 7, 7, 7, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 5];

/// `BigInteger.intRadix`: `radix ^ digitsPerInt[radix]`.
const INT_RADIX: [u32; 37] = [
    0, 0, 0x4000_0000, 0x4546_b3db, 0x4000_0000, 0x48c2_7395, 0x159f_d800, 0x75db_9c97, 0x4000_0000, 0x1717_9149,
    0x3b9a_ca00, 0x0cc6_db61, 0x19a1_0000, 0x309f_1021, 0x57f6_c100, 0x0a2f_1b6f, 0x1000_0000, 0x1875_4571,
    0x247d_bc80, 0x3547_667b, 0x4c4b_4000, 0x6b5a_6e1d, 0x06c2_0a40, 0x08d2_d931, 0x0b64_0000, 0x0e8d_4a51,
    0x1269_ae40, 0x1717_9149, 0x1cb9_1000, 0x2374_4899, 0x2b73_a840, 0x34e6_3b41, 0x4000_0000, 0x4cfa_3cc1,
    0x5c13_d840, 0x6d91_b519, 0x039a_a400,
];

// ---------------------------------------------------------------------------
// BigDecimal
// ---------------------------------------------------------------------------

/// An immutable arbitrary-precision signed decimal: an unscaled integer
/// value and a 32-bit scale, the value being `unscaledValue × 10^-scale`.
///
/// Equivalent to `java.math.BigDecimal` (see the module documentation for
/// the internal representation). The derived equality is
/// `equals(Object)`: same value **and** scale (`2.0` is not equal to
/// `2.00`); [`compare_to`](Self::compare_to) compares values only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BigDecimal {
    int_val: BigInteger,
    scale: i32,
}

fn ten_pow(n: i32) -> BigInteger {
    // `bigTenToThe`: ZERO for a negative exponent.
    if n < 0 {
        return BigInteger::zero();
    }
    BigInteger::ten().pow(n).unwrap_or_else(|_| BigInteger::zero())
}

fn big_multiply_power_ten(value: &BigInteger, n: i32) -> BigInteger {
    if n <= 0 {
        return value.clone();
    }
    value.multiply(&ten_pow(n))
}

/// `bigDigitLength(BigInteger b)`: the number of decimal digits of `|b|`,
/// 1 for zero.
fn big_digit_length(b: &BigInteger) -> i32 {
    if b.signum == 0 {
        return 1;
    }
    let r = (((b.abs().bit_length() as i64 + 1) * 646_456_993) as u64 >> 31) as i32;
    if b.compare_magnitude(&ten_pow(r)) < 0 {
        r
    } else {
        r + 1
    }
}

/// `compactValFor(BigInteger b)`: the value as a `long` when it fits the
/// JDK's compact form (any `long` but `Long.MIN_VALUE`, the `INFLATED`
/// marker), `None` otherwise.
fn compact_val_for(b: &BigInteger) -> Option<i64> {
    let len = b.mag.len();
    if len == 0 {
        return Some(0);
    }
    if len > 2 || (len == 2 && (b.mag[1] as i32) < 0) {
        return None;
    }
    let u = b.mag[0] as i64 | (b.mag.get(1).copied().unwrap_or(0) as i64) << 32;
    Some(if b.signum < 0 { -u } else { u })
}

fn overflow_or_underflow(as_int: i32) -> LuceneError {
    arithmetic(if as_int > 0 { "Underflow" } else { "Overflow" })
}

/// `checkScaleNonZero(long val)`: the sign test is on the truncated `int`,
/// as the JDK's.
fn check_scale_non_zero(val: i64) -> Result<i32> {
    let as_int = val as i32;
    if as_int as i64 != val {
        return Err(overflow_or_underflow(as_int));
    }
    Ok(as_int)
}

/// `checkScale(BigInteger intVal, long val)`: a scale outside the `int`
/// range saturates for a zero value and throws otherwise.
fn check_scale_for(int_val: &BigInteger, val: i64) -> Result<i32> {
    let mut as_int = val as i32;
    if as_int as i64 != val {
        as_int = if val > i32::MAX as i64 { i32::MAX } else { i32::MIN };
        if int_val.signum() != 0 {
            return Err(overflow_or_underflow(as_int));
        }
    }
    Ok(as_int)
}

fn saturate_long(s: i64) -> i32 {
    let i = s as i32;
    if s == i as i64 {
        i
    } else if s < 0 {
        i32::MIN
    } else {
        i32::MAX
    }
}

/// `commonNeedIncrement(int roundingMode, int qsign, int cmpFracHalf,
/// boolean oddQuot)`.
fn common_need_increment(rounding_mode: i32, qsign: i32, cmp_frac_half: i32, odd_quot: bool) -> Result<bool> {
    Ok(match rounding_mode {
        ROUND_UNNECESSARY => return Err(arithmetic("Rounding necessary")),
        ROUND_UP => true,
        ROUND_DOWN => false,
        ROUND_CEILING => qsign > 0,
        ROUND_FLOOR => qsign < 0,
        _ => {
            debug_assert!((ROUND_HALF_UP..=ROUND_HALF_EVEN).contains(&rounding_mode));
            match cmp_frac_half.cmp(&0) {
                Ordering::Less => false,
                Ordering::Greater => true,
                Ordering::Equal => match rounding_mode {
                    ROUND_HALF_DOWN => false,
                    ROUND_HALF_UP => true,
                    _ => odd_quot,
                },
            }
        }
    })
}

/// The quotient magnitude, remainder magnitude and sign of
/// `bdividend / bdivisor` (`MutableBigInteger.divide`).
fn divide_parts(bdividend: &BigInteger, bdivisor: &BigInteger) -> (Vec<u32>, Vec<u32>, i32) {
    let (q, r) = mag_divmod(&bdividend.mag, &bdivisor.mag);
    let qsign = if bdividend.signum != bdivisor.signum { -1 } else { 1 };
    (q, r, qsign)
}

/// `needIncrement(MutableBigInteger mdivisor, int roundingMode, int qsign,
/// MutableBigInteger mq, MutableBigInteger mr)`: `cmpFracHalf` is
/// `mr.compareHalf(mdivisor)`, the comparison of `2 × r` with the divisor.
fn need_increment(divisor_mag: &[u32], rounding_mode: i32, qsign: i32, q: &[u32], r: &[u32]) -> Result<bool> {
    let cmp_frac_half = mag_cmp(&mag_shl(r, 1), divisor_mag) as i32;
    let odd = q.first().is_some_and(|w| w & 1 != 0);
    common_need_increment(rounding_mode, qsign, cmp_frac_half, odd)
}

/// `divideAndRound(BigInteger bdividend, BigInteger bdivisor, int
/// roundingMode)`.
fn divide_and_round(bdividend: &BigInteger, bdivisor: &BigInteger, rounding_mode: i32) -> Result<BigInteger> {
    let (mut q, r, qsign) = divide_parts(bdividend, bdivisor);
    if !r.is_empty() && need_increment(&bdivisor.mag, rounding_mode, qsign, &q, &r)? {
        q = mag_add(&q, &[1]);
    }
    Ok(BigInteger::from_parts(qsign, q))
}

/// `divideAndRound(BigInteger bdividend, BigInteger bdivisor, int scale,
/// int roundingMode, int preferredScale)`.
fn divide_and_round_scaled(
    bdividend: &BigInteger,
    bdivisor: &BigInteger,
    scale: i32,
    rounding_mode: i32,
    preferred_scale: i32,
) -> Result<BigDecimal> {
    let (mut q, r, qsign) = divide_parts(bdividend, bdivisor);
    if !r.is_empty() {
        if need_increment(&bdivisor.mag, rounding_mode, qsign, &q, &r)? {
            q = mag_add(&q, &[1]);
        }
        Ok(BigDecimal::new_unscaled(BigInteger::from_parts(qsign, q), scale))
    } else if preferred_scale != scale {
        create_and_strip_zeros_to_match_scale(BigInteger::from_parts(qsign, q), scale, preferred_scale as i64)
    } else {
        Ok(BigDecimal::new_unscaled(BigInteger::from_parts(qsign, q), scale))
    }
}

/// `createAndStripZerosToMatchScale(BigInteger intVal, int scale, long
/// preferredScale)`: removes trailing decimal zeros while the scale is
/// above `preferredScale`.
///
/// The JDK finds the removable zeros by binary splitting over powers of
/// five; dividing by ten one zero at a time removes the same zeros and
/// reaches the same scale, with the same `checkScale` per step.
fn create_and_strip_zeros_to_match_scale(mut int_val: BigInteger, mut scale: i32, preferred_scale: i64) -> Result<BigDecimal> {
    let preferred_scale = preferred_scale.clamp(i32::MIN as i64 - 1, i32::MAX as i64);
    if int_val.signum == 0 {
        return Ok(BigDecimal::new_unscaled(int_val, scale));
    }
    while (scale as i64) > preferred_scale {
        let (q, r) = mag_divmod_small(&int_val.mag, 10);
        if r != 0 {
            break;
        }
        let next = BigInteger::from_parts(int_val.signum, q);
        scale = check_scale_for(&next, scale as i64 - 1)?;
        int_val = next;
    }
    Ok(BigDecimal::new_unscaled(int_val, scale))
}

/// `doRound(BigInteger intVal, int scale, MathContext mc)`.
fn do_round_big(mut int_val: BigInteger, mut scale: i32, mc: &MathContext) -> Result<BigDecimal> {
    let mcp = mc.precision;
    if mcp > 0 {
        let mode = mc.rounding_mode.old_mode();
        let mut prec = big_digit_length(&int_val);
        let mut drop = prec - mcp;
        while drop > 0 {
            scale = check_scale_non_zero(scale as i64 - drop as i64)?;
            int_val = divide_and_round(&int_val, &ten_pow(drop), mode)?;
            prec = big_digit_length(&int_val);
            drop = prec - mcp;
        }
    }
    Ok(BigDecimal::new_unscaled(int_val, scale))
}

impl BigDecimal {
    /// `BigDecimal.ZERO`: 0, scale 0.
    pub fn zero() -> Self {
        Self { int_val: BigInteger::zero(), scale: 0 }
    }

    /// `BigDecimal.ONE`: 1, scale 0.
    pub fn one() -> Self {
        Self { int_val: BigInteger::one(), scale: 0 }
    }

    /// `BigDecimal.TEN`: 10, scale 0.
    pub fn ten() -> Self {
        Self { int_val: BigInteger::ten(), scale: 0 }
    }

    /// `new BigDecimal(BigInteger unscaledVal, int scale)`.
    pub fn new_unscaled(unscaled_val: BigInteger, scale: i32) -> Self {
        Self { int_val: unscaled_val, scale }
    }

    /// `new BigDecimal(BigInteger val)`: scale 0.
    pub fn from_big_integer(val: BigInteger) -> Self {
        Self::new_unscaled(val, 0)
    }

    /// `new BigDecimal(int val)`: scale 0.
    pub fn from_int(val: i32) -> Self {
        Self::new_unscaled(BigInteger::value_of(val as i64), 0)
    }

    /// `new BigDecimal(long val)`: scale 0.
    pub fn from_long(val: i64) -> Self {
        Self::new_unscaled(BigInteger::value_of(val), 0)
    }

    /// `BigDecimal.valueOf(long val)`: scale 0.
    pub fn value_of(val: i64) -> Self {
        Self::from_long(val)
    }

    /// `BigDecimal.valueOf(long unscaledVal, int scale)`.
    pub fn value_of_scaled(unscaled_val: i64, scale: i32) -> Self {
        Self::new_unscaled(BigInteger::value_of(unscaled_val), scale)
    }

    /// `new BigDecimal(double val)`: the exact decimal expansion of the
    /// binary value.
    ///
    /// # Errors
    ///
    /// `NumberFormatException("Infinite or NaN")` for an infinite or NaN
    /// value.
    pub fn from_double(val: f64) -> Result<Self> {
        if val.is_infinite() || val.is_nan() {
            return Err(LuceneError::number_format("Infinite or NaN"));
        }
        let val_bits = val.to_bits() as i64;
        let sign: i64 = if (val_bits >> 63) == 0 { 1 } else { -1 };
        let mut exponent = ((val_bits >> 52) & 0x7ff) as i32;
        let mut significand = if exponent == 0 {
            (val_bits & ((1i64 << 52) - 1)) << 1
        } else {
            (val_bits & ((1i64 << 52) - 1)) | (1i64 << 52)
        };
        exponent -= 1075;
        if significand == 0 {
            return Ok(Self::zero());
        }
        while (significand & 1) == 0 {
            significand >>= 1;
            exponent += 1;
        }
        let compact_val = sign * significand;
        let (rb, scl) = if exponent == 0 {
            (BigInteger::value_of(compact_val), 0)
        } else if exponent < 0 {
            (BigInteger::value_of(5).pow(-exponent)?.multiply_long(compact_val), -exponent)
        } else {
            (BigInteger::value_of(2).pow(exponent)?.multiply_long(compact_val), 0)
        };
        Ok(Self::new_unscaled(rb, scl))
    }

    /// `BigDecimal.valueOf(double val)`: the value `Double.toString(val)`
    /// spells, with that string's scale.
    ///
    /// JDK 25 builds it from `FormattedFPDecimal.valueForDoubleToString`,
    /// the significand and exponent `Double.toString` renders (same digits,
    /// same scale); this port parses `Double.toString`'s text
    /// ([`rucene::util::java_fmt::double_to_string`]).
    ///
    /// # Errors
    ///
    /// `NumberFormatException("Infinite or NaN")` for an infinite or NaN
    /// value.
    pub fn value_of_double(val: f64) -> Result<Self> {
        if !val.is_finite() {
            return Err(LuceneError::number_format("Infinite or NaN"));
        }
        let s = rucene::util::java_fmt::double_to_string(val);
        Self::from_string(&crate::jdk::units(&s))
    }

    /// `new BigDecimal(String val)` (`BigDecimal(char[], int, int)` over the
    /// string's characters, `MathContext.UNLIMITED`).
    ///
    /// # Errors
    ///
    /// `NumberFormatException` with the JDK's messages (`No digits found.`,
    /// `Character array contains more than one decimal point.`, `Character
    /// <c> is neither a decimal digit number, decimal point, nor "e"
    /// notation exponential mark.` for at most 18 characters after the
    /// sign, `Character array is missing "e" notation exponential mark.`
    /// beyond, `No exponent digits.`, `Too many nonzero exponent digits.`,
    /// `Not a digit.`, `Exponent overflow.`), or without a message where
    /// the JDK reads past the end of the array (an empty string, a lone
    /// `e`).
    pub fn from_string(val: &[u16]) -> Result<Self> {
        let nfe_no_message = || LuceneError::unchecked_with_message("java.lang.NumberFormatException", None);
        let at = |i: usize| -> Result<u16> { val.get(i).copied().ok_or_else(nfe_no_message) };
        let mut offset = 0usize;
        let mut len = val.len() as i64;
        let mut prec = 0i32;
        let mut scl: i64 = 0;
        let isneg;
        let first = at(offset)?;
        if first == b'-' as u16 {
            isneg = true;
            offset += 1;
            len -= 1;
        } else {
            isneg = false;
            if first == b'+' as u16 {
                offset += 1;
                len -= 1;
            }
        }
        let mut dot = false;
        let is_compact = len <= 18;
        let mut coeff: Vec<u32> = Vec::new();
        let is_digit = |c: u16| rucene_analysis_common::jdk::character::is_digit(c as i32);
        while len > 0 {
            let c = at(offset)?;
            let digit = if (b'0' as u16..=b'9' as u16).contains(&c) {
                Some((c - b'0' as u16) as u32)
            } else if is_digit(c) {
                Some(character_digit(c as i32, 10) as u32)
            } else {
                None
            };
            if let Some(d) = digit {
                if d == 0 {
                    if prec == 0 {
                        prec = 1;
                    } else if !coeff.is_empty() {
                        coeff.push(0);
                        prec += 1;
                    }
                } else {
                    if prec != 1 || !coeff.is_empty() {
                        prec += 1;
                    }
                    coeff.push(d);
                }
                if dot {
                    scl += 1;
                }
            } else if c == b'.' as u16 {
                if dot {
                    return Err(LuceneError::number_format("Character array contains more than one decimal point."));
                }
                dot = true;
            } else if c == b'e' as u16 || c == b'E' as u16 {
                scl -= Self::parse_exp(val, offset, len)?;
                break;
            } else if is_compact {
                let ch = char::from_u32(c as u32).map(String::from).unwrap_or_else(|| to_rust_string(&[c]));
                return Err(LuceneError::number_format(format!(
                    "Character {ch} is neither a decimal digit number, decimal point, nor \"e\" notation exponential mark."
                )));
            } else {
                return Err(LuceneError::number_format("Character array is missing \"e\" notation exponential mark."));
            }
            offset += 1;
            len -= 1;
        }
        if prec == 0 {
            return Err(LuceneError::number_format("No digits found."));
        }
        let mut mag: Vec<u32> = Vec::new();
        for d in coeff {
            mag_mul_add_small(&mut mag, 10, d);
        }
        if scl as i32 as i64 != scl {
            return Err(LuceneError::number_format("Exponent overflow."));
        }
        Ok(Self::new_unscaled(BigInteger::from_parts(if isneg { -1 } else { 1 }, mag), scl as i32))
    }

    /// `parseExp(char[] in, int offset, int len)`.
    fn parse_exp(val: &[u16], mut offset: usize, mut len: i64) -> Result<i64> {
        let nfe_no_message = || LuceneError::unchecked_with_message("java.lang.NumberFormatException", None);
        let at = |i: usize| -> Result<u16> { val.get(i).copied().ok_or_else(nfe_no_message) };
        let mut exp: i64 = 0;
        offset += 1;
        let mut c = at(offset)?;
        len -= 1;
        let negexp = c == b'-' as u16;
        if negexp || c == b'+' as u16 {
            offset += 1;
            c = at(offset)?;
            len -= 1;
        }
        if len <= 0 {
            return Err(LuceneError::number_format("No exponent digits."));
        }
        while len > 10 && (c == b'0' as u16 || character_digit(c as i32, 10) == 0) {
            offset += 1;
            c = at(offset)?;
            len -= 1;
        }
        if len > 10 {
            return Err(LuceneError::number_format("Too many nonzero exponent digits."));
        }
        loop {
            let v = if (b'0' as u16..=b'9' as u16).contains(&c) {
                (c - b'0' as u16) as i64
            } else {
                let v = character_digit(c as i32, 10);
                if v < 0 {
                    return Err(LuceneError::number_format("Not a digit."));
                }
                v as i64
            };
            exp = exp * 10 + v;
            if len == 1 {
                break;
            }
            offset += 1;
            c = at(offset)?;
            len -= 1;
        }
        Ok(if negexp { -exp } else { exp })
    }

    /// `checkScale(long val)` of this value.
    fn check_scale(&self, val: i64) -> Result<i32> {
        check_scale_for(&self.int_val, val)
    }

    /// `signum()`.
    pub fn signum(&self) -> i32 {
        self.int_val.signum()
    }

    /// `scale()`.
    pub fn scale(&self) -> i32 {
        self.scale
    }

    /// `precision()`: the number of digits of the unscaled value (1 for
    /// zero).
    pub fn precision(&self) -> i32 {
        big_digit_length(&self.int_val)
    }

    /// `unscaledValue()`.
    pub fn unscaled_value(&self) -> BigInteger {
        self.int_val.clone()
    }

    /// `abs()`.
    pub fn abs(&self) -> Self {
        if self.signum() < 0 {
            self.negate()
        } else {
            self.clone()
        }
    }

    /// `negate()`.
    pub fn negate(&self) -> Self {
        Self::new_unscaled(self.int_val.negate(), self.scale)
    }

    /// `add(BigDecimal augend)`: the exact sum, scale `max(this.scale(),
    /// augend.scale())`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` (`Overflow`/`Underflow`) when aligning the
    /// scales overflows.
    pub fn add(&self, augend: &BigDecimal) -> Result<Self> {
        Self::add_big(&self.int_val, self.scale, &augend.int_val, augend.scale)
    }

    /// `add(BigInteger fst, int scale1, BigInteger snd, int scale2)`.
    fn add_big(fst: &BigInteger, scale1: i32, snd: &BigInteger, scale2: i32) -> Result<Self> {
        let mut rscale = scale1;
        let sdiff = rscale as i64 - scale2 as i64;
        let mut fst = fst.clone();
        let mut snd = snd.clone();
        if sdiff != 0 {
            if sdiff < 0 {
                let raise = check_scale_for(&fst, -sdiff)?;
                rscale = scale2;
                fst = big_multiply_power_ten(&fst, raise);
            } else {
                let raise = check_scale_for(&snd, sdiff)?;
                snd = big_multiply_power_ten(&snd, raise);
            }
        }
        Ok(Self::new_unscaled(fst.add(&snd), rscale))
    }

    /// `subtract(BigDecimal subtrahend)`.
    ///
    /// # Errors
    ///
    /// As [`add`](Self::add).
    pub fn subtract(&self, subtrahend: &BigDecimal) -> Result<Self> {
        Self::add_big(&self.int_val, self.scale, &subtrahend.int_val.negate(), subtrahend.scale)
    }

    /// `multiply(BigDecimal multiplicand)`: the exact product, scale
    /// `this.scale() + multiplicand.scale()`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` (`Overflow`/`Underflow`) when the scale
    /// overflows.
    pub fn multiply(&self, multiplicand: &BigDecimal) -> Result<Self> {
        let product_scale = self.check_scale(self.scale as i64 + multiplicand.scale as i64)?;
        Ok(Self::new_unscaled(self.int_val.multiply(&multiplicand.int_val), product_scale))
    }

    /// `multiply(BigDecimal multiplicand, MathContext mc)`: the product
    /// rounded to `mc`.
    ///
    /// # Errors
    ///
    /// As [`multiply`](Self::multiply), plus `ArithmeticException("Rounding
    /// necessary")` under `UNNECESSARY` rounding of an inexact result.
    pub fn multiply_mc(&self, multiplicand: &BigDecimal, mc: &MathContext) -> Result<Self> {
        if mc.precision == 0 {
            return self.multiply(multiplicand);
        }
        let product_scale = self.check_scale(self.scale as i64 + multiplicand.scale as i64)?;
        do_round_big(self.int_val.multiply(&multiplicand.int_val), product_scale, mc)
    }

    /// `divide(BigDecimal divisor, int scale, RoundingMode roundingMode)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` (`BigInteger divide by zero` for a zero
    /// divisor, `Rounding necessary`, `Overflow`/`Underflow`).
    pub fn divide_scale(&self, divisor: &BigDecimal, scale: i32, rounding_mode: RoundingMode) -> Result<Self> {
        Self::divide_to_scale(&self.int_val, self.scale, &divisor.int_val, divisor.scale, scale, rounding_mode.old_mode())
    }

    /// `divide(BigInteger dividend, int dividendScale, BigInteger divisor,
    /// int divisorScale, int scale, int roundingMode)`.
    fn divide_to_scale(
        dividend: &BigInteger,
        dividend_scale: i32,
        divisor: &BigInteger,
        divisor_scale: i32,
        scale: i32,
        rounding_mode: i32,
    ) -> Result<Self> {
        if divisor.signum == 0 {
            // A zero divisor is always held in the compact `long` form: a
            // dividend that fits it takes `divide(long, int, long, int, int,
            // int)`, whose `long` division throws `/ by zero`; a larger one
            // takes `divide(BigInteger, int, long, int, int, int)`, whose
            // `MutableBigInteger.divide(long, ...)` throws `BigInteger divide
            // by zero`.
            return Err(arithmetic(if compact_val_for(dividend).is_some() {
                "/ by zero"
            } else {
                "BigInteger divide by zero"
            }));
        }
        if check_scale_for(dividend, scale as i64 + divisor_scale as i64)? > dividend_scale {
            let new_scale = scale + divisor_scale;
            let raise = new_scale - dividend_scale;
            let scaled_dividend = big_multiply_power_ten(dividend, raise);
            divide_and_round_scaled(&scaled_dividend, divisor, scale, rounding_mode, scale)
        } else {
            let new_scale = check_scale_for(divisor, dividend_scale as i64 - scale as i64)?;
            let raise = new_scale - divisor_scale;
            let scaled_divisor = big_multiply_power_ten(divisor, raise);
            divide_and_round_scaled(dividend, &scaled_divisor, scale, rounding_mode, scale)
        }
    }

    /// `divide(BigDecimal divisor)`: the exact quotient, with the preferred
    /// scale `this.scale() - divisor.scale()` when it can be represented.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` (`Division undefined`, `Division by zero`,
    /// `Non-terminating decimal expansion; no exact representable decimal
    /// result.`).
    pub fn divide(&self, divisor: &BigDecimal) -> Result<Self> {
        if divisor.signum() == 0 {
            if self.signum() == 0 {
                return Err(arithmetic("Division undefined"));
            }
            return Err(arithmetic("Division by zero"));
        }
        let preferred_scale = saturate_long(self.scale as i64 - divisor.scale as i64);
        if self.signum() == 0 {
            return Ok(Self::new_unscaled(BigInteger::zero(), preferred_scale));
        }
        let mc_precision = (self.precision() as i64 + (10.0 * divisor.precision() as f64 / 3.0).ceil() as i64)
            .min(i32::MAX as i64) as i32;
        let mc = MathContext { precision: mc_precision, rounding_mode: RoundingMode::Unnecessary };
        let quotient = match self.divide_mc(divisor, &mc) {
            Ok(q) => q,
            Err(e) if matches!(e.kind(), rucene::error::LuceneErrorKind::Arithmetic(_)) => {
                return Err(arithmetic("Non-terminating decimal expansion; no exact representable decimal result."));
            }
            Err(e) => return Err(e),
        };
        let quotient_scale = quotient.scale();
        if preferred_scale > quotient_scale {
            return quotient.set_scale_mode(preferred_scale, ROUND_UNNECESSARY);
        }
        Ok(quotient)
    }

    /// `divide(BigDecimal divisor, MathContext mc)`: the quotient rounded to
    /// `mc`, trailing zeros removed down to the preferred scale
    /// `this.scale() - divisor.scale()`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` (`Division undefined`, `Division by zero`,
    /// `Rounding necessary`, `Overflow`/`Underflow`).
    pub fn divide_mc(&self, divisor: &BigDecimal, mc: &MathContext) -> Result<Self> {
        let mcp = mc.precision;
        if mcp == 0 {
            return self.divide(divisor);
        }
        let dividend = self;
        let preferred_scale = dividend.scale as i64 - divisor.scale as i64;
        if divisor.signum() == 0 {
            if dividend.signum() == 0 {
                return Err(arithmetic("Division undefined"));
            }
            return Err(arithmetic("Division by zero"));
        }
        if dividend.signum() == 0 {
            return Ok(Self::new_unscaled(BigInteger::zero(), saturate_long(preferred_scale)));
        }
        let xscale = dividend.precision();
        let mut yscale = divisor.precision();
        let xs = &dividend.int_val;
        let ys = &divisor.int_val;
        // divide(BigInteger xs, int xscale, BigInteger ys, int yscale, long
        // preferredScale, MathContext mc)
        if Self::compare_magnitude_normalized(xs, xscale, ys, yscale) > 0 {
            yscale -= 1;
        }
        let rounding_mode = mc.rounding_mode.old_mode();
        let scl = check_scale_non_zero(preferred_scale + yscale as i64 - xscale as i64 + mcp as i64)?;
        let quotient = if check_scale_non_zero(mcp as i64 + yscale as i64 - xscale as i64)? > 0 {
            let raise = check_scale_non_zero(mcp as i64 + yscale as i64 - xscale as i64)?;
            let rb = big_multiply_power_ten(xs, raise);
            divide_and_round_scaled(&rb, ys, scl, rounding_mode, check_scale_non_zero(preferred_scale)?)?
        } else {
            let new_scale = check_scale_non_zero(xscale as i64 - mcp as i64)?;
            let raise = check_scale_non_zero(new_scale as i64 - yscale as i64)?;
            let rb = big_multiply_power_ten(ys, raise);
            divide_and_round_scaled(xs, &rb, scl, rounding_mode, check_scale_non_zero(preferred_scale)?)?
        };
        Self::do_round(quotient, mc)
    }

    /// `compareMagnitudeNormalized(BigInteger xs, int xscale, BigInteger ys,
    /// int yscale)`.
    fn compare_magnitude_normalized(xs: &BigInteger, xscale: i32, ys: &BigInteger, yscale: i32) -> i32 {
        let sdiff = xscale - yscale;
        if sdiff < 0 {
            big_multiply_power_ten(xs, -sdiff).compare_magnitude(ys)
        } else {
            xs.compare_magnitude(&big_multiply_power_ten(ys, sdiff))
        }
    }

    /// `doRound(BigDecimal val, MathContext mc)`.
    fn do_round(val: BigDecimal, mc: &MathContext) -> Result<Self> {
        if mc.precision > 0 {
            do_round_big(val.int_val, val.scale, mc)
        } else {
            Ok(val)
        }
    }

    /// `pow(int n)`: the exact power, scale `this.scale() × n`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException("Invalid operation")` when `n` is outside
    /// `0..=999999999`; `Overflow`/`Underflow` when the scale overflows.
    pub fn pow(&self, n: i32) -> Result<Self> {
        if !(0..=999_999_999).contains(&n) {
            return Err(arithmetic("Invalid operation"));
        }
        let new_scale = self.check_scale(self.scale as i64 * n as i64)?;
        Ok(Self::new_unscaled(self.int_val.pow(n)?, new_scale))
    }

    /// `pow(int n, MathContext mc)`: the X3.274-1996 algorithm (working
    /// precision `mc.precision + digits(n) + 1`, left-to-right binary
    /// exponentiation, then one rounding to `mc`).
    ///
    /// # Errors
    ///
    /// `ArithmeticException("Invalid operation")` when `n` is outside
    /// `-999999999..=999999999` or has more digits than `mc.precision`, and
    /// the errors of the multiplications and divisions.
    pub fn pow_mc(&self, n: i32, mc: &MathContext) -> Result<Self> {
        if mc.precision == 0 {
            return self.pow(n);
        }
        if !(-999_999_999..=999_999_999).contains(&n) {
            return Err(arithmetic("Invalid operation"));
        }
        if n == 0 {
            return Ok(Self::one());
        }
        let lhs = self;
        let mut workmc = *mc;
        let mut mag = n.wrapping_abs();
        if mc.precision > 0 {
            let elength = big_digit_length(&BigInteger::value_of(mag as i64));
            if elength > mc.precision {
                return Err(arithmetic("Invalid operation"));
            }
            workmc = MathContext { precision: mc.precision + elength + 1, rounding_mode: mc.rounding_mode };
        }
        let mut acc = Self::one();
        let mut seenbit = false;
        let mut i = 1;
        loop {
            mag = mag.wrapping_add(mag);
            if mag < 0 {
                seenbit = true;
                acc = acc.multiply_mc(lhs, &workmc)?;
            }
            if i == 31 {
                break;
            }
            if seenbit {
                acc = acc.multiply_mc(&acc.clone(), &workmc)?;
            }
            i += 1;
        }
        if n < 0 {
            acc = Self::one().divide_mc(&acc, &workmc)?;
        }
        Self::do_round(acc, mc)
    }

    /// `round(MathContext mc)` (`plus(mc)`).
    ///
    /// # Errors
    ///
    /// `ArithmeticException("Rounding necessary")` under `UNNECESSARY`
    /// rounding of an inexact result; `Overflow`/`Underflow`.
    pub fn round(&self, mc: &MathContext) -> Result<Self> {
        Self::do_round(self.clone(), mc)
    }

    /// `setScale(int newScale, RoundingMode roundingMode)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException("Rounding necessary")` under `UNNECESSARY`
    /// rounding of an inexact result; `Overflow`/`Underflow`.
    pub fn set_scale(&self, new_scale: i32, rounding_mode: RoundingMode) -> Result<Self> {
        self.set_scale_mode(new_scale, rounding_mode.old_mode())
    }

    /// `setScale(int newScale)`: `UNNECESSARY` rounding.
    ///
    /// # Errors
    ///
    /// As [`set_scale`](Self::set_scale).
    pub fn set_scale_exact(&self, new_scale: i32) -> Result<Self> {
        self.set_scale_mode(new_scale, ROUND_UNNECESSARY)
    }

    /// `setScale(int newScale, int roundingMode)`.
    fn set_scale_mode(&self, new_scale: i32, rounding_mode: i32) -> Result<Self> {
        if !(ROUND_UP..=ROUND_UNNECESSARY).contains(&rounding_mode) {
            return Err(LuceneError::illegal_argument("Invalid rounding mode"));
        }
        let old_scale = self.scale;
        if new_scale == old_scale {
            return Ok(self.clone());
        }
        if self.signum() == 0 {
            return Ok(Self::new_unscaled(BigInteger::zero(), new_scale));
        }
        if new_scale > old_scale {
            let raise = self.check_scale(new_scale as i64 - old_scale as i64)?;
            Ok(Self::new_unscaled(big_multiply_power_ten(&self.int_val, raise), new_scale))
        } else {
            let drop = self.check_scale(old_scale as i64 - new_scale as i64)?;
            divide_and_round_scaled(&self.int_val, &ten_pow(drop), new_scale, rounding_mode, new_scale)
        }
    }

    /// `movePointRight(int n)`: the scale minus `n`, then scale 0 if that
    /// is negative.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` (`Overflow`/`Underflow`) when the scale
    /// overflows.
    pub fn move_point_right(&self, n: i32) -> Result<Self> {
        if n == 0 && self.scale >= 0 {
            return Ok(self.clone());
        }
        let new_scale = self.check_scale(self.scale as i64 - n as i64)?;
        let num = Self::new_unscaled(self.int_val.clone(), new_scale);
        if num.scale < 0 {
            num.set_scale_mode(0, ROUND_UNNECESSARY)
        } else {
            Ok(num)
        }
    }

    /// `movePointLeft(int n)`: the scale plus `n`, then scale 0 if that is
    /// negative.
    ///
    /// # Errors
    ///
    /// As [`move_point_right`](Self::move_point_right).
    pub fn move_point_left(&self, n: i32) -> Result<Self> {
        if n == 0 && self.scale >= 0 {
            return Ok(self.clone());
        }
        let new_scale = self.check_scale(self.scale as i64 + n as i64)?;
        let num = Self::new_unscaled(self.int_val.clone(), new_scale);
        if num.scale < 0 {
            num.set_scale_mode(0, ROUND_UNNECESSARY)
        } else {
            Ok(num)
        }
    }

    /// `scaleByPowerOfTen(int n)`: the same unscaled value, scale
    /// `this.scale() - n`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` (`Overflow`/`Underflow`) when the scale
    /// overflows.
    pub fn scale_by_power_of_ten(&self, n: i32) -> Result<Self> {
        Ok(Self::new_unscaled(self.int_val.clone(), self.check_scale(self.scale as i64 - n as i64)?))
    }

    /// `stripTrailingZeros()`: [`zero`](Self::zero) for a zero value,
    /// otherwise the value with every trailing zero of the unscaled value
    /// removed.
    ///
    /// # Errors
    ///
    /// `ArithmeticException("Overflow")` when the scale would leave the
    /// `int` range.
    pub fn strip_trailing_zeros(&self) -> Result<Self> {
        if self.int_val.signum() == 0 {
            return Ok(Self::zero());
        }
        create_and_strip_zeros_to_match_scale(self.int_val.clone(), self.scale, i64::MIN)
    }

    /// `compareTo(BigDecimal val)`: compares the values, ignoring the
    /// scales.
    pub fn compare_to(&self, val: &BigDecimal) -> i32 {
        let xsign = self.signum();
        let ysign = val.signum();
        if xsign != ysign {
            return if xsign > ysign { 1 } else { -1 };
        }
        if xsign == 0 {
            return 0;
        }
        let cmp = self.compare_magnitude(val);
        if xsign > 0 {
            cmp
        } else {
            -cmp
        }
    }

    /// `compareMagnitude(BigDecimal val)`.
    fn compare_magnitude(&self, val: &BigDecimal) -> i32 {
        if self.int_val.signum == 0 {
            return if val.int_val.signum == 0 { 0 } else { -1 };
        }
        if val.int_val.signum == 0 {
            return 1;
        }
        let sdiff = self.scale as i64 - val.scale as i64;
        if sdiff != 0 {
            let xae = self.precision() as i64 - self.scale as i64;
            let yae = val.precision() as i64 - val.scale as i64;
            if xae < yae {
                return -1;
            }
            if xae > yae {
                return 1;
            }
            if sdiff < 0 {
                return big_multiply_power_ten(&self.int_val, (-sdiff) as i32).compare_magnitude(&val.int_val);
            } else {
                return self.int_val.compare_magnitude(&big_multiply_power_ten(&val.int_val, sdiff as i32));
            }
        }
        self.int_val.compare_magnitude(&val.int_val)
    }

    /// `min(BigDecimal val)`: `this` when `compareTo(val) <= 0`, else
    /// `val`.
    pub fn min(&self, val: &BigDecimal) -> Self {
        if self.compare_to(val) <= 0 {
            self.clone()
        } else {
            val.clone()
        }
    }

    /// `max(BigDecimal val)`.
    pub fn max(&self, val: &BigDecimal) -> Self {
        if self.compare_to(val) >= 0 {
            self.clone()
        } else {
            val.clone()
        }
    }

    /// `equals(Object x)`: same value and scale (the derived `PartialEq`).
    pub fn java_equals(&self, x: &BigDecimal) -> bool {
        self == x
    }

    /// `hashCode()`: `31 × unscaledValue().hashCode() + scale()`.
    pub fn hash_code(&self) -> i32 {
        self.int_val.hash_code().wrapping_mul(31).wrapping_add(self.scale)
    }

    /// `toBigInteger()`: the integer part (fraction discarded).
    pub fn to_big_integer(&self) -> BigInteger {
        // setScale(0, ROUND_DOWN) cannot fail: ROUND_DOWN never throws and
        // a drop of `scale` digits stays within the int range.
        match self.set_scale_mode(0, ROUND_DOWN) {
            Ok(v) => v.int_val,
            Err(_) => BigInteger::zero(),
        }
    }

    /// `longValue()`: the low 64 bits of the integer part.
    pub fn long_value(&self) -> i64 {
        if self.scale == 0 {
            return self.int_val.long_value();
        }
        if self.signum() == 0 || self.precision() <= self.scale || self.scale <= -64 {
            0
        } else {
            self.to_big_integer().long_value()
        }
    }

    /// `intValue()`: `(int) longValue()`.
    pub fn int_value(&self) -> i32 {
        if self.scale == 0 {
            return self.int_val.int_value();
        }
        self.long_value() as i32
    }

    /// `longValueExact()`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException("Rounding necessary")` for a non-zero fraction,
    /// `ArithmeticException("Overflow")` outside the `long` range.
    pub fn long_value_exact(&self) -> Result<i64> {
        if self.scale == 0 {
            if let Some(v) = compact_val_for(&self.int_val) {
                return Ok(v);
            }
        }
        if self.signum() == 0 {
            return Ok(0);
        }
        if self.precision() <= self.scale {
            return Err(arithmetic("Rounding necessary"));
        }
        if (self.precision() - 19) > self.scale {
            return Err(arithmetic("Overflow"));
        }
        let num = self.set_scale_mode(0, ROUND_UNNECESSARY)?;
        if num.precision() >= 19
            && (num.int_val.compare_to(&BigInteger::value_of(i64::MIN)) < 0
                || num.int_val.compare_to(&BigInteger::value_of(i64::MAX)) > 0)
        {
            return Err(arithmetic("Overflow"));
        }
        Ok(num.int_val.long_value())
    }

    /// `doubleValue()`: the correctly rounded `double` (JDK 25's
    /// `fullDoubleValue` and its exact fast paths), infinite when out of
    /// range, `0.0` for zero.
    pub fn double_value(&self) -> f64 {
        if self.int_val.signum == 0 {
            return 0.0;
        }
        let text = format!("{}e{}", self.int_val, -(self.scale as i64));
        text.parse::<f64>().unwrap_or(0.0)
    }

    /// `toPlainString()`: no exponent field.
    pub fn to_plain_string(&self) -> String {
        if self.scale == 0 {
            return self.int_val.to_string();
        }
        if self.scale < 0 {
            if self.signum() == 0 {
                return "0".to_string();
            }
            let trailing_zeros = -(self.scale as i64);
            let mut buf = self.int_val.to_string();
            buf.extend(std::iter::repeat_n('0', trailing_zeros as usize));
            return buf;
        }
        let s = self.int_val.abs().to_string();
        Self::get_value_string(self.signum(), &s, self.scale)
    }

    /// `getValueString(int signum, String intString, int scale)`.
    fn get_value_string(signum: i32, int_string: &str, scale: i32) -> String {
        let insertion_point = int_string.len() as i64 - scale as i64;
        if insertion_point == 0 {
            return format!("{}{}", if signum < 0 { "-0." } else { "0." }, int_string);
        }
        let mut buf = String::new();
        if insertion_point > 0 {
            if signum < 0 {
                buf.push('-');
            }
            buf.push_str(&int_string[..insertion_point as usize]);
            buf.push('.');
            buf.push_str(&int_string[insertion_point as usize..]);
        } else {
            buf.push_str(if signum < 0 { "-0." } else { "0." });
            buf.extend(std::iter::repeat_n('0', (-insertion_point) as usize));
            buf.push_str(int_string);
        }
        buf
    }

    /// `layoutChars(boolean sci)`: the scientific (`toString`) or
    /// engineering (`toEngineeringString`) form.
    fn layout_chars(&self, sci: bool) -> String {
        let scale = self.scale;
        if scale == 0 {
            return self.int_val.to_string();
        }
        let coeff: Vec<char> = self.int_val.abs().to_string().chars().collect();
        let mut buf = String::with_capacity(32);
        if self.signum() < 0 {
            buf.push('-');
        }
        let coeff_len = coeff.len() as i64;
        let mut adjusted = -(scale as i64) + (coeff_len - 1);
        let push = |buf: &mut String, from: i64, n: i64| {
            for &c in &coeff[from as usize..(from + n) as usize] {
                buf.push(c);
            }
        };
        if scale >= 0 && adjusted >= -6 {
            let mut pad = scale as i64 - coeff_len;
            if pad >= 0 {
                buf.push('0');
                buf.push('.');
                while pad > 0 {
                    buf.push('0');
                    pad -= 1;
                }
                push(&mut buf, 0, coeff_len);
            } else {
                push(&mut buf, 0, -pad);
                buf.push('.');
                push(&mut buf, -pad, scale as i64);
            }
        } else {
            if sci {
                buf.push(coeff[0]);
                if coeff_len > 1 {
                    buf.push('.');
                    push(&mut buf, 1, coeff_len - 1);
                }
            } else {
                let mut sig = adjusted % 3;
                if sig < 0 {
                    sig += 3;
                }
                adjusted -= sig;
                sig += 1;
                if self.signum() == 0 {
                    match sig {
                        1 => buf.push('0'),
                        2 => {
                            buf.push_str("0.00");
                            adjusted += 3;
                        }
                        _ => {
                            buf.push_str("0.0");
                            adjusted += 3;
                        }
                    }
                } else if sig >= coeff_len {
                    push(&mut buf, 0, coeff_len);
                    for _ in 0..(sig - coeff_len) {
                        buf.push('0');
                    }
                } else {
                    push(&mut buf, 0, sig);
                    buf.push('.');
                    push(&mut buf, sig, coeff_len - sig);
                }
            }
            if adjusted != 0 {
                buf.push('E');
                if adjusted > 0 {
                    buf.push('+');
                }
                buf.push_str(&adjusted.to_string());
            }
        }
        buf
    }

    /// `toEngineeringString()`.
    pub fn to_engineering_string(&self) -> String {
        self.layout_chars(false)
    }
}

impl fmt::Display for BigDecimal {
    /// `toString()`: scientific notation when the exponent is positive or
    /// the adjusted exponent is below -6.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.layout_chars(true))
    }
}
