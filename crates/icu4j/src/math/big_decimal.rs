// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/* Generated from 'BigDecimal.nrx' 8 Sep 2000 11:10:50 [v2.00] */
/* Options: Binary Comments Crossref Format Java Logo Strictargs Strictcase Trace2 Verbose3 */
/* Copyright IBM Corporation, 1996-2016.  All Rights Reserved.       */

//! `com.ibm.icu.math.BigDecimal`.

use std::fmt;
use std::sync::LazyLock;

use rucene::error::{LuceneError, Result};

use crate::jdk::math::{BigDecimal as JBigDecimal, BigInteger};
use crate::jdk::{to_rust_string, units};
use crate::lang::u_character::UCharacter;
use crate::math::math_context::MathContext;

const ISPOS: i8 = 1; // ind: indicates positive (must be 1)
const ISZERO: i8 = 0; // ind: indicates zero (must be 0)
const ISNEG: i8 = -1; // ind: indicates negative (must be -1)

const MIN_EXP: i32 = -999999999; // minimum exponent allowed
const MAX_EXP: i32 = 999999999; // maximum exponent allowed
const MIN_ARG: i32 = -999999999; // minimum argument integer
const MAX_ARG: i32 = 999999999; // maximum argument integer

/// `plainMC`: the context for the methods without a `MathContext`
/// (digits 0, `PLAIN`, no lost-digits checking, `ROUND_HALF_UP`).
const PLAIN_MC: MathContext =
    MathContext { digits: 0, form: MathContext::PLAIN, lost_digits: false, rounding_mode: MathContext::ROUND_HALF_UP };

/// `BigDecimal.ZERO`: 0, scale 0.
pub static ZERO: LazyLock<BigDecimal> = LazyLock::new(|| BigDecimal::from_long(0));

/// `BigDecimal.ONE`: 1, scale 0.
pub static ONE: LazyLock<BigDecimal> = LazyLock::new(|| BigDecimal::from_long(1));

/// `BigDecimal.TEN`: 10, scale 0.
pub static TEN: LazyLock<BigDecimal> = LazyLock::new(|| BigDecimal::from_int(10));

/// An immutable arbitrary-precision decimal number, following the ANSI
/// X3.274-1996 arithmetic (with `MathContext` settings), and the
/// `java.math.BigDecimal` semantics for the operations without one.
///
/// Equivalent to `com.ibm.icu.math.BigDecimal`: a sign (`ind`), a notation
/// (`form`), a mantissa of decimal digits (`mant`, one digit per byte,
/// most significant first) and an exponent (`exp`). The methods that build
/// a result on a fresh copy and then mutate it (`clone(x).round(set)`,
/// `res.finish(...)`) are translated as the same mutations on an owned
/// copy.
///
/// Texts this class renders (`toString()`, `format`) are ASCII; they are
/// returned as Rust `String`s, and [`to_char_array`](Self::to_char_array)
/// returns the UTF-16 units.
#[derive(Debug, Clone)]
pub struct BigDecimal {
    /// `ind`: the indicator (sign).
    ind: i8,
    /// `form`: the formatting style (`PLAIN` unless built from an
    /// exponent, or by a non-`PLAIN` `MathContext`).
    form: i8,
    /// `mant`: the coefficient digits, no leading zeros (except a lone 0).
    mant: Vec<i8>,
    /// `exp`: the exponent.
    exp: i32,
}

impl PartialEq for BigDecimal {
    /// `equals(Object obj)`.
    fn eq(&self, other: &Self) -> bool {
        self.java_equals(other)
    }
}

fn bad(s: &[u16]) -> LuceneError {
    LuceneError::number_format(format!("Not a number: {}", to_rust_string(s)))
}

fn badarg(name: &str, pos: i32, value: &str) -> LuceneError {
    LuceneError::illegal_argument(format!("Bad argument {pos} to {name}: {value}"))
}

fn arith(message: String) -> LuceneError {
    crate::jdk::arithmetic(&message)
}

/// `extend(byte inarr[], int newlen)`: `inarr` padded with zeros to
/// `newlen` digits.
fn extend(inarr: &[i8], newlen: usize) -> Vec<i8> {
    let mut newarr = inarr.to_vec();
    newarr.resize(newlen, 0);
    newarr
}

/// The `bytedig`/`bytecar` tables of `diginit()`: the digit and the carry
/// of `dp90 - 90`.
fn digit_and_carry(dp90: i32) -> (i8, i8) {
    let mut digit = dp90 - 90;
    if digit >= 0 {
        return ((digit % 10) as i8, (digit / 10) as i8);
    }
    // borrowing...
    digit += 100;
    ((digit % 10) as i8, ((digit / 10) - 10) as i8)
}

/// `byteaddsub(byte a[], int avlen, byte b[], int bvlen, int m, boolean
/// reuse)`: `a[0..avlen] + b[0..bvlen] × m`, right-aligned. Java may write
/// the result into `a` when `reuse` holds and the lengths fit; the result
/// length and digits are the same either way, so the port always returns
/// a new array.
fn byteaddsub(a: &[i8], avlen: i32, b: &[i8], bvlen: i32, m: i32) -> Vec<i8> {
    let alength = a.len() as i32;
    let blength = b.len() as i32;
    let mut ap = avlen - 1;
    let mut bp = bvlen - 1;
    let mut maxarr = bp;
    if maxarr < ap {
        maxarr = ap;
    }
    let mut reb = vec![0i8; (maxarr + 1) as usize];
    let quickm = m == 1 || m == -1;
    let mut digit: i32 = 0;
    let mut op = maxarr;
    while op >= 0 {
        if ap >= 0 {
            if ap < alength {
                digit += a[ap as usize] as i32; // within A
            }
            ap -= 1;
        }
        if bp >= 0 {
            if bp < blength {
                // within B
                if quickm {
                    if m > 0 {
                        digit += b[bp as usize] as i32;
                    } else {
                        digit -= b[bp as usize] as i32;
                    }
                } else {
                    digit += b[bp as usize] as i32 * m;
                }
            }
            bp -= 1;
        }
        // result so far (digit) could be -90 through 99
        if (0..10).contains(&digit) {
            reb[op as usize] = digit as i8;
            digit = 0; // no carry
        } else {
            let (d, c) = digit_and_carry(digit + 90);
            reb[op as usize] = d; // this digit
            digit = c as i32; // carry or borrow
        }
        op -= 1;
    }
    if digit == 0 {
        return reb; // no carry
    }
    // following line will become an Assert, later
    // if digit<0 then signal ArithmeticException("internal.error ["digit"]")

    // We have carry -- need to make space for the extra digit
    let mut newarr = Vec::with_capacity((maxarr + 2) as usize);
    newarr.push(digit as i8);
    newarr.extend_from_slice(&reb);
    newarr
}

/// `allzero(byte array[], int start)`: whether every digit from `start`
/// (0 if negative) is zero.
fn allzero(array: &[i8], start: i32) -> bool {
    let start = start.max(0) as usize;
    array.iter().skip(start).all(|&d| d == 0)
}

impl BigDecimal {
    /// `ROUND_CEILING`.
    pub const ROUND_CEILING: i32 = MathContext::ROUND_CEILING;
    /// `ROUND_DOWN`.
    pub const ROUND_DOWN: i32 = MathContext::ROUND_DOWN;
    /// `ROUND_FLOOR`.
    pub const ROUND_FLOOR: i32 = MathContext::ROUND_FLOOR;
    /// `ROUND_HALF_DOWN`.
    pub const ROUND_HALF_DOWN: i32 = MathContext::ROUND_HALF_DOWN;
    /// `ROUND_HALF_EVEN`.
    pub const ROUND_HALF_EVEN: i32 = MathContext::ROUND_HALF_EVEN;
    /// `ROUND_HALF_UP`.
    pub const ROUND_HALF_UP: i32 = MathContext::ROUND_HALF_UP;
    /// `ROUND_UNNECESSARY`.
    pub const ROUND_UNNECESSARY: i32 = MathContext::ROUND_UNNECESSARY;
    /// `ROUND_UP`.
    pub const ROUND_UP: i32 = MathContext::ROUND_UP;

    /// The private constructor `BigDecimal()`: positive, `PLAIN`, empty
    /// mantissa, exponent 0.
    fn empty() -> Self {
        Self { ind: 0, form: MathContext::PLAIN as i8, mant: Vec::new(), exp: 0 }
    }

    /// `new BigDecimal(java.math.BigDecimal bd)`: from `bd.toString()`.
    ///
    /// # Errors
    ///
    /// As [`from_string`](Self::from_string).
    pub fn from_java_big_decimal(bd: &JBigDecimal) -> Result<Self> {
        Self::from_string(&units(&bd.to_string()))
    }

    /// `new BigDecimal(java.math.BigInteger bi)`.
    ///
    /// # Errors
    ///
    /// As [`from_string`](Self::from_string).
    pub fn from_big_integer(bi: &BigInteger) -> Result<Self> {
        Self::from_string(&units(&bi.to_string_radix(10)))
    }

    /// `new BigDecimal(java.math.BigInteger bi, int scale)`.
    ///
    /// # Errors
    ///
    /// `NumberFormatException("Negative scale: <scale>")`.
    pub fn from_big_integer_scaled(bi: &BigInteger, scale: i32) -> Result<Self> {
        let mut r = Self::from_string(&units(&bi.to_string_radix(10)))?;
        if scale < 0 {
            return Err(LuceneError::number_format(format!("Negative scale: {scale}")));
        }
        r.exp = -scale; // exponent is -scale
        Ok(r)
    }

    /// `new BigDecimal(char inchars[])` and `new BigDecimal(String
    /// string)`.
    ///
    /// # Errors
    ///
    /// As [`from_chars_range`](Self::from_chars_range).
    pub fn from_string(inchars: &[u16]) -> Result<Self> {
        Self::from_chars_range(inchars, 0, inchars.len() as i32)
    }

    /// `new BigDecimal(char inchars[], int offset, int length)`.
    ///
    /// # Errors
    ///
    /// `NumberFormatException("Not a number: <inchars>")` (the whole array)
    /// for a malformed number or an exponent out of range.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` when `offset` is outside the array.
    pub fn from_chars_range(inchars: &[u16], mut offset: i32, mut length: i32) -> Result<Self> {
        let mut this = Self::empty();
        let at = |i: i32| -> u16 {
            match inchars.get(i as usize) {
                Some(&c) if i >= 0 => c,
                _ => crate::jdk::array_index_out_of_bounds(i),
            }
        };
        let is_digit = |c: u16| UCharacter::is_digit(c as i32);
        let digit10 = |c: u16| UCharacter::digit_radix(c as i32, 10);
        // This is the primary constructor; all incoming strings end up
        // here; it uses explicit (inline) parsing for speed and to avoid
        // generating intermediate (temporary) objects of any kind.
        // 1998.06.25: exponent form built only if E/e in string
        // 1998.06.25: trailing zeros not removed for zero
        // 1999.03.06: no embedded blanks; allow offset and length
        if length <= 0 {
            return Err(bad(inchars)); // bad conversion (empty string)
        }
        // [bad offset will raise array bounds exception]

        // Handle and step past sign
        this.ind = ISPOS; // assume positive
        if at(offset) == b'-' as u16 {
            length -= 1;
            if length == 0 {
                return Err(bad(inchars)); // nothing after sign
            }
            this.ind = ISNEG;
            offset += 1;
        } else if at(offset) == b'+' as u16 {
            length -= 1;
            if length == 0 {
                return Err(bad(inchars)); // nothing after sign
            }
            offset += 1;
        }

        // We're at the start of the number
        let mut exotic = false; // have extra digits
        let mut hadexp = false; // had explicit exponent
        let mut d: i32 = 0; // count of digits found
        let mut dotoff: i32 = -1; // offset where dot was found
        let mut last: i32 = -1; // last character of mantissa
        {
            let mut n = length;
            let mut i = offset;
            'i: while n > 0 {
                let si = at(i);
                if si >= b'0' as u16 && si <= b'9' as u16 {
                    // test for Arabic digit
                    last = i;
                    d += 1; // still in mantissa
                    n -= 1;
                    i += 1;
                    continue 'i;
                }
                if si == b'.' as u16 {
                    // record and ignore
                    if dotoff >= 0 {
                        return Err(bad(inchars)); // two dots
                    }
                    dotoff = i - offset; // offset into mantissa
                    n -= 1;
                    i += 1;
                    continue 'i;
                }
                if si != b'e' as u16 && si != b'E' as u16 {
                    // expect an extra digit
                    if !is_digit(si) {
                        return Err(bad(inchars)); // not a number
                    }
                    // defer the base 10 check until later to avoid extra method call
                    exotic = true; // will need conversion later
                    last = i;
                    d += 1; // still in mantissa
                    n -= 1;
                    i += 1;
                    continue 'i;
                }
                // Found 'e' or 'E' -- now process explicit exponent
                // 1998.07.11: sign no longer required
                if (i - offset) > (length - 2) {
                    return Err(bad(inchars)); // no room for even one digit
                }
                let mut eneg = false;
                let k;
                if at(i + 1) == b'-' as u16 {
                    eneg = true;
                    k = i + 2;
                } else if at(i + 1) == b'+' as u16 {
                    k = i + 2;
                } else {
                    k = i + 1;
                }
                // k is offset of first expected digit
                let elen = length - (k - offset); // possible number of digits
                if (elen == 0) | (elen > 9) {
                    return Err(bad(inchars)); // 0 or more than 9 digits
                }
                {
                    let mut m = elen;
                    let mut j = k;
                    while m > 0 {
                        let sj = at(j);
                        if sj < b'0' as u16 {
                            return Err(bad(inchars)); // always bad
                        }
                        let dvalue;
                        if sj > b'9' as u16 {
                            // maybe an exotic digit
                            if !is_digit(sj) {
                                return Err(bad(inchars)); // not a number
                            }
                            dvalue = digit10(sj); // check base
                            if dvalue < 0 {
                                return Err(bad(inchars)); // not base 10
                            }
                        } else {
                            dvalue = (sj - b'0' as u16) as i32;
                        }
                        this.exp = (this.exp * 10) + dvalue;
                        m -= 1;
                        j += 1;
                    }
                }
                if eneg {
                    this.exp = -this.exp; // was negative
                }
                hadexp = true; // remember we had one
                break 'i; // we are done
            }
        }

        // Here when all inspected
        if d == 0 {
            return Err(bad(inchars)); // no mantissa digits
        }
        if dotoff >= 0 {
            this.exp = (this.exp + dotoff) - d; // adjust exponent if had dot
        }

        // strip leading zeros/dot (leave final if all 0's)
        {
            let mut i = offset;
            while i <= last - 1 {
                let si = at(i);
                if si == b'0' as u16 {
                    offset += 1;
                    dotoff -= 1;
                    d -= 1;
                } else if si == b'.' as u16 {
                    offset += 1; // step past dot
                    dotoff -= 1;
                } else if si <= b'9' as u16 {
                    break; // non-0
                } else {
                    // exotic
                    if digit10(si) != 0 {
                        break; // non-0 or bad
                    }
                    // is 0 .. strip like '0'
                    offset += 1;
                    dotoff -= 1;
                    d -= 1;
                }
                i += 1;
            }
        }

        // Create the mantissa array
        this.mant = vec![0; d as usize]; // we know the length
        let mut j = offset; // input offset
        if exotic {
            // slow: check for exotica
            for i in 0..d {
                if i == dotoff {
                    j += 1; // at dot
                }
                let sj = at(j);
                if sj <= b'9' as u16 {
                    this.mant[i as usize] = (sj as i32 - b'0' as i32) as i8; /* easy */
                } else {
                    let dvalue = digit10(sj);
                    if dvalue < 0 {
                        return Err(bad(inchars)); // not a number after all
                    }
                    this.mant[i as usize] = dvalue as i8;
                }
                j += 1;
            }
        } else {
            for i in 0..d {
                if i == dotoff {
                    j += 1;
                }
                this.mant[i as usize] = (at(j) as i32 - b'0' as i32) as i8;
                j += 1;
            }
        }

        // Looks good. Set the sign indicator and form, as needed.
        // Trailing zeros are preserved
        // The rule here for form is:
        // If no E-notation, then request plain notation
        // Otherwise act as though add(0,DEFAULT) and request scientific notation
        // [form is already PLAIN]
        if this.mant[0] == 0 {
            this.ind = ISZERO; // force to show zero
            // negative exponent is significant (e.g., -3 for 0.000) if plain
            if this.exp > 0 {
                this.exp = 0; // positive exponent can be ignored
            }
            if hadexp {
                // zero becomes single digit from add
                this.mant = vec![0];
                this.exp = 0;
            }
        } else {
            // non-zero
            // [ind was set earlier]
            // now determine form
            if hadexp {
                this.form = MathContext::SCIENTIFIC as i8;
                // 1999.06.29 check for overflow
                let mag = (this.exp + this.mant.len() as i32) - 1; // true exponent in scientific notation
                if (mag < MIN_EXP) | (mag > MAX_EXP) {
                    return Err(bad(inchars));
                }
            }
        }
        // say 'BD(c[]): mant[0] mantlen exp ind form:' mant[0] mant.length exp ind form
        Ok(this)
    }

    /// `new BigDecimal(double num)`: the exact value of the binary
    /// double, as `new java.math.BigDecimal(num).toString()`.
    ///
    /// # Errors
    ///
    /// `NumberFormatException` for an infinite or NaN value (from
    /// `java.math.BigDecimal`).
    pub fn from_double(num: f64) -> Result<Self> {
        // 1999.03.06: use exactly the old algorithm
        // 2000.01.01: note that this constructor does give an exact result,
        // so perhaps it should not be deprecated
        // 2000.06.18: no longer deprecated
        Self::from_string(&units(&JBigDecimal::from_double(num)?.to_string()))
    }

    /// `new BigDecimal(int num)`.
    pub fn from_int(mut num: i32) -> Self {
        let mut this = Self::empty();
        // We fastpath commoners
        if (-9..=9).contains(&num) {
            // very common single digit case
            if num == 0 {
                this.mant = vec![0];
                this.ind = ISZERO;
            } else if num == 1 {
                this.mant = vec![1];
                this.ind = ISPOS;
            } else if num == -1 {
                this.mant = vec![1];
                this.ind = ISNEG;
            } else if num > 0 {
                this.mant = vec![num as i8];
                this.ind = ISPOS;
            } else {
                // num<-1
                this.mant = vec![(-num) as i8];
                this.ind = ISNEG;
            }
            return this;
        }

        // We work on negative numbers so we handle the most negative number
        if num > 0 {
            this.ind = ISPOS;
            num = -num;
        } else {
            this.ind = ISNEG; /* negative */ // [0 case already handled]
        }
        // [it is quicker, here, to pre-calculate the length with
        // one loop, then allocate exactly the right length of byte array,
        // then re-fill it with another loop]
        let mut mun = num; // working copy
        let mut i = 9;
        loop {
            mun /= 10;
            if mun == 0 {
                break;
            }
            i -= 1;
        }
        // i is the position of the leftmost digit placed
        this.mant = vec![0; (10 - i) as usize];
        let mut i = (10 - i) - 1;
        loop {
            this.mant[i as usize] = -((num % 10) as i8);
            num /= 10;
            if num == 0 {
                break;
            }
            i -= 1;
        }
        this
    }

    /// `new BigDecimal(long num)`.
    pub fn from_long(mut num: i64) -> Self {
        let mut this = Self::empty();
        // Not really worth fastpathing commoners in this constructor [also,
        // we use this to construct the static constants].
        // This is much faster than: this(String.valueOf(num).toCharArray())
        /* We work on negative num so we handle the most negative number */
        if num > 0 {
            this.ind = ISPOS;
            num = -num;
        } else if num == 0 {
            this.ind = ISZERO;
        } else {
            this.ind = ISNEG; /* negative */
        }
        let mut mun = num;
        let mut i = 18;
        loop {
            mun /= 10;
            if mun == 0 {
                break;
            }
            i -= 1;
        }
        // i is the position of the leftmost digit placed
        this.mant = vec![0; (19 - i) as usize];
        let mut i = (19 - i) - 1;
        loop {
            this.mant[i as usize] = -((num % 10) as i8);
            num /= 10;
            if num == 0 {
                break;
            }
            i -= 1;
        }
        this
    }

    /// `abs()`.
    ///
    /// # Errors
    ///
    /// As [`abs_mc`](Self::abs_mc).
    pub fn abs(&self) -> Result<Self> {
        self.abs_mc(&PLAIN_MC)
    }

    /// `abs(MathContext set)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` for lost digits or an exponent overflow.
    pub fn abs_mc(&self, set: &MathContext) -> Result<Self> {
        if self.ind == ISNEG {
            return self.negate_mc(set);
        }
        self.plus_mc(set)
    }

    /// `add(BigDecimal rhs)`.
    ///
    /// # Errors
    ///
    /// As [`add_mc`](Self::add_mc).
    pub fn add(&self, rhs: &BigDecimal) -> Result<Self> {
        self.add_mc(rhs, &PLAIN_MC)
    }

    /// `add(BigDecimal rhs, MathContext set)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` (`Too many digits: ...` with lost-digits
    /// checking, `Exponent Overflow: ...`).
    pub fn add_mc(&self, rhs: &BigDecimal, set: &MathContext) -> Result<Self> {
        if set.lost_digits {
            self.checkdigits(Some(rhs), set.digits)?;
        }
        let mut lhs = self.clone(); // name for clarity and proxy
        let mut rhs = rhs.clone();

        /* Quick exit for add floating 0 */
        // plus() will optimize to return same object if possible
        if lhs.ind == 0 && set.form != MathContext::PLAIN {
            return rhs.plus_mc(set);
        }
        if rhs.ind == 0 && set.form != MathContext::PLAIN {
            return lhs.plus_mc(set);
        }

        /* Prepare numbers (round, unless unlimited precision) */
        let reqdig = set.digits; // local copy (heavily used)
        if reqdig > 0 {
            if lhs.mant.len() as i32 > reqdig {
                lhs.round_mc(set)?;
            }
            if rhs.mant.len() as i32 > reqdig {
                rhs.round_mc(set)?;
            }
            // [we could reuse the new LHS for result in this case]
        }

        let mut res = Self::empty(); // build result here

        /* Now see how much we have to pad or truncate lhs or rhs in order
           to align the numbers.  If one number is much larger than the
           other, then the smaller cannot affect the answer [but we may
           still need to pad with up to DIGITS trailing zeros]. */
        // Note sign may be 0 if digits (reqdig) is 0
        // usel and user will be the byte arrays passed to the adder; we'll
        // use them on all paths except quick exits
        let mut usel = lhs.mant.clone();
        let mut usellen = lhs.mant.len() as i32;
        let mut user = rhs.mant.clone();
        let mut userlen = rhs.mant.len() as i32;
        if lhs.exp == rhs.exp {
            /* no padding needed */
            // This is the most common, and fastest, path
            res.exp = lhs.exp;
        } else if lhs.exp > rhs.exp {
            // need to pad lhs and/or truncate rhs
            let mut newlen = (usellen + lhs.exp) - rhs.exp;
            /* If, after pad, lhs would be longer than rhs by digits+1 or
               more (and digits>0) then rhs cannot affect answer, so we only
               need to pad up to a length of DIGITS+1. */
            if newlen >= ((userlen + reqdig) + 1) && reqdig > 0 {
                // LHS is sufficient
                res.mant = usel;
                res.exp = lhs.exp;
                res.ind = lhs.ind;
                if usellen < reqdig {
                    // need 0 padding
                    res.mant = extend(&lhs.mant, reqdig as usize);
                    res.exp -= reqdig - usellen;
                }
                return res.finish(set, false);
            }
            // RHS may affect result
            res.exp = rhs.exp; // expected final exponent
            if newlen > (reqdig + 1) && reqdig > 0 {
                // LHS will be max; RHS truncated
                let tlen = (newlen - reqdig) - 1; // truncation length
                userlen -= tlen;
                res.exp += tlen;
                newlen = reqdig + 1;
            }
            if newlen > usellen {
                usellen = newlen; // need to pad LHS
            }
        } else {
            // need to pad rhs and/or truncate lhs
            let mut newlen = (userlen + rhs.exp) - lhs.exp;
            if newlen >= ((usellen + reqdig) + 1) && reqdig > 0 {
                // RHS is sufficient
                res.mant = user;
                res.exp = rhs.exp;
                res.ind = rhs.ind;
                if userlen < reqdig {
                    // need 0 padding
                    res.mant = extend(&rhs.mant, reqdig as usize);
                    res.exp -= reqdig - userlen;
                }
                return res.finish(set, false);
            }
            // LHS may affect result
            res.exp = lhs.exp; // expected final exponent
            if newlen > (reqdig + 1) && reqdig > 0 {
                // RHS will be max; LHS truncated
                let tlen = (newlen - reqdig) - 1; // truncation length
                usellen -= tlen;
                res.exp += tlen;
                newlen = reqdig + 1;
            }
            if newlen > userlen {
                userlen = newlen; // need to pad RHS
            }
        }

        /* OK, we have aligned mantissas.  Now add or subtract. */
        // 1998.06.27 Sign may now be 0 [e.g., 0.000] .. treat as positive
        // 1999.05.27 Allow for 00 on lhs [is not larger than 2 on rhs]
        // 1999.07.10 Allow for 00 on rhs [is not larger than 2 on rhs]
        if lhs.ind == ISZERO {
            res.ind = ISPOS;
        } else {
            res.ind = lhs.ind; // likely sign, all paths
        }
        let mult;
        if (lhs.ind == ISNEG) == (rhs.ind == ISNEG) {
            // same sign, 0 non-negative
            mult = 1;
        } else {
            mult = -1; // will cause subtract
            /* Before we can subtract we must determine which is the larger,
               as our add/subtract routine only handles non-negative results
               so we may need to swap the operands. */
            if rhs.ind == ISZERO {
                // original A bigger
            } else if (usellen < userlen) | (lhs.ind == ISZERO) {
                // original B bigger
                std::mem::swap(&mut usel, &mut user); // swap
                std::mem::swap(&mut usellen, &mut userlen);
                res.ind = -res.ind; // and set sign
            } else if usellen > userlen {
                // original A bigger
            } else {
                /* logical lengths the same */ // need compare
                /* may still need to swap: compare the strings */
                let mut ia = 0usize;
                let mut ib = 0usize;
                let ea = usel.len() as i32 - 1;
                let eb = user.len() as i32 - 1;
                loop {
                    let ca: i8;
                    if ia as i32 <= ea {
                        ca = usel[ia];
                    } else {
                        if ib as i32 > eb {
                            /* identical */
                            if set.form != MathContext::PLAIN {
                                return Ok(ZERO.clone());
                            }
                            // [if PLAIN we must do the subtract, in case of 0.000 results]
                            break;
                        }
                        ca = 0;
                    }
                    let cb: i8 = if ib as i32 <= eb { user[ib] } else { 0 };
                    if ca != cb {
                        if ca < cb {
                            /* swap needed */
                            std::mem::swap(&mut usel, &mut user); // swap
                            std::mem::swap(&mut usellen, &mut userlen);
                            res.ind = -res.ind;
                        }
                        break;
                    }
                    /* mantissas the same, so far */
                    ia += 1;
                    ib += 1;
                }
            } // lengths the same
        }

        /* here, A is > B if subtracting */
        // add [A+B*1] or subtract [A+(B*-1)]
        res.mant = byteaddsub(&usel, usellen, &user, userlen, mult);
        // [reuse possible only after chop; accounting makes not worthwhile]

        // Finish() rounds before stripping leading 0's, then sets form, etc.
        res.finish(set, false)
    }

    /// `compareTo(BigDecimal rhs)`: -1, 0 or 1 (numeric comparison, scale
    /// ignored).
    ///
    /// # Errors
    ///
    /// As [`compare_to_mc`](Self::compare_to_mc).
    pub fn compare_to(&self, rhs: &BigDecimal) -> Result<i32> {
        self.compare_to_mc(rhs, &PLAIN_MC)
    }

    /// `compareTo(BigDecimal rhs, MathContext set)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` for lost digits or an exponent overflow.
    pub fn compare_to_mc(&self, rhs: &BigDecimal, set: &MathContext) -> Result<i32> {
        if set.lost_digits {
            self.checkdigits(Some(rhs), set.digits)?;
        }
        // [would short-cut here if references are identical]
        if (self.ind == rhs.ind) & (self.exp == rhs.exp) {
            /* sign & exponent the same [very common] */
            let thislength = self.mant.len();
            if thislength < rhs.mant.len() {
                return Ok(-self.ind as i32);
            }
            if thislength > rhs.mant.len() {
                return Ok(self.ind as i32);
            }
            /* lengths are the same; we can do a straight mantissa compare
               unless maybe rounding [rounding is very unusual] */
            if (thislength as i32 <= set.digits) | (set.digits == 0) {
                for i in 0..thislength {
                    if self.mant[i] < rhs.mant[i] {
                        return Ok(-self.ind as i32);
                    }
                    if self.mant[i] > rhs.mant[i] {
                        return Ok(self.ind as i32);
                    }
                }
                return Ok(0); // identical
            }
            /* drop through for full comparison */
        } else {
            /* More fastpaths possible */
            if self.ind < rhs.ind {
                return Ok(-1);
            }
            if self.ind > rhs.ind {
                return Ok(1);
            }
        }
        /* carry out a subtract to make the comparison */
        let mut newrhs = rhs.clone(); // safe copy
        newrhs.ind = -newrhs.ind; // prepare to subtract
        Ok(self.add_mc(&newrhs, set)?.ind as i32) // add, and return sign of result
    }

    /// `divide(BigDecimal rhs)`: the `java.math` "divide with the scale of
    /// this, `ROUND_HALF_UP`" semantics.
    ///
    /// # Errors
    ///
    /// As [`divide_mc`](Self::divide_mc).
    pub fn divide(&self, rhs: &BigDecimal) -> Result<Self> {
        self.dodivide('D', rhs, &PLAIN_MC, -1)
    }

    /// `divide(BigDecimal rhs, int round)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a bad rounding mode, and those of
    /// [`divide_mc`](Self::divide_mc).
    pub fn divide_round(&self, rhs: &BigDecimal, round: i32) -> Result<Self> {
        let set = MathContext::with_all(0, MathContext::PLAIN, false, round)?; // [checks round, too]
        self.dodivide('D', rhs, &set, -1) // take scale from LHS
    }

    /// `divide(BigDecimal rhs, int scale, int round)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException("Negative scale: <scale>")`, and those of
    /// [`divide_round`](Self::divide_round).
    pub fn divide_scale(&self, rhs: &BigDecimal, scale: i32, round: i32) -> Result<Self> {
        if scale < 0 {
            return Err(arith(format!("Negative scale: {scale}")));
        }
        let set = MathContext::with_all(0, MathContext::PLAIN, false, round)?; // [checks round]
        self.dodivide('D', rhs, &set, scale)
    }

    /// `divide(BigDecimal rhs, MathContext set)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` (`Divide by 0`, `Rounding necessary`, lost
    /// digits, `Exponent Overflow: ...`).
    pub fn divide_mc(&self, rhs: &BigDecimal, set: &MathContext) -> Result<Self> {
        self.dodivide('D', rhs, set, -1)
    }

    /// `divideInteger(BigDecimal rhs)`.
    ///
    /// # Errors
    ///
    /// As [`divide_integer_mc`](Self::divide_integer_mc).
    pub fn divide_integer(&self, rhs: &BigDecimal) -> Result<Self> {
        // scale 0 to drop .000 when plain
        self.dodivide('I', rhs, &PLAIN_MC, 0)
    }

    /// `divideInteger(BigDecimal rhs, MathContext set)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` (`Divide by 0`, `Integer overflow`, ...).
    pub fn divide_integer_mc(&self, rhs: &BigDecimal, set: &MathContext) -> Result<Self> {
        // scale 0 to drop .000 when plain
        self.dodivide('I', rhs, set, 0)
    }

    /// `max(BigDecimal rhs)`.
    ///
    /// # Errors
    ///
    /// As [`max_mc`](Self::max_mc).
    pub fn max(&self, rhs: &BigDecimal) -> Result<Self> {
        self.max_mc(rhs, &PLAIN_MC)
    }

    /// `max(BigDecimal rhs, MathContext set)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` for lost digits or an exponent overflow.
    pub fn max_mc(&self, rhs: &BigDecimal, set: &MathContext) -> Result<Self> {
        if self.compare_to_mc(rhs, set)? >= 0 {
            self.plus_mc(set)
        } else {
            rhs.plus_mc(set)
        }
    }

    /// `min(BigDecimal rhs)`.
    ///
    /// # Errors
    ///
    /// As [`min_mc`](Self::min_mc).
    pub fn min(&self, rhs: &BigDecimal) -> Result<Self> {
        self.min_mc(rhs, &PLAIN_MC)
    }

    /// `min(BigDecimal rhs, MathContext set)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` for lost digits or an exponent overflow.
    pub fn min_mc(&self, rhs: &BigDecimal, set: &MathContext) -> Result<Self> {
        if self.compare_to_mc(rhs, set)? <= 0 {
            self.plus_mc(set)
        } else {
            rhs.plus_mc(set)
        }
    }

    /// `multiply(BigDecimal rhs)`.
    ///
    /// # Errors
    ///
    /// As [`multiply_mc`](Self::multiply_mc).
    pub fn multiply(&self, rhs: &BigDecimal) -> Result<Self> {
        self.multiply_mc(rhs, &PLAIN_MC)
    }

    /// `multiply(BigDecimal rhs, MathContext set)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` for lost digits or an exponent overflow.
    pub fn multiply_mc(&self, rhs: &BigDecimal, set: &MathContext) -> Result<Self> {
        if set.lost_digits {
            self.checkdigits(Some(rhs), set.digits)?;
        }
        let mut lhs = self.clone(); // name for clarity and proxy
        let mut rhs = rhs.clone();

        /* Prepare numbers (truncate, unless unlimited precision) */
        let mut padding = 0; // trailing 0's to add
        let reqdig = set.digits; // local copy
        if reqdig > 0 {
            if lhs.mant.len() as i32 > reqdig {
                lhs.round_mc(set)?;
            }
            if rhs.mant.len() as i32 > reqdig {
                rhs.round_mc(set)?;
            }
            // [we could reuse the new LHS for result in this case]
        } else {
            /* unlimited */
            // fixed point arithmetic will want every trailing 0; we add these
            // after the calculation rather than before, for speed.
            if lhs.exp > 0 {
                padding += lhs.exp;
            }
            if rhs.exp > 0 {
                padding += rhs.exp;
            }
        }

        // For best speed, as in DMSRCN, we use the shorter number as the
        // multiplier and the longer as the multiplicand.
        // 1999.12.22: We used to special case when the result would fit in
        // a long, but with Java 1.3 this gave no advantage.
        let (multer, multand) = if lhs.mant.len() < rhs.mant.len() {
            (&lhs.mant, &rhs.mant)
        } else {
            (&rhs.mant, &lhs.mant)
        };

        /* Calculate how long result byte array will be */
        let mut multandlen = (multer.len() + multand.len()) as i32 - 1; // effective length
        // optimization when first digit of each is small
        let acclen = if (multer[0] as i32 * multand[0] as i32) > 9 { multandlen + 1 } else { multandlen };

        /* Now the main long multiplication loop */
        let mut res = Self::empty(); // where we'll build result
        let mut acc = vec![0i8; acclen as usize]; // accumulator, all zeros
        // 1998.07.01: calculate from left to right so that accumulator goes
        // to likely final length on first addition; this avoids a one-digit
        // extension (and object allocation) each time around the loop.
        // Initial number therefore has virtual zeros added to right.
        for &mult in multer.iter() {
            if mult != 0 {
                // [optimization]
                // accumulate [accumulator is reusable array]
                acc = byteaddsub(&acc, acc.len() as i32, multand, multandlen, mult as i32);
            }
            // divide multiplicand by 10 for next digit to right
            multandlen -= 1; // 'virtual length'
        }

        res.ind = lhs.ind * rhs.ind; // final sign
        res.exp = (lhs.exp + rhs.exp) - padding; // final exponent
        // [overflow is checked by finish]

        /* add trailing zeros to the result, if necessary */
        if padding == 0 {
            res.mant = acc;
        } else {
            let len = acc.len() + padding as usize;
            res.mant = extend(&acc, len); // add trailing 0s
        }
        res.finish(set, false)
    }

    /// `negate()`.
    ///
    /// # Errors
    ///
    /// As [`negate_mc`](Self::negate_mc).
    pub fn negate(&self) -> Result<Self> {
        self.negate_mc(&PLAIN_MC)
    }

    /// `negate(MathContext set)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` for lost digits or an exponent overflow.
    pub fn negate_mc(&self, set: &MathContext) -> Result<Self> {
        // Originally called minus(), changed to matched Java precedents
        // This simply clones, flips the sign, and possibly rounds
        if set.lost_digits {
            self.checkdigits(None, set.digits)?;
        }
        let mut res = self.clone(); // safe copy
        res.ind = -res.ind;
        res.finish(set, false)
    }

    /// `plus()`.
    ///
    /// # Errors
    ///
    /// As [`plus_mc`](Self::plus_mc).
    pub fn plus(&self) -> Result<Self> {
        self.plus_mc(&PLAIN_MC)
    }

    /// `plus(MathContext set)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` for lost digits or an exponent overflow.
    pub fn plus_mc(&self, set: &MathContext) -> Result<Self> {
        // This clones and forces the result to the new settings
        // May return same object
        if set.lost_digits {
            self.checkdigits(None, set.digits)?;
        }
        // Optimization: returns same object for some common cases
        if set.form == MathContext::PLAIN && self.form as i32 == MathContext::PLAIN {
            if self.mant.len() as i32 <= set.digits {
                return Ok(self.clone());
            }
            if set.digits == 0 {
                return Ok(self.clone());
            }
        }
        self.clone().finish(set, false)
    }

    /// `pow(BigDecimal rhs)`.
    ///
    /// # Errors
    ///
    /// As [`pow_mc`](Self::pow_mc).
    pub fn pow(&self, rhs: &BigDecimal) -> Result<Self> {
        self.pow_mc(rhs, &PLAIN_MC)
    }

    // The name for this method is inherited from the precedent set by the
    // BigInteger and Math classes; we retain this name for consistency.
    /// `pow(BigDecimal rhs, MathContext set)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` (`Negative power: ...`, `Too many digits:
    /// ...`, `Conversion overflow: ...`, `Decimal part non-zero: ...`, and
    /// those of the multiplications and division).
    pub fn pow_mc(&self, rhs: &BigDecimal, set: &MathContext) -> Result<Self> {
        if set.lost_digits {
            self.checkdigits(Some(rhs), set.digits)?;
        }
        let mut n = rhs.intcheck(MIN_ARG, MAX_ARG)?; // check RHS by the rules
        let mut lhs = self.clone(); // clarified name

        let reqdig = set.digits; // local copy (heavily used)
        let workdigits;
        if reqdig == 0 {
            if rhs.ind == ISNEG {
                return Err(arith(format!("Negative power: {rhs}")));
            }
            workdigits = 0;
        } else {
            /* non-0 digits */
            if (rhs.mant.len() as i32 + rhs.exp) > reqdig {
                return Err(arith(format!("Too many digits: {rhs}")));
            }

            /* Round the lhs to DIGITS if need be */
            if lhs.mant.len() as i32 > reqdig {
                lhs.round_mc(set)?;
            }

            /* L for precision calculation [see ANSI X3.274-1996] */
            let l = rhs.mant.len() as i32 + rhs.exp; // length without decimal zeros/exp
            workdigits = (reqdig + l) + 1; // calculate the working DIGITS
        }

        /* Create a copy of set for working settings */
        // Note: no need to check for lostDigits again.
        // 1999.07.17 Note: this construction must follow RHS check
        let workset = MathContext { digits: workdigits, form: set.form, lost_digits: false, rounding_mode: set.rounding_mode };

        let mut res = ONE.clone(); // accumulator
        if n == 0 {
            return Ok(res); // x**0 == 1
        }
        if n < 0 {
            n = -n; // [rhs.ind records the sign]
        }
        let mut seenbit = false; // set once we've seen a 1-bit
        let mut i = 1;
        loop {
            // for each bit [top bit ignored]
            n = n.wrapping_add(n); // shift left 1 bit
            if n < 0 {
                // top bit is set
                seenbit = true; // OK, we're off
                res = res.multiply_mc(&lhs, &workset)?; // acc=acc*x
            }
            if i == 31 {
                break; // that was the last bit
            }
            i += 1;
            if !seenbit {
                continue; // we don't have to square 1
            }
            res = res.multiply_mc(&res.clone(), &workset)?; // acc=acc*acc [square]
        }
        // 32 bits
        if rhs.ind < 0 {
            // was a **-n [hence digits>0]
            res = ONE.divide_mc(&res, &workset)?; // .. so acc=1/acc
        }
        res.finish(set, true) // round and strip [original digits]
    }

    /// `remainder(BigDecimal rhs)`.
    ///
    /// # Errors
    ///
    /// As [`remainder_mc`](Self::remainder_mc).
    pub fn remainder(&self, rhs: &BigDecimal) -> Result<Self> {
        self.dodivide('R', rhs, &PLAIN_MC, -1)
    }

    /// `remainder(BigDecimal rhs, MathContext set)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` (`Divide by 0`, `Integer overflow`, ...).
    pub fn remainder_mc(&self, rhs: &BigDecimal, set: &MathContext) -> Result<Self> {
        self.dodivide('R', rhs, set, -1)
    }

    /// `subtract(BigDecimal rhs)`.
    ///
    /// # Errors
    ///
    /// As [`subtract_mc`](Self::subtract_mc).
    pub fn subtract(&self, rhs: &BigDecimal) -> Result<Self> {
        self.subtract_mc(rhs, &PLAIN_MC)
    }

    /// `subtract(BigDecimal rhs, MathContext set)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` for lost digits or an exponent overflow.
    pub fn subtract_mc(&self, rhs: &BigDecimal, set: &MathContext) -> Result<Self> {
        if set.lost_digits {
            self.checkdigits(Some(rhs), set.digits)?;
        }
        // [add will recheck .. but would report -rhs]
        /* carry out the subtraction */
        // we could fastpath -0, but it is too rare.
        let mut newrhs = rhs.clone(); // safe copy
        newrhs.ind = -newrhs.ind; // prepare to subtract
        self.add_mc(&newrhs, set) // arithmetic
    }

    /* ---------------------------------------------------------------- */
    /* Other methods */
    /* ---------------------------------------------------------------- */

    /// `byteValueExact()`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException("Conversion overflow: ...")` or `Decimal part
    /// non-zero: ...`.
    pub fn byte_value_exact(&self) -> Result<i8> {
        let num = self.int_value_exact()?; // will check decimal part too
        if (num > 127) | (num < -128) {
            return Err(arith(format!("Conversion overflow: {self}")));
        }
        Ok(num as i8)
    }

    /// `doubleValue()`: `Double.valueOf(toString())`.
    pub fn double_value(&self) -> f64 {
        // We go via a String [as does BigDecimal in JDK 1.2]
        // Next line could possibly raise NumberFormatException
        // (the layout is always a valid decimal string)
        self.to_string().parse::<f64>().unwrap_or(0.0)
    }

    /// `equals(Object obj)`: equal value **and** equal string
    /// representation (`2.0` is not equal to `2.00`).
    pub fn java_equals(&self, rhs: &BigDecimal) -> bool {
        // We are equal iff toString of both are exactly the same
        if self.ind != rhs.ind {
            return false; // different signs never match
        }
        if (self.mant.len() == rhs.mant.len()) & (self.exp == rhs.exp) & (self.form == rhs.form) {
            // mantissas say all
            // here with equal-length byte arrays to compare
            for i in 0..self.mant.len() {
                if self.mant[i] != rhs.mant[i] {
                    return false;
                }
            }
        } else {
            // need proper layout
            let lca = self.layout(); // layout to character array
            let rca = rhs.layout();
            if lca.len() != rca.len() {
                return false; // mismatch
            }
            // here with equal-length character arrays to compare
            if lca != rca {
                return false;
            }
        }
        true // arrays have identical content
    }

    /// `floatValue()`: `Float.valueOf(toString())`.
    pub fn float_value(&self) -> f32 {
        // (the layout is always a valid decimal string)
        self.to_string().parse::<f32>().unwrap_or(0.0)
    }

    /// `format(int before, int after)`.
    ///
    /// # Errors
    ///
    /// As [`format_full`](Self::format_full).
    pub fn format(&self, before: i32, after: i32) -> Result<String> {
        self.format_full(before, after, -1, -1, MathContext::SCIENTIFIC, Self::ROUND_HALF_UP)
    }

    /// `format(int before, int after, int explaces, int exdigits, int
    /// exformint, int exround)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Bad argument <n> to format: <value>")`
    /// for a bad argument or a result that does not fit, and
    /// `ArithmeticException` (`Exponent Overflow: ...`, `Rounding
    /// necessary`).
    pub fn format_full(&self, before: i32, after: i32, explaces: i32, exdigits: i32, mut exformint: i32, mut exround: i32) -> Result<String> {
        /* Check arguments */
        if (before < -1) | (before == 0) {
            return Err(badarg("format", 1, &before.to_string()));
        }
        if after < -1 {
            return Err(badarg("format", 2, &after.to_string()));
        }
        if (explaces < -1) | (explaces == 0) {
            return Err(badarg("format", 3, &explaces.to_string()));
        }
        if exdigits < -1 {
            return Err(badarg("format", 4, &explaces.to_string()));
        }
        if exformint == MathContext::SCIENTIFIC || exformint == MathContext::ENGINEERING {
        } else if exformint == -1 {
            exformint = MathContext::SCIENTIFIC;
        } else {
            // note PLAIN isn't allowed
            return Err(badarg("format", 5, &exformint.to_string()));
        }
        // checking the rounding mode is done by trying to construct a
        // MathContext object with that mode; it will fail if bad
        if exround != Self::ROUND_HALF_UP {
            // common case is fast
            if exround == -1 {
                exround = Self::ROUND_HALF_UP;
            } else if MathContext::with_all(9, MathContext::SCIENTIFIC, false, exround).is_err() {
                return Err(badarg("format", 6, &exround.to_string()));
            }
        }

        let mut num = self.clone(); // make private copy

        /* Here:
           num       is BigDecimal to format
           before    is places before point [>0]
           after     is places after point  [>=0]
           explaces  is exponent places     [>0]
           exdigits  is exponent digits     [>=0]
           exformint is exponent form       [one of two]
           exround   is rounding mode       [one of eight]
           'before' through 'exdigits' are -1 if not specified
        */

        /* determine form */
        if exdigits == -1 {
            num.form = MathContext::PLAIN as i8; // force JIT
        } else if num.ind == ISZERO {
            num.form = MathContext::PLAIN as i8; // always
        } else {
            // determine whether triggers
            let mag = num.exp + num.mant.len() as i32;
            if mag > exdigits || mag < -5 {
                num.form = exformint as i8;
            } else {
                num.form = MathContext::PLAIN as i8;
            }
        }

        /* If 'after' was specified then we may need to adjust the
           mantissa.  This is a little tricky, as we must conform to the
           rules of exponential layout if necessary (e.g., we cannot end up
           with 10.0 if scientific). */
        if after >= 0 {
            'setafter: loop {
                // calculate the current after-length
                let thisafter;
                if num.form as i32 == MathContext::PLAIN {
                    thisafter = -num.exp; // has decimal part
                } else if num.form as i32 == MathContext::SCIENTIFIC {
                    thisafter = num.mant.len() as i32 - 1;
                } else {
                    // engineering
                    let mut lead = ((num.exp + num.mant.len() as i32) - 1) % 3; // exponent to use
                    if lead < 0 {
                        lead += 3; // negative exponent case
                    }
                    lead += 1; // number of leading digits
                    if lead >= num.mant.len() as i32 {
                        thisafter = 0;
                    } else {
                        thisafter = num.mant.len() as i32 - lead;
                    }
                }
                if thisafter == after {
                    break 'setafter; // we're in luck
                }
                if thisafter < after {
                    // need added trailing zeros
                    // [thisafter can be negative]
                    let newmant = extend(&num.mant, ((num.mant.len() as i32 + after) - thisafter) as usize);
                    num.mant = newmant;
                    num.exp -= after - thisafter; // adjust exponent
                    if num.exp < MIN_EXP {
                        return Err(arith(format!("Exponent Overflow: {}", num.exp)));
                    }
                    break 'setafter;
                }
                // We have too many digits after the decimal point; this could
                // cause a carry, which could change the mantissa...
                // Watch out for implied leading zeros in PLAIN case
                let chop = thisafter - after; // digits to lop [is >0]
                if chop > num.mant.len() as i32 {
                    // all digits go, no chance of carry
                    // carry on with zero
                    num.mant = vec![0];
                    num.ind = ISZERO;
                    num.exp = 0;
                    continue 'setafter; // recheck: we may need trailing zeros
                }
                // we have a digit to inspect from existing mantissa
                // round the number as required
                let need = num.mant.len() as i32 - chop; // digits to end up with [may be 0]
                let oldexp = num.exp; // save old exponent
                num.round(need, exround)?;
                // if the exponent grew by more than the digits we chopped, then
                // we must have had a carry, so will need to recheck the layout
                if (num.exp - oldexp) == chop {
                    break 'setafter; // number did not have carry
                }
                // mantissa got extended .. so go around and check again
            }
        }

        let mut a = num.layout(); // lay out, with exponent if required, etc.

        /* Here we have laid-out number in 'a' */
        // now apply 'before' and 'explaces' as needed
        if before > 0 {
            // look for '.' or 'E'
            let mut p = 0usize;
            while p < a.len() {
                if a[p] == b'.' {
                    break;
                }
                if a[p] == b'E' {
                    break;
                }
                p += 1;
            }
            // p is now offset of '.', 'E', or character after end of array
            // that is, the current length of before part
            if p as i32 > before {
                return Err(badarg("format", 1, &before.to_string())); // won't fit
            }
            if (p as i32) < before {
                // need leading blanks
                let mut newa = vec![b' '; before as usize - p];
                newa.extend_from_slice(&a);
                a = newa;
            }
            // [if p=before then it's just the right length]
        }

        if explaces > 0 {
            // look for 'E' [cannot be at offset 0]
            let mut p = a.len() - 1;
            while p > 0 {
                if a[p] == b'E' {
                    break;
                }
                p -= 1;
            }
            // p is now offset of 'E', or 0
            if p == 0 {
                // no E part; add trailing blanks
                a.extend(std::iter::repeat_n(b' ', (explaces + 2) as usize));
            } else {
                // found E
                // may need to insert zeros
                let places = (a.len() - p) as i32 - 2; // number so far
                if places > explaces {
                    return Err(badarg("format", 3, &explaces.to_string())); // won't fit
                }
                if places < explaces {
                    // need to insert zeros
                    let mut newa = a[..p + 2].to_vec(); // through E and sign
                    newa.extend(std::iter::repeat_n(b'0', (explaces - places) as usize));
                    newa.extend_from_slice(&a[p + 2..p + 2 + places as usize]); // and the digits
                    a = newa;
                }
                // [if places=explaces then it's just the right length]
            }
        }
        Ok(String::from_utf8_lossy(&a).into_owned())
    }

    /// `hashCode()`: `toString().hashCode()`.
    pub fn hash_code(&self) -> i32 {
        // Maybe calculate ourselves, later.  If so, note that there can be
        // more than one internal representation for a given toString() result.
        let mut h: i32 = 0;
        for c in self.layout() {
            h = h.wrapping_mul(31).wrapping_add(c as i32);
        }
        h
    }

    /// `intValue()`: `toBigInteger().intValue()`.
    pub fn int_value(&self) -> i32 {
        self.to_big_integer().int_value()
    }

    /// `byteValue()` (`Number.byteValue()`: `(byte) intValue()`).
    pub fn byte_value(&self) -> i8 {
        self.int_value() as i8
    }

    /// `shortValue()` (`Number.shortValue()`: `(short) intValue()`).
    pub fn short_value(&self) -> i16 {
        self.int_value() as i16
    }

    /// `intValueExact()`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` (`Decimal part non-zero: ...`, `Conversion
    /// overflow: ...`).
    pub fn int_value_exact(&self) -> Result<i32> {
        // This does not use longValueExact() as the latter can be much
        // slower.
        // intcheck (from pow) relies on this to check decimal part
        if self.ind == ISZERO {
            return Ok(0); // easy, and quite common
        }
        /* test and drop any trailing decimal part */
        let mut lodigit = self.mant.len() as i32 - 1;
        let useexp;
        if self.exp < 0 {
            lodigit += self.exp; // reduces by -(-exp)
            /* all decimal places must be 0 */
            if !allzero(&self.mant, lodigit + 1) {
                return Err(arith(format!("Decimal part non-zero: {self}")));
            }
            if lodigit < 0 {
                return Ok(0); // -1<this<1
            }
            useexp = 0;
        } else {
            /* >=0 */
            if (self.exp + lodigit) > 9 {
                // early exit
                return Err(arith(format!("Conversion overflow: {self}")));
            }
            useexp = self.exp;
        }
        /* convert the mantissa to binary, inline for speed */
        let mut result: i32 = 0;
        for i in 0..=(lodigit + useexp) {
            result = result.wrapping_mul(10);
            if i <= lodigit {
                result = result.wrapping_add(self.mant[i as usize] as i32);
            }
        }

        /* Now, if the risky length, check for overflow */
        if (lodigit + useexp) == 9 {
            // note we cannot just test for -ve result, as overflow can move a
            // zero into the top bit [consider 5555555555]
            let topdig = result / 1000000000; // get top digit, preserving sign
            if topdig != self.mant[0] as i32 {
                // digit must match and be positive
                // except in the special case ...
                if result == i32::MIN && self.ind == ISNEG && self.mant[0] == 2 {
                    // looks like the special
                    return Ok(result); // really was negative
                }
                return Err(arith(format!("Conversion overflow: {self}")));
            }
        }

        /* Looks good */
        if self.ind == ISPOS {
            return Ok(result);
        }
        Ok(result.wrapping_neg())
    }

    /// `longValue()`: `toBigInteger().longValue()`.
    pub fn long_value(&self) -> i64 {
        self.to_big_integer().long_value()
    }

    /// `longValueExact()`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` (`Decimal part non-zero: ...`, `Conversion
    /// overflow: ...`).
    pub fn long_value_exact(&self) -> Result<i64> {
        /* Identical to intValueExact except for result=long, and exp>=20 test */
        if self.ind == 0 {
            return Ok(0); // easy, and quite common
        }
        let mut lodigit = self.mant.len() as i32 - 1; // last included digit
        let useexp;
        if self.exp < 0 {
            lodigit += self.exp; // -(-exp)
            /* all decimal places must be 0 */
            let cstart = if lodigit < 0 { 0 } else { lodigit + 1 };
            if !allzero(&self.mant, cstart) {
                return Err(arith(format!("Decimal part non-zero: {self}")));
            }
            if lodigit < 0 {
                return Ok(0); // -1<this<1
            }
            useexp = 0;
        } else {
            /* >=0 */
            if (self.exp + self.mant.len() as i32) > 18 {
                // early exit
                return Err(arith(format!("Conversion overflow: {self}")));
            }
            useexp = self.exp;
        }

        /* convert the mantissa to binary, inline for speed */
        // note that we could safely use the 'test for wrap to negative'
        // algorithm here, but instead we parallel the intValueExact
        // algorithm for ease of checking and maintenance.
        let mut result: i64 = 0;
        for i in 0..=(lodigit + useexp) {
            result = result.wrapping_mul(10);
            if i <= lodigit {
                result = result.wrapping_add(self.mant[i as usize] as i64);
            }
        }

        /* Now, if the risky length, check for overflow */
        if (lodigit + useexp) == 18 {
            let topdig = result / 1000000000000000000; // get top digit, preserving sign
            if topdig != self.mant[0] as i64 {
                // digit must match and be positive
                // except in the special case ...
                if result == i64::MIN && self.ind == ISNEG && self.mant[0] == 9 {
                    return Ok(result); // really was negative
                }
                return Err(arith(format!("Conversion overflow: {self}")));
            }
        }

        /* Looks good */
        if self.ind == ISPOS {
            return Ok(result);
        }
        Ok(result.wrapping_neg())
    }

    /// `movePointLeft(int n)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException("Exponent Overflow: ...")`.
    pub fn move_point_left(&self, n: i32) -> Result<Self> {
        // very little point in optimizing for shift of 0
        let mut res = self.clone();
        res.exp = res.exp.wrapping_sub(n);
        res.finish(&PLAIN_MC, false) // finish sets form and checks exponent
    }

    /// `movePointRight(int n)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException("Exponent Overflow: ...")`.
    pub fn move_point_right(&self, n: i32) -> Result<Self> {
        let mut res = self.clone();
        res.exp = res.exp.wrapping_add(n);
        res.finish(&PLAIN_MC, false)
    }

    /// `scale()`: the number of digits after the decimal point.
    pub fn scale(&self) -> i32 {
        if self.exp >= 0 {
            return 0; // scale can never be negative
        }
        -self.exp
    }

    /// `setScale(int scale)`: `ROUND_UNNECESSARY`.
    ///
    /// # Errors
    ///
    /// As [`set_scale_round`](Self::set_scale_round).
    pub fn set_scale(&self, scale: i32) -> Result<Self> {
        self.set_scale_round(scale, Self::ROUND_UNNECESSARY)
    }

    /// `setScale(int scale, int round)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` (`Negative scale: <n>`, `Rounding necessary`,
    /// `Exponent Overflow: ...`), `IllegalArgumentException("Bad round
    /// value: <n>")`.
    pub fn set_scale_round(&self, scale: i32, round: i32) -> Result<Self> {
        /* if the number is already at the scale required then we
           return it unless it has an exponent [form] */
        let ourscale = self.scale();
        if ourscale == scale && self.form as i32 == MathContext::PLAIN {
            // already in plain form
            return Ok(self.clone());
        }
        let mut res = self.clone(); // need copy
        if ourscale <= scale {
            // simply zero-padding/changing form
            // if ourscale is 0 we may have lots of 0s to add
            let padding = if ourscale == 0 { res.exp + scale } else { scale - ourscale };
            res.mant = extend(&res.mant, (res.mant.len() as i32 + padding) as usize);
            res.exp = -scale; // as requested
        } else {
            /* ourscale>scale: shortening, probably */
            if scale < 0 {
                return Err(arith(format!("Negative scale: {scale}")));
            }
            // [round() will raise exception if invalid round]
            let newlen = res.mant.len() as i32 - (ourscale - scale); // [<=0 is OK]
            res.round(newlen, round)?; // round to required length
            // This could have shifted left if round (say) 0.9->1[.0]
            // Repair if so by adding a zero and reducing exponent
            if res.exp != -scale {
                res.mant = extend(&res.mant, res.mant.len() + 1);
                res.exp -= 1;
            }
        }
        res.form = MathContext::PLAIN as i8; // by definition
        Ok(res)
    }

    /// `shortValueExact()`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException("Conversion overflow: ...")` or `Decimal part
    /// non-zero: ...`.
    pub fn short_value_exact(&self) -> Result<i16> {
        let num = self.int_value_exact()?; // will check decimal part too
        if (num > 32767) | (num < -32768) {
            return Err(arith(format!("Conversion overflow: {self}")));
        }
        Ok(num as i16)
    }

    /// `signum()`: -1, 0 or 1.
    pub fn signum(&self) -> i32 {
        self.ind as i32 // [note this assumes values for ind.]
    }

    /// `toBigDecimal()`: the `java.math.BigDecimal` of the same value and
    /// scale.
    pub fn to_big_decimal(&self) -> JBigDecimal {
        JBigDecimal::new_unscaled(self.unscaled_value(), self.scale())
    }

    /// `toBigInteger()`: the integer part (fraction discarded).
    pub fn to_big_integer(&self) -> BigInteger {
        let res: BigDecimal;
        if (self.exp >= 0) & (self.form as i32 == MathContext::PLAIN) {
            res = self.clone(); // can layout simply
        } else if self.exp >= 0 {
            let mut r = self.clone(); // safe copy
            r.form = MathContext::PLAIN as i8; // .. and request PLAIN
            res = r;
        } else {
            // exp<0; scale to be truncated
            // we could use divideInteger, but we may as well be quicker
            if -self.exp >= self.mant.len() as i32 {
                res = ZERO.clone(); // all blows away
            } else {
                let mut r = self.clone(); // safe copy
                let newlen = (r.mant.len() as i32 + r.exp) as usize;
                r.mant.truncate(newlen);
                r.form = MathContext::PLAIN as i8;
                r.exp = 0;
                res = r;
            }
        }
        // The layout of a plain integer is a valid BigInteger string.
        BigInteger::from_string(&units(&String::from_utf8_lossy(&res.layout())))
            .unwrap_or_else(|e| panic!("INVARIANT: a plain integer layout is a valid BigInteger: {e}"))
    }

    /// `toBigIntegerExact()`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException("Decimal part non-zero: ...")`.
    pub fn to_big_integer_exact(&self) -> Result<BigInteger> {
        /* test any trailing decimal part */
        if self.exp < 0 {
            // possible decimal part
            /* all decimal places must be 0; note exp<0 */
            if !allzero(&self.mant, self.mant.len() as i32 + self.exp) {
                return Err(arith(format!("Decimal part non-zero: {self}")));
            }
        }
        Ok(self.to_big_integer())
    }

    /// `toCharArray()`: the UTF-16 units of [`to_string`](ToString::to_string).
    pub fn to_char_array(&self) -> Vec<u16> {
        self.layout().into_iter().map(u16::from).collect()
    }

    /// `unscaledValue()`: the digits as an integer, ignoring the decimal
    /// point.
    pub fn unscaled_value(&self) -> BigInteger {
        if self.exp >= 0 {
            self.to_big_integer()
        } else {
            let mut res = self.clone(); // safe copy
            res.exp = 0; // drop scale
            res.to_big_integer()
        }
    }

    /// `valueOf(double dub)`: `new BigDecimal(Double.toString(dub))`.
    ///
    /// # Errors
    ///
    /// `NumberFormatException("Not a number: NaN")` (or `Infinity`) for a
    /// non-finite value.
    pub fn value_of_double(dub: f64) -> Result<Self> {
        // Reminder: a zero double returns '0.0', so we cannot fastpath to
        // use the constant ZERO.  This might be important enough to justify
        // a factory approach, a cache, or a few private constants, later.
        Self::from_string(&units(&rucene::util::java_fmt::double_to_string(dub)))
    }

    /// `valueOf(long lint)`.
    pub fn value_of(lint: i64) -> Self {
        // the scale 0 cannot raise
        Self::value_of_scaled(lint, 0).unwrap_or_else(|_| Self::from_long(lint))
    }

    /// `valueOf(long lint, int scale)`.
    ///
    /// # Errors
    ///
    /// `NumberFormatException("Negative scale: <scale>")`.
    pub fn value_of_scaled(lint: i64, scale: i32) -> Result<Self> {
        let mut res = if lint == 0 {
            ZERO.clone()
        } else if lint == 1 {
            ONE.clone()
        } else if lint == 10 {
            TEN.clone()
        } else {
            Self::from_long(lint)
        };
        if scale == 0 {
            return Ok(res);
        }
        if scale < 0 {
            return Err(LuceneError::number_format(format!("Negative scale: {scale}")));
        }
        res.exp = -scale; // exponent is -scale
        Ok(res)
    }

    /* ---------------------------------------------------------------- */
    /* Private methods */
    /* ---------------------------------------------------------------- */

    /// `layout()`: the characters of the number in its form (ASCII bytes).
    fn layout(&self) -> Vec<u8> {
        let cmant: Vec<u8> = self.mant.iter().map(|&d| (d as i32 + b'0' as i32) as u8).collect();

        if self.form as i32 != MathContext::PLAIN {
            /* exponential notation needed */
            let mut sb: Vec<u8> = Vec::with_capacity(cmant.len() + 15); // -x.xxxE+999999999
            if self.ind == ISNEG {
                sb.push(b'-');
            }
            let mut euse = (self.exp + cmant.len() as i32) - 1; // exponent to use
            /* setup sig=significant digits and copy to result */
            if self.form as i32 == MathContext::SCIENTIFIC {
                // [default]
                sb.push(cmant[0]); // significant character
                if cmant.len() > 1 {
                    // have decimal part
                    sb.push(b'.');
                    sb.extend_from_slice(&cmant[1..]);
                }
            } else {
                let mut sig = euse % 3; // common
                if sig < 0 {
                    sig += 3; // negative exponent
                }
                euse -= sig;
                sig += 1;
                let sig = sig as usize;
                if sig >= cmant.len() {
                    // zero padding may be needed
                    sb.extend_from_slice(&cmant);
                    sb.extend(std::iter::repeat_n(b'0', sig - cmant.len()));
                } else {
                    // decimal point needed
                    sb.extend_from_slice(&cmant[..sig]);
                    sb.push(b'.');
                    sb.extend_from_slice(&cmant[sig..]);
                }
            }
            if euse != 0 {
                let csign;
                if euse < 0 {
                    csign = b'-';
                    euse = -euse;
                } else {
                    csign = b'+';
                }
                sb.push(b'E');
                sb.push(csign);
                sb.extend_from_slice(euse.to_string().as_bytes());
            }
            return sb;
        }

        /* Here for non-exponential (plain) notation */
        if self.exp == 0 {
            /* easy */
            if self.ind >= 0 {
                return cmant; // non-negative integer
            }
            let mut rec = Vec::with_capacity(cmant.len() + 1);
            rec.push(b'-');
            rec.extend_from_slice(&cmant);
            return rec;
        }

        /* Need a '.' and/or some zeros */
        let needsign = if self.ind == ISNEG { 1 } else { 0 }; // space for sign?  0 or 1

        /* MAG is the position of the point in the mantissa (index of the
           character it follows) */
        let mag = self.exp + cmant.len() as i32;

        let mut rec: Vec<u8> = Vec::new();
        if needsign != 0 {
            rec.push(b'-');
        }
        if mag < 1 {
            /* 0.00xxxx form */
            rec.push(b'0');
            rec.push(b'.');
            rec.extend(std::iter::repeat_n(b'0', (-mag) as usize));
            rec.extend_from_slice(&cmant);
            return rec;
        }

        if mag > cmant.len() as i32 {
            /* xxxx0000 form */
            rec.extend_from_slice(&cmant);
            rec.extend(std::iter::repeat_n(b'0', (mag - cmant.len() as i32) as usize));
            return rec;
        }

        /* decimal point is in the middle of the mantissa */
        let mag = mag as usize;
        rec.extend_from_slice(&cmant[..mag]);
        rec.push(b'.');
        rec.extend_from_slice(&cmant[mag..]);
        rec
    }

    /// `intcheck(int min, int max)`: this value as an `int` within
    /// `min..=max`.
    fn intcheck(&self, min: i32, max: i32) -> Result<i32> {
        let i = self.int_value_exact()?; // [checks for non-0 decimal part]
        // Use same message as though intValueExact failed due to size
        if (i < min) | (i > max) {
            return Err(arith(format!("Conversion overflow: {i}")));
        }
        Ok(i)
    }

    /// `dodivide(char code, BigDecimal rhs, MathContext set, int scale)`:
    /// division (`'D'`), integer division (`'I'`) or remainder (`'R'`).
    fn dodivide(&self, code: char, rhs: &BigDecimal, set: &MathContext, mut scale: i32) -> Result<Self> {
        if set.lost_digits {
            self.checkdigits(Some(rhs), set.digits)?;
        }
        let mut lhs = self.clone(); // clarified name
        let mut rhs = rhs.clone();

        // [note we must have checked lostDigits before the following checks]
        if rhs.ind == 0 {
            return Err(arith("Divide by 0".to_string())); // includes 0/0
        }
        if lhs.ind == 0 {
            // 0/x => 0 [possibly with .0s]
            if set.form != MathContext::PLAIN {
                return Ok(ZERO.clone());
            }
            if scale == -1 {
                return Ok(lhs);
            }
            return lhs.set_scale(scale);
        }

        /* Prepare numbers according to BigDecimal rules */
        let mut reqdig = set.digits; // local copy (heavily used)
        if reqdig > 0 {
            if lhs.mant.len() as i32 > reqdig {
                lhs.round_mc(set)?;
            }
            if rhs.mant.len() as i32 > reqdig {
                rhs.round_mc(set)?;
            }
        } else {
            /* scaled divide */
            if scale == -1 {
                scale = lhs.scale();
            }
            // set reqdig to be at least large enough for the computation
            reqdig = lhs.mant.len() as i32; // base length
            // next line handles both positive lhs.exp and also scale mismatch
            if scale != -lhs.exp {
                reqdig = (reqdig + scale) + lhs.exp;
            }
            reqdig = (reqdig - (rhs.mant.len() as i32 - 1)) - rhs.exp; // reduce by RHS effect
            if reqdig < lhs.mant.len() as i32 {
                reqdig = lhs.mant.len() as i32; // clamp
            }
            if reqdig < rhs.mant.len() as i32 {
                reqdig = rhs.mant.len() as i32; // ..
            }
        }

        /* precalculate exponent */
        let newexp = ((lhs.exp - rhs.exp) + lhs.mant.len() as i32) - rhs.mant.len() as i32;
        /* If new exponent -ve, then some quick exits are possible */
        if newexp < 0 && code != 'D' {
            if code == 'I' {
                return Ok(ZERO.clone()); // easy - no integer part
            }
            /* Must be 'R'; remainder is [finished clone of] input value */
            return lhs.finish(set, false);
        }

        /* We need slow division */
        let mut res = Self::empty(); // where we'll build result
        res.ind = lhs.ind * rhs.ind; // final sign (for D/I)
        res.exp = newexp; // initial exponent (for D/I)
        res.mant = vec![0; (reqdig + 1) as usize]; // where build the result

        /* Now [virtually pad the mantissae with trailing zeros */
        // Also copy the LHS, which will be our working array
        let newlen = (reqdig + reqdig) + 1;
        let mut var1 = extend(&lhs.mant, newlen as usize); // always makes longer, so new safe array
        let mut var1len = newlen; // [remaining digits are 0]

        let var2 = rhs.mant.clone();
        let mut var2len = newlen;

        /* Calculate first two digits of rhs (var2), +1 for later estimations */
        let mut b2b = (var2[0] as i32 * 10) + 1;
        if var2.len() > 1 {
            b2b += var2[1] as i32;
        }

        /* start the long-division loops */
        let mut have: i32 = 0;
        'outer: loop {
            let mut thisdigit: i32 = 0;
            /* find the next digit */
            'inner: loop {
                if var1len < var2len {
                    break 'inner; // V1 too low
                }
                let ba: i32;
                if var1len == var2len {
                    // compare needed
                    // comparison: true when var1 >= var2 for the full length
                    let mut greater = false;
                    for i in 0..var1len as usize {
                        // var1len is always <= var1.length
                        let v2: i8 = if i < var2.len() { var2[i] } else { 0 };
                        if var1[i] < v2 {
                            break 'inner; // V1 too low
                        }
                        if var1[i] > v2 {
                            greater = true;
                            break; // OK to subtract
                        }
                    }
                    if !greater {
                        /* reach here if lhs and rhs are identical; subtraction will
                           increase digit by one, and the residue will be 0 so we
                           are done; leave the loop with residue set to 0 (in case
                           code is 'R' or ROUND_UNNECESSARY or a DIVIDE) */
                        thisdigit += 1;
                        res.mant[have as usize] = thisdigit as i8;
                        have += 1;
                        var1[0] = 0; // residue to 0 [this is all we'll test]
                        // var1len=1 -- [optimized out]
                        break 'outer;
                    }
                    /* prepare for subtraction.  Estimate BA (lengths the same) */
                    ba = var1[0] as i32; // use only first digit
                } else {
                    // lengths the same
                    /* lhs longer than rhs */
                    /* use first two digits for estimate */
                    let mut b = var1[0] as i32 * 10;
                    if var1len > 1 {
                        b += var1[1] as i32;
                    }
                    ba = b;
                }
                /* subtraction needed; V1>=V2 */
                let mut mult = (ba * 10) / b2b;
                if mult == 0 {
                    mult = 1;
                }
                thisdigit += mult;
                // subtract; var1 reusable
                var1 = byteaddsub(&var1, var1len, &var2, var2len, -mult);
                if var1[0] != 0 {
                    continue 'inner; // maybe another subtract needed
                }
                /* V1 now probably has leading zeros, remove leading 0's and try
                   again. (It could be longer than V2) */
                let mut start = 0usize;
                while (start as i32) <= var1len - 2 {
                    if var1[start] != 0 {
                        break;
                    }
                    var1len -= 1;
                    start += 1;
                }
                if start == 0 {
                    continue 'inner;
                }
                // shift left
                var1.copy_within(start..start + var1len as usize, 0);
            }

            /* We have the next digit */
            if (have != 0) | (thisdigit != 0) {
                // put the digit we got
                res.mant[have as usize] = thisdigit as i8;
                have += 1;
                if have == (reqdig + 1) {
                    break 'outer; // we have all we need
                }
                if var1[0] == 0 {
                    break 'outer; // residue now 0
                }
            }
            /* can leave now if a scaled divide and exponent is small enough */
            if scale >= 0 && -res.exp > scale {
                break 'outer;
            }
            /* can leave now if not Divide and no integer part left */
            if code != 'D' && res.exp <= 0 {
                break 'outer;
            }
            res.exp -= 1; // reduce the exponent
            /* to get here, V1 is less than V2, so divide V2 by 10 and go for
               the next digit */
            var2len -= 1;
        }

        /* here when we have finished dividing, for some reason */
        // have is the number of digits we collected in res.mant
        if have == 0 {
            have = 1; // res.mant[0] is 0; we always want a digit
        }

        if (code == 'I') | (code == 'R') {
            /* check for integer overflow needed */
            if (have + res.exp) > reqdig {
                return Err(arith("Integer overflow".to_string()));
            }

            if code == 'R' {
                /* We were doing Remainder -- return the residue */
                if res.mant[0] == 0 {
                    // no integer part was found
                    return lhs.finish(set, false); // .. so return lhs, canonical
                }
                if var1[0] == 0 {
                    return Ok(ZERO.clone()); // simple 0 residue
                }
                res.ind = lhs.ind; // sign is always as LHS
                /* Calculate the exponent by subtracting the number of padding zeros
                   we added and adding the original exponent */
                let padding = ((reqdig + reqdig) + 1) - lhs.mant.len() as i32;
                res.exp = (res.exp - padding) + lhs.exp;

                /* strip insignificant padding zeros from residue, and create/copy
                   the resulting mantissa if need be */
                let mut d = var1len;
                let mut i = d - 1;
                while i >= 1 {
                    if !((res.exp < lhs.exp) & (res.exp < rhs.exp)) {
                        break;
                    }
                    if var1[i as usize] != 0 {
                        break;
                    }
                    d -= 1;
                    res.exp += 1;
                    i -= 1;
                }
                if (d as usize) < var1.len() {
                    // need to reduce
                    var1.truncate(d as usize);
                }
                res.mant = var1;
                return res.finish(set, false);
            }
        } else {
            /* 'D' -- no overflow check needed */
            // If there was a residue then bump the final digit (iff 0 or 5)
            // so that the residue is visible for ROUND_UP, ROUND_HALF_xxx and
            // ROUND_UNNECESSARY checks (etc.) later.
            // [if we finished early, the residue will be 0]
            if var1[0] != 0 {
                // residue not 0
                let lasthave = res.mant[(have - 1) as usize];
                if (lasthave % 5) == 0 {
                    res.mant[(have - 1) as usize] = lasthave + 1;
                }
            }
        }

        /* Here for Divide or Integer Divide */
        // handle scaled results first ['I' always scale 0, optional for 'D']
        if scale >= 0 {
            // say 'scale have res.exp len' scale have res.exp res.mant.length
            if have != res.mant.len() as i32 {
                // already padded with 0's, so just adjust exponent
                res.exp -= res.mant.len() as i32 - have;
            }
            // calculate number of digits we really want [may be 0]
            let actdig = res.mant.len() as i32 - (-res.exp - scale);
            res.round(actdig, set.rounding_mode)?; // round to desired length
            // This could have shifted left if round (say) 0.9->1[.0]
            // Repair if so by adding a zero and reducing exponent
            if res.exp != -scale {
                res.mant = extend(&res.mant, res.mant.len() + 1);
                res.exp -= 1;
            }
            return res.finish(set, true); // [strip if not PLAIN]
        }

        /* reach here only if a non-scaled */
        if have == res.mant.len() as i32 {
            // got digits+1 digits
            res.round_mc(set)?;
        } else {
            /* have<=reqdig */
            if res.mant[0] == 0 {
                return Ok(ZERO.clone()); // fastpath
            }
            // make the mantissa truly just 'have' long
            // [we could let finish do this, during strip, if we adjusted
            // the exponent; however, truncation avoids the strip loop]
            res.mant.truncate(have as usize);
        }
        res.finish(set, true)
    }

    /// `checkdigits(BigDecimal rhs, int dig)`: the lost-digits check.
    fn checkdigits(&self, rhs: Option<&BigDecimal>, dig: i32) -> Result<()> {
        if dig == 0 {
            return Ok(()); // don't check if digits=0
        }
        // first check lhs...
        if self.mant.len() as i32 > dig && !allzero(&self.mant, dig) {
            return Err(arith(format!("Too many digits: {self}")));
        }
        let Some(rhs) = rhs else {
            return Ok(()); // monadic
        };
        if rhs.mant.len() as i32 > dig && !allzero(&rhs.mant, dig) {
            return Err(arith(format!("Too many digits: {rhs}")));
        }
        Ok(())
    }

    /// `round(MathContext set)`.
    fn round_mc(&mut self, set: &MathContext) -> Result<&mut Self> {
        self.round(set.digits, set.rounding_mode)
    }

    /// `round(int len, int mode)`: rounds this number in place to `len`
    /// digits.
    fn round(&mut self, len: i32, mode: i32) -> Result<&mut Self> {
        let adjust = self.mant.len() as i32 - len;
        if adjust <= 0 {
            return Ok(self); // nowt to do
        }

        self.exp += adjust; // exponent of result
        let sign = self.ind as i32; // save [assumes -1, 0, 1]
        let oldmant = std::mem::take(&mut self.mant); // save
        let first: i8;
        if len > 0 {
            // remove the unwanted digits
            self.mant = oldmant[..len as usize].to_vec();
            first = oldmant[len as usize]; // first of discarded digits
        } else {
            /* len<=0 */
            self.mant = vec![0];
            self.ind = ISZERO;
            first = if len == 0 { oldmant[0] } else { 0 }; // [virtual digit]
        }

        // decide rounding adjustment depending on mode, sign, and discarded digits
        let mut increment = 0; // bumper
        if mode == Self::ROUND_HALF_UP {
            // default first [most common]
            if first >= 5 {
                increment = sign;
            }
        } else if mode == Self::ROUND_UNNECESSARY {
            // default for setScale()
            // discarding any non-zero digits is an error
            if !allzero(&oldmant, len) {
                return Err(arith("Rounding necessary".to_string()));
            }
        } else if mode == Self::ROUND_HALF_DOWN {
            // 0.5000 goes down
            if first > 5 {
                increment = sign;
            } else if first == 5 && !allzero(&oldmant, len + 1) {
                increment = sign;
            }
        } else if mode == Self::ROUND_HALF_EVEN {
            // 0.5000 goes down if left digit even
            if first > 5 {
                increment = sign;
            } else if first == 5 {
                if !allzero(&oldmant, len + 1) {
                    increment = sign;
                } else
                /* 0.5000 */
                if (self.mant[self.mant.len() - 1] % 2) != 0 {
                    increment = sign;
                }
            }
        } else if mode == Self::ROUND_DOWN {
            // never increment
        } else if mode == Self::ROUND_UP {
            // increment if discarded non-zero
            if !allzero(&oldmant, len) {
                increment = sign;
            }
        } else if mode == Self::ROUND_CEILING {
            // more positive
            if sign > 0 && !allzero(&oldmant, len) {
                increment = sign;
            }
        } else if mode == Self::ROUND_FLOOR {
            // more negative
            if sign < 0 && !allzero(&oldmant, len) {
                increment = sign;
            }
        } else {
            return Err(LuceneError::illegal_argument(format!("Bad round value: {mode}")));
        }

        if increment != 0 {
            if self.ind == ISZERO {
                // we must not subtract from 0, but result is trivial anyway
                self.mant = vec![1];
                self.ind = increment as i8;
            } else {
                // mantissa is non-0; we can safely add or subtract 1
                if self.ind == ISNEG {
                    increment = -increment;
                }
                let newmant = byteaddsub(&self.mant, self.mant.len() as i32, &[1], 1, increment);
                if newmant.len() > self.mant.len() {
                    // had a carry
                    // drop rightmost digit and raise exponent
                    self.exp += 1;
                    // mant is already the correct length
                    let n = self.mant.len();
                    self.mant.copy_from_slice(&newmant[..n]);
                } else {
                    self.mant = newmant;
                }
            }
        }
        // rounding can increase exponent significantly
        if self.exp > MAX_EXP {
            return Err(arith(format!("Exponent Overflow: {}", self.exp)));
        }
        Ok(self)
    }

    /// `finish(MathContext set, boolean strip)`: rounds to the context,
    /// optionally strips trailing zeros, removes leading zeros, chooses the
    /// form and checks the exponent.
    fn finish(mut self, set: &MathContext, strip: bool) -> Result<Self> {
        /* Round if mantissa too long and digits requested */
        if set.digits != 0 && self.mant.len() as i32 > set.digits {
            self.round_mc(set)?;
        }

        /* If strip requested (and standard formatting), remove
           insignificant trailing zeros. */
        if strip && set.form != MathContext::PLAIN {
            let mut d = self.mant.len();
            /* see if we need to drop any trailing zeros */
            let mut i = d as i32 - 1;
            while i >= 1 {
                if self.mant[i as usize] != 0 {
                    break;
                }
                d -= 1;
                self.exp += 1;
                i -= 1;
            }
            if d < self.mant.len() {
                /* need to reduce */
                self.mant.truncate(d);
            }
        }

        self.form = MathContext::PLAIN as i8; // preset

        /* Now check for leading- and all- zeros in mantissa */
        for i in 0..self.mant.len() {
            if self.mant[i] != 0 {
                // non-0 result; ind will be correct
                // remove leading zeros [e.g., after subtract]
                if i > 0 {
                    self.mant.drain(..i);
                }
                // now determine form if not PLAIN
                let mut mag = self.exp + self.mant.len() as i32;
                if mag > 0 {
                    // most common path
                    if mag > set.digits && set.digits != 0 {
                        self.form = set.form as i8;
                    }
                    if (mag - 1) <= MAX_EXP {
                        return Ok(self); // no overflow; quick return
                    }
                } else if mag < -5 {
                    self.form = set.form as i8;
                }
                /* check for overflow */
                mag -= 1;
                if (mag < MIN_EXP) | (mag > MAX_EXP) {
                    // possible reprieve if form is engineering
                    if self.form as i32 == MathContext::ENGINEERING {
                        let mut sig = mag % 3; // leftover
                        if sig < 0 {
                            sig += 3; // negative exponent
                        }
                        mag -= sig; // exponent to use
                        // 1999.06.29: second test here must be MaxExp
                        if (MIN_EXP..=MAX_EXP).contains(&mag) {
                            return Ok(self);
                        }
                    }
                    return Err(arith(format!("Exponent Overflow: {mag}")));
                }
                return Ok(self);
            }
        }

        // Drop through to here only if mantissa is all zeros
        self.ind = ISZERO;
        if set.form != MathContext::PLAIN {
            self.exp = 0; // standard result; go to '0'
        } else if self.exp > 0 {
            self.exp = 0; // +ve exponent also goes to '0'
        } else {
            // a plain number with -ve exponent; preserve and check exponent
            if self.exp < MIN_EXP {
                return Err(arith(format!("Exponent Overflow: {}", self.exp)));
            }
        }
        self.mant = vec![0]; // canonical mantissa
        Ok(self)
    }
}

impl fmt::Display for BigDecimal {
    /// `toString()`: the number in its form (plain, or with an exponent).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&String::from_utf8_lossy(&self.layout()))
    }
}
