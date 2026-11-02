//! Nested resource accounting ported from
//! `org.apache.lucene.util.Accountables`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`Accountables`] | `Accountables` |
//! | [`NamedAccountable`] | the anonymous class returned by `Accountables.namedAccountable` |
//!
//! The tree is the one [`Accountable`] describes:
//! [`Accountable::child_resources`] lists the nested resources and
//! [`Accountable::accountable_to_string`] is the `toString()` every node is
//! labelled with.

#![deny(unsafe_code)]

use crate::util::{Accountable, RamUsageEstimator};

/// A point-in-time, type-safe view of a resource.
///
/// Port of the anonymous `Accountable` that
/// `Accountables.namedAccountable(String, Collection, long)` returns
/// (`Accountables.java:104-122`): it answers the byte count it was given, the
/// children it was given and, from `toString()`, the description it was
/// given. Consumers cannot cast it back or manipulate the underlying resource
/// in any way.
pub struct NamedAccountable<'a> {
    description: String,
    children: Vec<Box<dyn Accountable + 'a>>,
    bytes: i64,
}

impl std::fmt::Debug for NamedAccountable<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.description)
    }
}

impl std::fmt::Display for NamedAccountable<'_> {
    /// The description, as the anonymous class's `toString()` returns it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.description)
    }
}

impl NamedAccountable<'_> {
    /// Returns a view that no longer borrows the resources its children
    /// reference: each child becomes a [`NamedAccountable`] with that child's
    /// description, byte count and children, taken now.
    ///
    /// Java's view keeps references to its children; a caller that builds the
    /// view under a lock it must release before returning — as
    /// `LRUQueryCache.getChildResources()` does — cannot hand those references
    /// out in Rust, so it detaches them. The figures are the ones Java's
    /// children would report at that moment.
    pub fn detach(self) -> NamedAccountable<'static> {
        NamedAccountable {
            description: self.description,
            children: self
                .children
                .iter()
                .map(|child| Box::new(snapshot(child.as_ref())) as Box<dyn Accountable>)
                .collect(),
            bytes: self.bytes,
        }
    }
}

/// A detached description of `a`: see [`NamedAccountable::detach`].
fn snapshot(a: &dyn Accountable) -> NamedAccountable<'static> {
    NamedAccountable {
        description: a.accountable_to_string(),
        children: a
            .child_resources()
            .iter()
            .map(|child| Box::new(snapshot(child.as_ref())) as Box<dyn Accountable>)
            .collect(),
        bytes: a.ram_bytes_used(),
    }
}

impl Accountable for NamedAccountable<'_> {
    fn ram_bytes_used(&self) -> i64 {
        self.bytes
    }

    fn child_resources(&self) -> Vec<Box<dyn Accountable + '_>> {
        self.children
            .iter()
            .map(|child| Box::new(child.as_ref()) as Box<dyn Accountable + '_>)
            .collect()
    }

    fn accountable_to_string(&self) -> String {
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
    pub fn to_string(a: &dyn Accountable) -> String {
        let mut sb = String::new();
        Self::append(&mut sb, a, 0);
        sb
    }

    /// Equivalent to the private `Accountables.toString(StringBuilder, Accountable, int)`.
    fn append(dest: &mut String, a: &dyn Accountable, depth: usize) {
        for _ in 1..depth {
            dest.push_str("    ");
        }

        if depth > 0 {
            dest.push_str("|-- ");
        }

        dest.push_str(&a.accountable_to_string());
        dest.push_str(": ");
        dest.push_str(&RamUsageEstimator::human_readable_units(a.ram_bytes_used()));
        // Java appends `System.lineSeparator()`; the port always uses `\n`, so
        // that a description rendered on one platform reads the same on another.
        dest.push('\n');

        for child in a.child_resources() {
            Self::append(dest, child.as_ref(), depth + 1);
        }
    }

    /// Augments an existing accountable with the provided description.
    ///
    /// The description is built as `description [toString()]`. The result is a
    /// point-in-time, type-safe view.
    ///
    /// Equivalent to `Accountables.namedAccountable(String, Accountable)`
    /// (`Accountables.java:73-77`).
    pub fn named_accountable_of<'a>(
        description: &str,
        in_: &'a dyn Accountable,
    ) -> NamedAccountable<'a> {
        Self::named_accountable_with_children(
            &format!("{description} [{}]", in_.accountable_to_string()),
            in_.child_resources(),
            in_.ram_bytes_used(),
        )
    }

    /// Returns an accountable with the provided description and byte count.
    ///
    /// Equivalent to `Accountables.namedAccountable(String, long)`.
    pub fn named_accountable(description: &str, bytes: i64) -> NamedAccountable<'static> {
        Self::named_accountable_with_children(description, Vec::new(), bytes)
    }

    /// Returns an accountable with the provided description, children and byte
    /// count.
    ///
    /// Equivalent to
    /// `Accountables.namedAccountable(String, Collection, long)`.
    pub fn named_accountable_with_children<'a>(
        description: &str,
        children: Vec<Box<dyn Accountable + 'a>>,
        bytes: i64,
    ) -> NamedAccountable<'a> {
        NamedAccountable {
            description: description.to_string(),
            children,
            bytes,
        }
    }

    /// Converts the entries of a map of resources into a list of named
    /// accountables sorted by description.
    ///
    /// Each description is built as `prefix 'key' [toString()]`.
    ///
    /// Equivalent to `Accountables.namedAccountables(String, Map)`
    /// (`Accountables.java:92-100`): `entries` is the map's entry set, in its
    /// iteration order, and the result is sorted with Java's stable
    /// `List.sort` on each description. Java's result is an unmodifiable
    /// list; Rust returns an owned `Vec`, which the caller cannot use to
    /// mutate the inputs either.
    pub fn named_accountables<'a, K: std::fmt::Display>(
        prefix: &str,
        entries: impl IntoIterator<Item = (K, &'a dyn Accountable)>,
    ) -> Vec<NamedAccountable<'a>> {
        let mut resources: Vec<NamedAccountable<'a>> = entries
            .into_iter()
            .map(|(k, v)| Self::named_accountable_of(&format!("{prefix} '{k}'"), v))
            .collect();
        resources.sort_by(|o1, o2| o1.description.cmp(&o2.description));
        resources
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Accountables.toString` renders each size with
    /// `RamUsageEstimator.humanReadableUnits` (`Accountables.java:58`), which
    /// rounds 1280 bytes (exactly 1.25 KB) half to even.
    #[test]
    fn to_string_renders_sizes_with_human_readable_units() {
        let child: Box<dyn Accountable> = Box::new(Accountables::named_accountable("leaf", 1536));
        let root = Accountables::named_accountable_with_children("root", vec![child], 1280);
        assert_eq!(
            Accountables::to_string(&root),
            "root: 1.2 KB\n|-- leaf: 1.5 KB\n"
        );
    }
}
