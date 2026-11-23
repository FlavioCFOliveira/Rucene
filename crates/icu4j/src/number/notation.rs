// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.Notation`.

use std::sync::{Arc, LazyLock};

use rucene::util::java_collections::java_identity_hash_code_stand_in;

use crate::number::compact_notation::{CompactNotation, CompactNotationData};
use crate::number::number_formatter::SignDisplay;
use crate::number::scientific_notation::{ScientificNotation, ScientificNotationData};
use crate::number::simple_notation::SimpleNotation;
use crate::text::compact_decimal_format::CompactStyle;

/// The concrete Java class of a `Notation` object, with its fields.
#[derive(Debug, Clone)]
pub(crate) enum NotationKind {
    /// `ScientificNotation`.
    Scientific(ScientificNotationData),
    /// `CompactNotation`.
    Compact(CompactNotationData),
    /// `SimpleNotation`.
    Simple,
}

/// A class that defines the notation style to be used when formatting
/// numbers in NumberFormatter.
///
/// Equivalent to `com.ibm.icu.number.Notation`. Java's notation objects are
/// immutable and compared by identity (`Object.equals`); this handle shares
/// one object through an `Arc` ([`ptr_eq`](Self::ptr_eq) is Java's `==`).
/// The subclasses `ScientificNotation`, `CompactNotation` and
/// `SimpleNotation` are the views [`ScientificNotation`],
/// [`CompactNotation`] and [`SimpleNotation`].
#[derive(Debug, Clone)]
pub struct Notation(pub(crate) Arc<NotationKind>);

/// `SCIENTIFIC`.
static SCIENTIFIC: LazyLock<ScientificNotation> =
    LazyLock::new(|| ScientificNotation::new(1, false, 1, SignDisplay::Auto));
/// `ENGINEERING`.
static ENGINEERING: LazyLock<ScientificNotation> =
    LazyLock::new(|| ScientificNotation::new(3, false, 1, SignDisplay::Auto));
/// `COMPACT_SHORT`.
static COMPACT_SHORT: LazyLock<CompactNotation> = LazyLock::new(|| CompactNotation::new(CompactStyle::Short));
/// `COMPACT_LONG`.
static COMPACT_LONG: LazyLock<CompactNotation> = LazyLock::new(|| CompactNotation::new(CompactStyle::Long));
/// `SIMPLE`.
static SIMPLE: LazyLock<SimpleNotation> = LazyLock::new(SimpleNotation::new);

impl Notation {
    /// The concrete class and its fields.
    pub(crate) fn kind(&self) -> &NotationKind {
        &self.0
    }

    /// Whether two handles are the same Java object (`==`, and
    /// `Object.equals`).
    pub fn ptr_eq(&self, other: &Notation) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// `getClass().getName()`.
    pub fn java_class_name(&self) -> &'static str {
        match &*self.0 {
            NotationKind::Scientific(_) => "com.ibm.icu.number.ScientificNotation",
            NotationKind::Compact(_) => "com.ibm.icu.number.CompactNotation",
            NotationKind::Simple => "com.ibm.icu.number.SimpleNotation",
        }
    }

    /// `Object.hashCode()` (identity): see
    /// `java_identity_hash_code_stand_in`.
    pub fn hash_code(&self) -> i32 {
        java_identity_hash_code_stand_in(self.java_class_name())
    }

    /// Print the number using scientific notation (also known as scientific
    /// form, standard index form, or standard form in the UK). Equivalent
    /// to `scientific()`.
    pub fn scientific() -> ScientificNotation {
        SCIENTIFIC.clone()
    }

    /// Print the number using engineering notation, a variant of scientific
    /// notation in which the exponent must be divisible by 3. Equivalent to
    /// `engineering()`.
    pub fn engineering() -> ScientificNotation {
        ENGINEERING.clone()
    }

    /// Print the number using short-form compact notation. Equivalent to
    /// `compactShort()`.
    pub fn compact_short() -> CompactNotation {
        COMPACT_SHORT.clone()
    }

    /// Print the number using long-form compact notation. Equivalent to
    /// `compactLong()`.
    pub fn compact_long() -> CompactNotation {
        COMPACT_LONG.clone()
    }

    /// Print the number using simple notation without any scaling by powers
    /// of ten. This is the default behavior. Equivalent to `simple()`.
    pub fn simple() -> SimpleNotation {
        SIMPLE.clone()
    }
}
