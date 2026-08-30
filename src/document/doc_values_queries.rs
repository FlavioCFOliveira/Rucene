//! Doc-values range and set queries, ported from `org.apache.lucene.document`.
//!
//! These answer a range or a set membership by scanning doc values, which beats
//! a points lookup when the field is already being read for another reason, or
//! when no points index exists.

use crate::util::BytesRef;

/// Sentinel standing for a value the table cannot hold, which is also the
/// smallest long.
const MISSING: i64 = i64::MIN;

/// An open-addressed set of longs, built once from a sorted array.
///
/// Equivalent to `org.apache.lucene.document.DocValuesLongHashSet`, which a set
/// query uses to test membership without a per-document allocation.
#[derive(Clone, Debug)]
pub struct DocValuesLongHashSet {
    table: Vec<i64>,
    mask: usize,
    /// Whether `i64::MIN` itself is a member, which the table cannot store.
    has_missing_value: bool,
    size: usize,
    min_value: i64,
    max_value: i64,
}

impl DocValuesLongHashSet {
    /// Builds the set from `values`, which must be sorted ascending.
    ///
    /// The table is sized at 1.5x the input and rounded up to a power of two,
    /// so the linear probe below stays short.
    pub fn new(values: &[i64]) -> Self {
        let wanted = (values.len() as i64 * 3 / 2).max(1) as usize;
        let table_size = wanted.next_power_of_two();
        let mut set = Self {
            table: vec![MISSING; table_size],
            mask: table_size - 1,
            has_missing_value: false,
            size: 0,
            min_value: i64::MAX,
            max_value: i64::MIN,
        };
        for &value in values {
            if value == MISSING {
                if !set.has_missing_value {
                    set.size += 1;
                }
                set.has_missing_value = true;
            } else if set.insert(value) {
                set.size += 1;
            }
        }
        if !values.is_empty() {
            set.min_value = values[0];
            set.max_value = values[values.len() - 1];
        }
        set
    }

    /// Inserts `value`, returning whether it was new.
    fn insert(&mut self, value: i64) -> bool {
        let mut slot = (hash_long(value) as usize) & self.mask;
        loop {
            if self.table[slot] == MISSING {
                self.table[slot] = value;
                return true;
            }
            if self.table[slot] == value {
                return false;
            }
            slot = (slot + 1) & self.mask;
        }
    }

    /// Returns whether `value` is a member.
    ///
    /// Equivalent to `DocValuesLongHashSet.contains(long)`.
    pub fn contains(&self, value: i64) -> bool {
        if value == MISSING {
            return self.has_missing_value;
        }
        if value < self.min_value || value > self.max_value {
            return false;
        }
        let mut slot = (hash_long(value) as usize) & self.mask;
        loop {
            match self.table[slot] {
                MISSING => return false,
                v if v == value => return true,
                _ => slot = (slot + 1) & self.mask,
            }
        }
    }

    /// Returns how many values the set holds.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Returns the smallest member.
    pub fn min_value(&self) -> i64 {
        self.min_value
    }

    /// Returns the largest member.
    pub fn max_value(&self) -> i64 {
        self.max_value
    }
}

/// Hashes a long the way `Long.hashCode` does, so the probe sequence matches
/// Java's.
fn hash_long(value: i64) -> i32 {
    ((value ^ (value >> 32)) & 0xFFFF_FFFF) as i32
}

/// A range query over a numeric doc-values field.
///
/// Equivalent to
/// `org.apache.lucene.document.SortedNumericDocValuesRangeQuery`.
///
/// **Divergence from Lucene 10.5.0.** Java's query is a `Query` that builds a
/// `ScorerSupplier` over the segment's doc values, using the doc-values skipper
/// when the field has one. This port carries the predicate — the field, the
/// bounds, and the per-value test — because the `Query`/`Weight`/`Scorer`
/// hierarchy it plugs into is not ported yet.
#[derive(Clone, Debug)]
pub struct SortedNumericDocValuesRangeQuery {
    field: String,
    lower_value: i64,
    upper_value: i64,
}

impl SortedNumericDocValuesRangeQuery {
    /// Creates the query over `[lower_value, upper_value]`, inclusive.
    pub fn new(field: impl Into<String>, lower_value: i64, upper_value: i64) -> Self {
        Self {
            field: field.into(),
            lower_value,
            upper_value,
        }
    }

    /// Returns the field the query reads.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns the lower bound.
    pub fn lower_value(&self) -> i64 {
        self.lower_value
    }

    /// Returns the upper bound.
    pub fn upper_value(&self) -> i64 {
        self.upper_value
    }

    /// Returns whether the query matches nothing, which an inverted range does.
    pub fn matches_no_docs(&self) -> bool {
        self.lower_value > self.upper_value
    }

    /// Returns whether `value` falls in the range.
    pub fn matches(&self, value: i64) -> bool {
        value >= self.lower_value && value <= self.upper_value
    }
}

/// A set-membership query over a numeric doc-values field.
///
/// Equivalent to `org.apache.lucene.document.SortedNumericDocValuesSetQuery`.
/// The same divergence as the range query above applies.
#[derive(Clone, Debug)]
pub struct SortedNumericDocValuesSetQuery {
    field: String,
    values: DocValuesLongHashSet,
}

impl SortedNumericDocValuesSetQuery {
    /// Creates the query from `values`, which must be sorted ascending.
    pub fn new(field: impl Into<String>, values: &[i64]) -> Self {
        Self {
            field: field.into(),
            values: DocValuesLongHashSet::new(values),
        }
    }

    /// Returns the field the query reads.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns whether `value` is in the set.
    pub fn matches(&self, value: i64) -> bool {
        self.values.contains(value)
    }

    /// Returns the set the query tests against.
    pub fn values(&self) -> &DocValuesLongHashSet {
        &self.values
    }
}

/// A range query over a sorted-set doc-values field, comparing terms as bytes.
///
/// Equivalent to `org.apache.lucene.document.SortedSetDocValuesRangeQuery`.
/// The same divergence as the numeric range query above applies.
#[derive(Clone, Debug)]
pub struct SortedSetDocValuesRangeQuery {
    field: String,
    lower_value: Option<BytesRef>,
    upper_value: Option<BytesRef>,
    lower_inclusive: bool,
    upper_inclusive: bool,
}

impl SortedSetDocValuesRangeQuery {
    /// Creates the query. A `None` bound is open on that side.
    pub fn new(
        field: impl Into<String>,
        lower_value: Option<BytesRef>,
        upper_value: Option<BytesRef>,
        lower_inclusive: bool,
        upper_inclusive: bool,
    ) -> Self {
        Self {
            field: field.into(),
            // An exclusive open bound is the same as an inclusive one, as Java
            // normalises in its constructor.
            lower_inclusive: lower_inclusive || lower_value.is_none(),
            upper_inclusive: upper_inclusive || upper_value.is_none(),
            lower_value,
            upper_value,
        }
    }

    /// Returns the field the query reads.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns whether `term` falls in the range.
    pub fn matches(&self, term: &BytesRef) -> bool {
        if let Some(lower) = &self.lower_value {
            let cmp = term.slice().cmp(lower.slice());
            let ok = if self.lower_inclusive {
                cmp != std::cmp::Ordering::Less
            } else {
                cmp == std::cmp::Ordering::Greater
            };
            if !ok {
                return false;
            }
        }
        if let Some(upper) = &self.upper_value {
            let cmp = term.slice().cmp(upper.slice());
            let ok = if self.upper_inclusive {
                cmp != std::cmp::Ordering::Greater
            } else {
                cmp == std::cmp::Ordering::Less
            };
            if !ok {
                return false;
            }
        }
        true
    }
}

/// Decides whether a doc-values range query can use the field's skipper, and
/// what it can skip.
///
/// Equivalent to `org.apache.lucene.document.SortedSkipperScorerSupplier`,
/// which reads the per-block minimum and maximum a doc-values skipper records
/// and drops a block whose whole range falls outside the query's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipperRelation {
    /// Every document of the block matches, so no value need be read.
    AllMatch,
    /// No document of the block matches, so the block can be skipped whole.
    NoneMatch,
    /// Some may match, so the block must be scanned.
    SomeMatch,
}

/// Chooses how a block relates to a query range from the block's own bounds.
///
/// Equivalent to the block-level decision inside `SortedSkipperScorerSupplier`.
pub fn skipper_relation(
    block_min: i64,
    block_max: i64,
    query_min: i64,
    query_max: i64,
) -> SkipperRelation {
    if block_min > query_max || block_max < query_min {
        SkipperRelation::NoneMatch
    } else if block_min >= query_min && block_max <= query_max {
        SkipperRelation::AllMatch
    } else {
        SkipperRelation::SomeMatch
    }
}

/// Returns the sorted-set ordinals a range spans, given the field's term
/// dictionary bounds.
///
/// Equivalent to the ordinal lookup `SortedSetDocValuesRangeQuery` performs
/// before it scans: a range that spans no ordinal matches nothing.
pub fn ordinal_range(min_ord: i64, max_ord: i64) -> Option<(i64, i64)> {
    if min_ord > max_ord {
        None
    } else {
        Some((min_ord, max_ord))
    }
}
