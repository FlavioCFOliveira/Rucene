// © 2019 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.ModifierStore`.

use std::fmt;
use std::sync::Arc;

use crate::r#impl::number::modifier::{Modifier, Signum};
use crate::r#impl::standard_plural::StandardPlural;

/// This is *not* a modifier; rather, it is an object that can return
/// modifiers based on given parameters.
///
/// Equivalent to `com.ibm.icu.impl.number.ModifierStore`.
pub trait ModifierStore: fmt::Debug + Send + Sync + 'static {
    /// Returns a Modifier with the given parameters (best-effort). Equivalent
    /// to `getModifier(Signum, StandardPlural)`.
    fn get_modifier(&self, signum: Signum, plural: StandardPlural) -> Option<Arc<dyn Modifier>>;
}
