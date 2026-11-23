// © 2018 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.range.RangeMacroProps`.

use crate::number::number_formatter_settings::NumberFormatterSettings;
use crate::number::number_range_formatter::{RangeCollapse, RangeIdentityFallback};
use crate::number::unlocalized_number_formatter::UnlocalizedNumberFormatter;
use crate::util::u_locale::ULocale;

/// The settings of a number range formatter.
///
/// Equivalent to `com.ibm.icu.impl.number.range.RangeMacroProps`; its
/// `null`s are `None`.
#[derive(Debug, Clone)]
pub struct RangeMacroProps {
    /// `formatter1`.
    pub formatter1: Option<UnlocalizedNumberFormatter>,
    /// `formatter2`.
    pub formatter2: Option<UnlocalizedNumberFormatter>,
    /// `sameFormatters`: -1 for unset, 0 for false, 1 for true.
    pub same_formatters: i32,
    /// `collapse`.
    pub collapse: Option<RangeCollapse>,
    /// `identityFallback`.
    pub identity_fallback: Option<RangeIdentityFallback>,
    /// `loc`.
    pub loc: Option<ULocale>,
}

impl Default for RangeMacroProps {
    fn default() -> Self {
        RangeMacroProps {
            formatter1: None,
            formatter2: None,
            same_formatters: -1,
            collapse: None,
            identity_fallback: None,
            loc: None,
        }
    }
}

impl RangeMacroProps {
    /// Equivalent to `hashCode()`: `Objects.hash(formatter1, formatter2,
    /// collapse, identityFallback, loc)`.
    pub fn hash_code(&self) -> i32 {
        let hashes = [
            self.formatter1.as_ref().map_or(0, |f| f.hash_code()),
            self.formatter2.as_ref().map_or(0, |f| f.hash_code()),
            self.collapse.map_or(0, |c| c.hash_code()),
            self.identity_fallback.map_or(0, |i| i.hash_code()),
            self.loc.as_ref().map_or(0, |l| l.hash_code()),
        ];
        hashes.iter().fold(1i32, |h, &e| h.wrapping_mul(31).wrapping_add(e))
    }

    /// Equivalent to `equals(Object)`.
    pub fn java_equals(&self, other: &RangeMacroProps) -> bool {
        fn formatter_eq(a: &Option<UnlocalizedNumberFormatter>, b: &Option<UnlocalizedNumberFormatter>) -> bool {
            match (a, b) {
                (None, None) => true,
                (Some(a), Some(b)) => a.java_equals(b),
                _ => false,
            }
        }
        formatter_eq(&self.formatter1, &other.formatter1)
            && formatter_eq(&self.formatter2, &other.formatter2)
            && self.collapse == other.collapse
            && self.identity_fallback == other.identity_fallback
            && self.loc == other.loc
    }
}
