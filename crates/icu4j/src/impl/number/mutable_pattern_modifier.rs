// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.MutablePatternModifier`.

use std::any::Any;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use rucene::error::Result;

use crate::jdk::text::FieldRef;
use crate::jdk::units;
use crate::number::number_formatter::{SignDisplay, UnitWidth};
use crate::r#impl::formatted_string_builder::FormattedStringBuilder;
use crate::r#impl::number::adopting_modifier_store::AdoptingModifierStore;
use crate::r#impl::number::affix_pattern_provider::AffixPatternProvider;
use crate::r#impl::number::affix_utils::{self, AffixUtils, SymbolProvider};
use crate::r#impl::number::constant_multi_field_modifier::ConstantMultiFieldModifier;
use crate::r#impl::number::currency_spacing_enabled_modifier::CurrencySpacingEnabledModifier;
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::micro_props::MicroProps;
use crate::r#impl::number::micro_props_generator::MicroPropsGenerator;
use crate::r#impl::number::modifier::{Modifier, Parameters, Signum};
use crate::r#impl::number::modifier_store::ModifierStore;
use crate::r#impl::number::pattern_string_utils::PatternStringUtils;
use crate::r#impl::number::rounding_utils::RoundingUtils;
use crate::r#impl::standard_plural::StandardPlural;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;
use crate::text::plural_rules::PluralRules;
use crate::util::currency::{self, Currency};

/// The mutable fields of a `MutablePatternModifier`.
#[derive(Debug, Default)]
struct State {
    // Modifier details
    /// `patternInfo`.
    pattern_info: Option<Arc<dyn AffixPatternProvider>>,
    /// `field`.
    field: Option<FieldRef>,
    /// `signDisplay`.
    sign_display: Option<SignDisplay>,
    /// `perMilleReplacesPercent`.
    per_mille_replaces_percent: bool,
    /// `approximately`.
    approximately: bool,

    // Symbol details
    /// `symbols`.
    symbols: Option<Arc<DecimalFormatSymbols>>,
    /// `unitWidth`.
    unit_width: Option<UnitWidth>,
    /// `currency`.
    currency: Option<Currency>,
    /// `rules`.
    rules: Option<Arc<PluralRules>>,

    // Number details
    /// `signum`.
    signum: Option<Signum>,
    /// `plural`.
    plural: Option<StandardPlural>,

    // QuantityChain details
    /// `parent`.
    parent: Option<Arc<dyn MicroPropsGenerator>>,

    // Transient fields for rendering
    /// `currentAffix`.
    current_affix: Option<Vec<u16>>,
}

impl State {
    fn pattern_info(&self) -> &Arc<dyn AffixPatternProvider> {
        self.pattern_info
            .as_ref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: patternInfo"))
    }

    fn symbols(&self) -> &Arc<DecimalFormatSymbols> {
        self.symbols
            .as_ref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: symbols"))
    }

    fn currency(&self) -> &Currency {
        self.currency
            .as_ref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: currency"))
    }

    /// `needsPlurals()`.
    fn needs_plurals(&self) -> Result<bool> {
        // Plural forms are only needed if the pattern contains the
        // currency-triple pattern ("¤¤¤").
        self.pattern_info()
            .contains_symbol_type(affix_utils::TYPE_CURRENCY_TRIPLE)
    }

    /// `setNumberProperties(Signum, StandardPlural)`.
    fn set_number_properties(&mut self, signum: Signum, plural: Option<StandardPlural>) {
        debug_assert!(plural.is_some() == self.needs_plurals().unwrap_or(false));
        self.signum = Some(signum);
        self.plural = plural;
    }

    /// `prepareAffix(boolean)`: fills `currentAffix` with the affix
    /// pattern.
    fn prepare_affix(&mut self, is_prefix: bool) -> Result<()> {
        let mut current_affix = self.current_affix.take().unwrap_or_default();
        let sign_type = PatternStringUtils::resolve_sign_display(
            self.sign_display
                .unwrap_or_else(|| panic!("java.lang.NullPointerException: signDisplay")),
            self.signum
                .unwrap_or_else(|| panic!("java.lang.NullPointerException: signum")),
        );
        let r = PatternStringUtils::pattern_info_to_string_builder(
            self.pattern_info().as_ref(),
            is_prefix,
            sign_type,
            self.approximately,
            self.plural,
            self.per_mille_replaces_percent,
            &mut current_affix,
        );
        // Java's StringBuilder field keeps what was appended before the throw.
        self.current_affix = Some(current_affix);
        r
    }

    /// `insertPrefix(FormattedStringBuilder, int)`.
    fn insert_prefix(&mut self, sb: &mut FormattedStringBuilder, position: i32) -> Result<i32> {
        self.prepare_affix(true)?;
        let affix = self.current_affix.clone().unwrap_or_default();
        AffixUtils::unescape(&affix, sb, position, self, self.field)
    }

    /// `insertSuffix(FormattedStringBuilder, int)`.
    fn insert_suffix(&mut self, sb: &mut FormattedStringBuilder, position: i32) -> Result<i32> {
        self.prepare_affix(false)?;
        let affix = self.current_affix.clone().unwrap_or_default();
        AffixUtils::unescape(&affix, sb, position, self, self.field)
    }

    /// `createConstantModifier(FormattedStringBuilder,
    /// FormattedStringBuilder)`: uses the current properties to create a
    /// single `ConstantMultiFieldModifier` with currency spacing support if
    /// required.
    fn create_constant_modifier(
        &mut self,
        a: &mut FormattedStringBuilder,
        b: &mut FormattedStringBuilder,
        is_strong: bool,
    ) -> Result<Arc<dyn Modifier>> {
        a.clear();
        self.insert_prefix(a, 0)?;
        b.clear();
        self.insert_suffix(b, 0)?;
        let has_body = self.pattern_info().has_body();
        if self.pattern_info().has_currency_sign() {
            Ok(Arc::new(CurrencySpacingEnabledModifier::new(
                a,
                b,
                !has_body,
                is_strong,
                self.symbols(),
            )?))
        } else {
            Ok(Arc::new(ConstantMultiFieldModifier::new(
                a, b, !has_body, is_strong,
            )))
        }
    }

    /// `getCurrencySymbolForUnitWidth()`.
    fn get_currency_symbol_for_unit_width(&self) -> Result<Vec<u16>> {
        // UnitWidth ISO, HIDDEN, or NARROW overrides the singular currency
        // symbol.
        match self.unit_width {
            Some(UnitWidth::IsoCode) => Ok(units(&self.currency().get_currency_code())),
            Some(UnitWidth::Hidden) => Ok(Vec::new()),
            width => {
                let selector = match width {
                    Some(UnitWidth::Short) => currency::SYMBOL_NAME,
                    Some(UnitWidth::Narrow) => currency::NARROW_SYMBOL_NAME,
                    Some(UnitWidth::Formal) => currency::FORMAL_SYMBOL_NAME,
                    Some(UnitWidth::Variant) => currency::VARIANT_SYMBOL_NAME,
                    _ => panic!("java.lang.AssertionError"),
                };
                let name =
                    self.currency()
                        .get_name(&self.symbols().get_u_locale(), selector, None)?;
                Ok(units(&name.unwrap_or_else(|| {
                    panic!("java.lang.NullPointerException")
                })))
            }
        }
    }
}

impl SymbolProvider for State {
    /// Returns the string that substitutes a given symbol type in a pattern.
    /// Equivalent to `MutablePatternModifier.getSymbol(int)`.
    fn get_symbol(&self, type_: i32) -> Result<Vec<u16>> {
        let symbols = self.symbols();
        Ok(match type_ {
            affix_utils::TYPE_MINUS_SIGN => symbols.get_minus_sign_string().to_vec(),
            affix_utils::TYPE_PLUS_SIGN => symbols.get_plus_sign_string().to_vec(),
            affix_utils::TYPE_APPROXIMATELY_SIGN => {
                symbols.get_approximately_sign_string().to_vec()
            }
            affix_utils::TYPE_PERCENT => symbols.get_percent_string().to_vec(),
            affix_utils::TYPE_PERMILLE => symbols.get_per_mill_string().to_vec(),
            affix_utils::TYPE_CURRENCY_SINGLE => self.get_currency_symbol_for_unit_width()?,
            affix_utils::TYPE_CURRENCY_DOUBLE => units(&self.currency().get_currency_code()),
            affix_utils::TYPE_CURRENCY_TRIPLE => {
                // NOTE: This is the code path only for patterns containing
                // "¤¤¤". Plural currencies set via the API are formatted in
                // LongNameHandler. This code path is used by DecimalFormat
                // via CurrencyPluralInfo.
                debug_assert!(self.plural.is_some());
                let plural = self
                    .plural
                    .unwrap_or_else(|| panic!("java.lang.NullPointerException: plural"));
                let name = self.currency().get_name_plural(
                    &symbols.get_u_locale(),
                    currency::PLURAL_LONG_NAME,
                    plural.get_keyword(),
                    None,
                )?;
                units(&name.unwrap_or_else(|| panic!("java.lang.NullPointerException")))
            }
            affix_utils::TYPE_CURRENCY_QUAD => units("\u{FFFD}"),
            affix_utils::TYPE_CURRENCY_QUINT => {
                let name = self.currency().get_name(
                    &symbols.get_u_locale(),
                    currency::NARROW_SYMBOL_NAME,
                    None,
                )?;
                units(&name.unwrap_or_else(|| panic!("java.lang.NullPointerException")))
            }
            _ => panic!("java.lang.AssertionError"),
        })
    }
}

/// This class is a {@link Modifier} that wraps a decimal format pattern. It
/// applies the pattern's affixes in {@link Modifier#apply}.
///
/// In addition to being a Modifier, this class contains the business logic
/// for substituting the correct locale symbols into the affixes of the
/// decimal format pattern.
///
/// In order to use this class, create a new instance and call the following
/// four setters: {@link #setPatternInfo}, {@link #setPatternAttributes},
/// {@link #setSymbols}, and {@link #setNumberProperties}. After calling
/// these four setters, the instance will be ready for use as a Modifier.
///
/// This is a MUTABLE, NON-THREAD-SAFE class designed for performance. Do
/// NOT save references to this or attempt to use it from multiple threads!!!
/// Instead, you can obtain a safe, immutable decimal format pattern modifier
/// by calling {@link MutablePatternModifier#createImmutable}, in effect
/// treating this instance as a builder for the immutable variant.
///
/// Equivalent to `com.ibm.icu.impl.number.MutablePatternModifier` (a
/// `Modifier`, `AffixUtils.SymbolProvider` and `MicroPropsGenerator`).
/// Java hands this object itself to the micros (`micros.modMiddle =
/// this`) after setting its number properties; the port keeps the mutable
/// fields behind a lock and a weak reference to the shared object, so the
/// same object is handed out ([`new`](Self::new) answers an `Arc`). The
/// symbol provider role is played by the locked fields
/// ([`get_symbol`](Self::get_symbol)).
#[derive(Debug)]
pub struct MutablePatternModifier {
    // Modifier details
    /// `isStrong`.
    is_strong: bool,
    /// The mutable fields.
    state: Mutex<State>,
    /// This object, for `micros.modMiddle = this`.
    self_ref: Weak<MutablePatternModifier>,
}

impl MutablePatternModifier {
    /// Equivalent to `MutablePatternModifier(boolean isStrong)`: whether the
    /// modifier should be considered strong.
    pub fn new(is_strong: bool) -> Arc<MutablePatternModifier> {
        Arc::new_cyclic(|self_ref| MutablePatternModifier {
            is_strong,
            state: Mutex::new(State::default()),
            self_ref: self_ref.clone(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Sets a reference to the parsed decimal format pattern, usually
    /// obtained from `PatternStringParser.parseToPatternInfo`, but any
    /// implementation of `AffixPatternProvider` is accepted. Equivalent to
    /// `setPatternInfo(AffixPatternProvider, NumberFormat.Field)`.
    pub fn set_pattern_info(
        &self,
        pattern_info: Arc<dyn AffixPatternProvider>,
        field: Option<FieldRef>,
    ) {
        let mut s = self.lock();
        s.pattern_info = Some(pattern_info);
        s.field = field;
    }

    /// Sets attributes that imply changes to the literal interpretation of
    /// the pattern string affixes. Equivalent to
    /// `setPatternAttributes(SignDisplay, boolean, boolean)`.
    pub fn set_pattern_attributes(
        &self,
        sign_display: SignDisplay,
        per_mille: bool,
        approximately: bool,
    ) {
        let mut s = self.lock();
        s.sign_display = Some(sign_display);
        s.per_mille_replaces_percent = per_mille;
        s.approximately = approximately;
    }

    /// Sets locale-specific details that affect the symbols substituted into
    /// the pattern string affixes. Equivalent to
    /// `setSymbols(DecimalFormatSymbols, Currency, UnitWidth, PluralRules)`.
    pub fn set_symbols(
        &self,
        symbols: Arc<DecimalFormatSymbols>,
        currency: Currency,
        unit_width: UnitWidth,
        rules: Option<Arc<PluralRules>>,
    ) {
        let mut s = self.lock();
        debug_assert!(rules.is_some() == s.needs_plurals().unwrap_or(false));
        s.symbols = Some(symbols);
        s.currency = Some(currency);
        s.unit_width = Some(unit_width);
        s.rules = rules;
    }

    /// Sets attributes of the current number being processed. Equivalent to
    /// `setNumberProperties(Signum, StandardPlural)`.
    pub fn set_number_properties(&self, signum: Signum, plural: Option<StandardPlural>) {
        self.lock().set_number_properties(signum, plural);
    }

    /// Returns true if the pattern represented by this MurkyModifier
    /// requires a plural keyword in order to localize. This is currently
    /// true only if there is a currency long name placeholder in the
    /// pattern ("¤¤¤"). Equivalent to `needsPlurals()`.
    ///
    /// # Errors
    ///
    /// None in practice; kept for the provider contract.
    pub fn needs_plurals(&self) -> Result<bool> {
        self.lock().needs_plurals()
    }

    /// Creates a new quantity-dependent Modifier that behaves the same as
    /// the current instance, but which is immutable and can be saved for
    /// future use. Equivalent to `createImmutable()`.
    ///
    /// # Errors
    ///
    /// The errors of the affix rendering and of the currency spacing data.
    pub fn create_immutable(&self) -> Result<ImmutablePatternModifier> {
        let mut s = self.lock();
        let mut a = FormattedStringBuilder::new();
        let mut b = FormattedStringBuilder::new();
        if s.needs_plurals()? {
            // Slower path when we require the plural keyword.
            let mut pm = AdoptingModifierStore::new();
            for plural in StandardPlural::VALUES {
                s.set_number_properties(Signum::Pos, Some(plural));
                pm.set_modifier(
                    Signum::Pos,
                    plural,
                    Some(s.create_constant_modifier(&mut a, &mut b, self.is_strong)?),
                );
                s.set_number_properties(Signum::PosZero, Some(plural));
                pm.set_modifier(
                    Signum::PosZero,
                    plural,
                    Some(s.create_constant_modifier(&mut a, &mut b, self.is_strong)?),
                );
                s.set_number_properties(Signum::NegZero, Some(plural));
                pm.set_modifier(
                    Signum::NegZero,
                    plural,
                    Some(s.create_constant_modifier(&mut a, &mut b, self.is_strong)?),
                );
                s.set_number_properties(Signum::Neg, Some(plural));
                pm.set_modifier(
                    Signum::Neg,
                    plural,
                    Some(s.create_constant_modifier(&mut a, &mut b, self.is_strong)?),
                );
            }
            pm.freeze();
            Ok(ImmutablePatternModifier::new(pm, s.rules.clone()))
        } else {
            // Faster path when plural keyword is not needed.
            s.set_number_properties(Signum::Pos, None);
            let positive = s.create_constant_modifier(&mut a, &mut b, self.is_strong)?;
            s.set_number_properties(Signum::PosZero, None);
            let pos_zero = s.create_constant_modifier(&mut a, &mut b, self.is_strong)?;
            s.set_number_properties(Signum::NegZero, None);
            let neg_zero = s.create_constant_modifier(&mut a, &mut b, self.is_strong)?;
            s.set_number_properties(Signum::Neg, None);
            let negative = s.create_constant_modifier(&mut a, &mut b, self.is_strong)?;
            let pm = AdoptingModifierStore::with_signs(
                Some(positive),
                Some(pos_zero),
                Some(neg_zero),
                Some(negative),
            );
            Ok(ImmutablePatternModifier::new(pm, None))
        }
    }

    /// Equivalent to `addToChain(MicroPropsGenerator)`: links the parent and
    /// answers this object.
    pub fn add_to_chain(
        &self,
        parent: Arc<dyn MicroPropsGenerator>,
    ) -> Arc<dyn MicroPropsGenerator> {
        self.lock().parent = Some(parent);
        self.self_ref
            .upgrade()
            .expect("the modifier is alive while it is used")
    }

    /// Equivalent to `getSymbol(int)` (the `SymbolProvider` method).
    ///
    /// # Errors
    ///
    /// The errors of the currency display data.
    pub fn get_symbol(&self, type_: i32) -> Result<Vec<u16>> {
        self.lock().get_symbol(type_)
    }

    /// Equivalent to `getCurrencySymbolForUnitWidth()`.
    ///
    /// # Errors
    ///
    /// The errors of the currency display data.
    pub fn get_currency_symbol_for_unit_width(&self) -> Result<Vec<u16>> {
        self.lock().get_currency_symbol_for_unit_width()
    }
}

impl MicroPropsGenerator for MutablePatternModifier {
    fn process_quantity(&self, fq: &mut dyn DecimalQuantity) -> Result<MicroProps> {
        let parent = self
            .lock()
            .parent
            .clone()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: parent"));
        let mut micros = parent.process_quantity(fq)?;
        if let Some(rounder) = &micros.rounder {
            rounder.apply(fq)?;
        }
        if micros.mod_middle.is_some() {
            return Ok(micros);
        }
        {
            let mut s = self.lock();
            if s.needs_plurals()? {
                let plural_form = RoundingUtils::get_plural_safe(
                    micros.rounder.as_ref(),
                    s.rules.as_deref(),
                    fq,
                )?;
                s.set_number_properties(fq.signum(), Some(plural_form));
            } else {
                s.set_number_properties(fq.signum(), None);
            }
        }
        let this: Arc<dyn Modifier> = self
            .self_ref
            .upgrade()
            .expect("the modifier is alive while it runs");
        micros.mod_middle = Some(this);
        Ok(micros)
    }
}

impl Modifier for MutablePatternModifier {
    fn apply(
        &self,
        output: &mut FormattedStringBuilder,
        left_index: i32,
        right_index: i32,
    ) -> Result<i32> {
        let mut s = self.lock();
        let prefix_len = s.insert_prefix(output, left_index)?;
        let suffix_len = s.insert_suffix(output, right_index + prefix_len)?;
        // If the pattern had no decimal stem body (like #,##0.00),
        // overwrite the value.
        let mut overwrite_len = 0;
        if !s.pattern_info().has_body() {
            overwrite_len = output.splice(
                left_index + prefix_len,
                right_index + prefix_len,
                &[],
                0,
                0,
                None,
            );
        }
        CurrencySpacingEnabledModifier::apply_currency_spacing(
            output,
            left_index,
            prefix_len,
            right_index + prefix_len + overwrite_len,
            suffix_len,
            s.symbols(),
        )?;
        Ok(prefix_len + overwrite_len + suffix_len)
    }

    fn get_prefix_length(&self) -> Result<i32> {
        let mut s = self.lock();
        // Render the affix to get the length
        s.prepare_affix(true)?;
        let affix = s.current_affix.clone().unwrap_or_default();
        AffixUtils::unescaped_count(&affix, true, &*s) // prefix length
    }

    fn get_code_point_count(&self) -> Result<i32> {
        let mut s = self.lock();
        // Render the affixes to get the length
        s.prepare_affix(true)?;
        let affix = s.current_affix.clone().unwrap_or_default();
        let mut result = AffixUtils::unescaped_count(&affix, false, &*s)?; // prefix length
        s.prepare_affix(false)?;
        let affix = s.current_affix.clone().unwrap_or_default();
        result += AffixUtils::unescaped_count(&affix, false, &*s)?; // suffix length
        Ok(result)
    }

    fn is_strong(&self) -> bool {
        self.is_strong
    }

    fn contains_field(&self, _field: FieldRef) -> bool {
        // This method is not currently used. (unsafe path not used in range
        // formatting)
        debug_assert!(false);
        false
    }

    fn get_parameters(&self) -> Option<&Parameters> {
        // This method is not currently used.
        debug_assert!(false);
        None
    }

    fn strict_equals(&self, _other: &dyn Modifier) -> bool {
        // This method is not currently used. (unsafe path not used in range
        // formatting)
        debug_assert!(false);
        false
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Equivalent to `MutablePatternModifier.ImmutablePatternModifier`.
#[derive(Debug, Clone)]
pub struct ImmutablePatternModifier {
    /// `pm`.
    pm: AdoptingModifierStore,
    /// `rules`.
    rules: Option<Arc<PluralRules>>,
    /// `parent` (`None` for Java's `null`).
    parent: Option<Arc<dyn MicroPropsGenerator>>,
}

impl ImmutablePatternModifier {
    /// `ImmutablePatternModifier(AdoptingModifierStore, PluralRules)`.
    fn new(pm: AdoptingModifierStore, rules: Option<Arc<PluralRules>>) -> Self {
        ImmutablePatternModifier {
            pm,
            rules,
            parent: None,
        }
    }

    /// Equivalent to `addToChain(MicroPropsGenerator)`: links the parent and
    /// answers this object.
    pub fn add_to_chain(
        mut self,
        parent: Arc<dyn MicroPropsGenerator>,
    ) -> Arc<dyn MicroPropsGenerator> {
        self.parent = Some(parent);
        Arc::new(self)
    }

    /// Equivalent to `applyToMicros(MicroProps, DecimalQuantity)`.
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of the plural rounding.
    pub fn apply_to_micros(
        &self,
        micros: &mut MicroProps,
        quantity: &dyn DecimalQuantity,
    ) -> Result<()> {
        match &self.rules {
            None => {
                micros.mod_middle = self.pm.get_modifier_without_plural(quantity.signum());
            }
            Some(rules) => {
                let plural_form =
                    RoundingUtils::get_plural_safe(micros.rounder.as_ref(), Some(rules), quantity)?;
                micros.mod_middle = self.pm.get_modifier(quantity.signum(), plural_form);
            }
        }
        Ok(())
    }
}

impl MicroPropsGenerator for ImmutablePatternModifier {
    fn process_quantity(&self, quantity: &mut dyn DecimalQuantity) -> Result<MicroProps> {
        let parent = self
            .parent
            .as_ref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: parent"));
        let mut micros = parent.process_quantity(quantity)?;
        if let Some(rounder) = &micros.rounder {
            rounder.apply(quantity)?;
        }
        if micros.mod_middle.is_some() {
            return Ok(micros);
        }
        self.apply_to_micros(&mut micros, quantity)?;
        Ok(micros)
    }
}
