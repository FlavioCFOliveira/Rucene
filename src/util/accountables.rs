//! Nested resource accounting ported from
//! `org.apache.lucene.util.Accountables`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`AccountableTree`] | the `Accountable.getChildResources()` half of `Accountable` |
//! | [`Accountables`] | `Accountables` |
//! | [`NamedAccountable`] | the anonymous class returned by `Accountables.namedAccountable` |
//!
//! **Divergence from Lucene 10.5.0.** Java labels every node of the tree with
//! `Accountable.toString()`, which every Java object has. Rust has no universal
//! `toString`, and Rucene's [`Accountable`] carries no `Display` bound, so the
//! label is declared by [`AccountableTree`], a sub-trait that also re-declares
//! the child list as owned `Arc`s. The re-declaration is forced: Rucene's
//! [`Accountable::child_resources`] hands back `&dyn Accountable`, and before
//! Rust 1.86 a `&dyn AccountableTree` cannot be coerced back to its supertrait
//! object, so a described tree cannot be expressed through it. The method is
//! therefore named [`AccountableTree::described_child_resources`], distinct
//! from the inherited one, and the two coexist: a [`NamedAccountable`] reports
//! its children through the described accessor, and the inherited accessor
//! keeps its empty default.

#![deny(unsafe_code)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use crate::util::{Accountable, RamUsageEstimator};

/// An [`Accountable`] that can enumerate the resources nested inside it.
///
/// Port of `Accountable.getChildResources()` plus the `toString()` every
/// `Accountables` helper relies on to label a node. The `Send + Sync` bound is
/// not Lucene's: it is what makes the `Arc` children shareable, which a Java
/// reference is by default.
pub trait AccountableTree: Accountable + Send + Sync {
    /// Returns the nested resources of this instance, or an empty list.
    ///
    /// Equivalent to `Accountable.getChildResources()`, whose Java default is
    /// `Collections.emptyList()`.
    fn described_child_resources(&self) -> Vec<Arc<dyn AccountableTree>> {
        Vec::new()
    }

    /// Returns the label this resource is rendered with.
    ///
    /// Equivalent to what `Accountables.toString` obtains from
    /// `Accountable.toString()`.
    fn description(&self) -> String;
}

/// A point-in-time, type-safe view of a resource.
///
/// Port of the anonymous `Accountable` that
/// `Accountables.namedAccountable(String, Collection, long)` returns: consumers
/// cannot cast it back or manipulate the underlying resource in any way.
pub struct NamedAccountable {
    description: String,
    children: Vec<Arc<dyn AccountableTree>>,
    bytes: i64,
}

impl std::fmt::Debug for NamedAccountable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.description)
    }
}

impl Accountable for NamedAccountable {
    fn ram_bytes_used(&self) -> i64 {
        self.bytes
    }
}

impl AccountableTree for NamedAccountable {
    fn described_child_resources(&self) -> Vec<Arc<dyn AccountableTree>> {
        self.children.clone()
    }

    fn description(&self) -> String {
        self.description.clone()
    }
}

/// Helpers for building nested resource descriptions and debugging RAM usage.
///
/// Port of `org.apache.lucene.util.Accountables`.
pub struct Accountables;

impl Accountables {
    /// Returns a description of an accountable and its nested resources,
    /// intended for development and debugging.
    ///
    /// Equivalent to `Accountables.toString(Accountable)`.
    pub fn to_string(a: &dyn AccountableTree) -> String {
        let mut sb = String::new();
        Self::append(&mut sb, a, 0);
        sb
    }

    /// Equivalent to the private `Accountables.toString(StringBuilder, Accountable, int)`.
    fn append(dest: &mut String, a: &dyn AccountableTree, depth: usize) {
        for _ in 1..depth {
            dest.push_str("    ");
        }

        if depth > 0 {
            dest.push_str("|-- ");
        }

        dest.push_str(&a.description());
        dest.push_str(": ");
        dest.push_str(&Self::human_readable_units(a.ram_bytes_used()));
        // Java appends `System.lineSeparator()`; the port always uses `\n`, so
        // that a description rendered on one platform reads the same on another.
        dest.push('\n');

        for child in a.described_child_resources() {
            Self::append(dest, child.as_ref(), depth + 1);
        }
    }

    /// Returns a human-readable byte count such as `1.2 MB`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java calls
    /// `RamUsageEstimator.humanReadableUnits(long)`; Rucene's
    /// [`RamUsageEstimator`] port does not expose it, so the formatting lives
    /// here. The thresholds, the at-most-one-decimal rendering and the unit
    /// names match Lucene, which formats with `Locale.ROOT` and the pattern
    /// `0.#`; the only difference is the rounding of the discarded digits,
    /// half-away-from-zero here against `DecimalFormat`'s half-to-even.
    pub fn human_readable_units(bytes: i64) -> String {
        if bytes / RamUsageEstimator::ONE_GB > 0 {
            Self::format_one_decimal(bytes as f64 / RamUsageEstimator::ONE_GB as f64, "GB")
        } else if bytes / RamUsageEstimator::ONE_MB > 0 {
            Self::format_one_decimal(bytes as f64 / RamUsageEstimator::ONE_MB as f64, "MB")
        } else if bytes / RamUsageEstimator::ONE_KB > 0 {
            Self::format_one_decimal(bytes as f64 / RamUsageEstimator::ONE_KB as f64, "KB")
        } else {
            format!("{bytes} bytes")
        }
    }

    /// `DecimalFormat("0.#")`: one optional decimal digit.
    fn format_one_decimal(value: f64, unit: &str) -> String {
        let rounded = (value * 10.0).round() / 10.0;
        let mut out = String::new();
        if (rounded - rounded.trunc()).abs() < f64::EPSILON {
            let _ = write!(out, "{} {unit}", rounded.trunc() as i64);
        } else {
            let _ = write!(out, "{rounded:.1} {unit}");
        }
        out
    }

    /// Augments an existing accountable with the provided description.
    ///
    /// The description is built as `description [toString()]`. The result is a
    /// point-in-time, type-safe view.
    ///
    /// Equivalent to `Accountables.namedAccountable(String, Accountable)`.
    pub fn named_accountable_of(description: &str, in_: &dyn AccountableTree) -> NamedAccountable {
        Self::named_accountable_with_children(
            &format!("{description} [{}]", in_.description()),
            in_.described_child_resources(),
            in_.ram_bytes_used(),
        )
    }

    /// Returns an accountable with the provided description and byte count.
    ///
    /// Equivalent to `Accountables.namedAccountable(String, long)`.
    pub fn named_accountable(description: &str, bytes: i64) -> NamedAccountable {
        Self::named_accountable_with_children(description, Vec::new(), bytes)
    }

    /// Returns an accountable with the provided description, children and byte
    /// count.
    ///
    /// Equivalent to
    /// `Accountables.namedAccountable(String, Collection, long)`.
    pub fn named_accountable_with_children(
        description: &str,
        children: Vec<Arc<dyn AccountableTree>>,
        bytes: i64,
    ) -> NamedAccountable {
        NamedAccountable {
            description: description.to_string(),
            children,
            bytes,
        }
    }

    /// Converts a map of resources into a list of named accountables sorted by
    /// description.
    ///
    /// Each description is built as `prefix 'key' [toString()]`.
    ///
    /// Equivalent to `Accountables.namedAccountables(String, Map)`. Java's
    /// result is an unmodifiable list; Rust returns an owned `Vec`, which the
    /// caller cannot use to mutate the inputs either.
    pub fn named_accountables<K: std::fmt::Display + Ord>(
        prefix: &str,
        in_: &BTreeMap<K, Arc<dyn AccountableTree>>,
    ) -> Vec<Arc<dyn AccountableTree>> {
        let mut resources: Vec<Arc<dyn AccountableTree>> = in_
            .iter()
            .map(|(k, v)| {
                let named = Self::named_accountable_of(&format!("{prefix} '{k}'"), v.as_ref());
                Arc::new(named) as Arc<dyn AccountableTree>
            })
            .collect();
        resources.sort_by_key(|a| a.description());
        resources
    }
}
