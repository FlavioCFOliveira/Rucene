// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.CompactNotation`.

use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use std::sync::Arc;

use rucene::error::Result;
use rucene::util::java_collections::JavaHashMap;

use crate::jdk::text::FieldRef;
use crate::number::notation::{Notation, NotationKind};
use crate::r#impl::number::compact_data::{CompactData, CompactType};
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::micro_props::MicroProps;
use crate::r#impl::number::micro_props_generator::MicroPropsGenerator;
use crate::r#impl::number::mutable_pattern_modifier::{
    ImmutablePatternModifier, MutablePatternModifier,
};
use crate::r#impl::number::pattern_string_parser::PatternStringParser;
use crate::text::compact_decimal_format::CompactStyle;
use crate::text::number_format::field as nf_field;
use crate::text::plural_rules::PluralRules;
use crate::util::u_locale::ULocale;

/// Custom compact data: powers of ten (`"1000"`, ...) to plural keywords to
/// patterns. Equivalent to Java's `Map<String, Map<String, String>>`.
pub type CompactCustomData = JavaHashMap<JavaHashMap<String>>;

/// The fields of a `CompactNotation` object.
#[derive(Debug, Clone)]
pub(crate) struct CompactNotationData {
    /// `compactStyle` (`None` for Java's `null`).
    pub(crate) compact_style: Option<CompactStyle>,
    /// `compactCustomData` (`None` for Java's `null`).
    pub(crate) compact_custom_data: Option<Arc<CompactCustomData>>,
}

/// A class that defines the scientific notation style to be used when
/// formatting numbers in NumberFormatter.
///
/// This class exposes no public functionality. To create a
/// CompactNotation, use one of the factory methods in [`Notation`].
///
/// Equivalent to `com.ibm.icu.number.CompactNotation`: a view over a
/// [`Notation`] of that class, which it dereferences to.
#[derive(Debug, Clone)]
pub struct CompactNotation(Notation);

impl CompactNotation {
    /// Create a compact notation with custom data. Equivalent to
    /// `forCustomData(Map)` (internal).
    pub fn for_custom_data(compact_custom_data: Option<Arc<CompactCustomData>>) -> CompactNotation {
        CompactNotation::with_custom_data(compact_custom_data)
    }

    /// `CompactNotation(CompactStyle)` (package-private).
    pub(crate) fn new(compact_style: CompactStyle) -> Self {
        CompactNotation(Notation(Arc::new(NotationKind::Compact(
            CompactNotationData {
                compact_style: Some(compact_style),
                compact_custom_data: None,
            },
        ))))
    }

    /// `CompactNotation(Map)` (package-private).
    pub(crate) fn with_custom_data(compact_custom_data: Option<Arc<CompactCustomData>>) -> Self {
        CompactNotation(Notation(Arc::new(NotationKind::Compact(
            CompactNotationData {
                compact_style: None,
                compact_custom_data,
            },
        ))))
    }

    /// `notation instanceof CompactNotation ? (CompactNotation) notation :
    /// null`.
    pub fn try_cast(notation: &Notation) -> Option<CompactNotation> {
        match notation.kind() {
            NotationKind::Compact(_) => Some(CompactNotation(notation.clone())),
            _ => None,
        }
    }

    /// The `Notation` this view is.
    pub fn into_notation(self) -> Notation {
        self.0
    }

    /// The fields.
    pub(crate) fn data(&self) -> &CompactNotationData {
        match self.0.kind() {
            NotationKind::Compact(d) => d,
            _ => unreachable!("a CompactNotation view holds a CompactNotation"),
        }
    }

    /// `withLocaleData(ULocale, String, CompactType, PluralRules,
    /// MutablePatternModifier, boolean, MicroPropsGenerator)`
    /// (package-private).
    ///
    /// # Errors
    ///
    /// The errors of the compact data loading and of the pattern parsing.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn with_locale_data(
        &self,
        locale: &ULocale,
        ns_name: &str,
        compact_type: CompactType,
        rules: Arc<PluralRules>,
        build_reference: &Arc<MutablePatternModifier>,
        safe: bool,
        parent: Arc<dyn MicroPropsGenerator>,
    ) -> Result<Arc<dyn MicroPropsGenerator>> {
        // TODO: Add a data cache? It would be keyed by locale, nsName,
        // compact type, and compact style.
        Ok(Arc::new(CompactHandler::new(
            self,
            locale,
            ns_name,
            compact_type,
            rules,
            build_reference,
            safe,
            parent,
        )?))
    }
}

impl Deref for CompactNotation {
    type Target = Notation;

    fn deref(&self) -> &Notation {
        &self.0
    }
}

impl From<CompactNotation> for Notation {
    fn from(n: CompactNotation) -> Notation {
        n.0
    }
}

/// Equivalent to the private class `CompactNotation.CompactHandler`.
#[derive(Debug)]
struct CompactHandler {
    /// `rules`.
    rules: Arc<PluralRules>,
    /// `parent`.
    parent: Arc<dyn MicroPropsGenerator>,
    /// `precomputedMods` (`None` for Java's `null`).
    precomputed_mods: Option<HashMap<Vec<u16>, ImmutablePatternModifier>>,
    /// `unsafePatternModifier` (`None` for Java's `null`).
    unsafe_pattern_modifier: Option<Arc<MutablePatternModifier>>,
    /// `data`.
    data: CompactData,
}

impl CompactHandler {
    #[allow(clippy::too_many_arguments)]
    fn new(
        notation: &CompactNotation,
        locale: &ULocale,
        ns_name: &str,
        compact_type: CompactType,
        rules: Arc<PluralRules>,
        build_reference: &Arc<MutablePatternModifier>,
        safe: bool,
        parent: Arc<dyn MicroPropsGenerator>,
    ) -> Result<CompactHandler> {
        let mut data = CompactData::new();
        let nd = notation.data();
        match nd.compact_style {
            Some(style) => data.populate(locale, ns_name, style, compact_type)?,
            None => data.populate_custom(nd.compact_custom_data.as_deref())?,
        }
        let (precomputed_mods, unsafe_pattern_modifier) = if safe {
            // Safe code path requires copying the patterns for each new
            // number.
            (
                Some(Self::precompute_all_modifiers(&data, build_reference)?),
                None,
            )
        } else {
            // Unsafe code path; requires the build reference to be used.
            (None, Some(build_reference.clone()))
        };
        Ok(CompactHandler {
            rules,
            parent,
            precomputed_mods,
            unsafe_pattern_modifier,
            data,
        })
    }

    /// Used by the safe code path.
    fn precompute_all_modifiers(
        data: &CompactData,
        build_reference: &MutablePatternModifier,
    ) -> Result<HashMap<Vec<u16>, ImmutablePatternModifier>> {
        let mut precomputed_mods = HashMap::new();
        let mut all_patterns: HashSet<Vec<u16>> = HashSet::new();
        data.get_unique_patterns(&mut all_patterns);

        for pattern_string in all_patterns {
            let pattern_info = PatternStringParser::parse_to_pattern_info(&pattern_string)?;
            build_reference
                .set_pattern_info(Arc::new(pattern_info), Some(&nf_field::COMPACT as FieldRef));
            let imm = build_reference.create_immutable()?;
            precomputed_mods.insert(pattern_string, imm);
        }
        Ok(precomputed_mods)
    }
}

impl MicroPropsGenerator for CompactHandler {
    fn process_quantity(&self, quantity: &mut dyn DecimalQuantity) -> Result<MicroProps> {
        let mut micros = self.parent.process_quantity(quantity)?;
        debug_assert!(micros.rounder.is_some());
        let rounder = micros
            .rounder
            .clone()
            .expect("micros.rounder is set by the pipeline");

        // Treat zero, NaN, and infinity as if they had magnitude 0
        let magnitude;
        let mut multiplier = 0;
        if quantity.is_zeroish() {
            magnitude = 0;
            rounder.apply(quantity)?;
        } else {
            multiplier = rounder.choose_multiplier_and_apply(quantity, &self.data)?;
            let m = if quantity.is_zeroish() {
                0
            } else {
                quantity.get_magnitude()?
            };
            magnitude = m - multiplier;
        }

        let pattern_string = self.data.get_pattern(magnitude, &self.rules, quantity);
        match pattern_string {
            None => {
                // Use the default (non-compact) modifier.
                // No need to take any action.
            }
            Some(pattern_string) => match &self.precomputed_mods {
                Some(mods) => {
                    // Safe code path.
                    // Java NPE when the pattern is missing from the map.
                    let m = mods
                        .get(&pattern_string)
                        .unwrap_or_else(|| panic!("java.lang.NullPointerException"));
                    m.apply_to_micros(&mut micros, quantity)?;
                }
                None => {
                    // Unsafe code path.
                    // Overwrite the PatternInfo in the existing modMiddle.
                    let pattern_info = PatternStringParser::parse_to_pattern_info(&pattern_string)?;
                    let upm = self
                        .unsafe_pattern_modifier
                        .as_ref()
                        .expect("set on the unsafe path");
                    upm.set_pattern_info(
                        Arc::new(pattern_info),
                        Some(&nf_field::COMPACT as FieldRef),
                    );
                    upm.set_number_properties(quantity.signum(), None);
                    micros.mod_middle = Some(upm.clone());
                }
            },
        }

        // Change the exponent only after we select appropriate plural form
        // for formatting purposes so that we preserve expected formatted
        // string behavior.
        quantity.adjust_exponent(-multiplier);

        // We already performed rounding. Do not perform it again.
        micros.rounder = None;

        Ok(micros)
    }
}
