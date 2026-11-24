// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.math.MathContext`.

use std::fmt;

use rucene::error::{LuceneError, Result};

const DEFAULT_FORM: i32 = MathContext::SCIENTIFIC;
const DEFAULT_DIGITS: i32 = 9;
const DEFAULT_LOSTDIGITS: bool = false;
const DEFAULT_ROUNDINGMODE: i32 = MathContext::ROUND_HALF_UP;

const MIN_DIGITS: i32 = 0; // smallest value for DIGITS.
const MAX_DIGITS: i32 = 999999999; // largest value for DIGITS.  If increased,
                                   // the BigDecimal class may need update.

// list of valid rounding mode values, most common two first
const ROUNDS: [i32; 8] = [
    MathContext::ROUND_HALF_UP,
    MathContext::ROUND_UNNECESSARY,
    MathContext::ROUND_CEILING,
    MathContext::ROUND_DOWN,
    MathContext::ROUND_FLOOR,
    MathContext::ROUND_HALF_DOWN,
    MathContext::ROUND_HALF_EVEN,
    MathContext::ROUND_UP,
];

const ROUNDWORDS: [&str; 8] = [
    "ROUND_HALF_UP",
    "ROUND_UNNECESSARY",
    "ROUND_CEILING",
    "ROUND_DOWN",
    "ROUND_FLOOR",
    "ROUND_HALF_DOWN",
    "ROUND_HALF_EVEN",
    "ROUND_UP",
]; // matching names of the ROUNDS values

/// The context settings which describe certain rules for the operators of
/// `com.ibm.icu.math.BigDecimal`: digits (precision), notation form,
/// lost-digits checking and rounding mode.
///
/// Equivalent to `com.ibm.icu.math.MathContext` (immutable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MathContext {
    pub(crate) digits: i32,
    pub(crate) form: i32,
    pub(crate) lost_digits: bool,
    pub(crate) rounding_mode: i32,
}

/// `MathContext.DEFAULT`: 9 digits, scientific, no lost-digits checking,
/// `ROUND_HALF_UP`.
pub const DEFAULT: MathContext = MathContext {
    digits: DEFAULT_DIGITS,
    form: DEFAULT_FORM,
    lost_digits: DEFAULT_LOSTDIGITS,
    rounding_mode: DEFAULT_ROUNDINGMODE,
};

impl MathContext {
    /// `PLAIN`: plain (fixed point) notation, without any exponent.
    pub const PLAIN: i32 = 0;
    /// `SCIENTIFIC`: standard floating point notation (with scientific
    /// exponential format, where there is one digit before any decimal
    /// point).
    pub const SCIENTIFIC: i32 = 1;
    /// `ENGINEERING`: standard floating point notation (with engineering
    /// exponential format, where the power of ten is a multiple of 3).
    pub const ENGINEERING: i32 = 2;
    /// `ROUND_CEILING`.
    pub const ROUND_CEILING: i32 = 2;
    /// `ROUND_DOWN`.
    pub const ROUND_DOWN: i32 = 1;
    /// `ROUND_FLOOR`.
    pub const ROUND_FLOOR: i32 = 3;
    /// `ROUND_HALF_DOWN`.
    pub const ROUND_HALF_DOWN: i32 = 5;
    /// `ROUND_HALF_EVEN`.
    pub const ROUND_HALF_EVEN: i32 = 6;
    /// `ROUND_HALF_UP`.
    pub const ROUND_HALF_UP: i32 = 4;
    /// `ROUND_UNNECESSARY`.
    pub const ROUND_UNNECESSARY: i32 = 7;
    /// `ROUND_UP`.
    pub const ROUND_UP: i32 = 0;

    /// `new MathContext(int setdigits)`.
    ///
    /// # Errors
    ///
    /// As [`with_all`](Self::with_all).
    pub fn new(setdigits: i32) -> Result<Self> {
        Self::with_all(
            setdigits,
            DEFAULT_FORM,
            DEFAULT_LOSTDIGITS,
            DEFAULT_ROUNDINGMODE,
        )
    }

    /// `new MathContext(int setdigits, int setform)`.
    ///
    /// # Errors
    ///
    /// As [`with_all`](Self::with_all).
    pub fn with_form(setdigits: i32, setform: i32) -> Result<Self> {
        Self::with_all(setdigits, setform, DEFAULT_LOSTDIGITS, DEFAULT_ROUNDINGMODE)
    }

    /// `new MathContext(int setdigits, int setform, boolean setlostdigits)`.
    ///
    /// # Errors
    ///
    /// As [`with_all`](Self::with_all).
    pub fn with_lost_digits(setdigits: i32, setform: i32, setlostdigits: bool) -> Result<Self> {
        Self::with_all(setdigits, setform, setlostdigits, DEFAULT_ROUNDINGMODE)
    }

    /// `new MathContext(int setdigits, int setform, boolean setlostdigits,
    /// int setroundingmode)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` (`Digits too small: <n>`, `Digits too
    /// large: <n>`, `Bad form value: <n>`, `Bad roundingMode value: <n>`).
    pub fn with_all(
        setdigits: i32,
        setform: i32,
        setlostdigits: bool,
        setroundingmode: i32,
    ) -> Result<Self> {
        // set values, after checking
        if setdigits != DEFAULT_DIGITS {
            if setdigits < MIN_DIGITS {
                return Err(LuceneError::illegal_argument(format!(
                    "Digits too small: {setdigits}"
                )));
            }
            if setdigits > MAX_DIGITS {
                return Err(LuceneError::illegal_argument(format!(
                    "Digits too large: {setdigits}"
                )));
            }
        }
        if setform != Self::SCIENTIFIC && setform != Self::ENGINEERING && setform != Self::PLAIN {
            return Err(LuceneError::illegal_argument(format!(
                "Bad form value: {setform}"
            )));
        }
        if !is_valid_round(setroundingmode) {
            return Err(LuceneError::illegal_argument(format!(
                "Bad roundingMode value: {setroundingmode}"
            )));
        }
        Ok(Self {
            digits: setdigits,
            form: setform,
            lost_digits: setlostdigits,
            rounding_mode: setroundingmode,
        })
    }

    /// `getDigits()`.
    pub fn get_digits(&self) -> i32 {
        self.digits
    }

    /// `getForm()`.
    pub fn get_form(&self) -> i32 {
        self.form
    }

    /// `getLostDigits()`.
    pub fn get_lost_digits(&self) -> bool {
        self.lost_digits
    }

    /// `getRoundingMode()`.
    pub fn get_rounding_mode(&self) -> i32 {
        self.rounding_mode
    }
}

impl fmt::Display for MathContext {
    /// `toString()`: `digits=<d> form=<FORM> lostDigits=<0|1>
    /// roundingMode=<ROUND_x>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let formstr = if self.form == Self::SCIENTIFIC {
            "SCIENTIFIC"
        } else if self.form == Self::ENGINEERING {
            "ENGINEERING"
        } else {
            "PLAIN"
        };
        let mut roundword = "null";
        for r in 0..ROUNDS.len() {
            if self.rounding_mode == ROUNDS[r] {
                roundword = ROUNDWORDS[r];
                break;
            }
        }
        write!(
            f,
            "digits={} form={} lostDigits={} roundingMode={}",
            self.digits,
            formstr,
            if self.lost_digits { "1" } else { "0" },
            roundword
        )
    }
}

/// `isValidRound(int testround)`.
fn is_valid_round(testround: i32) -> bool {
    ROUNDS.contains(&testround)
}
