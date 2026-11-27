// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.DecimalQuantity_DualStorageBCD`.

use rucene::error::{LuceneError, Result};

use crate::jdk::math::{BigDecimal, BigInteger};
use crate::jdk::units;
use crate::math::big_decimal::BigDecimal as IcuBigDecimal;
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::decimal_quantity_abstract_bcd::{
    AbstractBcdFields, DecimalQuantityAbstractBcd, NEGATIVE_FLAG,
};

/// A `java.lang.Number` the `Number` constructor accepts: the subclasses
/// `DecimalQuantity_DualStorageBCD(Number)` tests with `instanceof`.
#[derive(Debug, Clone)]
pub enum JavaNumber {
    /// `java.lang.Long`.
    Long(i64),
    /// `java.lang.Integer`.
    Integer(i32),
    /// `java.lang.Float`.
    Float(f32),
    /// `java.lang.Double`.
    Double(f64),
    /// `java.math.BigInteger`.
    BigInteger(BigInteger),
    /// `java.math.BigDecimal`.
    BigDecimal(BigDecimal),
    /// `com.ibm.icu.math.BigDecimal`.
    IcuBigDecimal(IcuBigDecimal),
    /// Any other `Number` subclass (`Short`, `Byte`, `AtomicInteger`, ...):
    /// its class name, its `doubleValue()` and its `toString()`.
    Other {
        /// `getClass().getName()`.
        class_name: String,
        /// `doubleValue()`.
        double_value: f64,
        /// `toString()`.
        string: String,
    },
}

impl JavaNumber {
    /// `Number.doubleValue()`.
    pub fn double_value(&self) -> f64 {
        match self {
            JavaNumber::Long(n) => *n as f64,
            JavaNumber::Integer(n) => *n as f64,
            JavaNumber::Float(n) => *n as f64,
            JavaNumber::Double(n) => *n,
            JavaNumber::BigInteger(n) => n.double_value(),
            JavaNumber::BigDecimal(n) => n.double_value(),
            JavaNumber::IcuBigDecimal(n) => n.double_value(),
            JavaNumber::Other { double_value, .. } => *double_value,
        }
    }

    /// `Object.equals(Object)` of the boxed numbers: same class and equal
    /// value (`Double`/`Float` by bits, `BigDecimal` by value and scale).
    pub fn java_equals(&self, other: &JavaNumber) -> bool {
        match (self, other) {
            (JavaNumber::Long(a), JavaNumber::Long(b)) => a == b,
            (JavaNumber::Integer(a), JavaNumber::Integer(b)) => a == b,
            (JavaNumber::Float(a), JavaNumber::Float(b)) => float_bits(*a) == float_bits(*b),
            (JavaNumber::Double(a), JavaNumber::Double(b)) => double_bits(*a) == double_bits(*b),
            (JavaNumber::BigInteger(a), JavaNumber::BigInteger(b)) => a.java_equals(b),
            (JavaNumber::BigDecimal(a), JavaNumber::BigDecimal(b)) => a.java_equals(b),
            (JavaNumber::IcuBigDecimal(a), JavaNumber::IcuBigDecimal(b)) => a.java_equals(b),
            (
                JavaNumber::Other {
                    class_name: c1,
                    string: s1,
                    ..
                },
                JavaNumber::Other {
                    class_name: c2,
                    string: s2,
                    ..
                },
            ) => c1 == c2 && s1 == s2,
            _ => false,
        }
    }

    /// `Number.toString()`.
    pub fn to_java_string(&self) -> String {
        match self {
            JavaNumber::Long(n) => n.to_string(),
            JavaNumber::Integer(n) => n.to_string(),
            JavaNumber::Float(n) => rucene::util::java_fmt::float_to_string(*n),
            JavaNumber::Double(n) => rucene::util::java_fmt::double_to_string(*n),
            JavaNumber::BigInteger(n) => n.to_string(),
            JavaNumber::BigDecimal(n) => n.to_string(),
            JavaNumber::IcuBigDecimal(n) => n.to_string(),
            JavaNumber::Other { string, .. } => string.clone(),
        }
    }

    /// `getClass().getName()`.
    pub fn class_name(&self) -> &str {
        match self {
            JavaNumber::Long(_) => "java.lang.Long",
            JavaNumber::Integer(_) => "java.lang.Integer",
            JavaNumber::Float(_) => "java.lang.Float",
            JavaNumber::Double(_) => "java.lang.Double",
            JavaNumber::BigInteger(_) => "java.math.BigInteger",
            JavaNumber::BigDecimal(_) => "java.math.BigDecimal",
            JavaNumber::IcuBigDecimal(_) => "com.ibm.icu.math.BigDecimal",
            JavaNumber::Other { class_name, .. } => class_name,
        }
    }
}

/// `Float.floatToIntBits` (canonical NaN).
fn float_bits(f: f32) -> u32 {
    if f.is_nan() {
        0x7fc00000
    } else {
        f.to_bits()
    }
}

/// `Double.doubleToLongBits` (canonical NaN).
fn double_bits(d: f64) -> u64 {
    if d.is_nan() {
        0x7ff8000000000000
    } else {
        d.to_bits()
    }
}

/// A `DecimalQuantity` with internal storage as a 64-bit BCD, with fallback
/// to a byte array for numbers that don't fit into the standard BCD.
///
/// Equivalent to `com.ibm.icu.impl.number.DecimalQuantity_DualStorageBCD`.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone)]
pub struct DecimalQuantity_DualStorageBCD {
    /// The fields of `DecimalQuantity_AbstractBCD`.
    base: AbstractBcdFields,
    /// `bcdBytes`: the BCD of a long number, least significant digit first
    /// (Java `null` when not using bytes).
    bcd_bytes: Option<Vec<i8>>,
    /// `bcdLong`: the BCD of a short number, 4 bits per digit.
    bcd_long: i64,
    /// `usingBytes`.
    using_bytes: bool,
}

impl Default for DecimalQuantity_DualStorageBCD {
    fn default() -> Self {
        Self::new()
    }
}

impl DecimalQuantity_DualStorageBCD {
    fn empty() -> Self {
        Self {
            base: AbstractBcdFields::default(),
            bcd_bytes: None,
            bcd_long: 0,
            using_bytes: false,
        }
    }

    /// Equivalent to `DecimalQuantity_DualStorageBCD()`.
    pub fn new() -> Self {
        let mut dq = Self::empty();
        dq.set_bcd_to_zero();
        dq.base.flags = 0;
        dq
    }

    /// Equivalent to `DecimalQuantity_DualStorageBCD(long)`.
    pub fn from_long(input: i64) -> Self {
        let mut dq = Self::empty();
        dq.set_to_long(input);
        dq
    }

    /// Equivalent to `DecimalQuantity_DualStorageBCD(int)`.
    pub fn from_int(input: i32) -> Self {
        let mut dq = Self::empty();
        dq.set_to_int(input);
        dq
    }

    /// Equivalent to `DecimalQuantity_DualStorageBCD(double)`.
    pub fn from_double(input: f64) -> Self {
        let mut dq = Self::empty();
        dq.set_to_double(input);
        dq
    }

    /// Equivalent to `DecimalQuantity_DualStorageBCD(BigInteger)`.
    pub fn from_big_integer(input: &BigInteger) -> Self {
        let mut dq = Self::empty();
        dq.set_to_big_integer(input);
        dq
    }

    /// Equivalent to `DecimalQuantity_DualStorageBCD(BigDecimal)`.
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of `BigDecimal.scaleByPowerOfTen` (scale
    /// overflow).
    pub fn from_big_decimal(input: &BigDecimal) -> Result<Self> {
        let mut dq = Self::empty();
        dq.set_to_big_decimal(input)?;
        Ok(dq)
    }

    /// Equivalent to
    /// `DecimalQuantity_DualStorageBCD(DecimalQuantity_DualStorageBCD)`.
    pub fn from_other(other: &DecimalQuantity_DualStorageBCD) -> Self {
        let mut dq = Self::empty();
        dq.copy_from(other);
        dq
    }

    /// Equivalent to `DecimalQuantity_DualStorageBCD(Number)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Number is of an unsupported type: ...")`.
    pub fn from_number(number: &JavaNumber) -> Result<Self> {
        let mut dq = Self::empty();
        // NOTE: Number type expansion happens both here and in
        // NumberFormat.java
        match number {
            JavaNumber::Long(n) => dq.set_to_long(*n),
            JavaNumber::Integer(n) => dq.set_to_int(*n),
            JavaNumber::Float(n) => dq.set_to_double(*n as f64),
            JavaNumber::Double(n) => dq.set_to_double(*n),
            JavaNumber::BigInteger(n) => dq.set_to_big_integer(n),
            JavaNumber::BigDecimal(n) => dq.set_to_big_decimal(n)?,
            JavaNumber::IcuBigDecimal(n) => dq.set_to_big_decimal(&n.to_big_decimal())?,
            JavaNumber::Other { class_name, .. } => {
                return Err(LuceneError::illegal_argument(format!(
                    "Number is of an unsupported type: {class_name}"
                )));
            }
        }
        Ok(dq)
    }

    /// Parses a string in the exponent notation of the plural rules samples
    /// (`1.5e3`, `1.5c3`). Equivalent to `fromExponentString(String)`.
    ///
    /// # Errors
    ///
    /// The `NumberFormatException` of `Integer.parseInt` or `new
    /// BigDecimal(String)` on malformed input.
    pub fn from_exponent_string(num: &str) -> Result<DecimalQuantity_DualStorageBCD> {
        if num.contains('e') || num.contains('c') || num.contains('E') || num.contains('C') {
            let mut e_pos = java_last_index_of(num, 'e');
            if e_pos < 0 {
                e_pos = java_last_index_of(num, 'c');
            }
            if e_pos < 0 {
                e_pos = java_last_index_of(num, 'E');
            }
            if e_pos < 0 {
                e_pos = java_last_index_of(num, 'C');
            }
            let exp_num_pos = (e_pos + 1) as usize;
            let exponent_str = &num[exp_num_pos..];
            let exponent = rucene_parse_int(exponent_str)?;

            let fraction_str = &num[0..e_pos as usize];
            let fraction = BigDecimal::from_string(&units(fraction_str))?;

            let mut dq = DecimalQuantity_DualStorageBCD::from_big_decimal(&fraction)?;
            let num_frac_digit = Self::get_visible_fraction_count(fraction_str);
            dq.set_min_fraction(num_frac_digit);
            dq.adjust_exponent(exponent);

            Ok(dq)
        } else {
            let num_frac_digit = Self::get_visible_fraction_count(num);
            let mut dq = DecimalQuantity_DualStorageBCD::from_big_decimal(
                &BigDecimal::from_string(&units(num))?,
            )?;
            dq.set_min_fraction(num_frac_digit);
            Ok(dq)
        }
    }

    /// `getVisibleFractionCount(String)`.
    fn get_visible_fraction_count(value: &str) -> i32 {
        let decimal_pos = value.find('.').map(|i| i as i32).unwrap_or(-1) + 1;
        if decimal_pos == 0 {
            0
        } else {
            utf16_len(value) - decimal_pos
        }
    }

    /// `ensureCapacity()`.
    fn ensure_capacity_default(&mut self) {
        self.ensure_capacity(40);
    }

    /// `ensureCapacity(int)`.
    fn ensure_capacity(&mut self, capacity: i32) {
        if capacity == 0 {
            return;
        }
        let old_capacity = if self.using_bytes {
            self.bytes().len() as i32
        } else {
            0
        };
        if !self.using_bytes {
            self.bcd_bytes = Some(vec![0; capacity as usize]);
        } else if old_capacity < capacity {
            let mut bcd1 = vec![0i8; (capacity * 2) as usize];
            bcd1[..old_capacity as usize].copy_from_slice(&self.bytes()[..old_capacity as usize]);
            self.bcd_bytes = Some(bcd1);
        }
        self.using_bytes = true;
    }

    /// `switchStorage()`: switches the internal storage mechanism between
    /// the 64-bit long and the byte array.
    fn switch_storage(&mut self) {
        if self.using_bytes {
            // Change from bytes to long
            self.bcd_long = 0;
            let mut i = self.base.precision - 1;
            while i >= 0 {
                self.bcd_long <<= 4;
                self.bcd_long |= self.bytes()[i as usize] as i64;
                i -= 1;
            }
            self.bcd_bytes = None;
            self.using_bytes = false;
        } else {
            // Change from long to bytes
            self.ensure_capacity_default();
            for i in 0..self.base.precision {
                let digit = (self.bcd_long & 0xf) as i8;
                self.bytes_mut()[i as usize] = digit;
                self.bcd_long = ((self.bcd_long as u64) >> 4) as i64;
            }
            debug_assert!(self.using_bytes);
        }
    }

    /// The byte array (Java dereferences `bcdBytes` where it is non-null).
    fn bytes(&self) -> &[i8] {
        self.bcd_bytes
            .as_deref()
            .expect("java.lang.NullPointerException: bcdBytes")
    }

    /// The byte array, mutably.
    fn bytes_mut(&mut self) -> &mut [i8] {
        self.bcd_bytes
            .as_deref_mut()
            .expect("java.lang.NullPointerException: bcdBytes")
    }

    /// Checks whether the bytes are valid; `None` when healthy, else the
    /// reason. Equivalent to `checkHealth()` (deprecated, testing).
    pub fn check_health(&self) -> Option<&'static str> {
        let p = self.base.precision;
        if self.using_bytes {
            if self.bcd_long != 0 {
                return Some("Value in bcdLong but we are in byte mode");
            }
            if p == 0 {
                return Some("Zero precision but we are in byte mode");
            }
            if p > self.bytes().len() as i32 {
                return Some("Precision exceeds length of byte array");
            }
            if self.get_digit_pos(p - 1) == 0 {
                return Some("Most significant digit is zero in byte mode");
            }
            if self.get_digit_pos(0) == 0 {
                return Some("Least significant digit is zero in long mode");
            }
            for i in 0..p {
                if self.get_digit_pos(i) >= 10 {
                    return Some("Digit exceeding 10 in byte array");
                }
                if self.get_digit_pos(i) < 0 {
                    return Some("Digit below 0 in byte array");
                }
            }
            for i in p..self.bytes().len() as i32 {
                if self.get_digit_pos(i) != 0 {
                    return Some("Nonzero digits outside of range in byte array");
                }
            }
        } else {
            if let Some(bytes) = &self.bcd_bytes {
                for b in bytes {
                    if *b != 0 {
                        return Some("Nonzero digits in byte array but we are in long mode");
                    }
                }
            }
            if p == 0 && self.bcd_long != 0 {
                return Some("Value in bcdLong even though precision is zero");
            }
            if p > 16 {
                return Some("Precision exceeds length of long");
            }
            if p != 0 && self.get_digit_pos(p - 1) == 0 {
                return Some("Most significant digit is zero in long mode");
            }
            if p != 0 && self.get_digit_pos(0) == 0 {
                return Some("Least significant digit is zero in long mode");
            }
            for i in 0..p {
                if self.get_digit_pos(i) >= 10 {
                    return Some("Digit exceeding 10 in long");
                }
                if self.get_digit_pos(i) < 0 {
                    return Some("Digit below 0 in long (?!)");
                }
            }
            for i in p..16 {
                if self.get_digit_pos(i) != 0 {
                    return Some("Nonzero digits outside of range in long");
                }
            }
        }

        None
    }

    /// Whether the byte array is in use (testing). Equivalent to
    /// `isUsingBytes()` (deprecated).
    pub fn is_using_bytes(&self) -> bool {
        self.using_bytes
    }

    /// `toNumberString()`.
    fn to_number_string(&self) -> String {
        let mut sb = String::new();
        if self.using_bytes {
            if self.base.precision == 0 {
                sb.push('0');
            }
            let mut i = self.base.precision - 1;
            while i >= 0 {
                sb.push_str(&self.bytes()[i as usize].to_string());
                i -= 1;
            }
        } else {
            // Long.toHexString: the unsigned hexadecimal rendering.
            sb.push_str(&format!("{:x}", self.bcd_long as u64));
        }
        sb.push('E');
        sb.push_str(&self.base.scale.to_string());
        sb
    }
}

/// `String.lastIndexOf(int)` over the ASCII-only sample strings, as a byte
/// index (`-1` when absent).
fn java_last_index_of(s: &str, ch: char) -> i32 {
    s.rfind(ch).map(|i| i as i32).unwrap_or(-1)
}

/// `String.length()`: the UTF-16 length.
fn utf16_len(s: &str) -> i32 {
    s.encode_utf16().count() as i32
}

/// `Integer.parseInt(String)`.
fn rucene_parse_int(s: &str) -> Result<i32> {
    crate::jdk::parse_int(&units(s), 10)
}

impl DecimalQuantityAbstractBcd for DecimalQuantity_DualStorageBCD {
    fn fields(&self) -> &AbstractBcdFields {
        &self.base
    }

    fn fields_mut(&mut self) -> &mut AbstractBcdFields {
        &mut self.base
    }

    fn bcd_max_representable_digits(&self) -> i32 {
        i32::MAX
    }

    fn bcd_create_copy(&self) -> Box<dyn DecimalQuantity> {
        Box::new(DecimalQuantity_DualStorageBCD::from_other(self))
    }

    fn get_digit_pos(&self, position: i32) -> i8 {
        if self.using_bytes {
            if position < 0 || position >= self.base.precision {
                return 0;
            }
            self.bytes()[position as usize]
        } else {
            if !(0..16).contains(&position) {
                return 0;
            }
            (((self.bcd_long as u64) >> (position * 4)) & 0xf) as i8
        }
    }

    fn set_digit_pos(&mut self, position: i32, value: i8) {
        debug_assert!(position >= 0);
        if self.using_bytes {
            self.ensure_capacity(position + 1);
            self.bytes_mut()[position as usize] = value;
        } else if position >= 16 {
            self.switch_storage();
            self.ensure_capacity(position + 1);
            self.bytes_mut()[position as usize] = value;
        } else {
            let shift = position * 4;
            self.bcd_long = self.bcd_long & !(0xfi64 << shift) | ((value as i64) << shift);
        }
    }

    fn shift_left(&mut self, num_digits: i32) {
        if !self.using_bytes && self.base.precision + num_digits >= 16 {
            self.switch_storage();
        }
        if self.using_bytes {
            let precision = self.base.precision;
            self.ensure_capacity(precision + num_digits);
            let bytes = self.bytes_mut();
            bytes.copy_within(0..precision as usize, num_digits as usize);
            bytes[0..num_digits as usize].fill(0);
        } else {
            // Java masks the shift distance to 6 bits.
            self.bcd_long = self.bcd_long.wrapping_shl((num_digits * 4) as u32);
        }
        self.base.scale -= num_digits;
        self.base.precision += num_digits;
    }

    fn shift_right(&mut self, num_digits: i32) {
        if self.using_bytes {
            let precision = self.base.precision;
            let bytes = self.bytes_mut();
            let mut i = 0;
            while i < precision - num_digits {
                bytes[i as usize] = bytes[(i + num_digits) as usize];
                i += 1;
            }
            while i < precision {
                bytes[i as usize] = 0;
                i += 1;
            }
        } else {
            // Java masks the shift distance to 6 bits.
            self.bcd_long = (self.bcd_long as u64).wrapping_shr((num_digits * 4) as u32) as i64;
        }
        self.base.scale += num_digits;
        self.base.precision -= num_digits;
    }

    fn pop_from_left(&mut self, num_digits: i32) {
        debug_assert!(num_digits <= self.base.precision);
        let precision = self.base.precision;
        if self.using_bytes {
            let bytes = self.bytes_mut();
            let mut i = precision - 1;
            while i >= precision - num_digits {
                bytes[i as usize] = 0;
                i -= 1;
            }
        } else {
            self.bcd_long &= 1i64
                .wrapping_shl(((precision - num_digits) * 4) as u32)
                .wrapping_sub(1);
        }
        self.base.precision -= num_digits;
    }

    fn set_bcd_to_zero(&mut self) {
        if self.using_bytes {
            self.bcd_bytes = None;
            self.using_bytes = false;
        }
        self.bcd_long = 0;
        self.base.scale = 0;
        self.base.precision = 0;
        self.base.is_approximate = false;
        self.base.orig_double = 0.0;
        self.base.orig_delta = 0;
        self.base.exponent = 0;
    }

    fn read_int_to_bcd(&mut self, mut n: i32) {
        debug_assert!(n != 0);
        // ints always fit inside the long implementation.
        let mut result: i64 = 0;
        let mut i = 16;
        while n != 0 {
            result = ((result as u64) >> 4) as i64 + (((n as i64) % 10) << 60);
            n /= 10;
            i -= 1;
        }
        debug_assert!(!self.using_bytes);
        self.bcd_long = ((result as u64).wrapping_shr((i * 4) as u32)) as i64;
        self.base.scale = 0;
        self.base.precision = 16 - i;
    }

    fn read_long_to_bcd(&mut self, mut n: i64) {
        debug_assert!(n != 0);
        if n >= 10000000000000000 {
            self.ensure_capacity_default();
            let mut i = 0;
            while n != 0 {
                self.bytes_mut()[i as usize] = (n % 10) as i8;
                n /= 10;
                i += 1;
            }
            debug_assert!(self.using_bytes);
            self.base.scale = 0;
            self.base.precision = i;
        } else {
            let mut result: i64 = 0;
            let mut i = 16;
            while n != 0 {
                result = ((result as u64) >> 4) as i64 + ((n % 10) << 60);
                n /= 10;
                i -= 1;
            }
            debug_assert!(i >= 0);
            debug_assert!(!self.using_bytes);
            self.bcd_long = ((result as u64).wrapping_shr((i * 4) as u32)) as i64;
            self.base.scale = 0;
            self.base.precision = 16 - i;
        }
    }

    fn read_big_integer_to_bcd(&mut self, n: &BigInteger) {
        debug_assert!(n.signum() != 0);
        self.ensure_capacity_default(); // allocate initial byte array
        let mut n = n.clone();
        let mut i = 0;
        while n.signum() != 0 {
            let [quotient, remainder] = n
                .divide_and_remainder(&BigInteger::ten())
                .expect("BigInteger.TEN is not zero");
            self.ensure_capacity(i + 1);
            self.bytes_mut()[i as usize] = remainder.byte_value();
            n = quotient;
            i += 1;
        }
        self.base.scale = 0;
        self.base.precision = i;
    }

    fn bcd_to_big_decimal(&self) -> Result<BigDecimal> {
        if self.using_bytes {
            // Converting to a string here is faster than doing BigInteger/
            // BigDecimal arithmetic.
            let mut result = BigDecimal::from_string(&units(&self.to_number_string()))?;
            if (self.base.flags & NEGATIVE_FLAG) != 0 {
                result = result.negate();
            }
            Ok(result)
        } else {
            let mut temp_long: i64 = 0;
            let mut shift = self.base.precision - 1;
            while shift >= 0 {
                temp_long = temp_long
                    .wrapping_mul(10)
                    .wrapping_add(self.get_digit_pos(shift) as i64);
                shift -= 1;
            }
            let mut result = BigDecimal::value_of(temp_long);
            // Test that the new scale fits inside the BigDecimal
            let new_scale =
                result.scale() as i64 + self.base.scale as i64 + self.base.exponent as i64;
            if new_scale <= i32::MIN as i64 {
                result = BigDecimal::zero();
            } else {
                result = result
                    .scale_by_power_of_ten(self.base.scale.wrapping_add(self.base.exponent))?;
            }
            if (self.base.flags & NEGATIVE_FLAG) != 0 {
                result = result.negate();
            }
            Ok(result)
        }
    }

    fn compact(&mut self) {
        if self.using_bytes {
            let mut delta = 0;
            while delta < self.base.precision && self.bytes()[delta as usize] == 0 {
                delta += 1;
            }
            if delta == self.base.precision {
                // Number is zero
                self.set_bcd_to_zero();
                return;
            } else {
                // Remove trailing zeros
                self.shift_right(delta);
            }

            // Compute precision
            let mut leading = self.base.precision - 1;
            while leading >= 0 && self.bytes()[leading as usize] == 0 {
                leading -= 1;
            }
            self.base.precision = leading + 1;

            // Switch storage mechanism if possible
            if self.base.precision <= 16 {
                self.switch_storage();
            }
        } else {
            if self.bcd_long == 0 {
                // Number is zero
                self.set_bcd_to_zero();
                return;
            }

            // Compact the number (remove trailing zeros)
            let delta = (self.bcd_long.trailing_zeros() / 4) as i32;
            self.bcd_long = ((self.bcd_long as u64) >> (delta * 4)) as i64;
            self.base.scale += delta;

            // Compute precision
            self.base.precision = 16 - (self.bcd_long.leading_zeros() / 4) as i32;
        }
    }

    fn copy_bcd_from(&mut self, other: &dyn DecimalQuantity) {
        let other = other
            .as_any()
            .downcast_ref::<DecimalQuantity_DualStorageBCD>()
            .unwrap_or_else(|| {
                panic!(
                    "java.lang.ClassCastException: class cannot be cast to class \
                 com.ibm.icu.impl.number.DecimalQuantity_DualStorageBCD"
                )
            });
        self.set_bcd_to_zero();
        if other.using_bytes {
            self.ensure_capacity(other.base.precision);
            let n = other.base.precision as usize;
            let src = other.bytes()[..n].to_vec();
            self.bytes_mut()[..n].copy_from_slice(&src);
        } else {
            self.bcd_long = other.bcd_long;
        }
    }

    fn bcd_to_string(&self) -> String {
        format!(
            "<DecimalQuantity {}:{} {} {}{}>",
            self.base.l_req_pos,
            self.base.r_req_pos,
            if self.using_bytes { "bytes" } else { "long" },
            if (self.base.flags & NEGATIVE_FLAG) != 0 {
                "-"
            } else {
                ""
            },
            self.to_number_string()
        )
    }
}
