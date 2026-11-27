/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! `jdk.internal.math.FloatingDecimal.getBinaryToASCIIConverter(double)`:
//! the decimal digits of a `double` that `java.text.DigitList.set(boolean,
//! double, int, boolean)` (and so `DecimalFormat.format(double)`) starts from,
//! with the two flags it rounds by.
//!
//! **Divergence.** An emulation of JDK 25, not a port of a Lucene file,
//! written from OpenJDK 25's `FloatingDecimal.BinaryToASCIIBuffer` (`dtoa`,
//! `developLongDigits`, `roundup`, `estimateDecExp`, `getChars`) and the
//! contract of `FDBigInteger` (read as specification; no JDK code is
//! reproduced). It is the JDK's original digit generator, not the
//! shortest-digits algorithm of `Double.toString` since JDK 19: `DigitList`
//! still calls it in JDK 25, and the two differ (`8.41E21` here is
//! `8.409999999999999E21`). `FDBigInteger` is replaced by an exact unsigned
//! big integer: the generator only asks it for exact products, quotients,
//! remainders and comparisons, so any exact arithmetic gives the same digits.

use std::cmp::Ordering;

const EXP_SHIFT: i32 = 52;
const FRACT_HOB: i64 = 1 << EXP_SHIFT;
const EXP_ONE: i64 = 1023 << EXP_SHIFT;
const SIGNIF_BIT_MASK: i64 = (1 << EXP_SHIFT) - 1;
const EXP_BIT_MASK: i64 = 0x7FF << EXP_SHIFT;
const EXP_BIAS: i32 = 1023;
const MAX_SMALL_BIN_EXP: i32 = 62;
const MIN_SMALL_BIN_EXP: i32 = -(63 / 3);

/// `FDBigInteger.SMALL_5_POW`.
const SMALL_5_POW: [i32; 14] = [
    1, 5, 25, 125, 625, 3125, 15625, 78125, 390625, 1953125, 9765625, 48828125, 244140625,
    1220703125,
];

/// `FDBigInteger.LONG_5_POW`.
const LONG_5_POW: [i64; 27] = {
    let mut r = [1i64; 27];
    let mut i = 1;
    while i < 27 {
        r[i] = r[i - 1] * 5;
        i += 1;
    }
    r
};

/// `BinaryToASCIIBuffer.N_5_BITS`.
const N_5_BITS: [i32; 27] = [
    0, 3, 5, 7, 10, 12, 14, 17, 19, 21, 24, 26, 28, 31, 33, 35, 38, 40, 42, 45, 47, 49, 52, 54, 56,
    59, 61,
];

/// `BinaryToASCIIBuffer.insignificantDigitsNumber`.
const INSIGNIFICANT_DIGITS_NUMBER: [i32; 64] = [
    0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 5, 5, 5, 6, 6, 6, 6, 7, 7, 7, 8, 8, 8, 9, 9,
    9, 9, 10, 10, 10, 11, 11, 11, 12, 12, 12, 12, 13, 13, 13, 14, 14, 14, 15, 15, 15, 15, 16, 16,
    16, 17, 17, 17, 18, 18, 18, 19,
];

/// The result of `getBinaryToASCIIConverter(double)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryToAscii {
    /// `toJavaFormatString()`.
    pub java_format_string: String,
    /// `digitsRoundedUp()`.
    pub digits_rounded_up: bool,
    /// `decimalDigitsExact()`.
    pub decimal_digits_exact: bool,
}

/// `FloatingDecimal.getBinaryToASCIIConverter(d)` for a finite `d`.
pub fn binary_to_ascii(d: f64) -> BinaryToAscii {
    let d_bits = d.to_bits() as i64;
    let is_negative = d_bits < 0;
    let mut fract_bits = d_bits & SIGNIF_BIT_MASK;
    let mut bin_exp = ((d_bits & EXP_BIT_MASK) >> EXP_SHIFT) as i32;
    debug_assert!(bin_exp != 0x7FF, "isExceptional is excluded by the callers");
    let n_significant_bits;
    if bin_exp == 0 {
        if fract_bits == 0 {
            // `B2AC_POSITIVE_ZERO` / `B2AC_NEGATIVE_ZERO`
            let mut b = Buffer::new(is_negative);
            b.digits[0] = b'0';
            b.first_digit_index = 0;
            b.n_digits = 1;
            b.dec_exponent = 0;
            return b.result();
        }
        let leading_zeros = (fract_bits as u64).leading_zeros() as i32;
        let shift = leading_zeros - (63 - EXP_SHIFT);
        fract_bits <<= shift;
        bin_exp = 1 - shift;
        n_significant_bits = 64 - leading_zeros;
    } else {
        fract_bits |= FRACT_HOB;
        n_significant_bits = EXP_SHIFT + 1;
    }
    bin_exp -= EXP_BIAS;
    let mut buf = Buffer::new(is_negative);
    buf.dtoa(bin_exp, fract_bits, n_significant_bits, true);
    buf.result()
}

/// `BinaryToASCIIBuffer`.
struct Buffer {
    is_negative: bool,
    dec_exponent: i32,
    first_digit_index: usize,
    n_digits: usize,
    digits: [u8; 20],
    exact_decimal_conversion: bool,
    decimal_digits_rounded_up: bool,
}

impl Buffer {
    fn new(is_negative: bool) -> Self {
        Self {
            is_negative,
            dec_exponent: 0,
            first_digit_index: 0,
            n_digits: 0,
            digits: [0; 20],
            exact_decimal_conversion: false,
            decimal_digits_rounded_up: false,
        }
    }

    fn result(&self) -> BinaryToAscii {
        BinaryToAscii {
            java_format_string: self.java_format_string(),
            digits_rounded_up: self.decimal_digits_rounded_up,
            decimal_digits_exact: self.exact_decimal_conversion,
        }
    }

    /// `developLongDigits(decExponent, lvalue, insignificantDigits)`.
    fn develop_long_digits(&mut self, dec_exponent: i32, lvalue: i64, insignificant_digits: i32) {
        let mut dec_exponent = dec_exponent;
        let mut lvalue = lvalue;
        if insignificant_digits != 0 {
            let pow10 = LONG_5_POW[insignificant_digits as usize] << insignificant_digits;
            let residue = lvalue % pow10;
            lvalue /= pow10;
            dec_exponent += insignificant_digits;
            if residue >= (pow10 >> 1) {
                lvalue += 1;
            }
        }
        let mut digitno = self.digits.len() - 1;
        let mut c;
        if lvalue <= i64::from(i32::MAX) {
            let mut ivalue = lvalue as i32;
            c = ivalue % 10;
            ivalue /= 10;
            while c == 0 {
                dec_exponent += 1;
                c = ivalue % 10;
                ivalue /= 10;
            }
            while ivalue != 0 {
                self.digits[digitno] = (c as u8) + b'0';
                digitno -= 1;
                dec_exponent += 1;
                c = ivalue % 10;
                ivalue /= 10;
            }
            self.digits[digitno] = (c as u8) + b'0';
        } else {
            c = (lvalue % 10) as i32;
            lvalue /= 10;
            while c == 0 {
                dec_exponent += 1;
                c = (lvalue % 10) as i32;
                lvalue /= 10;
            }
            while lvalue != 0 {
                self.digits[digitno] = (c as u8) + b'0';
                digitno -= 1;
                dec_exponent += 1;
                c = (lvalue % 10) as i32;
                lvalue /= 10;
            }
            self.digits[digitno] = (c as u8) + b'0';
        }
        self.dec_exponent = dec_exponent + 1;
        self.first_digit_index = digitno;
        self.n_digits = self.digits.len() - digitno;
    }

    /// `dtoa(binExp, fractBits, nSignificantBits, isCompatibleFormat)`.
    fn dtoa(
        &mut self,
        bin_exp: i32,
        fract_bits: i64,
        n_significant_bits: i32,
        is_compatible_format: bool,
    ) {
        let mut fract_bits = fract_bits;
        let tail_zeros = fract_bits.trailing_zeros() as i32;
        let n_fract_bits = EXP_SHIFT + 1 - tail_zeros;
        self.decimal_digits_rounded_up = false;
        self.exact_decimal_conversion = false;
        let n_tiny_bits = 0.max(n_fract_bits - bin_exp - 1);
        if (MIN_SMALL_BIN_EXP..=MAX_SMALL_BIN_EXP).contains(&bin_exp)
            && (n_tiny_bits as usize) < LONG_5_POW.len()
            && (n_fract_bits + N_5_BITS[n_tiny_bits as usize]) < 64
            && n_tiny_bits == 0
        {
            let insignificant = if bin_exp > n_significant_bits {
                insignificant_digits_for_pow2(bin_exp - n_significant_bits - 1)
            } else {
                0
            };
            if bin_exp >= EXP_SHIFT {
                fract_bits <<= bin_exp - EXP_SHIFT;
            } else {
                fract_bits = ((fract_bits as u64) >> (EXP_SHIFT - bin_exp)) as i64;
            }
            self.develop_long_digits(0, fract_bits, insignificant);
            return;
        }
        let mut dec_exp = estimate_dec_exp(fract_bits, bin_exp);
        let b5 = 0.max(-dec_exp);
        let mut b2 = b5 + n_tiny_bits + bin_exp;
        let s5 = 0.max(dec_exp);
        let mut s2 = s5 + n_tiny_bits;
        let m5 = b5;
        let mut m2 = b2 - n_significant_bits;
        fract_bits = ((fract_bits as u64) >> tail_zeros) as i64;
        b2 -= n_fract_bits - 1;
        let common2factor = b2.min(s2);
        b2 -= common2factor;
        s2 -= common2factor;
        m2 -= common2factor;
        if n_fract_bits == 1 {
            m2 -= 1;
        }
        if m2 < 0 {
            b2 -= m2;
            s2 -= m2;
            m2 = 0;
        }
        let mut ndigit = 0usize;
        let mut low;
        let mut high;
        let low_digit_difference: i64;
        let bbits = n_fract_bits
            + b2
            + if (b5 as usize) < N_5_BITS.len() {
                N_5_BITS[b5 as usize]
            } else {
                b5 * 3
            };
        let ten_sbits = s2
            + 1
            + if ((s5 + 1) as usize) < N_5_BITS.len() {
                N_5_BITS[(s5 + 1) as usize]
            } else {
                (s5 + 1) * 3
            };
        if bbits < 64 && ten_sbits < 64 {
            if bbits < 32 && ten_sbits < 32 {
                let mut b = (fract_bits as i32)
                    .wrapping_mul(SMALL_5_POW[b5 as usize])
                    .wrapping_shl(b2 as u32);
                let s = SMALL_5_POW[s5 as usize].wrapping_shl(s2 as u32);
                let mut m = SMALL_5_POW[m5 as usize].wrapping_shl(m2 as u32);
                let tens = s.wrapping_mul(10);
                let mut q = b / s;
                b = 10i32.wrapping_mul(b % s);
                m = m.wrapping_mul(10);
                low = b < m;
                high = b.wrapping_add(m) > tens;
                if q == 0 && !high {
                    dec_exp -= 1;
                } else {
                    self.digits[ndigit] = b'0' + q as u8;
                    ndigit += 1;
                }
                if !is_compatible_format || !(-3..8).contains(&dec_exp) {
                    high = false;
                    low = false;
                }
                while !low && !high {
                    q = b / s;
                    b = 10i32.wrapping_mul(b % s);
                    m = m.wrapping_mul(10);
                    if m > 0 {
                        low = b < m;
                        high = b.wrapping_add(m) > tens;
                    } else {
                        low = true;
                        high = true;
                    }
                    self.digits[ndigit] = b'0' + q as u8;
                    ndigit += 1;
                }
                low_digit_difference = i64::from((b << 1).wrapping_sub(tens));
                self.exact_decimal_conversion = b == 0;
            } else {
                let mut b = fract_bits
                    .wrapping_mul(LONG_5_POW[b5 as usize])
                    .wrapping_shl(b2 as u32);
                let s = LONG_5_POW[s5 as usize].wrapping_shl(s2 as u32);
                let mut m = LONG_5_POW[m5 as usize].wrapping_shl(m2 as u32);
                let tens = s.wrapping_mul(10);
                let mut q = (b / s) as i32;
                b = 10i64.wrapping_mul(b % s);
                m = m.wrapping_mul(10);
                low = b < m;
                high = b.wrapping_add(m) > tens;
                if q == 0 && !high {
                    dec_exp -= 1;
                } else {
                    self.digits[ndigit] = b'0' + q as u8;
                    ndigit += 1;
                }
                if !is_compatible_format || !(-3..8).contains(&dec_exp) {
                    high = false;
                    low = false;
                }
                while !low && !high {
                    q = (b / s) as i32;
                    b = 10i64.wrapping_mul(b % s);
                    m = m.wrapping_mul(10);
                    if m > 0 {
                        low = b < m;
                        high = b.wrapping_add(m) > tens;
                    } else {
                        low = true;
                        high = true;
                    }
                    self.digits[ndigit] = b'0' + q as u8;
                    ndigit += 1;
                }
                low_digit_difference = (b << 1).wrapping_sub(tens);
                self.exact_decimal_conversion = b == 0;
            }
        } else {
            let s_val = BigUint::pow52(s5, s2);
            let mut b_val = BigUint::from_u64(fract_bits as u64).mul_pow52(b5, b2);
            let mut m_val = BigUint::pow52(m5 + 1, m2 + 1);
            let ten_s_val = BigUint::pow52(s5 + 1, s2 + 1);
            let mut q = b_val.quo_rem_iteration(&s_val);
            low = b_val.cmp(&m_val) == Ordering::Less;
            high = ten_s_val.cmp(&b_val.add(&m_val)) != Ordering::Greater;
            if q == 0 && !high {
                dec_exp -= 1;
            } else {
                self.digits[ndigit] = b'0' + q as u8;
                ndigit += 1;
            }
            if !is_compatible_format || !(-3..8).contains(&dec_exp) {
                high = false;
                low = false;
            }
            while !low && !high {
                q = b_val.quo_rem_iteration(&s_val);
                m_val = m_val.mul_small(10);
                low = b_val.cmp(&m_val) == Ordering::Less;
                high = ten_s_val.cmp(&b_val.add(&m_val)) != Ordering::Greater;
                self.digits[ndigit] = b'0' + q as u8;
                ndigit += 1;
            }
            if high && low {
                b_val = b_val.shl(1);
                low_digit_difference = match b_val.cmp(&ten_s_val) {
                    Ordering::Less => -1,
                    Ordering::Equal => 0,
                    Ordering::Greater => 1,
                };
            } else {
                low_digit_difference = 0;
            }
            self.exact_decimal_conversion = b_val.is_zero();
        }
        self.dec_exponent = dec_exp + 1;
        self.first_digit_index = 0;
        self.n_digits = ndigit;
        if high {
            if low {
                if low_digit_difference == 0 {
                    if (self.digits[self.first_digit_index + self.n_digits - 1] & 1) != 0 {
                        self.roundup();
                    }
                } else if low_digit_difference > 0 {
                    self.roundup();
                }
            } else {
                self.roundup();
            }
        }
    }

    /// `roundup()`.
    fn roundup(&mut self) {
        let mut i = self.first_digit_index + self.n_digits - 1;
        let mut q = self.digits[i];
        if q == b'9' {
            while q == b'9' && i > self.first_digit_index {
                self.digits[i] = b'0';
                i -= 1;
                q = self.digits[i];
            }
            if q == b'9' {
                self.dec_exponent += 1;
                self.digits[self.first_digit_index] = b'1';
                return;
            }
        }
        self.digits[i] = q + 1;
        self.decimal_digits_rounded_up = true;
    }

    /// `getChars(char[])`, as a string.
    fn java_format_string(&self) -> String {
        let digits = &self.digits[self.first_digit_index..self.first_digit_index + self.n_digits];
        let n_digits = self.n_digits;
        let mut r = String::with_capacity(26);
        if self.is_negative {
            r.push('-');
        }
        let push = |r: &mut String, d: &[u8]| {
            for &c in d {
                r.push(char::from(c));
            }
        };
        let dec_exponent = self.dec_exponent;
        if dec_exponent > 0 && dec_exponent < 8 {
            let char_length = n_digits.min(dec_exponent as usize);
            push(&mut r, &digits[..char_length]);
            if char_length < dec_exponent as usize {
                for _ in 0..(dec_exponent as usize - char_length) {
                    r.push('0');
                }
                r.push_str(".0");
            } else {
                r.push('.');
                if char_length < n_digits {
                    push(&mut r, &digits[char_length..]);
                } else {
                    r.push('0');
                }
            }
        } else if dec_exponent <= 0 && dec_exponent > -3 {
            r.push_str("0.");
            for _ in 0..(-dec_exponent) {
                r.push('0');
            }
            push(&mut r, digits);
        } else {
            r.push(char::from(digits[0]));
            r.push('.');
            if n_digits > 1 {
                push(&mut r, &digits[1..]);
            } else {
                r.push('0');
            }
            r.push('E');
            let e = if dec_exponent <= 0 {
                r.push('-');
                -dec_exponent + 1
            } else {
                dec_exponent - 1
            };
            r.push_str(&e.to_string());
        }
        r
    }
}

/// `estimateDecExp(fractBits, binExp)`, with Java's own constants (the last
/// is Java's literal `0.301029995663981`, not `log10(2)` rounded to a
/// `double`).
#[allow(clippy::approx_constant)]
fn estimate_dec_exp(fract_bits: i64, bin_exp: i32) -> i32 {
    let d2 = f64::from_bits((EXP_ONE | (fract_bits & SIGNIF_BIT_MASK)) as u64);
    let d = (d2 - 1.5) * 0.289529654 + 0.176091259 + f64::from(bin_exp) * 0.301029995663981;
    let d_bits = d.to_bits() as i64;
    let exponent = ((d_bits & EXP_BIT_MASK) >> EXP_SHIFT) as i32 - EXP_BIAS;
    let is_negative = d_bits < 0;
    if (0..52).contains(&exponent) {
        let mask = SIGNIF_BIT_MASK >> exponent;
        let r = (((d_bits & SIGNIF_BIT_MASK) | FRACT_HOB) >> (EXP_SHIFT - exponent)) as i32;
        if is_negative {
            if (mask & d_bits) == 0 {
                -r
            } else {
                -r - 1
            }
        } else {
            r
        }
    } else if exponent < 0 {
        if (d_bits & !i64::MIN) == 0 {
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

/// `insignificantDigitsForPow2(p2)`.
fn insignificant_digits_for_pow2(p2: i32) -> i32 {
    if p2 > 1 && (p2 as usize) < INSIGNIFICANT_DIGITS_NUMBER.len() {
        return INSIGNIFICANT_DIGITS_NUMBER[p2 as usize];
    }
    0
}

/// An exact unsigned big integer, little-endian 32-bit words, standing for
/// `FDBigInteger` (see the module documentation).
#[derive(Debug, Clone, PartialEq, Eq)]
struct BigUint {
    words: Vec<u32>,
}

impl BigUint {
    fn from_u64(v: u64) -> Self {
        let mut r = Self {
            words: vec![v as u32, (v >> 32) as u32],
        };
        r.trim();
        r
    }

    fn trim(&mut self) {
        while self.words.last() == Some(&0) {
            self.words.pop();
        }
    }

    fn is_zero(&self) -> bool {
        self.words.is_empty()
    }

    /// `valueOfPow52(p5, p2)`: `5^p5 * 2^p2`.
    fn pow52(p5: i32, p2: i32) -> Self {
        Self::from_u64(1).mul_pow52(p5, p2)
    }

    /// `this * 5^p5 * 2^p2`.
    fn mul_pow52(&self, p5: i32, p2: i32) -> Self {
        let mut r = self.clone();
        for _ in 0..p5 {
            r = r.mul_small(5);
        }
        r.shl(p2 as u32)
    }

    fn mul_small(&self, m: u32) -> Self {
        let mut carry = 0u64;
        let mut words = Vec::with_capacity(self.words.len() + 1);
        for &w in &self.words {
            let p = u64::from(w) * u64::from(m) + carry;
            words.push(p as u32);
            carry = p >> 32;
        }
        if carry != 0 {
            words.push(carry as u32);
        }
        let mut r = Self { words };
        r.trim();
        r
    }

    fn shl(&self, bits: u32) -> Self {
        if self.is_zero() {
            return self.clone();
        }
        let word_shift = (bits / 32) as usize;
        let bit_shift = bits % 32;
        let mut words = vec![0u32; word_shift];
        let mut carry = 0u32;
        for &w in &self.words {
            if bit_shift == 0 {
                words.push(w);
            } else {
                words.push((w << bit_shift) | carry);
                carry = w >> (32 - bit_shift);
            }
        }
        if carry != 0 {
            words.push(carry);
        }
        let mut r = Self { words };
        r.trim();
        r
    }

    fn add(&self, other: &Self) -> Self {
        let n = self.words.len().max(other.words.len());
        let mut words = Vec::with_capacity(n + 1);
        let mut carry = 0u64;
        for i in 0..n {
            let s = u64::from(*self.words.get(i).unwrap_or(&0))
                + u64::from(*other.words.get(i).unwrap_or(&0))
                + carry;
            words.push(s as u32);
            carry = s >> 32;
        }
        if carry != 0 {
            words.push(carry as u32);
        }
        let mut r = Self { words };
        r.trim();
        r
    }

    /// `this - other`, `this >= other`.
    fn sub(&self, other: &Self) -> Self {
        let mut words = Vec::with_capacity(self.words.len());
        let mut borrow = 0i64;
        for (i, &word) in self.words.iter().enumerate() {
            let mut d = i64::from(word) - i64::from(*other.words.get(i).unwrap_or(&0)) - borrow;
            if d < 0 {
                d += 1 << 32;
                borrow = 1;
            } else {
                borrow = 0;
            }
            words.push(d as u32);
        }
        let mut r = Self { words };
        r.trim();
        r
    }

    fn cmp(&self, other: &Self) -> Ordering {
        if self.words.len() != other.words.len() {
            return self.words.len().cmp(&other.words.len());
        }
        for i in (0..self.words.len()).rev() {
            match self.words[i].cmp(&other.words[i]) {
                Ordering::Equal => continue,
                o => return o,
            }
        }
        Ordering::Equal
    }

    /// `quoRemIteration(S)`: returns `this / S` and sets `this` to
    /// `10 * (this mod S)`; the quotient is below 10.
    fn quo_rem_iteration(&mut self, s: &Self) -> i32 {
        let mut q = 0;
        while self.cmp(s) != Ordering::Less {
            *self = self.sub(s);
            q += 1;
        }
        *self = self.mul_small(10);
        q
    }
}
