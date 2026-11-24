// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.AdoptingModifierStore`.

use std::sync::Arc;

use crate::r#impl::number::modifier::{Modifier, Signum};
use crate::r#impl::number::modifier_store::ModifierStore;
use crate::r#impl::standard_plural::StandardPlural;

/// This implementation of ModifierStore adopts references to Modifiers.
///
/// Equivalent to `com.ibm.icu.impl.number.AdoptingModifierStore`.
#[derive(Debug, Clone)]
pub struct AdoptingModifierStore {
    positive: Option<Arc<dyn Modifier>>,
    pos_zero: Option<Arc<dyn Modifier>>,
    neg_zero: Option<Arc<dyn Modifier>>,
    negative: Option<Arc<dyn Modifier>>,
    /// `mods` (Java `null` for the four-modifier store).
    pub(crate) mods: Option<Vec<Option<Arc<dyn Modifier>>>>,
    /// `frozen`.
    pub(crate) frozen: bool,
}

impl Default for AdoptingModifierStore {
    fn default() -> Self {
        Self::new()
    }
}

impl AdoptingModifierStore {
    /// This constructor populates the ParameterizedModifier with a single
    /// positive and negative form. Equivalent to
    /// `AdoptingModifierStore(Modifier, Modifier, Modifier, Modifier)`.
    pub fn with_signs(
        positive: Option<Arc<dyn Modifier>>,
        pos_zero: Option<Arc<dyn Modifier>>,
        neg_zero: Option<Arc<dyn Modifier>>,
        negative: Option<Arc<dyn Modifier>>,
    ) -> Self {
        Self {
            positive,
            pos_zero,
            neg_zero,
            negative,
            mods: None,
            frozen: true,
        }
    }

    /// This constructor prepares the ImmutableParameterizedModifier to be
    /// populated with a positive and negative Modifier for multiple plural
    /// forms. Equivalent to `AdoptingModifierStore()`.
    pub fn new() -> Self {
        Self {
            positive: None,
            pos_zero: None,
            neg_zero: None,
            negative: None,
            mods: Some(vec![None; (4 * StandardPlural::COUNT) as usize]),
            frozen: false,
        }
    }

    /// Equivalent to `setModifier(Signum, StandardPlural, Modifier)`.
    pub fn set_modifier(
        &mut self,
        signum: Signum,
        plural: StandardPlural,
        m: Option<Arc<dyn Modifier>>,
    ) {
        debug_assert!(!self.frozen);
        let index = Self::get_mod_index(signum, plural);
        self.mods
            .as_mut()
            .expect("java.lang.NullPointerException: mods")[index] = m;
    }

    /// Equivalent to `freeze()`.
    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    /// Equivalent to `getModifierWithoutPlural(Signum)`.
    pub fn get_modifier_without_plural(&self, signum: Signum) -> Option<Arc<dyn Modifier>> {
        debug_assert!(self.frozen);
        debug_assert!(self.mods.is_none());
        match signum {
            Signum::Pos => self.positive.clone(),
            Signum::PosZero => self.pos_zero.clone(),
            Signum::NegZero => self.neg_zero.clone(),
            Signum::Neg => self.negative.clone(),
        }
    }

    /// `getModIndex(Signum, StandardPlural)`.
    fn get_mod_index(signum: Signum, plural: StandardPlural) -> usize {
        (plural.ordinal() * Signum::COUNT + signum.ordinal()) as usize
    }
}

impl ModifierStore for AdoptingModifierStore {
    fn get_modifier(&self, signum: Signum, plural: StandardPlural) -> Option<Arc<dyn Modifier>> {
        debug_assert!(self.frozen);
        debug_assert!(self.positive.is_none());
        self.mods
            .as_ref()
            .expect("java.lang.NullPointerException: mods")[Self::get_mod_index(signum, plural)]
        .clone()
    }
}
