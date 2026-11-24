// © 2020 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.units.MeasureUnitImpl`.

use std::cmp::Ordering;
use std::fmt;
use std::sync::LazyLock;

use rucene::error::{LuceneError, Result};

use crate::jdk::math::BigDecimal;
use crate::jdk::units;
use crate::r#impl::units::conversion_rates::ConversionRates;
use crate::r#impl::units::single_unit_impl::SingleUnitImpl;
use crate::r#impl::units::units_data::{Constants, UnitsData};
use crate::util::bytes_trie::Result as TrieResult;
use crate::util::chars_trie::CharsTrie;
use crate::util::chars_trie_builder::CharsTrieBuilder;
use crate::util::measure_unit::{Complexity, MeasurePrefix, MeasureUnit};
use crate::util::string_trie_builder::StringTrieBuilderOption;

/// The internal representation of a unit: its single units, complexity and
/// constant denominator.
///
/// Equivalent to `com.ibm.icu.impl.units.MeasureUnitImpl`; `copy()` is
/// [`copy`](Self::copy) (and `Clone`), `toString()` is
/// [`to_java_string`](Self::to_java_string).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasureUnitImpl {
    /// `identifier`: the full unit identifier. `None` (Java `null`) if not
    /// computed.
    identifier: Option<String>,
    /// `complexity`: the complexity, either SINGLE, COMPOUND, or MIXED.
    complexity: Complexity,
    /// `constantDenominator`: the constant denominator, 0 if none.
    constant_denominator: i64,
    /// `singleUnits`: the list of single units. These may be summed or
    /// multiplied, based on the value of the complexity field.
    single_units: Vec<SingleUnitImpl>,
}

impl Default for MeasureUnitImpl {
    fn default() -> Self {
        MeasureUnitImpl {
            identifier: None,
            complexity: Complexity::Single,
            constant_denominator: 0,
            single_units: Vec::new(),
        }
    }
}

impl MeasureUnitImpl {
    /// Equivalent to `MeasureUnitImpl()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Equivalent to `MeasureUnitImpl(SingleUnitImpl)`.
    pub fn from_single_unit(single_unit: &SingleUnitImpl) -> Self {
        let mut result = Self::new();
        result.append_single_unit(Some(single_unit));
        result
    }

    /// Parse a unit identifier into a MeasureUnitImpl. Equivalent to
    /// `forIdentifier(String)`.
    ///
    /// **Divergence.** Java answers `null` for an empty identifier
    /// (`MeasureUnitImpl.java:63-65` via `UnitsParser.parseForIdentifier`)
    /// and every caller dereferences the result; the port raises that
    /// `NullPointerException` here.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a syntax error;
    /// `NullPointerException` for an empty identifier.
    pub fn for_identifier(identifier: &str) -> Result<MeasureUnitImpl> {
        UnitsParser::parse_for_identifier(Some(identifier))?
            .ok_or_else(|| crate::jdk::null_pointer("MeasureUnitImpl.forIdentifier: empty identifier"))
    }

    /// Used for currency units. Equivalent to `forCurrencyCode(String)`.
    pub fn for_currency_code(currency_code: &str) -> MeasureUnitImpl {
        MeasureUnitImpl { identifier: Some(currency_code.to_string()), ..Self::new() }
    }

    /// Create a copy of this MeasureUnitImpl. Equivalent to `copy()`.
    pub fn copy(&self) -> MeasureUnitImpl {
        self.clone()
    }

    /// Create a copy of this MeasureUnitImpl, merging the single units with
    /// the same simple unit and prefix. Equivalent to `copyAndSimplify()`.
    pub fn copy_and_simplify(&self) -> MeasureUnitImpl {
        let mut result = MeasureUnitImpl::new();
        for single_unit in self.get_single_units() {
            // This `for` loop will cause time complexity to be O(n^2).
            // However, n is very small (number of units, generally, at
            // maximum equal to 10)
            let mut unit_exist = false;
            for result_single_unit in result.single_units.iter_mut() {
                if result_single_unit.get_simple_unit_id() == single_unit.get_simple_unit_id()
                    && result_single_unit.get_prefix().get_identifier() == single_unit.get_prefix().get_identifier()
                {
                    unit_exist = true;
                    result_single_unit
                        .set_dimensionality(result_single_unit.get_dimensionality() + single_unit.get_dimensionality());
                    break;
                }
            }

            if !unit_exist {
                result.append_single_unit(Some(single_unit));
            }
        }

        result
    }

    /// Returns the list of simple units. Equivalent to `getSingleUnits()`.
    pub fn get_single_units(&self) -> &Vec<SingleUnitImpl> {
        &self.single_units
    }

    /// The mutable list Java's `getSingleUnits()` answers, for the callers
    /// that change its elements.
    pub fn get_single_units_mut(&mut self) -> &mut Vec<SingleUnitImpl> {
        &mut self.single_units
    }

    /// Mutates this MeasureUnitImpl to take the reciprocal. Equivalent to
    /// `takeReciprocal()`.
    pub fn take_reciprocal(&mut self) {
        self.identifier = None;
        for single_unit in self.single_units.iter_mut() {
            single_unit.set_dimensionality(-single_unit.get_dimensionality());
        }
    }

    /// Extracts the list of all the individual units inside the
    /// `MeasureUnitImpl` with their indices. Equivalent to
    /// `extractIndividualUnitsWithIndices()`.
    pub fn extract_individual_units_with_indices(&self) -> Vec<MeasureUnitImplWithIndex> {
        let mut result = Vec::new();
        if self.get_complexity() == Complexity::Mixed {
            // In case of mixed units, each single unit can be considered as
            // a stand alone MeasureUnitImpl.
            for (i, single_unit) in self.get_single_units().iter().enumerate() {
                result.push(MeasureUnitImplWithIndex::new(i as i32, MeasureUnitImpl::from_single_unit(single_unit)));
            }
            return result;
        }

        result.push(MeasureUnitImplWithIndex::new(0, self.copy()));
        result
    }

    /// Applies dimensionality to all the internal single units. Equivalent
    /// to `applyDimensionality(int)`.
    pub fn apply_dimensionality(&mut self, dimensionality: i32) {
        for single_unit in self.single_units.iter_mut() {
            single_unit.set_dimensionality(single_unit.get_dimensionality() * dimensionality);
        }
    }

    /// Mutates this MeasureUnitImpl to append a single unit. Equivalent to
    /// `appendSingleUnit(SingleUnitImpl)` (`None` for Java's `null`):
    /// whether the single unit was added as a new unit (not merged).
    pub fn append_single_unit(&mut self, single_unit: Option<&SingleUnitImpl>) -> bool {
        self.identifier = None;

        let single_unit = match single_unit {
            // Do not append dimensionless units.
            None => return false,
            Some(s) => s,
        };

        // Find a similar unit that already exists, to attempt to coalesce
        let old_unit = self.single_units.iter_mut().find(|candidate| candidate.is_compatible_with(single_unit));

        if let Some(old_unit) = old_unit {
            // Both dimensionalities will be positive, or both will be
            // negative, by virtue of isCompatibleWith().
            old_unit.set_dimensionality(old_unit.get_dimensionality() + single_unit.get_dimensionality());
            return false;
        }

        // Add a copy of singleUnit
        self.single_units.push(single_unit.copy());

        // If the MeasureUnitImpl is `UMEASURE_UNIT_SINGLE` and after the
        // appending a unit, the singleUnits contains more than one. thus
        // means the complexity should be `UMEASURE_UNIT_COMPOUND`
        if self.single_units.len() > 1 && self.complexity == Complexity::Single {
            self.set_complexity(Complexity::Compound);
        }

        true
    }

    /// Transform this MeasureUnitImpl into a MeasureUnit, simplifying if
    /// possible. Equivalent to `build()`.
    ///
    /// # Errors
    ///
    /// The errors of the identifier serialization and the unit lookup.
    pub fn build(&mut self) -> Result<MeasureUnit> {
        MeasureUnit::from_measure_unit_impl(self)
    }

    /// Returns the single unit, or the dimensionless unit when there is
    /// none. Equivalent to `getSingleUnitImpl()`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when there is more than one single
    /// unit.
    pub fn get_single_unit_impl(&self) -> Result<SingleUnitImpl> {
        if self.single_units.is_empty() {
            return Ok(SingleUnitImpl::new());
        }
        if self.single_units.len() == 1 {
            return Ok(self.single_units[0].copy());
        }

        Err(crate::jdk::unchecked_no_message("java.lang.UnsupportedOperationException"))
    }

    /// Returns the CLDR Unit Identifier, `None` (Java `null`) if not
    /// computed. Equivalent to `getIdentifier()`.
    pub fn get_identifier(&self) -> Option<String> {
        self.identifier.clone()
    }

    /// Equivalent to `getComplexity()`.
    pub fn get_complexity(&self) -> Complexity {
        self.complexity
    }

    /// Equivalent to `setComplexity(Complexity)`.
    pub fn set_complexity(&mut self, complexity: Complexity) {
        self.complexity = complexity;
    }

    /// Equivalent to `getConstantDenominator()`.
    pub fn get_constant_denominator(&self) -> i64 {
        self.constant_denominator
    }

    /// Equivalent to `setConstantDenominator(long)`.
    pub fn set_constant_denominator(&mut self, constant_denominator: i64) {
        self.constant_denominator = constant_denominator;
    }

    /// `countCharacter(String, char)`.
    fn count_character(s: &str, ch: char) -> i32 {
        s.chars().filter(|&c| c == ch).count() as i32
    }

    /// Internal function that returns a string of the constants in the
    /// correct format: "1e3" for 1000, "1000" otherwise. Equivalent to
    /// `getConstantsString(long)`.
    fn get_constants_string(constant_denominator: i64) -> String {
        debug_assert!(constant_denominator > 0);

        let result = constant_denominator.to_string();
        if constant_denominator <= 1000 {
            return result;
        }

        let zero_count = Self::count_character(&result, '0');
        if zero_count == result.len() as i32 - 1 && result.starts_with('1') {
            return format!("1e{zero_count}");
        }

        result
    }

    /// Normalizes the MeasureUnitImpl and generates the identifier string
    /// in place. Equivalent to `serialize()`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `getNeutralIdentifier` for a power
    /// above 15.
    pub fn serialize(&mut self) -> Result<()> {
        if self.get_single_units().is_empty() && self.constant_denominator == 0 {
            // Dimensionless, constructed by the default constructor: no
            // appending to this.result, we wish it to contain the zero-length
            // string.
            return Ok(());
        }

        if self.complexity == Complexity::Compound {
            // Note: don't sort a MIXED unit
            self.single_units.sort_by(SingleUnitComparator::compare);
        }

        let mut result = String::new();
        let mut before_per = true;
        let mut first_time_negative_dimension = false;
        let mut is_constant_denominator_added = false;
        for single_unit in &self.single_units {
            if before_per && single_unit.get_dimensionality() < 0 {
                before_per = false;
                first_time_negative_dimension = true;
            } else if single_unit.get_dimensionality() < 0 {
                first_time_negative_dimension = false;
            }

            if first_time_negative_dimension && self.constant_denominator > 0 {
                result.push_str("-per-");
                result.push_str(&Self::get_constants_string(self.constant_denominator));
                first_time_negative_dimension = false;
                is_constant_denominator_added = true;
            }

            if self.complexity == Complexity::Mixed {
                if !result.is_empty() {
                    result.push_str("-and-");
                }
            } else if first_time_negative_dimension {
                if result.is_empty() {
                    result.push_str("per-");
                } else {
                    result.push_str("-per-");
                }
            } else if !result.is_empty() {
                result.push('-');
            }

            result.push_str(&single_unit.get_neutral_identifier()?);
        }

        if self.constant_denominator > 0 && !is_constant_denominator_added {
            result.push_str("-per-");
            result.push_str(&Self::get_constants_string(self.constant_denominator));
        }

        self.identifier = Some(result);
        Ok(())
    }

    /// Equivalent to `toString()`: `"MeasureUnitImpl [" +
    /// build().getIdentifier() + "]"` (on a copy: `build()` serializes in
    /// place).
    ///
    /// # Errors
    ///
    /// Those of [`build`](Self::build).
    pub fn to_java_string(&self) -> Result<String> {
        let mut copy = self.clone();
        Ok(format!("MeasureUnitImpl [{}]", copy.build()?.get_identifier()))
    }
}

impl fmt::Display for MeasureUnitImpl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.to_java_string() {
            Ok(s) => f.write_str(&s),
            Err(_) => Err(fmt::Error),
        }
    }
}

/// Equivalent to `MeasureUnitImpl.CompoundPart`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompoundPart {
    /// Represents "-per-"
    Per,
    /// Represents "-"
    Times,
    /// Represents "-and-"
    And,
}

impl CompoundPart {
    /// Equivalent to `getCompoundPartFromTrieIndex(int)`.
    ///
    /// # Panics
    ///
    /// Java's `AssertionError("CompoundPart index must be 0, 1 or 2")`.
    pub fn get_compound_part_from_trie_index(trie_index: i32) -> CompoundPart {
        let index = trie_index - Constants::K_COMPOUND_PART_OFFSET;
        match index {
            0 => CompoundPart::Per,
            1 => CompoundPart::Times,
            2 => CompoundPart::And,
            _ => panic!("java.lang.AssertionError: CompoundPart index must be 0, 1 or 2"),
        }
    }

    /// Equivalent to `getTrieIndex()`.
    pub fn get_trie_index(self) -> i32 {
        self.get_value() + Constants::K_COMPOUND_PART_OFFSET
    }

    /// Equivalent to `getValue()`.
    pub fn get_value(self) -> i32 {
        self as i32
    }
}

/// Equivalent to `MeasureUnitImpl.PowerPart` (its constants `P2`..`P15` are
/// the powers 2 to 15).
pub struct PowerPart;

impl PowerPart {
    /// `getPowerFromTrieIndex(int)`.
    pub fn get_power_from_trie_index(trie_index: i32) -> i32 {
        trie_index - Constants::K_POWER_PART_OFFSET
    }

    /// `getTrieIndex()` of the constant with the given power.
    pub fn get_trie_index(power: i32) -> i32 {
        power + Constants::K_POWER_PART_OFFSET
    }
}

/// Equivalent to `MeasureUnitImpl.InitialCompoundPart`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitialCompoundPart {
    /// `INITIAL_COMPOUND_PART_PER`: represents "per-", the only compound
    /// part that can come first.
    InitialCompoundPartPer,
}

impl InitialCompoundPart {
    /// Equivalent to `getInitialCompoundPartFromTrieIndex(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Incorrect trieIndex")`.
    pub fn get_initial_compound_part_from_trie_index(trie_index: i32) -> Result<InitialCompoundPart> {
        let index = trie_index - Constants::K_INITIAL_COMPOUND_PART_OFFSET;
        if index == 0 {
            return Ok(InitialCompoundPart::InitialCompoundPartPer);
        }
        Err(LuceneError::illegal_argument("Incorrect trieIndex"))
    }

    /// Equivalent to `getTrieIndex()`.
    pub fn get_trie_index(self) -> i32 {
        self.get_value() + Constants::K_INITIAL_COMPOUND_PART_OFFSET
    }

    /// Equivalent to `getValue()`.
    pub fn get_value(self) -> i32 {
        0
    }
}

/// Equivalent to `MeasureUnitImpl.MeasureUnitImplWithIndex`.
#[derive(Debug, Clone)]
pub struct MeasureUnitImplWithIndex {
    /// `index`.
    pub(crate) index: i32,
    /// `unitImpl`.
    pub(crate) unit_impl: MeasureUnitImpl,
}

impl MeasureUnitImplWithIndex {
    /// `MeasureUnitImplWithIndex(int, MeasureUnitImpl)`.
    pub(crate) fn new(index: i32, unit_impl: MeasureUnitImpl) -> Self {
        MeasureUnitImplWithIndex { index, unit_impl }
    }
}

/// `UnitsParser.savedTrie`, built by the static initializer.
static SAVED_TRIE: LazyLock<CharsTrie> = LazyLock::new(|| match build_saved_trie() {
    Ok(t) => t,
    Err(e) => panic!("java.lang.ExceptionInInitializerError: {e}"),
});

/// The static initializer of `UnitsParser`.
fn build_saved_trie() -> Result<CharsTrie> {
    let mut trie_builder = CharsTrieBuilder::new();

    // Add SI and binary prefixes
    for unit_prefix in MeasurePrefix::VALUES {
        trie_builder.add(&units(unit_prefix.get_identifier()), UnitsParser::get_trie_index_for_prefix(unit_prefix))?;
    }

    // Add syntax parts (compound, power prefixes)
    trie_builder.add(&units("-per-"), CompoundPart::Per.get_trie_index())?;
    trie_builder.add(&units("-"), CompoundPart::Times.get_trie_index())?;
    trie_builder.add(&units("-and-"), CompoundPart::And.get_trie_index())?;
    trie_builder.add(&units("per-"), InitialCompoundPart::InitialCompoundPartPer.get_trie_index())?;
    trie_builder.add(&units("square-"), PowerPart::get_trie_index(2))?;
    trie_builder.add(&units("cubic-"), PowerPart::get_trie_index(3))?;
    for power in 2..=15 {
        trie_builder.add(&units(&format!("pow{power}-")), PowerPart::get_trie_index(power))?;
    }

    // Add simple units
    let simple_units = UnitsData::get_simple_units();
    for (i, unit) in simple_units.iter().enumerate() {
        trie_builder.add(&units(unit), i as i32 + Constants::K_SIMPLE_UNIT_OFFSET)?;
    }

    // TODO: Use SLOW or FAST here?
    trie_builder.build(StringTrieBuilderOption::Fast)
}

/// Equivalent to `UnitsParser.SingleUnitOrConstant`: a single unit or a
/// constant, not both.
struct SingleUnitOrConstant {
    /// `singleUnit`.
    single_unit: Option<SingleUnitImpl>,
    /// `constant`.
    constant: Option<i64>,
}

impl SingleUnitOrConstant {
    fn new(single_unit: Option<SingleUnitImpl>, constant: Option<i64>) -> Result<Self> {
        if single_unit.is_some() && constant.is_some() {
            return Err(LuceneError::illegal_argument("It is a SingleUnit Or a Constant, not both"));
        }
        Ok(SingleUnitOrConstant { single_unit, constant })
    }
}

/// Equivalent to `UnitsParser.TokenState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenState {
    /// `NO_TOKENS_SEEN`.
    NoTokensSeen,
    /// `POWER_TOKEN_SEEN`.
    PowerTokenSeen,
    /// `PREFIX_TOKEN_SEEN`.
    PrefixTokenSeen,
}

/// The parser of unit identifiers. Equivalent to
/// `MeasureUnitImpl.UnitsParser`.
pub struct UnitsParser {
    /// `trie`: a clone of the shared trie.
    trie: CharsTrie,
    /// `fSource`: the UTF-16 identifier.
    f_source: Vec<u16>,
    /// `fIndex`: tracks parser progress: the offset into fSource.
    f_index: i32,
    /// `fAfterPer`: set to true when we've just seen a "-per-" or a
    /// "per-", so that the next unit is in the denominator.
    f_after_per: bool,
    /// `fJustAfterPer`: set to true when we've just seen a "per-" or a
    /// "-per-" and the next token must be a constant or a unit.
    f_just_after_per: bool,
    /// `fSawAnd`: if an "-and-" was parsed prior to finding the "single
    /// unit", sawAnd is set to true.
    f_saw_and: bool,
}

impl UnitsParser {
    /// `UnitsParser(String)` (private).
    fn new(identifier: &str) -> Self {
        UnitsParser {
            trie: CharsTrie::from_other(&SAVED_TRIE),
            f_source: units(identifier),
            f_index: 0,
            f_after_per: false,
            f_just_after_per: false,
            f_saw_and: false,
        }
    }

    /// Construct a MeasureUnit from a CLDR Unit Identifier, defined in UTS
    /// 35. Validates and canonicalizes the identifier. Equivalent to
    /// `parseForIdentifier(String)`: `None` for Java's `null` (a `null` or
    /// empty identifier).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a syntax error.
    pub fn parse_for_identifier(identifier: Option<&str>) -> Result<Option<MeasureUnitImpl>> {
        match identifier {
            None | Some("") => Ok(None),
            Some(identifier) => {
                let mut parser = UnitsParser::new(identifier);
                parser.parse()
            }
        }
    }

    /// `getPrefixFromTrieIndex(int)`.
    fn get_prefix_from_trie_index(trie_index: i32) -> MeasurePrefix {
        MeasurePrefix::VALUES[(trie_index - Constants::K_PREFIX_OFFSET) as usize]
    }

    /// `getTrieIndexForPrefix(MeasurePrefix)`.
    fn get_trie_index_for_prefix(prefix: MeasurePrefix) -> i32 {
        prefix.ordinal() + Constants::K_PREFIX_OFFSET
    }

    /// `parse()`.
    fn parse(&mut self) -> Result<Option<MeasureUnitImpl>> {
        let mut result = MeasureUnitImpl::new();

        if self.f_source.is_empty() {
            // The dimensionless unit: nothing to parse. return null.
            return Ok(None);
        }

        while self.has_next() {
            self.f_saw_and = false;
            let next_single_unit_pair = self.next_single_unit()?;

            let single_unit = match next_single_unit_pair.single_unit {
                None => {
                    result.set_constant_denominator(next_single_unit_pair.constant.unwrap_or(0));
                    result.set_complexity(Complexity::Compound);
                    continue;
                }
                Some(s) => s,
            };

            let added = result.append_single_unit(Some(&single_unit));
            if self.f_saw_and && !added {
                return Err(LuceneError::illegal_argument("Two similar units are not allowed in a mixed unit."));
            }

            if result.single_units.len() >= 2 {
                // nextSingleUnit fails appropriately for "per" and "and" in
                // the same identifier. It doesn't fail for other compound
                // units (COMPOUND_PART_TIMES). Consequently we take care of
                // that here.
                let complexity = if self.f_saw_and { Complexity::Mixed } else { Complexity::Compound };
                if result.get_single_units().len() == 2 {
                    // After appending two singleUnits, the complexity will
                    // be `UMEASURE_UNIT_COMPOUND`
                    debug_assert!(result.get_complexity() == Complexity::Compound);
                    result.set_complexity(complexity);
                } else if result.get_complexity() != complexity {
                    // Can't have mixed compound units
                    return Err(LuceneError::illegal_argument("Can't have mixed compound units"));
                }
            }
        }

        if result.get_single_units().is_empty() {
            return Err(LuceneError::illegal_argument("Error in parsing a unit identifier."));
        }

        Ok(Some(result))
    }

    /// Returns the next "single unit" via result. Equivalent to
    /// `nextSingleUnit()`.
    fn next_single_unit(&mut self) -> Result<SingleUnitOrConstant> {
        let mut result = SingleUnitImpl::new();

        // state:
        // 0 = no tokens seen yet (will accept power, SI or binary prefix, or
        // simple unit)
        // 1 = power token seen (will not accept another power token)
        // 2 = SI or binary prefix token seen (will not accept a power, or SI
        // or binary prefix token)
        let mut state = TokenState::NoTokensSeen;

        // Maybe read a compound part
        let at_start = self.f_index == 0;
        let mut token = self.next_token()?;
        self.f_just_after_per = false;

        if at_start {
            // Identifiers optionally start with "per-".
            if token.get_type() == TokenType::TypeUnitConstant {
                return Err(LuceneError::illegal_argument("Unit constant cannot be the first token"));
            }

            if token.get_type() == TokenType::TypeInitialCompoundPart {
                debug_assert!(token.get_initial_compound_part()? == InitialCompoundPart::InitialCompoundPartPer);

                self.f_after_per = true;
                self.f_just_after_per = true;
                result.set_dimensionality(-1);

                token = self.next_token()?;
            }
        } else {
            // All other SingleUnit's are separated from previous SingleUnit's
            // via a compound part:
            if token.get_type() != TokenType::TypeCompoundPart {
                return Err(LuceneError::illegal_argument("token type must be TYPE_COMPOUND_PART"));
            }

            let compound_part = CompoundPart::get_compound_part_from_trie_index(token.get_match());
            match compound_part {
                CompoundPart::Per => {
                    if self.f_saw_and {
                        return Err(LuceneError::illegal_argument("Mixed compound units not yet supported"));
                        // TODO(CLDR-13701).
                    }

                    self.f_after_per = true;
                    self.f_just_after_per = true;
                    result.set_dimensionality(-1);
                }
                CompoundPart::Times => {
                    if self.f_after_per {
                        result.set_dimensionality(-1);
                    }
                }
                CompoundPart::And => {
                    if self.f_after_per {
                        // not yet supported, TODO(CLDR-13701).
                        return Err(LuceneError::illegal_argument(
                            "Can't start with \"-and-\", and mixed compound units",
                        ));
                    }
                    self.f_saw_and = true;
                }
            }

            token = self.next_token()?;
        }

        if token.get_type() == TokenType::TypeUnitConstant {
            if !self.f_just_after_per {
                return Err(LuceneError::illegal_argument("Unit constant cannot be the first token"));
            }

            return SingleUnitOrConstant::new(None, Some(token.get_constant_denominator()));
        }

        // Read a unit
        loop {
            match token.get_type() {
                TokenType::TypePowerPart => {
                    if state != TokenState::NoTokensSeen {
                        return Err(crate::jdk::illegal_argument_no_message());
                    }

                    result.set_dimensionality(result.get_dimensionality() * token.get_power());
                    state = TokenState::PowerTokenSeen;
                }
                TokenType::TypePrefix => {
                    if state == TokenState::PrefixTokenSeen {
                        return Err(crate::jdk::illegal_argument_no_message());
                    }

                    result.set_prefix(token.get_prefix());
                    state = TokenState::PrefixTokenSeen;
                }
                TokenType::TypeSimpleUnit => {
                    result.set_simple_unit(token.get_simple_unit_index(), UnitsData::get_simple_units());
                    return SingleUnitOrConstant::new(Some(result), None);
                }
                _ => return Err(crate::jdk::illegal_argument_no_message()),
            }

            if !self.has_next() {
                return Err(LuceneError::illegal_argument(
                    "We ran out of tokens before finding a complete single unit.",
                ));
            }

            token = self.next_token()?;
        }
    }

    /// `hasNext()`.
    fn has_next(&self) -> bool {
        self.f_index < self.f_source.len() as i32
    }

    /// `nextToken()`.
    fn next_token(&mut self) -> Result<Token> {
        self.trie.reset();
        let mut matching_value = -1;
        // Saving the index into fSource, so that we can know where the
        // token ended.
        let mut prev_index = -1;
        let saved_index = self.f_index;
        while self.f_index < self.f_source.len() as i32 {
            let c = self.f_source[self.f_index as usize];
            self.f_index += 1;
            let result = self.trie.next(c as i32);
            if result == TrieResult::NoMatch {
                break;
            } else if result == TrieResult::NoValue {
                continue;
            }

            matching_value = self.trie.get_value();
            prev_index = self.f_index;

            if result == TrieResult::FinalValue {
                break;
            }

            if result != TrieResult::IntermediateValue {
                return Err(LuceneError::illegal_argument("Result must have an intermediate value"));
            }

            // continue;
        }

        if matching_value < 0 {
            if self.f_just_after_per {
                // We've just parsed a "per-", so we can expect a unit
                // constant.
                let hyphen_index = crate::jdk::index_of_unit(&self.f_source, '-' as u16, saved_index);

                // extract unit constant from the string
                let end = if hyphen_index == -1 { self.f_source.len() as i32 } else { hyphen_index };
                let unit_constant = self.f_source[saved_index as usize..end as usize].to_vec();
                self.f_index = end;

                return Token::token_with_constant(&unit_constant);
            } else {
                return Err(LuceneError::illegal_argument(format!(
                    "Encountered unknown token starting at index {prev_index}"
                )));
            }
        } else {
            self.f_index = prev_index;
        }

        Token::new(matching_value as i64)
    }
}

/// Equivalent to `UnitsParser.Token.Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenType {
    /// `TYPE_UNDEFINED`.
    #[allow(dead_code)]
    TypeUndefined,
    /// `TYPE_PREFIX`.
    TypePrefix,
    /// `TYPE_COMPOUND_PART`.
    TypeCompoundPart,
    /// `TYPE_INITIAL_COMPOUND_PART`.
    TypeInitialCompoundPart,
    /// `TYPE_POWER_PART`.
    TypePowerPart,
    /// `TYPE_SIMPLE_UNIT`.
    TypeSimpleUnit,
    /// `TYPE_UNIT_CONSTANT`.
    TypeUnitConstant,
}

/// Equivalent to `UnitsParser.Token`.
struct Token {
    /// `fMatch`.
    f_match: i64,
    /// `type`.
    type_: TokenType,
}

impl Token {
    /// `Token(long)`.
    fn new(f_match: i64) -> Result<Token> {
        Ok(Token { f_match, type_: Self::calculate_type(f_match) })
    }

    /// `tokenWithConstant(String)`.
    fn token_with_constant(constant_str: &[u16]) -> Result<Token> {
        let unit_constant_value = BigDecimal::from_string(constant_str)?;
        if unit_constant_value.scale() <= 0
            && unit_constant_value.compare_to(&BigDecimal::zero()) >= 0
            && unit_constant_value.compare_to(&BigDecimal::value_of(i64::MAX)) <= 0
        {
            Ok(Token { f_match: unit_constant_value.long_value_exact()?, type_: TokenType::TypeUnitConstant })
        } else {
            Err(LuceneError::illegal_argument("The unit constant value is not a valid non-negative long integer."))
        }
    }

    fn get_type(&self) -> TokenType {
        self.type_
    }

    fn get_prefix(&self) -> MeasurePrefix {
        debug_assert!(self.type_ == TokenType::TypePrefix);
        debug_assert!(self.f_match <= i32::MAX as i64);
        UnitsParser::get_prefix_from_trie_index(self.f_match as i32)
    }

    fn get_constant_denominator(&self) -> i64 {
        debug_assert!(self.type_ == TokenType::TypeUnitConstant);
        self.f_match
    }

    /// Valid only for tokens with type TYPE_COMPOUND_PART.
    fn get_match(&self) -> i32 {
        debug_assert!(self.get_type() == TokenType::TypeCompoundPart);
        debug_assert!(self.f_match <= i32::MAX as i64);
        self.f_match as i32
    }

    /// Even if there is only one InitialCompoundPart value, we have this
    /// function for the simplicity of code consistency.
    fn get_initial_compound_part(&self) -> Result<InitialCompoundPart> {
        debug_assert!(self.type_ == TokenType::TypeInitialCompoundPart);
        debug_assert!(self.f_match <= i32::MAX as i64);
        InitialCompoundPart::get_initial_compound_part_from_trie_index(self.f_match as i32)
    }

    fn get_power(&self) -> i32 {
        debug_assert!(self.type_ == TokenType::TypePowerPart);
        debug_assert!(self.f_match <= i32::MAX as i64);
        PowerPart::get_power_from_trie_index(self.f_match as i32)
    }

    fn get_simple_unit_index(&self) -> i32 {
        debug_assert!(self.type_ == TokenType::TypeSimpleUnit);
        debug_assert!(self.f_match <= i32::MAX as i64);
        self.f_match as i32 - Constants::K_SIMPLE_UNIT_OFFSET
    }

    // Calling calculateType() is invalid, resulting in an assertion failure,
    // if Token value isn't positive.
    fn calculate_type(f_match: i64) -> TokenType {
        if f_match <= 0 {
            panic!("java.lang.AssertionError: fMatch must have a positive value");
        }

        if f_match < Constants::K_COMPOUND_PART_OFFSET as i64 {
            return TokenType::TypePrefix;
        }
        if f_match < Constants::K_INITIAL_COMPOUND_PART_OFFSET as i64 {
            return TokenType::TypeCompoundPart;
        }
        if f_match < Constants::K_POWER_PART_OFFSET as i64 {
            return TokenType::TypeInitialCompoundPart;
        }
        if f_match < Constants::K_SIMPLE_UNIT_OFFSET as i64 {
            return TokenType::TypePowerPart;
        }

        TokenType::TypeSimpleUnit
    }
}

/// Equivalent to `MeasureUnitImpl.MeasureUnitImplComparator`.
pub(crate) struct MeasureUnitImplComparator<'a> {
    /// `conversionRates`.
    conversion_rates: &'a ConversionRates,
}

impl<'a> MeasureUnitImplComparator<'a> {
    pub(crate) fn new(conversion_rates: &'a ConversionRates) -> Self {
        MeasureUnitImplComparator { conversion_rates }
    }

    /// `compare(MeasureUnitImpl, MeasureUnitImpl)`.
    ///
    /// # Errors
    ///
    /// The errors of the conversion-rate computation.
    pub(crate) fn compare(&self, o1: &MeasureUnitImpl, o2: &MeasureUnitImpl) -> Result<i32> {
        let special1 = self.conversion_rates.get_special_mapping_name(o1);
        let special2 = self.conversion_rates.get_special_mapping_name(o2);
        if special1.is_some() || special2.is_some() {
            return Ok(match (special1, special2) {
                (None, _) => -1,
                (_, None) => 1,
                (Some(a), Some(b)) => java_compare_str(&a, &b),
            });
        }
        let factor1 = self.conversion_rates.get_factor_to_base(o1)?.get_conversion_rate()?;
        let factor2 = self.conversion_rates.get_factor_to_base(o2)?.get_conversion_rate()?;

        Ok(factor1.compare_to(&factor2))
    }
}

/// The sign of `String.compareTo(String)` (the comparator's callers use
/// only the sign).
fn java_compare_str(a: &str, b: &str) -> i32 {
    rucene::util::java_collections::java_compare_to(a, b) as i32
}

/// Equivalent to `MeasureUnitImpl.MeasureUnitImplWithIndexComparator`.
pub(crate) struct MeasureUnitImplWithIndexComparator<'a> {
    /// `measureUnitImplComparator`.
    measure_unit_impl_comparator: MeasureUnitImplComparator<'a>,
}

impl<'a> MeasureUnitImplWithIndexComparator<'a> {
    pub(crate) fn new(conversion_rates: &'a ConversionRates) -> Self {
        MeasureUnitImplWithIndexComparator { measure_unit_impl_comparator: MeasureUnitImplComparator::new(conversion_rates) }
    }

    /// `compare(MeasureUnitImplWithIndex, MeasureUnitImplWithIndex)`.
    ///
    /// # Errors
    ///
    /// As [`MeasureUnitImplComparator::compare`].
    pub(crate) fn compare(&self, o1: &MeasureUnitImplWithIndex, o2: &MeasureUnitImplWithIndex) -> Result<i32> {
        self.measure_unit_impl_comparator.compare(&o1.unit_impl, &o2.unit_impl)
    }
}

/// Equivalent to `MeasureUnitImpl.SingleUnitComparator`.
pub(crate) struct SingleUnitComparator;

impl SingleUnitComparator {
    /// `compare(SingleUnitImpl, SingleUnitImpl)`.
    pub(crate) fn compare(o1: &SingleUnitImpl, o2: &SingleUnitImpl) -> Ordering {
        o1.compare_to(o2).cmp(&0)
    }
}
