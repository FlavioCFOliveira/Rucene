// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.SimpleNotation`.

use std::ops::Deref;
use std::sync::Arc;

use crate::number::notation::{Notation, NotationKind};

/// A class that defines the simple notation style to be used when
/// formatting numbers in NumberFormatter.
///
/// This class exposes no public functionality. To create a SimpleNotation,
/// use one of the factory methods in [`Notation`].
///
/// Equivalent to `com.ibm.icu.number.SimpleNotation`: a view over a
/// [`Notation`] of that class, which it dereferences to.
#[derive(Debug, Clone)]
pub struct SimpleNotation(Notation);

impl SimpleNotation {
    /// `SimpleNotation()` (package-private).
    pub(crate) fn new() -> Self {
        SimpleNotation(Notation(Arc::new(NotationKind::Simple)))
    }

    /// `notation instanceof SimpleNotation ? (SimpleNotation) notation :
    /// null`.
    pub fn try_cast(notation: &Notation) -> Option<SimpleNotation> {
        match notation.kind() {
            NotationKind::Simple => Some(SimpleNotation(notation.clone())),
            _ => None,
        }
    }

    /// The `Notation` this view is.
    pub fn into_notation(self) -> Notation {
        self.0
    }
}

impl Deref for SimpleNotation {
    type Target = Notation;

    fn deref(&self) -> &Notation {
        &self.0
    }
}

impl From<SimpleNotation> for Notation {
    fn from(n: SimpleNotation) -> Notation {
        n.0
    }
}
