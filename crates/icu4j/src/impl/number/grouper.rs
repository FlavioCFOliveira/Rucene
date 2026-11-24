// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.Grouper`.

use std::sync::{Arc, LazyLock};

use rucene::error::{LuceneError, Result};
use rucene::util::java_collections::java_identity_hash_code_stand_in;

use crate::jdk::to_rust_string;
use crate::number::number_formatter::GroupingStrategy;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::number::decimal_format_properties::DecimalFormatProperties;
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::pattern_string_parser::ParsedPatternInfo;
use crate::util::u_locale::ULocale;
use crate::util::u_resource_bundle::UResourceBundle;

/// The fields of a `Grouper` object.
#[derive(Debug)]
struct GrouperData {
    /// `grouping1`: -2 means "needs locale data"; -4 means "fall back to
    /// Western grouping if not in locale".
    grouping1: i16,
    /// `grouping2`.
    grouping2: i16,
    /// `minGrouping`: -2 means "needs locale data"; -3 means "needs locale
    /// data with min 2".
    min_grouping: i16,
}

/// A full options object for grouping sizes.
///
/// Equivalent to `com.ibm.icu.impl.number.Grouper`. Java's objects are
/// immutable and compared by identity; this handle shares one object
/// ([`ptr_eq`](Self::ptr_eq) is Java's `==` and `equals`).
#[derive(Debug, Clone)]
pub struct Grouper(Arc<GrouperData>);

/// `GROUPER_NEVER`.
static GROUPER_NEVER: LazyLock<Grouper> = LazyLock::new(|| Grouper::new(-1, -1, -2));
/// `GROUPER_MIN2`.
static GROUPER_MIN2: LazyLock<Grouper> = LazyLock::new(|| Grouper::new(-2, -2, -3));
/// `GROUPER_AUTO`.
static GROUPER_AUTO: LazyLock<Grouper> = LazyLock::new(|| Grouper::new(-2, -2, -2));
/// `GROUPER_ON_ALIGNED`.
static GROUPER_ON_ALIGNED: LazyLock<Grouper> = LazyLock::new(|| Grouper::new(-4, -4, 1));
/// `GROUPER_WESTERN`.
static GROUPER_WESTERN: LazyLock<Grouper> = LazyLock::new(|| Grouper::new(3, 3, 1));
/// `GROUPER_INDIC`.
static GROUPER_INDIC: LazyLock<Grouper> = LazyLock::new(|| Grouper::new(3, 2, 1));
/// `GROUPER_WESTERN_MIN2`.
static GROUPER_WESTERN_MIN2: LazyLock<Grouper> = LazyLock::new(|| Grouper::new(3, 3, 2));
/// `GROUPER_INDIC_MIN2`.
static GROUPER_INDIC_MIN2: LazyLock<Grouper> = LazyLock::new(|| Grouper::new(3, 2, 2));

impl Grouper {
    /// `Grouper(short, short, short)` (private).
    fn new(grouping1: i16, grouping2: i16, min_grouping: i16) -> Self {
        Grouper(Arc::new(GrouperData {
            grouping1,
            grouping2,
            min_grouping,
        }))
    }

    /// Whether two handles are the same Java object.
    pub fn ptr_eq(&self, other: &Grouper) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// `Object.hashCode()` (identity): see
    /// `java_identity_hash_code_stand_in`.
    pub fn hash_code(&self) -> i32 {
        java_identity_hash_code_stand_in("com.ibm.icu.impl.number.Grouper")
    }

    /// Equivalent to `forStrategy(GroupingStrategy)`.
    pub fn for_strategy(grouping: GroupingStrategy) -> Grouper {
        match grouping {
            GroupingStrategy::Off => GROUPER_NEVER.clone(),
            GroupingStrategy::Min2 => GROUPER_MIN2.clone(),
            GroupingStrategy::Auto => GROUPER_AUTO.clone(),
            GroupingStrategy::OnAligned => GROUPER_ON_ALIGNED.clone(),
            GroupingStrategy::Thousands => GROUPER_WESTERN.clone(),
        }
    }

    /// Resolve the values in Properties to a Grouper object. Equivalent to
    /// `forProperties(DecimalFormatProperties)`.
    pub fn for_properties(properties: &DecimalFormatProperties) -> Grouper {
        if !properties.get_grouping_used() {
            return GROUPER_NEVER.clone();
        }
        let mut grouping1 = properties.get_grouping_size() as i16;
        let mut grouping2 = properties.get_secondary_grouping_size() as i16;
        let min_grouping = properties.get_minimum_grouping_digits() as i16;
        grouping1 = if grouping1 > 0 {
            grouping1
        } else if grouping2 > 0 {
            grouping2
        } else {
            grouping1
        };
        grouping2 = if grouping2 > 0 { grouping2 } else { grouping1 };
        Self::get_instance(grouping1, grouping2, min_grouping)
    }

    /// Equivalent to `getInstance(short, short, short)`.
    pub fn get_instance(grouping1: i16, grouping2: i16, min_grouping: i16) -> Grouper {
        if grouping1 == -1 {
            GROUPER_NEVER.clone()
        } else if grouping1 == 3 && grouping2 == 3 && min_grouping == 1 {
            GROUPER_WESTERN.clone()
        } else if grouping1 == 3 && grouping2 == 2 && min_grouping == 1 {
            GROUPER_INDIC.clone()
        } else if grouping1 == 3 && grouping2 == 3 && min_grouping == 2 {
            GROUPER_WESTERN_MIN2.clone()
        } else if grouping1 == 3 && grouping2 == 2 && min_grouping == 2 {
            GROUPER_INDIC_MIN2.clone()
        } else {
            Grouper::new(grouping1, grouping2, min_grouping)
        }
    }

    /// `getMinGroupingForLocale(ULocale)`.
    fn get_min_grouping_for_locale(locale: &ULocale) -> Result<i16> {
        // TODO: Cache this?
        let resource =
            UResourceBundle::get_bundle_instance_for(Some(ICUData::ICU_BASE_NAME), Some(locale))?;
        let result = resource
            .as_icu()?
            .get_string_with_fallback("NumberElements/minimumGroupingDigits")?;
        parse_short(&to_rust_string(&result))
    }

    /// Equivalent to `withLocaleData(ULocale, ParsedPatternInfo)`.
    ///
    /// # Errors
    ///
    /// The errors of the data loading and the `NumberFormatException` of
    /// `Short.parseShort`.
    pub fn with_locale_data(
        &self,
        locale: &ULocale,
        pattern_info: &ParsedPatternInfo,
    ) -> Result<Grouper> {
        let min_grouping = if self.0.min_grouping == -2 {
            Self::get_min_grouping_for_locale(locale)?
        } else if self.0.min_grouping == -3 {
            2.max(Self::get_min_grouping_for_locale(locale)?)
        } else {
            self.0.min_grouping
        };

        if self.0.grouping1 != -2 && self.0.grouping2 != -4 {
            if min_grouping == self.0.min_grouping {
                return Ok(self.clone());
            }
            return Ok(Self::get_instance(
                self.0.grouping1,
                self.0.grouping2,
                min_grouping,
            ));
        }

        let grouping_sizes = pattern_info.positive.grouping_sizes;
        let mut grouping1 = (grouping_sizes & 0xffff) as i16;
        let mut grouping2 = (((grouping_sizes as u64) >> 16) & 0xffff) as i16;
        let grouping3 = (((grouping_sizes as u64) >> 32) & 0xffff) as i16;
        if grouping2 == -1 {
            grouping1 = if self.0.grouping1 == -4 { 3 } else { -1 };
        }
        if grouping3 == -1 {
            grouping2 = grouping1;
        }
        Ok(Self::get_instance(grouping1, grouping2, min_grouping))
    }

    /// Equivalent to `groupAtPosition(int, DecimalQuantity)`.
    pub fn group_at_position(&self, position: i32, value: &dyn DecimalQuantity) -> bool {
        let g = &self.0;
        debug_assert!(g.grouping1 != -2 && g.grouping1 != -4);
        if g.grouping1 == -1 || g.grouping1 == 0 {
            // Either -1 or 0 means "no grouping"
            return false;
        }
        let position = position - g.grouping1 as i32;
        position >= 0
            && (position % g.grouping2 as i32) == 0
            && value.get_upper_display_magnitude() - g.grouping1 as i32 + 1 >= g.min_grouping as i32
    }

    /// Equivalent to `getPrimary()`.
    pub fn get_primary(&self) -> i16 {
        self.0.grouping1
    }

    /// Equivalent to `getSecondary()`.
    pub fn get_secondary(&self) -> i16 {
        self.0.grouping2
    }
}

/// `Short.parseShort(String)`: radix 10, an optional sign, the range of a
/// `short`.
fn parse_short(s: &str) -> Result<i16> {
    let i = crate::jdk::parse_int(&crate::jdk::units(s), 10)?;
    if !(i16::MIN as i32..=i16::MAX as i32).contains(&i) {
        return Err(LuceneError::number_format(format!(
            "Value out of range. Value:\"{s}\" Radix:10"
        )));
    }
    Ok(i as i16)
}
