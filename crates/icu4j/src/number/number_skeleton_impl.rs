// © 2018 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.NumberSkeletonImpl`.

use std::sync::{Arc, LazyLock};

use rucene::error::{LuceneError, Result};

use crate::jdk::math::{BigDecimal, RoundingMode};
use crate::jdk::{char_count, to_rust_string, units};
use crate::number::compact_notation::CompactNotation;
use crate::number::fraction_precision::FractionPrecision;
use crate::number::integer_width::{self, IntegerWidth};
use crate::number::notation::Notation;
use crate::number::number_formatter::{
    DecimalSeparatorDisplay, GroupingStrategy, NumberFormatter, RoundingPriority, SignDisplay, TrailingZeroDisplay,
    UnitWidth,
};
use crate::number::number_formatter_settings::NumberFormatterSettings;
use crate::number::precision::{Precision, PrecisionKind};
use crate::number::scale::Scale;
use crate::number::scientific_notation::ScientificNotation;
use crate::number::simple_notation::SimpleNotation;
use crate::number::skeleton_syntax_exception::{new_skeleton, new_skeleton_with_cause};
use crate::number::unlocalized_number_formatter::UnlocalizedNumberFormatter;
use crate::r#impl::cache_base::CacheBase;
use crate::r#impl::number::macro_props::{GroupingObject, MacroProps, SymbolsObject};
use crate::r#impl::number::rounding_utils::RoundingUtils;
use crate::r#impl::pattern_props::PatternProps;
use crate::r#impl::soft_cache::SoftCache;
use crate::r#impl::string_segment::StringSegment;
use crate::text::numbering_system::{self, NumberingSystem};
use crate::util::bytes_trie::Result as TrieResult;
use crate::util::chars_trie::CharsTrie;
use crate::util::chars_trie_builder::CharsTrieBuilder;
use crate::util::currency::{Currency, CurrencyUsage};
use crate::util::measure_unit::{self, MeasureUnit};
use crate::util::no_unit::NoUnit;
use crate::util::string_trie_builder::StringTrieBuilderOption;

/// While parsing a skeleton, this enum records what type of option we
/// expect to find next. Equivalent to `NumberSkeletonImpl.ParseState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseState {
    // Section 0: We expect whitespace or a stem, but not an option:
    StateNull,

    // Section 1: We might accept an option, but it is not required:
    StateScientific,
    StateFractionPrecision,
    StatePrecision,

    // Section 2: An option is required:
    StateIncrementPrecision,
    StateMeasureUnit,
    StatePerMeasureUnit,
    StateIdentifierUnit,
    StateUnitUsage,
    StateCurrencyUnit,
    StateIntegerWidth,
    StateNumberingSystem,
    StateScale,
}

/// All possible stem literals have an entry in the StemEnum. The enum name
/// is the kebab case stem string literal written in upper snake case.
/// Equivalent to `NumberSkeletonImpl.StemEnum`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StemEnum {
    // Section 1: Stems that do not require an option:
    StemCompactShort,
    StemCompactLong,
    StemScientific,
    StemEngineering,
    StemNotationSimple,
    StemBaseUnit,
    StemPercent,
    StemPermille,
    StemPercent100, // concise-only
    StemPrecisionInteger,
    StemPrecisionUnlimited,
    StemPrecisionCurrencyStandard,
    StemPrecisionCurrencyCash,
    StemRoundingModeCeiling,
    StemRoundingModeFloor,
    StemRoundingModeDown,
    StemRoundingModeUp,
    StemRoundingModeHalfEven,
    StemRoundingModeHalfDown,
    StemRoundingModeHalfUp,
    StemRoundingModeUnnecessary,
    StemIntegerWidthTrunc,
    StemGroupOff,
    StemGroupMin2,
    StemGroupAuto,
    StemGroupOnAligned,
    StemGroupThousands,
    StemLatin,
    StemUnitWidthNarrow,
    StemUnitWidthShort,
    StemUnitWidthFullName,
    StemUnitWidthIsoCode,
    StemUnitWidthFormal,
    StemUnitWidthVariant,
    StemUnitWidthHidden,
    StemSignAuto,
    StemSignAlways,
    StemSignNever,
    StemSignAccounting,
    StemSignAccountingAlways,
    StemSignExceptZero,
    StemSignAccountingExceptZero,
    StemSignNegative,
    StemSignAccountingNegative,
    StemDecimalAuto,
    StemDecimalAlways,

    // Section 2: Stems that DO require an option:
    StemPrecisionIncrement,
    StemMeasureUnit,
    StemPerMeasureUnit,
    StemUnit,
    StemUnitUsage,
    StemCurrency,
    StemIntegerWidth,
    StemNumberingSystem,
    StemScale,
}

/// `STEM_ENUM_VALUES`: for convenience, an array of all stem enum values.
const STEM_ENUM_VALUES: [StemEnum; 55] = {
    use StemEnum::*;
    [
        StemCompactShort,
        StemCompactLong,
        StemScientific,
        StemEngineering,
        StemNotationSimple,
        StemBaseUnit,
        StemPercent,
        StemPermille,
        StemPercent100,
        StemPrecisionInteger,
        StemPrecisionUnlimited,
        StemPrecisionCurrencyStandard,
        StemPrecisionCurrencyCash,
        StemRoundingModeCeiling,
        StemRoundingModeFloor,
        StemRoundingModeDown,
        StemRoundingModeUp,
        StemRoundingModeHalfEven,
        StemRoundingModeHalfDown,
        StemRoundingModeHalfUp,
        StemRoundingModeUnnecessary,
        StemIntegerWidthTrunc,
        StemGroupOff,
        StemGroupMin2,
        StemGroupAuto,
        StemGroupOnAligned,
        StemGroupThousands,
        StemLatin,
        StemUnitWidthNarrow,
        StemUnitWidthShort,
        StemUnitWidthFullName,
        StemUnitWidthIsoCode,
        StemUnitWidthFormal,
        StemUnitWidthVariant,
        StemUnitWidthHidden,
        StemSignAuto,
        StemSignAlways,
        StemSignNever,
        StemSignAccounting,
        StemSignAccountingAlways,
        StemSignExceptZero,
        StemSignAccountingExceptZero,
        StemSignNegative,
        StemSignAccountingNegative,
        StemDecimalAuto,
        StemDecimalAlways,
        StemPrecisionIncrement,
        StemMeasureUnit,
        StemPerMeasureUnit,
        StemUnit,
        StemUnitUsage,
        StemCurrency,
        StemIntegerWidth,
        StemNumberingSystem,
        StemScale,
    ]
};

/// `WILDCARD_CHAR`.
const WILDCARD_CHAR: u16 = '*' as u16;
/// `ALT_WILDCARD_CHAR`.
const ALT_WILDCARD_CHAR: u16 = '+' as u16;

/// `isWildcardChar(char)`.
fn is_wildcard_char(c: u16) -> bool {
    c == WILDCARD_CHAR || c == ALT_WILDCARD_CHAR
}

/// `SERIALIZED_STEM_TRIE`: a data structure for mapping from stem strings
/// to the stem enum. Built at startup.
static SERIALIZED_STEM_TRIE: LazyLock<Arc<[u16]>> = LazyLock::new(|| match build_stem_trie() {
    Ok(t) => Arc::from(t),
    Err(e) => panic!("java.lang.ExceptionInInitializerError: {e}"),
});

/// Equivalent to `buildStemTrie()`.
fn build_stem_trie() -> Result<Vec<u16>> {
    use StemEnum::*;
    let mut b = CharsTrieBuilder::new();
    let entries: &[(&str, StemEnum)] = &[
        // Section 1:
        ("compact-short", StemCompactShort),
        ("compact-long", StemCompactLong),
        ("scientific", StemScientific),
        ("engineering", StemEngineering),
        ("notation-simple", StemNotationSimple),
        ("base-unit", StemBaseUnit),
        ("percent", StemPercent),
        ("permille", StemPermille),
        ("precision-integer", StemPrecisionInteger),
        ("precision-unlimited", StemPrecisionUnlimited),
        ("precision-currency-standard", StemPrecisionCurrencyStandard),
        ("precision-currency-cash", StemPrecisionCurrencyCash),
        ("rounding-mode-ceiling", StemRoundingModeCeiling),
        ("rounding-mode-floor", StemRoundingModeFloor),
        ("rounding-mode-down", StemRoundingModeDown),
        ("rounding-mode-up", StemRoundingModeUp),
        ("rounding-mode-half-even", StemRoundingModeHalfEven),
        ("rounding-mode-half-down", StemRoundingModeHalfDown),
        ("rounding-mode-half-up", StemRoundingModeHalfUp),
        ("rounding-mode-unnecessary", StemRoundingModeUnnecessary),
        ("integer-width-trunc", StemIntegerWidthTrunc),
        ("group-off", StemGroupOff),
        ("group-min2", StemGroupMin2),
        ("group-auto", StemGroupAuto),
        ("group-on-aligned", StemGroupOnAligned),
        ("group-thousands", StemGroupThousands),
        ("latin", StemLatin),
        ("unit-width-narrow", StemUnitWidthNarrow),
        ("unit-width-short", StemUnitWidthShort),
        ("unit-width-full-name", StemUnitWidthFullName),
        ("unit-width-iso-code", StemUnitWidthIsoCode),
        ("unit-width-formal", StemUnitWidthFormal),
        ("unit-width-variant", StemUnitWidthVariant),
        ("unit-width-hidden", StemUnitWidthHidden),
        ("sign-auto", StemSignAuto),
        ("sign-always", StemSignAlways),
        ("sign-never", StemSignNever),
        ("sign-accounting", StemSignAccounting),
        ("sign-accounting-always", StemSignAccountingAlways),
        ("sign-except-zero", StemSignExceptZero),
        ("sign-accounting-except-zero", StemSignAccountingExceptZero),
        ("sign-negative", StemSignNegative),
        ("sign-accounting-negative", StemSignAccountingNegative),
        ("decimal-auto", StemDecimalAuto),
        ("decimal-always", StemDecimalAlways),
        // Section 2:
        ("precision-increment", StemPrecisionIncrement),
        ("measure-unit", StemMeasureUnit),
        ("per-measure-unit", StemPerMeasureUnit),
        ("unit", StemUnit),
        ("usage", StemUnitUsage),
        ("currency", StemCurrency),
        ("integer-width", StemIntegerWidth),
        ("numbering-system", StemNumberingSystem),
        ("scale", StemScale),
        // Section 3 (concise tokens):
        ("K", StemCompactShort),
        ("KK", StemCompactLong),
        ("%", StemPercent),
        ("%x100", StemPercent100),
        (",_", StemGroupOff),
        (",?", StemGroupMin2),
        (",!", StemGroupOnAligned),
        ("+!", StemSignAlways),
        ("+_", StemSignNever),
        ("()", StemSignAccounting),
        ("()!", StemSignAccountingAlways),
        ("+?", StemSignExceptZero),
        ("()?", StemSignAccountingExceptZero),
        ("+-", StemSignNegative),
        ("()-", StemSignAccountingNegative),
    ];
    for (s, stem) in entries {
        b.add(&units(s), *stem as i32)?;
    }

    // Build the CharsTrie
    // TODO: Use SLOW or FAST here?
    b.build_char_sequence(StringTrieBuilderOption::Fast)
}

/// Utility class for methods that convert from StemEnum to corresponding
/// objects or enums. This applies to only the "Section 1" stems, those that
/// are well-defined without an option. Equivalent to
/// `NumberSkeletonImpl.StemToObject`.
struct StemToObject;

impl StemToObject {
    fn notation(stem: StemEnum) -> Notation {
        match stem {
            StemEnum::StemCompactShort => Notation::compact_short().into_notation(),
            StemEnum::StemCompactLong => Notation::compact_long().into_notation(),
            StemEnum::StemScientific => Notation::scientific().into_notation(),
            StemEnum::StemEngineering => Notation::engineering().into_notation(),
            StemEnum::StemNotationSimple => Notation::simple().into_notation(),
            _ => panic!("java.lang.AssertionError"),
        }
    }

    /// `NoUnit.BASE` is `None`.
    fn unit(stem: StemEnum) -> Option<MeasureUnit> {
        match stem {
            StemEnum::StemBaseUnit => NoUnit::BASE,
            StemEnum::StemPercent => Some(NoUnit::percent()),
            StemEnum::StemPermille => Some(NoUnit::permille()),
            _ => panic!("java.lang.AssertionError"),
        }
    }

    fn precision(stem: StemEnum) -> Result<Precision> {
        match stem {
            StemEnum::StemPrecisionInteger => Ok(Precision::integer().into_precision()),
            StemEnum::StemPrecisionUnlimited => Ok(Precision::unlimited()),
            StemEnum::StemPrecisionCurrencyStandard => Ok(Precision::currency(Some(CurrencyUsage::Standard))?.into_precision()),
            StemEnum::StemPrecisionCurrencyCash => Ok(Precision::currency(Some(CurrencyUsage::Cash))?.into_precision()),
            _ => panic!("java.lang.AssertionError"),
        }
    }

    fn rounding_mode(stem: StemEnum) -> RoundingMode {
        match stem {
            StemEnum::StemRoundingModeCeiling => RoundingMode::Ceiling,
            StemEnum::StemRoundingModeFloor => RoundingMode::Floor,
            StemEnum::StemRoundingModeDown => RoundingMode::Down,
            StemEnum::StemRoundingModeUp => RoundingMode::Up,
            StemEnum::StemRoundingModeHalfEven => RoundingMode::HalfEven,
            StemEnum::StemRoundingModeHalfDown => RoundingMode::HalfDown,
            StemEnum::StemRoundingModeHalfUp => RoundingMode::HalfUp,
            StemEnum::StemRoundingModeUnnecessary => RoundingMode::Unnecessary,
            _ => panic!("java.lang.AssertionError"),
        }
    }

    fn grouping_strategy(stem: StemEnum) -> Option<GroupingStrategy> {
        match stem {
            StemEnum::StemGroupOff => Some(GroupingStrategy::Off),
            StemEnum::StemGroupMin2 => Some(GroupingStrategy::Min2),
            StemEnum::StemGroupAuto => Some(GroupingStrategy::Auto),
            StemEnum::StemGroupOnAligned => Some(GroupingStrategy::OnAligned),
            StemEnum::StemGroupThousands => Some(GroupingStrategy::Thousands),
            _ => None, // for objects, throw; for enums, return null
        }
    }

    fn unit_width(stem: StemEnum) -> Option<UnitWidth> {
        match stem {
            StemEnum::StemUnitWidthNarrow => Some(UnitWidth::Narrow),
            StemEnum::StemUnitWidthShort => Some(UnitWidth::Short),
            StemEnum::StemUnitWidthFullName => Some(UnitWidth::FullName),
            StemEnum::StemUnitWidthIsoCode => Some(UnitWidth::IsoCode),
            StemEnum::StemUnitWidthFormal => Some(UnitWidth::Formal),
            StemEnum::StemUnitWidthVariant => Some(UnitWidth::Variant),
            StemEnum::StemUnitWidthHidden => Some(UnitWidth::Hidden),
            _ => None, // for objects, throw; for enums, return null
        }
    }

    fn sign_display(stem: StemEnum) -> Option<SignDisplay> {
        match stem {
            StemEnum::StemSignAuto => Some(SignDisplay::Auto),
            StemEnum::StemSignAlways => Some(SignDisplay::Always),
            StemEnum::StemSignNever => Some(SignDisplay::Never),
            StemEnum::StemSignAccounting => Some(SignDisplay::Accounting),
            StemEnum::StemSignAccountingAlways => Some(SignDisplay::AccountingAlways),
            StemEnum::StemSignExceptZero => Some(SignDisplay::ExceptZero),
            StemEnum::StemSignAccountingExceptZero => Some(SignDisplay::AccountingExceptZero),
            StemEnum::StemSignNegative => Some(SignDisplay::Negative),
            StemEnum::StemSignAccountingNegative => Some(SignDisplay::AccountingNegative),
            _ => None, // for objects, throw; for enums, return null
        }
    }

    fn decimal_separator_display(stem: StemEnum) -> Option<DecimalSeparatorDisplay> {
        match stem {
            StemEnum::StemDecimalAuto => Some(DecimalSeparatorDisplay::Auto),
            StemEnum::StemDecimalAlways => Some(DecimalSeparatorDisplay::Always),
            _ => None, // for objects, throw; for enums, return null
        }
    }
}

/// `StringBuilder.append(String)`.
fn push(sb: &mut Vec<u16>, s: &str) {
    sb.extend(s.encode_utf16());
}

/// Utility class for methods that convert from enums to stem strings. More
/// complex object conversions are in GeneratorHelpers. Equivalent to
/// `NumberSkeletonImpl.EnumToStemString`.
struct EnumToStemString;

impl EnumToStemString {
    fn rounding_mode(value: RoundingMode, sb: &mut Vec<u16>) {
        push(
            sb,
            match value {
                RoundingMode::Ceiling => "rounding-mode-ceiling",
                RoundingMode::Floor => "rounding-mode-floor",
                RoundingMode::Down => "rounding-mode-down",
                RoundingMode::Up => "rounding-mode-up",
                RoundingMode::HalfEven => "rounding-mode-half-even",
                RoundingMode::HalfDown => "rounding-mode-half-down",
                RoundingMode::HalfUp => "rounding-mode-half-up",
                RoundingMode::Unnecessary => "rounding-mode-unnecessary",
            },
        );
    }

    fn grouping_strategy(value: GroupingStrategy, sb: &mut Vec<u16>) {
        push(
            sb,
            match value {
                GroupingStrategy::Off => "group-off",
                GroupingStrategy::Min2 => "group-min2",
                GroupingStrategy::Auto => "group-auto",
                GroupingStrategy::OnAligned => "group-on-aligned",
                GroupingStrategy::Thousands => "group-thousands",
            },
        );
    }

    fn unit_width(value: UnitWidth, sb: &mut Vec<u16>) {
        push(
            sb,
            match value {
                UnitWidth::Narrow => "unit-width-narrow",
                UnitWidth::Short => "unit-width-short",
                UnitWidth::FullName => "unit-width-full-name",
                UnitWidth::IsoCode => "unit-width-iso-code",
                UnitWidth::Formal => "unit-width-formal",
                UnitWidth::Variant => "unit-width-variant",
                UnitWidth::Hidden => "unit-width-hidden",
            },
        );
    }

    fn sign_display(value: SignDisplay, sb: &mut Vec<u16>) {
        push(
            sb,
            match value {
                SignDisplay::Auto => "sign-auto",
                SignDisplay::Always => "sign-always",
                SignDisplay::Never => "sign-never",
                SignDisplay::Accounting => "sign-accounting",
                SignDisplay::AccountingAlways => "sign-accounting-always",
                SignDisplay::ExceptZero => "sign-except-zero",
                SignDisplay::AccountingExceptZero => "sign-accounting-except-zero",
                SignDisplay::Negative => "sign-negative",
                SignDisplay::AccountingNegative => "sign-accounting-negative",
            },
        );
    }

    fn decimal_separator_display(value: DecimalSeparatorDisplay, sb: &mut Vec<u16>) {
        push(
            sb,
            match value {
                DecimalSeparatorDisplay::Auto => "decimal-auto",
                DecimalSeparatorDisplay::Always => "decimal-always",
            },
        );
    }
}

/// `cache`: cache for parsed skeleton strings.
static CACHE: LazyLock<SoftCache<Vec<u16>, UnlocalizedNumberFormatter, ()>> =
    LazyLock::new(|| SoftCache::new(|skeleton_string: &Vec<u16>, _unused: ()| Ok(Some(NumberSkeletonImpl::create(skeleton_string)?))));

/// The parser and generator of number skeleton strings.
///
/// Equivalent to the package-private class
/// `com.ibm.icu.number.NumberSkeletonImpl` (static members only).
pub struct NumberSkeletonImpl;

impl NumberSkeletonImpl {
    /// Gets the number formatter for the given number skeleton string from
    /// the cache, creating it if it does not exist in the cache. Equivalent
    /// to `getOrCreate(String)`.
    ///
    /// # Errors
    ///
    /// `SkeletonSyntaxException` for an invalid skeleton.
    pub fn get_or_create(skeleton_string: &[u16]) -> Result<UnlocalizedNumberFormatter> {
        // TODO: This does not currently check the cache for the normalized
        // form of the skeleton.
        Ok(CACHE.get_instance(skeleton_string.to_vec(), ())?.expect("the cache creates a formatter"))
    }

    /// Creates a NumberFormatter corresponding to the given skeleton string.
    /// Equivalent to `create(String)`.
    ///
    /// # Errors
    ///
    /// `SkeletonSyntaxException` for an invalid skeleton.
    pub fn create(skeleton_string: &[u16]) -> Result<UnlocalizedNumberFormatter> {
        let macros = parse_skeleton(skeleton_string)?;
        Ok(NumberFormatter::with().macros(macros))
    }

    /// Create a skeleton string corresponding to the given NumberFormatter.
    /// Equivalent to `generate(MacroProps)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` for a setting a skeleton cannot
    /// represent.
    pub fn generate(macros: &MacroProps) -> Result<Vec<u16>> {
        let mut sb = Vec::new();
        generate_skeleton(macros, &mut sb)?;
        Ok(sb)
    }
}

/// The `SkeletonSyntaxException` of a segment.
fn syntax(message: &str, segment: &StringSegment) -> LuceneError {
    new_skeleton(message, &segment.to_java_string())
}

/// Parses a skeleton string into a MacroProps. Equivalent to
/// `parseSkeleton(String)`.
fn parse_skeleton(skeleton_string: &[u16]) -> Result<MacroProps> {
    // Add a trailing whitespace to the end of the skeleton string to make
    // code cleaner.
    let mut skeleton_string = skeleton_string.to_vec();
    skeleton_string.push(' ' as u16);

    let mut macros = MacroProps::default();
    let mut segment = StringSegment::new(&skeleton_string, false);
    let mut stem_trie = CharsTrie::new(SERIALIZED_STEM_TRIE.clone(), 0);
    let mut stem = ParseState::StateNull;
    let mut offset = 0;

    // Primary skeleton parse loop:
    while offset < segment.length() {
        let cp = segment.code_point_at(offset);
        let is_token_separator = PatternProps::is_white_space(cp);
        let is_option_separator = cp == '/' as i32;

        if !is_token_separator && !is_option_separator {
            // Non-separator token; consume it.
            offset += char_count(cp);
            if stem == ParseState::StateNull {
                // We are currently consuming a stem.
                // Go to the next state in the stem trie.
                stem_trie.next_for_code_point(cp);
            }
            continue;
        }

        // We are looking at a token or option separator.
        // If the segment is nonempty, parse it and reset the segment.
        // Otherwise, make sure it is a valid repeating separator.
        if offset != 0 {
            segment.set_length(offset);
            if stem == ParseState::StateNull {
                // The first separator after the start of a token. Parse it
                // as a stem.
                stem = parse_stem(&segment, &stem_trie, &mut macros)?;
                stem_trie.reset();
            } else {
                // A separator after the first separator of a token. Parse it
                // as an option.
                stem = parse_option(stem, &segment, &mut macros)?;
            }
            segment.reset_length();

            // Consume the segment:
            segment.adjust_offset(offset);
            offset = 0;
        } else if stem != ParseState::StateNull {
            // A separator ('/' or whitespace) following an option separator
            // ('/')
            segment.set_length(char_count(cp)); // for error message
            return Err(syntax("Unexpected separator character", &segment));
        } else {
            // Two spaces in a row; this is OK.
        }

        // Does the current stem forbid options?
        if is_option_separator && stem == ParseState::StateNull {
            segment.set_length(char_count(cp)); // for error message
            return Err(syntax("Unexpected option separator", &segment));
        }

        // Does the current stem require an option?
        if is_token_separator && stem != ParseState::StateNull {
            match stem {
                ParseState::StateIncrementPrecision
                | ParseState::StateMeasureUnit
                | ParseState::StatePerMeasureUnit
                | ParseState::StateIdentifierUnit
                | ParseState::StateUnitUsage
                | ParseState::StateCurrencyUnit
                | ParseState::StateIntegerWidth
                | ParseState::StateNumberingSystem
                | ParseState::StateScale => {
                    segment.set_length(char_count(cp)); // for error message
                    return Err(syntax("Stem requires an option", &segment));
                }
                _ => {}
            }
            stem = ParseState::StateNull;
        }

        // Consume the separator:
        segment.adjust_offset(char_count(cp));
    }
    debug_assert!(stem == ParseState::StateNull);
    Ok(macros)
}

/// Given that the current segment represents a stem, parse it and save the
/// result. Equivalent to `parseStem(StringSegment, CharsTrie, MacroProps)`:
/// the next state in the parser.
fn parse_stem(segment: &StringSegment, stem_trie: &CharsTrie, macros: &mut MacroProps) -> Result<ParseState> {
    // First check for "blueprint" stems, which start with a "signal char"
    match segment.char_at(0) {
        c if c == '.' as u16 => {
            check_null(macros.precision.is_some(), segment)?;
            BlueprintHelpers::parse_fraction_stem(segment, macros)?;
            return Ok(ParseState::StateFractionPrecision);
        }
        c if c == '@' as u16 => {
            check_null(macros.precision.is_some(), segment)?;
            BlueprintHelpers::parse_digits_stem(segment, macros)?;
            return Ok(ParseState::StatePrecision);
        }
        c if c == 'E' as u16 => {
            check_null(macros.notation.is_some(), segment)?;
            BlueprintHelpers::parse_scientific_stem(segment, macros)?;
            return Ok(ParseState::StateNull);
        }
        c if c == '0' as u16 => {
            check_null(macros.integer_width.is_some(), segment)?;
            BlueprintHelpers::parse_integer_stem(segment, macros)?;
            return Ok(ParseState::StateNull);
        }
        _ => {}
    }

    // Now look at the stemsTrie, which is already be pointing at our stem.
    let stem_result = stem_trie.current();

    if stem_result != TrieResult::IntermediateValue && stem_result != TrieResult::FinalValue {
        return Err(syntax("Unknown stem", segment));
    }

    let stem = STEM_ENUM_VALUES[stem_trie.get_value() as usize];
    use StemEnum::*;
    match stem {
        // Stems with meaning on their own, not requiring an option:
        StemCompactShort | StemCompactLong | StemScientific | StemEngineering | StemNotationSimple => {
            check_null(macros.notation.is_some(), segment)?;
            macros.notation = Some(StemToObject::notation(stem));
            match stem {
                StemScientific | StemEngineering => Ok(ParseState::StateScientific), // allows for scientific options
                _ => Ok(ParseState::StateNull),
            }
        }

        StemBaseUnit | StemPercent | StemPermille => {
            check_null(macros.unit.is_some(), segment)?;
            macros.unit = StemToObject::unit(stem);
            Ok(ParseState::StateNull)
        }

        StemPercent100 => {
            check_null(macros.scale.is_some(), segment)?;
            check_null(macros.unit.is_some(), segment)?;
            macros.scale = Some(Scale::power_of_ten(2));
            macros.unit = Some(NoUnit::percent());
            Ok(ParseState::StateNull)
        }

        StemPrecisionInteger | StemPrecisionUnlimited | StemPrecisionCurrencyStandard | StemPrecisionCurrencyCash => {
            check_null(macros.precision.is_some(), segment)?;
            macros.precision = Some(StemToObject::precision(stem)?);
            match stem {
                StemPrecisionInteger => Ok(ParseState::StateFractionPrecision), // allows for "precision-integer/@##"
                _ => Ok(ParseState::StatePrecision),
            }
        }

        StemRoundingModeCeiling
        | StemRoundingModeFloor
        | StemRoundingModeDown
        | StemRoundingModeUp
        | StemRoundingModeHalfEven
        | StemRoundingModeHalfDown
        | StemRoundingModeHalfUp
        | StemRoundingModeUnnecessary => {
            check_null(macros.rounding_mode.is_some(), segment)?;
            macros.rounding_mode = Some(StemToObject::rounding_mode(stem));
            Ok(ParseState::StateNull)
        }

        StemIntegerWidthTrunc => {
            check_null(macros.integer_width.is_some(), segment)?;
            macros.integer_width = Some(IntegerWidth::zero_fill_to(0)?.truncate_at(0)?);
            Ok(ParseState::StateNull)
        }

        StemGroupOff | StemGroupMin2 | StemGroupAuto | StemGroupOnAligned | StemGroupThousands => {
            check_null(macros.grouping.is_some(), segment)?;
            macros.grouping = StemToObject::grouping_strategy(stem).map(GroupingObject::Strategy);
            Ok(ParseState::StateNull)
        }

        StemLatin => {
            check_null(macros.symbols.is_some(), segment)?;
            macros.symbols = numbering_system::LATIN.clone().map(SymbolsObject::NumberingSystem);
            Ok(ParseState::StateNull)
        }

        StemUnitWidthNarrow
        | StemUnitWidthShort
        | StemUnitWidthFullName
        | StemUnitWidthIsoCode
        | StemUnitWidthFormal
        | StemUnitWidthVariant
        | StemUnitWidthHidden => {
            check_null(macros.unit_width.is_some(), segment)?;
            macros.unit_width = StemToObject::unit_width(stem);
            Ok(ParseState::StateNull)
        }

        StemSignAuto
        | StemSignAlways
        | StemSignNever
        | StemSignAccounting
        | StemSignAccountingAlways
        | StemSignExceptZero
        | StemSignAccountingExceptZero
        | StemSignNegative
        | StemSignAccountingNegative => {
            check_null(macros.sign.is_some(), segment)?;
            macros.sign = StemToObject::sign_display(stem);
            Ok(ParseState::StateNull)
        }

        StemDecimalAuto | StemDecimalAlways => {
            check_null(macros.decimal.is_some(), segment)?;
            macros.decimal = StemToObject::decimal_separator_display(stem);
            Ok(ParseState::StateNull)
        }

        // Stems requiring an option:
        StemPrecisionIncrement => {
            check_null(macros.precision.is_some(), segment)?;
            Ok(ParseState::StateIncrementPrecision)
        }

        StemMeasureUnit => {
            check_null(macros.unit.is_some(), segment)?;
            Ok(ParseState::StateMeasureUnit)
        }

        StemPerMeasureUnit => {
            if macros.unit.as_ref().is_some_and(|u| Currency::try_cast(u).is_some()) {
                return Err(syntax("Duplicated setting", segment));
            }
            check_null(macros.per_unit.is_some(), segment)?;
            Ok(ParseState::StatePerMeasureUnit)
        }

        StemUnit => {
            check_null(macros.unit.is_some(), segment)?;
            check_null(macros.per_unit.is_some(), segment)?;
            Ok(ParseState::StateIdentifierUnit)
        }

        StemUnitUsage => {
            check_null(macros.usage.is_some(), segment)?;
            Ok(ParseState::StateUnitUsage)
        }

        StemCurrency => {
            check_null(macros.unit.is_some(), segment)?;
            check_null(macros.per_unit.is_some(), segment)?;
            Ok(ParseState::StateCurrencyUnit)
        }

        StemIntegerWidth => {
            check_null(macros.integer_width.is_some(), segment)?;
            Ok(ParseState::StateIntegerWidth)
        }

        StemNumberingSystem => {
            check_null(macros.symbols.is_some(), segment)?;
            Ok(ParseState::StateNumberingSystem)
        }

        StemScale => {
            check_null(macros.scale.is_some(), segment)?;
            Ok(ParseState::StateScale)
        }
    }
}

/// Given that the current segment represents an option, parse it and save
/// the result. Equivalent to `parseOption(ParseState, StringSegment,
/// MacroProps)`: the next state in the parser.
fn parse_option(stem: ParseState, segment: &StringSegment, macros: &mut MacroProps) -> Result<ParseState> {
    ///// Required options: /////

    match stem {
        ParseState::StateCurrencyUnit => {
            BlueprintHelpers::parse_currency_option(segment, macros)?;
            return Ok(ParseState::StateNull);
        }
        ParseState::StateMeasureUnit => {
            BlueprintHelpers::parse_measure_unit_option(segment, macros)?;
            return Ok(ParseState::StateNull);
        }
        ParseState::StatePerMeasureUnit => {
            BlueprintHelpers::parse_measure_per_unit_option(segment, macros)?;
            return Ok(ParseState::StateNull);
        }
        ParseState::StateIdentifierUnit => {
            BlueprintHelpers::parse_identifier_unit_option(segment, macros)?;
            return Ok(ParseState::StateNull);
        }
        ParseState::StateUnitUsage => {
            BlueprintHelpers::parse_unit_usage_option(segment, macros);
            return Ok(ParseState::StateNull);
        }
        ParseState::StateIncrementPrecision => {
            BlueprintHelpers::parse_increment_option(segment, macros)?;
            return Ok(ParseState::StatePrecision);
        }
        ParseState::StateIntegerWidth => {
            BlueprintHelpers::parse_integer_width_option(segment, macros)?;
            return Ok(ParseState::StateNull);
        }
        ParseState::StateNumberingSystem => {
            BlueprintHelpers::parse_numbering_system_option(segment, macros)?;
            return Ok(ParseState::StateNull);
        }
        ParseState::StateScale => {
            BlueprintHelpers::parse_scale_option(segment, macros)?;
            return Ok(ParseState::StateNull);
        }
        _ => {}
    }

    ///// Non-required options: /////

    // Scientific options
    if stem == ParseState::StateScientific {
        if BlueprintHelpers::parse_exponent_width_option(segment, macros)? {
            return Ok(ParseState::StateScientific);
        }
        if BlueprintHelpers::parse_exponent_sign_option(segment, macros) {
            return Ok(ParseState::StateScientific);
        }
    }

    // Frac-sig option
    let mut stem = stem;
    if stem == ParseState::StateFractionPrecision {
        if BlueprintHelpers::parse_frac_sig_option(segment, macros)? {
            return Ok(ParseState::StatePrecision);
        }
        // If the fraction-sig option was not found, try normal precision
        // options.
        stem = ParseState::StatePrecision;
    }

    // Trailing zeros option
    if stem == ParseState::StatePrecision && BlueprintHelpers::parse_trailing_zero_option(segment, macros) {
        return Ok(ParseState::StateNull);
    }

    // Unknown option
    Err(syntax("Invalid option", segment))
}

/////

/// Main skeleton generator function. Appends the normalized skeleton for
/// the MacroProps to the given StringBuilder. Equivalent to
/// `generateSkeleton(MacroProps, StringBuilder)`.
fn generate_skeleton(macros: &MacroProps, sb: &mut Vec<u16>) -> Result<()> {
    // Supported options
    if macros.notation.is_some() && GeneratorHelpers::notation(macros, sb)? {
        sb.push(' ' as u16);
    }
    if macros.unit.is_some() && GeneratorHelpers::unit(macros, sb)? {
        sb.push(' ' as u16);
    }
    if macros.usage.is_some() && GeneratorHelpers::usage(macros, sb) {
        sb.push(' ' as u16);
    }
    if macros.precision.is_some() && GeneratorHelpers::precision(macros, sb) {
        sb.push(' ' as u16);
    }
    if macros.rounding_mode.is_some() && GeneratorHelpers::rounding_mode(macros, sb) {
        sb.push(' ' as u16);
    }
    if macros.grouping.is_some() && GeneratorHelpers::grouping(macros, sb)? {
        sb.push(' ' as u16);
    }
    if macros.integer_width.is_some() && GeneratorHelpers::integer_width(macros, sb) {
        sb.push(' ' as u16);
    }
    if macros.symbols.is_some() && GeneratorHelpers::symbols(macros, sb)? {
        sb.push(' ' as u16);
    }
    if macros.unit_width.is_some() && GeneratorHelpers::unit_width(macros, sb) {
        sb.push(' ' as u16);
    }
    if macros.sign.is_some() && GeneratorHelpers::sign(macros, sb) {
        sb.push(' ' as u16);
    }
    if macros.decimal.is_some() && GeneratorHelpers::decimal(macros, sb) {
        sb.push(' ' as u16);
    }
    if macros.scale.is_some() && GeneratorHelpers::scale(macros, sb)? {
        sb.push(' ' as u16);
    }

    // Unsupported options
    if macros.padder.is_some() {
        return Err(LuceneError::unsupported_operation("Cannot generate number skeleton with custom padder"));
    }
    if macros.unit_display_case.as_deref().is_some_and(|c| !c.is_empty()) {
        return Err(LuceneError::unsupported_operation(
            "Cannot generate number skeleton with custom unit display case",
        ));
    }
    if macros.affix_provider.is_some() {
        return Err(LuceneError::unsupported_operation("Cannot generate number skeleton with custom affix provider"));
    }
    if macros.rules.is_some() {
        return Err(LuceneError::unsupported_operation("Cannot generate number skeleton with custom plural rules"));
    }

    // Remove the trailing space
    if !sb.is_empty() {
        sb.pop();
    }
    Ok(())
}

/// Utility class for methods for processing stems and options that cannot
/// be interpreted literally. Equivalent to
/// `NumberSkeletonImpl.BlueprintHelpers`.
struct BlueprintHelpers;

impl BlueprintHelpers {
    /// `parseExponentWidthOption(StringSegment, MacroProps)`.
    fn parse_exponent_width_option(segment: &StringSegment, macros: &mut MacroProps) -> Result<bool> {
        if !is_wildcard_char(segment.char_at(0)) {
            return Ok(false);
        }
        let mut offset = 1;
        let mut min_exp = 0;
        while offset < segment.length() {
            if segment.char_at(offset) == 'e' as u16 {
                min_exp += 1;
            } else {
                break;
            }
            offset += 1;
        }
        if offset < segment.length() {
            return Ok(false);
        }
        // Use the public APIs to enforce bounds checking
        let notation = ScientificNotation::cast(macros.notation.clone().expect("a scientific notation was parsed"));
        macros.notation = Some(notation.with_min_exponent_digits(min_exp)?.into_notation());
        Ok(true)
    }

    /// `generateExponentWidthOption(int, StringBuilder)`.
    fn generate_exponent_width_option(min_exponent_digits: i32, sb: &mut Vec<u16>) {
        sb.push(WILDCARD_CHAR);
        append_multiple(sb, 'e' as i32, min_exponent_digits);
    }

    /// `parseExponentSignOption(StringSegment, MacroProps)`.
    fn parse_exponent_sign_option(segment: &StringSegment, macros: &mut MacroProps) -> bool {
        // Get the sign display type out of the CharsTrie data structure.
        // TODO: Make this more efficient (avoid object allocation)? It
        // shouldn't be very hot code.
        let mut temp_stem_trie = CharsTrie::new(SERIALIZED_STEM_TRIE.clone(), 0);
        let s = segment.to_java_string();
        let result = temp_stem_trie.next_chars(&s, 0, segment.length());
        if result != TrieResult::IntermediateValue && result != TrieResult::FinalValue {
            return false;
        }
        let sign = match StemToObject::sign_display(STEM_ENUM_VALUES[temp_stem_trie.get_value() as usize]) {
            None => return false,
            Some(s) => s,
        };
        let notation = ScientificNotation::cast(macros.notation.clone().expect("a scientific notation was parsed"));
        macros.notation = Some(notation.with_exponent_sign_display(sign).into_notation());
        true
    }

    /// `parseCurrencyOption(StringSegment, MacroProps)`.
    fn parse_currency_option(segment: &StringSegment, macros: &mut MacroProps) -> Result<()> {
        let currency_code = to_rust_string(&segment.sub_sequence(0, segment.length()));
        let currency = match Currency::get_instance(Some(&currency_code)) {
            Ok(c) => c,
            Err(e) if e.is_illegal_argument_exception() => {
                // Not 3 ascii chars
                return Err(new_skeleton_with_cause("Invalid currency", &segment.to_java_string(), e));
            }
            Err(e) => return Err(e),
        };
        macros.unit = Some(currency.into_measure_unit());
        Ok(())
    }

    /// `generateCurrencyOption(Currency, StringBuilder)`.
    fn generate_currency_option(currency: &Currency, sb: &mut Vec<u16>) {
        push(sb, &currency.get_currency_code());
    }

    /// `parseMeasureUnitOption(StringSegment, MacroProps)`.
    fn parse_measure_unit_option(segment: &StringSegment, macros: &mut MacroProps) -> Result<()> {
        // NOTE: The category (type) of the unit is guaranteed to be a valid
        // subtag (alphanumeric) So we split on the first hyphen.
        let mut first_hyphen = 0;
        while first_hyphen < segment.length() && segment.char_at(first_hyphen) != '-' as u16 {
            first_hyphen += 1;
        }
        if first_hyphen == segment.length() {
            return Err(syntax("Invalid measure unit option", segment));
        }
        let type_ = to_rust_string(&segment.sub_sequence(0, first_hyphen));
        let sub_type = to_rust_string(&segment.sub_sequence(first_hyphen + 1, segment.length()));
        let units = MeasureUnit::get_available_of_type(&type_)?;
        for unit in units {
            if unit.get_subtype() == Some(sub_type.as_str()) {
                macros.unit = Some(unit);
                return Ok(());
            }
        }
        Err(syntax("Unknown measure unit", segment))
    }

    /// `parseMeasurePerUnitOption(StringSegment, MacroProps)`.
    fn parse_measure_per_unit_option(segment: &StringSegment, macros: &mut MacroProps) -> Result<()> {
        // A little bit of a hack: save the current unit (numerator), call
        // the main measure unit parsing code, put back the numerator unit,
        // and put the new unit into per-unit.
        let numerator = macros.unit.take();
        Self::parse_measure_unit_option(segment, macros)?;
        macros.per_unit = macros.unit.take();
        macros.unit = numerator;
        Ok(())
    }

    /// Parses unit identifiers like "meter-per-second" and
    /// "foot-and-inch", as specified via a "unit/" concise skeleton.
    /// Equivalent to `parseIdentifierUnitOption(StringSegment, MacroProps)`.
    fn parse_identifier_unit_option(segment: &StringSegment, macros: &mut MacroProps) -> Result<()> {
        match MeasureUnit::for_identifier(Some(&to_rust_string(&segment.as_string()))) {
            Ok(u) => {
                macros.unit = u;
                Ok(())
            }
            Err(e) if e.is_illegal_argument_exception() => Err(syntax("Invalid unit stem", segment)),
            Err(e) => Err(e),
        }
    }

    /// `parseUnitUsageOption(StringSegment, MacroProps)`.
    fn parse_unit_usage_option(segment: &StringSegment, macros: &mut MacroProps) {
        macros.usage = Some(to_rust_string(&segment.as_string()));
        // We do not do any validation of the usage string: it depends on
        // the unitPreferenceData in the units resources.
    }

    /// `parseFractionStem(StringSegment, MacroProps)`.
    fn parse_fraction_stem(segment: &StringSegment, macros: &mut MacroProps) -> Result<()> {
        debug_assert!(segment.char_at(0) == '.' as u16);
        let mut offset = 1;
        let mut min_frac = 0;
        let max_frac;
        while offset < segment.length() {
            if segment.char_at(offset) == '0' as u16 {
                min_frac += 1;
            } else {
                break;
            }
            offset += 1;
        }
        if offset < segment.length() {
            if is_wildcard_char(segment.char_at(offset)) {
                max_frac = -1;
                offset += 1;
            } else {
                let mut mf = min_frac;
                while offset < segment.length() {
                    if segment.char_at(offset) == '#' as u16 {
                        mf += 1;
                    } else {
                        break;
                    }
                    offset += 1;
                }
                max_frac = mf;
            }
        } else {
            max_frac = min_frac;
        }
        if offset < segment.length() {
            return Err(syntax("Invalid fraction stem", segment));
        }
        // Use the public APIs to enforce bounds checking
        if max_frac == -1 {
            if min_frac == 0 {
                macros.precision = Some(Precision::unlimited());
            } else {
                macros.precision = Some(Precision::min_fraction(min_frac)?.into_precision());
            }
        } else {
            macros.precision = Some(Precision::min_max_fraction(min_frac, max_frac)?.into_precision());
        }
        Ok(())
    }

    /// `generateFractionStem(int, int, StringBuilder)`.
    fn generate_fraction_stem(min_frac: i32, max_frac: i32, sb: &mut Vec<u16>) {
        if min_frac == 0 && max_frac == 0 {
            push(sb, "precision-integer");
            return;
        }
        sb.push('.' as u16);
        append_multiple(sb, '0' as i32, min_frac);
        if max_frac == -1 {
            sb.push(WILDCARD_CHAR);
        } else {
            append_multiple(sb, '#' as i32, max_frac - min_frac);
        }
    }

    /// `parseDigitsStem(StringSegment, MacroProps)`.
    fn parse_digits_stem(segment: &StringSegment, macros: &mut MacroProps) -> Result<()> {
        debug_assert!(segment.char_at(0) == '@' as u16);
        let (offset, min_sig, max_sig) = parse_sig_digits(segment);
        if offset < segment.length() {
            return Err(syntax("Invalid significant digits stem", segment));
        }
        // Use the public APIs to enforce bounds checking
        if max_sig == -1 {
            macros.precision = Some(Precision::min_significant_digits(min_sig)?);
        } else {
            macros.precision = Some(Precision::min_max_significant_digits(min_sig, max_sig)?);
        }
        Ok(())
    }

    /// `generateDigitsStem(int, int, StringBuilder)`.
    fn generate_digits_stem(min_sig: i32, max_sig: i32, sb: &mut Vec<u16>) {
        append_multiple(sb, '@' as i32, min_sig);
        if max_sig == -1 {
            sb.push(WILDCARD_CHAR);
        } else {
            append_multiple(sb, '#' as i32, max_sig - min_sig);
        }
    }

    /// `parseScientificStem(StringSegment, MacroProps)`.
    fn parse_scientific_stem(segment: &StringSegment, macros: &mut MacroProps) -> Result<()> {
        debug_assert!(segment.char_at(0) == 'E' as u16);
        // `block:` with `break block` in Java: any early exit lands on the
        // "Invalid scientific stem" error.
        let parsed = (|| -> Result<Option<Notation>> {
            let mut offset = 1;
            if segment.length() == offset {
                return Ok(None);
            }
            let mut is_engineering = false;
            if segment.char_at(offset) == 'E' as u16 {
                is_engineering = true;
                offset += 1;
                if segment.length() == offset {
                    return Ok(None);
                }
            }
            let mut sign_display = SignDisplay::Auto;
            if segment.char_at(offset) == '+' as u16 {
                offset += 1;
                if segment.length() == offset {
                    return Ok(None);
                }
                if segment.char_at(offset) == '!' as u16 {
                    sign_display = SignDisplay::Always;
                } else if segment.char_at(offset) == '?' as u16 {
                    sign_display = SignDisplay::ExceptZero;
                } else {
                    return Ok(None);
                }
                offset += 1;
                if segment.length() == offset {
                    return Ok(None);
                }
            }
            let mut min_digits = 0;
            while offset < segment.length() {
                if segment.char_at(offset) != '0' as u16 {
                    return Ok(None);
                }
                min_digits += 1;
                offset += 1;
            }
            let base = if is_engineering { Notation::engineering() } else { Notation::scientific() };
            Ok(Some(base.with_exponent_sign_display(sign_display).with_min_exponent_digits(min_digits)?.into_notation()))
        })()?;
        match parsed {
            Some(n) => {
                macros.notation = Some(n);
                Ok(())
            }
            None => Err(syntax("Invalid scientific stem", segment)),
        }
    }

    /// `parseIntegerStem(StringSegment, MacroProps)`.
    fn parse_integer_stem(segment: &StringSegment, macros: &mut MacroProps) -> Result<()> {
        debug_assert!(segment.char_at(0) == '0' as u16);
        let mut offset = 1;
        while offset < segment.length() {
            if segment.char_at(offset) != '0' as u16 {
                offset -= 1;
                break;
            }
            offset += 1;
        }
        if offset < segment.length() {
            return Err(syntax("Invalid integer stem", segment));
        }
        macros.integer_width = Some(IntegerWidth::zero_fill_to(offset)?);
        Ok(())
    }

    /// `parseFracSigOption(StringSegment, MacroProps)`: true if the option
    /// was consumed.
    fn parse_frac_sig_option(segment: &StringSegment, macros: &mut MacroProps) -> Result<bool> {
        if segment.char_at(0) != '@' as u16 {
            return Ok(false);
        }
        let (mut offset, min_sig, max_sig) = parse_sig_digits(segment);
        let old_rounder = FractionPrecision::cast(macros.precision.clone().expect("a fraction precision was parsed"));
        if offset < segment.length() {
            if max_sig == -1 {
                return Err(syntax(
                    "Invalid digits option: Wildcard character not allowed with the priority annotation",
                    segment,
                ));
            }
            let priority;
            if segment.code_point_at(offset) == 'r' as i32 {
                priority = RoundingPriority::Relaxed;
                offset += 1;
            } else if segment.code_point_at(offset) == 's' as i32 {
                priority = RoundingPriority::Strict;
                offset += 1;
            } else {
                debug_assert!(offset < segment.length());
                priority = RoundingPriority::Relaxed;
            }
            if offset < segment.length() {
                // Invalid digits option for fraction rounder
                return Err(syntax("Invalid digits option for fraction rounder", segment));
            }
            macros.precision = Some(old_rounder.with_significant_digits(min_sig, max_sig, priority)?);
        } else if max_sig == -1 {
            // withMinDigits
            macros.precision = Some(old_rounder.with_min_digits(min_sig)?);
        } else if min_sig == 1 {
            // withMaxDigits
            macros.precision = Some(old_rounder.with_max_digits(max_sig)?);
        } else {
            return Err(syntax("Invalid digits option: Priority annotation required", segment));
        }

        Ok(true)
    }

    /// `parseTrailingZeroOption(StringSegment, MacroProps)`.
    fn parse_trailing_zero_option(segment: &StringSegment, macros: &mut MacroProps) -> bool {
        if segment.content_equals(Some(&units("w"))) {
            let p = macros.precision.as_ref().unwrap_or_else(|| panic!("java.lang.NullPointerException: precision"));
            macros.precision = Some(p.trailing_zero_display(Some(TrailingZeroDisplay::HideIfWhole)));
            return true;
        }
        false
    }

    /// `parseIncrementOption(StringSegment, MacroProps)`.
    fn parse_increment_option(segment: &StringSegment, macros: &mut MacroProps) -> Result<()> {
        // Call segment.subSequence() because segment.toString() doesn't
        // create a clean string.
        let s = segment.sub_sequence(0, segment.length());
        let increment = match BigDecimal::from_string(&s) {
            Ok(b) => b,
            Err(e) if e.is_number_format_exception() => {
                return Err(new_skeleton_with_cause("Invalid rounding increment", &segment.to_java_string(), e));
            }
            Err(e) => return Err(e),
        };
        // Use the public APIs to enforce bounds checking
        macros.precision = Some(Precision::increment(Some(&increment))?);
        Ok(())
    }

    /// `generateIncrementOption(BigDecimal, StringBuilder)`.
    fn generate_increment_option(increment: &BigDecimal, sb: &mut Vec<u16>) {
        push(sb, &increment.to_plain_string());
    }

    /// `parseIntegerWidthOption(StringSegment, MacroProps)`.
    fn parse_integer_width_option(segment: &StringSegment, macros: &mut MacroProps) -> Result<()> {
        let mut offset = 0;
        let mut min_int = 0;
        let mut max_int;
        if is_wildcard_char(segment.char_at(0)) {
            max_int = -1;
            offset += 1;
        } else {
            max_int = 0;
        }
        while offset < segment.length() {
            if max_int != -1 && segment.char_at(offset) == '#' as u16 {
                max_int += 1;
            } else {
                break;
            }
            offset += 1;
        }
        if offset < segment.length() {
            while offset < segment.length() {
                if segment.char_at(offset) == '0' as u16 {
                    min_int += 1;
                } else {
                    break;
                }
                offset += 1;
            }
        }
        if max_int != -1 {
            max_int += min_int;
        }
        if offset < segment.length() {
            return Err(syntax("Invalid integer width stem", segment));
        }
        // Use the public APIs to enforce bounds checking
        if max_int == -1 {
            macros.integer_width = Some(IntegerWidth::zero_fill_to(min_int)?);
        } else {
            macros.integer_width = Some(IntegerWidth::zero_fill_to(min_int)?.truncate_at(max_int)?);
        }
        Ok(())
    }

    /// `generateIntegerWidthOption(int, int, StringBuilder)`.
    fn generate_integer_width_option(min_int: i32, max_int: i32, sb: &mut Vec<u16>) {
        if max_int == -1 {
            sb.push(WILDCARD_CHAR);
        } else {
            append_multiple(sb, '#' as i32, max_int - min_int);
        }
        append_multiple(sb, '0' as i32, min_int);
    }

    /// `parseNumberingSystemOption(StringSegment, MacroProps)`.
    fn parse_numbering_system_option(segment: &StringSegment, macros: &mut MacroProps) -> Result<()> {
        let ns_name = to_rust_string(&segment.sub_sequence(0, segment.length()));
        match NumberingSystem::get_instance_by_name(&ns_name)? {
            None => Err(syntax("Unknown numbering system", segment)),
            Some(ns) => {
                macros.symbols = Some(SymbolsObject::NumberingSystem(ns));
                Ok(())
            }
        }
    }

    /// `generateNumberingSystemOption(NumberingSystem, StringBuilder)`.
    fn generate_numbering_system_option(ns: &NumberingSystem, sb: &mut Vec<u16>) {
        push(sb, ns.get_name().unwrap_or("null"));
    }

    /// `parseScaleOption(StringSegment, MacroProps)`.
    fn parse_scale_option(segment: &StringSegment, macros: &mut MacroProps) -> Result<()> {
        // Call segment.subSequence() because segment.toString() doesn't
        // create a clean string.
        let s = segment.sub_sequence(0, segment.length());
        let bd = match BigDecimal::from_string(&s) {
            Ok(b) => b,
            Err(e) if e.is_number_format_exception() => {
                return Err(new_skeleton_with_cause("Invalid scale", &segment.to_java_string(), e));
            }
            Err(e) => return Err(e),
        };
        // NOTE: If bd is a power of ten, the Scale API optimizes it for us.
        macros.scale = Some(Scale::by_big_decimal(&bd)?);
        Ok(())
    }

    /// `generateScaleOption(Scale, StringBuilder)`.
    fn generate_scale_option(scale: &Scale, sb: &mut Vec<u16>) -> Result<()> {
        let bd = scale.arbitrary().cloned().unwrap_or_else(BigDecimal::one);
        let bd = bd.scale_by_power_of_ten(scale.magnitude())?;
        // We do not need to check for negative scales here (we checked
        // already).
        push(sb, &bd.to_plain_string());
        Ok(())
    }
}

/// The digit counting loop shared by `parseDigitsStem` and
/// `parseFracSigOption`: the offset reached, the minimum and the maximum
/// (-1 for a wildcard) significant digits.
fn parse_sig_digits(segment: &StringSegment) -> (i32, i32, i32) {
    let mut offset = 0;
    let mut min_sig = 0;
    let max_sig;
    while offset < segment.length() {
        if segment.char_at(offset) == '@' as u16 {
            min_sig += 1;
        } else {
            break;
        }
        offset += 1;
    }
    if offset < segment.length() {
        if is_wildcard_char(segment.char_at(offset)) {
            max_sig = -1;
            offset += 1;
        } else {
            let mut ms = min_sig;
            while offset < segment.length() {
                if segment.char_at(offset) == '#' as u16 {
                    ms += 1;
                } else {
                    break;
                }
                offset += 1;
            }
            max_sig = ms;
        }
    } else {
        max_sig = min_sig;
    }
    (offset, min_sig, max_sig)
}

/// Class for methods for generating a token corresponding to each macro-prop.
/// Each method returns whether or not a token was written to the string
/// builder. Equivalent to `NumberSkeletonImpl.GeneratorHelpers`.
struct GeneratorHelpers;

impl GeneratorHelpers {
    fn notation(macros: &MacroProps, sb: &mut Vec<u16>) -> Result<bool> {
        let notation = macros.notation.as_ref().expect("checked by the caller");
        if CompactNotation::try_cast(notation).is_some() {
            if notation.ptr_eq(&Notation::compact_long()) {
                push(sb, "compact-long");
                Ok(true)
            } else if notation.ptr_eq(&Notation::compact_short()) {
                push(sb, "compact-short");
                Ok(true)
            } else {
                // Compact notation generated from custom data (not
                // supported in skeleton) The other compact notations are
                // literals
                Err(LuceneError::unsupported_operation("Cannot generate number skeleton with custom compact data"))
            }
        } else if let Some(sci) = ScientificNotation::try_cast(notation) {
            let impl_ = sci.data();
            if impl_.engineering_interval == 3 {
                push(sb, "engineering");
            } else {
                push(sb, "scientific");
            }
            if impl_.min_exponent_digits > 1 {
                sb.push('/' as u16);
                BlueprintHelpers::generate_exponent_width_option(impl_.min_exponent_digits, sb);
            }
            if impl_.exponent_sign_display != SignDisplay::Auto {
                sb.push('/' as u16);
                EnumToStemString::sign_display(impl_.exponent_sign_display, sb);
            }
            Ok(true)
        } else {
            debug_assert!(SimpleNotation::try_cast(notation).is_some());
            // Default value is not shown in normalized form
            Ok(false)
        }
    }

    fn unit(macros: &MacroProps, sb: &mut Vec<u16>) -> Result<bool> {
        let mut unit = macros.unit.clone();
        if let Some(per_unit) = &macros.per_unit {
            let is_currency = |u: &MeasureUnit| Currency::try_cast(u).is_some();
            if macros.unit.as_ref().is_some_and(is_currency) || is_currency(per_unit) {
                return Err(LuceneError::unsupported_operation(
                    "Cannot generate number skeleton with currency unit and per-unit",
                ));
            }
            let u = unit.as_ref().ok_or_else(|| crate::jdk::null_pointer("unit"))?;
            unit = Some(u.product(Some(&per_unit.reciprocal()?))?);
        }

        let unit = unit.ok_or_else(|| crate::jdk::null_pointer("unit"))?;
        if let Some(currency) = Currency::try_cast(&unit) {
            push(sb, "currency/");
            BlueprintHelpers::generate_currency_option(&currency, sb);
            Ok(true)
        } else if unit.java_equals(&measure_unit::PERCENT) {
            push(sb, "percent");
            Ok(true)
        } else if unit.java_equals(&measure_unit::PERMILLE) {
            push(sb, "permille");
            Ok(true)
        } else {
            push(sb, "unit/");
            push(sb, &unit.get_identifier());
            Ok(true)
        }
    }

    fn usage(macros: &MacroProps, sb: &mut Vec<u16>) -> bool {
        match macros.usage.as_deref() {
            Some(usage) if !usage.is_empty() => {
                push(sb, "usage/");
                push(sb, usage);
                true
            }
            _ => false,
        }
    }

    fn precision(macros: &MacroProps, sb: &mut Vec<u16>) -> bool {
        let precision = macros.precision.as_ref().expect("checked by the caller");
        match precision.kind() {
            PrecisionKind::Infinite => push(sb, "precision-unlimited"),
            PrecisionKind::Fraction { min_frac, max_frac } => {
                BlueprintHelpers::generate_fraction_stem(*min_frac, *max_frac, sb);
            }
            PrecisionKind::Significant { min_sig, max_sig } => {
                BlueprintHelpers::generate_digits_stem(*min_sig, *max_sig, sb);
            }
            PrecisionKind::FracSig { min_frac, max_frac, min_sig, max_sig, priority, retain } => {
                BlueprintHelpers::generate_fraction_stem(*min_frac, *max_frac, sb);
                sb.push('/' as u16);
                if *retain {
                    if *priority == RoundingPriority::Relaxed {
                        // withMinDigits
                        BlueprintHelpers::generate_digits_stem(*max_sig, -1, sb);
                    } else {
                        // withMaxDigits
                        BlueprintHelpers::generate_digits_stem(1, *max_sig, sb);
                    }
                } else {
                    BlueprintHelpers::generate_digits_stem(*min_sig, *max_sig, sb);
                    if *priority == RoundingPriority::Relaxed {
                        sb.push('r' as u16);
                    } else {
                        sb.push('s' as u16);
                    }
                }
            }
            PrecisionKind::Increment { increment }
            | PrecisionKind::IncrementOne { increment, .. }
            | PrecisionKind::IncrementFive { increment, .. } => {
                push(sb, "precision-increment/");
                BlueprintHelpers::generate_increment_option(increment, sb);
            }
            PrecisionKind::Currency { usage } => {
                if *usage == CurrencyUsage::Standard {
                    push(sb, "precision-currency-standard");
                } else {
                    push(sb, "precision-currency-cash");
                }
            }
            PrecisionKind::Bogus => {
                // Java asserts CurrencyRounderImpl and then casts:
                // ClassCastException.
                panic!(
                    "java.lang.ClassCastException: class com.ibm.icu.number.Precision$BogusRounder cannot be cast to class com.ibm.icu.number.Precision$CurrencyRounderImpl"
                );
            }
        }

        if precision.trailing_zero_display_value() == Some(TrailingZeroDisplay::HideIfWhole) {
            push(sb, "/w");
        }

        // NOTE: Always return true for rounding because the default value
        // depends on other options.
        true
    }

    fn rounding_mode(macros: &MacroProps, sb: &mut Vec<u16>) -> bool {
        let rm = macros.rounding_mode.expect("checked by the caller");
        if rm == RoundingUtils::DEFAULT_ROUNDING_MODE {
            return false; // Default value
        }
        EnumToStemString::rounding_mode(rm, sb);
        true
    }

    fn grouping(macros: &MacroProps, sb: &mut Vec<u16>) -> Result<bool> {
        match macros.grouping.as_ref().expect("checked by the caller") {
            GroupingObject::Strategy(s) => {
                if *s == GroupingStrategy::Auto {
                    return Ok(false); // Default value
                }
                EnumToStemString::grouping_strategy(*s, sb);
                Ok(true)
            }
            GroupingObject::Grouper(_) => {
                Err(LuceneError::unsupported_operation("Cannot generate number skeleton with custom Grouper"))
            }
        }
    }

    fn integer_width(macros: &MacroProps, sb: &mut Vec<u16>) -> bool {
        let iw = macros.integer_width.as_ref().expect("checked by the caller");
        if iw.ptr_eq(&integer_width::DEFAULT) {
            return false; // Default
        }
        if iw.min_int() == 0 && iw.max_int() == 0 {
            push(sb, "integer-width-trunc");
            return true;
        }
        push(sb, "integer-width/");
        BlueprintHelpers::generate_integer_width_option(iw.min_int(), iw.max_int(), sb);
        true
    }

    fn symbols(macros: &MacroProps, sb: &mut Vec<u16>) -> Result<bool> {
        match macros.symbols.as_ref().expect("checked by the caller") {
            SymbolsObject::NumberingSystem(ns) => {
                if ns.get_name() == Some("latn") {
                    push(sb, "latin");
                } else {
                    push(sb, "numbering-system/");
                    BlueprintHelpers::generate_numbering_system_option(ns, sb);
                }
                Ok(true)
            }
            SymbolsObject::Symbols(_) => Err(LuceneError::unsupported_operation(
                "Cannot generate number skeleton with custom DecimalFormatSymbols",
            )),
        }
    }

    fn unit_width(macros: &MacroProps, sb: &mut Vec<u16>) -> bool {
        let w = macros.unit_width.expect("checked by the caller");
        if w == UnitWidth::Short {
            return false; // Default value
        }
        EnumToStemString::unit_width(w, sb);
        true
    }

    fn sign(macros: &MacroProps, sb: &mut Vec<u16>) -> bool {
        let s = macros.sign.expect("checked by the caller");
        if s == SignDisplay::Auto {
            return false; // Default value
        }
        EnumToStemString::sign_display(s, sb);
        true
    }

    fn decimal(macros: &MacroProps, sb: &mut Vec<u16>) -> bool {
        let d = macros.decimal.expect("checked by the caller");
        if d == DecimalSeparatorDisplay::Auto {
            return false; // Default value
        }
        EnumToStemString::decimal_separator_display(d, sb);
        true
    }

    fn scale(macros: &MacroProps, sb: &mut Vec<u16>) -> Result<bool> {
        let scale = macros.scale.as_ref().expect("checked by the caller");
        if !scale.is_valid() {
            return Ok(false); // Default value
        }
        push(sb, "scale/");
        BlueprintHelpers::generate_scale_option(scale, sb)?;
        Ok(true)
    }
}

/////

/// `checkNull(Object, CharSequence)`: `SkeletonSyntaxException("Duplicated
/// setting", content)` when the setting is present.
fn check_null(present: bool, content: &StringSegment) -> Result<()> {
    if present {
        return Err(syntax("Duplicated setting", content));
    }
    Ok(())
}

/// `appendMultiple(StringBuilder, int, int)`.
fn append_multiple(sb: &mut Vec<u16>, cp: i32, count: i32) {
    for _ in 0..count {
        let _ = crate::jdk::append_code_point(sb, cp);
    }
}
