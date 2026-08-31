//! Numeric doc-values range queries, ported from
//! `org.apache.lucene.search.NumericDocValuesRangeQuery`.

#![deny(unsafe_code)]

/// The state an inclusive numeric doc-values range query carries.
///
/// Equivalent to `org.apache.lucene.search.NumericDocValuesRangeQuery`, an
/// abstract class that adds three fields and their accessors to `Query` and
/// leaves every `Query` method abstract. Rust has no implementation
/// inheritance, so a concrete range query — such as the one
/// `SortedNumericDocValuesField.newSlowRangeQuery` builds — embeds this state
/// and implements [`Query`](crate::search::Query) itself.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NumericDocValuesRangeQuery {
    field: String,
    lower_value: i64,
    upper_value: i64,
}

impl NumericDocValuesRangeQuery {
    /// Creates the state of a range query over `field`, both bounds inclusive.
    ///
    /// Equivalent to the protected
    /// `NumericDocValuesRangeQuery(String, long, long)` constructor; Java's
    /// `Objects.requireNonNull` on the field is unnecessary here.
    pub fn new(field: &str, lower_value: i64, upper_value: i64) -> Self {
        Self {
            field: field.to_string(),
            lower_value,
            upper_value,
        }
    }

    /// Returns the field this query ranges over.
    ///
    /// Equivalent to the `final NumericDocValuesRangeQuery.getField()`.
    pub fn get_field(&self) -> &str {
        &self.field
    }

    /// Returns the inclusive lower bound.
    ///
    /// Equivalent to the `final NumericDocValuesRangeQuery.lowerValue()`.
    pub fn lower_value(&self) -> i64 {
        self.lower_value
    }

    /// Returns the inclusive upper bound.
    ///
    /// Equivalent to the `final NumericDocValuesRangeQuery.upperValue()`.
    pub fn upper_value(&self) -> i64 {
        self.upper_value
    }
}
