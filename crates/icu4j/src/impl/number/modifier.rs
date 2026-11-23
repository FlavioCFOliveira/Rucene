// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.Modifier`.

use rucene::error::Result;
use std::any::Any;
use std::fmt;
use std::sync::Arc;

use crate::jdk::text::FieldRef;
use crate::r#impl::formatted_string_builder::FormattedStringBuilder;
use crate::r#impl::number::modifier_store::ModifierStore;
use crate::r#impl::standard_plural::StandardPlural;

/// The sign of a number for modifier selection. Equivalent to
/// `Modifier.Signum`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Signum {
    /// `NEG`.
    Neg,
    /// `NEG_ZERO`.
    NegZero,
    /// `POS_ZERO`.
    PosZero,
    /// `POS`.
    Pos,
}

impl Signum {
    /// `COUNT`.
    pub const COUNT: i32 = 4;

    /// `VALUES`.
    pub const VALUES: [Signum; 4] = [Signum::Neg, Signum::NegZero, Signum::PosZero, Signum::Pos];

    /// Equivalent to `ordinal()`.
    pub fn ordinal(self) -> i32 {
        self as i32
    }

    /// Equivalent to `name()`.
    pub fn name(self) -> &'static str {
        match self {
            Signum::Neg => "NEG",
            Signum::NegZero => "NEG_ZERO",
            Signum::PosZero => "POS_ZERO",
            Signum::Pos => "POS",
        }
    }
}

impl fmt::Display for Signum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Parameters a modifier was created with, used for semantic equality.
/// Equivalent to `Modifier.Parameters` (public mutable fields).
#[derive(Debug, Clone, Default)]
pub struct Parameters {
    /// `obj`.
    pub obj: Option<Arc<dyn ModifierStore>>,
    /// `signum`.
    pub signum: Option<Signum>,
    /// `plural`.
    pub plural: Option<StandardPlural>,
}

/// A Modifier is an object that can be passed through the formatting
/// pipeline until it is finally applied to the string builder. A Modifier
/// usually contains a prefix and a suffix that are applied, but it could
/// contain something else, like a {@link com.ibm.icu.text.SimpleFormatter}
/// pattern.
///
/// Equivalent to `com.ibm.icu.impl.number.Modifier`. Modifiers are shared
/// objects (`Arc<dyn Modifier>`) compared by identity where Java uses `==`;
/// [`as_any`](Self::as_any) answers Java's `instanceof` tests.
pub trait Modifier: fmt::Debug + Send + Sync + 'static {
    /// Applies this Modifier to the string builder; answers the number of
    /// characters added. Equivalent to `apply(FormattedStringBuilder, int,
    /// int)`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the implementation.
    fn apply(&self, output: &mut FormattedStringBuilder, left_index: i32, right_index: i32) -> Result<i32>;

    /// Gets the length of the prefix. Equivalent to `getPrefixLength()`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the implementation (the
    /// `IllegalArgumentException` of a malformed affix pattern).
    fn get_prefix_length(&self) -> Result<i32>;

    /// Returns the number of code points in the modifier, prefix plus
    /// suffix. Equivalent to `getCodePointCount()`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the implementation (the
    /// `IllegalArgumentException` of a malformed affix pattern).
    fn get_code_point_count(&self) -> Result<i32>;

    /// Whether this modifier is strong. Equivalent to `isStrong()`.
    fn is_strong(&self) -> bool;

    /// Whether the modifier contains at least one instance of the given
    /// field. Equivalent to `containsField(Field)`.
    fn contains_field(&self, currency: FieldRef) -> bool;

    /// Gets a set of "parameters" for this Modifier. Equivalent to
    /// `getParameters()`.
    fn get_parameters(&self) -> Option<&Parameters>;

    /// Returns whether this Modifier is *semantically equivalent* to the
    /// other Modifier; in many cases, this is the same as equal, but parameters
    /// should be ignored. Equivalent to `strictEquals(Modifier)`.
    fn strict_equals(&self, other: &dyn Modifier) -> bool;

    /// Returns whether this Modifier is *semantically equivalent* to the
    /// other Modifier. Equivalent to `semanticallyEquivalent(Modifier)`
    /// (default method).
    fn semantically_equivalent(&self, other: &dyn Modifier) -> bool {
        let params_this = self.get_parameters();
        let params_other = other.get_parameters();
        match (params_this, params_other) {
            (None, None) => return self.strict_equals(other),
            (None, _) | (_, None) => return false,
            _ => {}
        }
        let (params_this, params_other) = (params_this.unwrap(), params_other.unwrap());
        let (obj_this, obj_other) = match (&params_this.obj, &params_other.obj) {
            (None, None) => return self.strict_equals(other),
            (None, _) | (_, None) => return false,
            (Some(a), Some(b)) => (a, b),
        };
        for signum in Signum::VALUES {
            for plural in StandardPlural::VALUES {
                let mod1 = obj_this.get_modifier(signum, plural);
                let mod2 = obj_other.get_modifier(signum, plural);
                match (&mod1, &mod2) {
                    (None, None) => {
                        // Either pointers are null or they are the same.
                        continue;
                    }
                    (Some(a), Some(b)) if same_modifier(a, b) => {
                        // Either pointers are null or they are the same.
                        continue;
                    }
                    (None, _) | (_, None) => {
                        // One pointer is null but not the other.
                        return false;
                    }
                    (Some(a), Some(b)) => {
                        if !a.strict_equals(b.as_ref()) {
                            // The modifiers are not equal.
                            return false;
                        }
                        // The modifiers are equal.
                        continue;
                    }
                }
            }
        }
        true
    }

    /// This object as `Any`, for Java's `instanceof` tests.
    fn as_any(&self) -> &dyn Any;
}

/// Whether two shared modifiers are the same object (Java `==`).
pub fn same_modifier(a: &Arc<dyn Modifier>, b: &Arc<dyn Modifier>) -> bool {
    std::ptr::eq(Arc::as_ptr(a) as *const (), Arc::as_ptr(b) as *const ())
}
