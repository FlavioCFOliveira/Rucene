//! Numeric field statistics, ported from
//! `org.apache.lucene.search.NumericFieldStats`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::Result;
use crate::index::point_values::{doc_count, max_packed_value, min_packed_value};
use crate::index::IndexReader;

/// The value range and document count of a numeric field.
///
/// Equivalent to the record `NumericFieldStats.Stats`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    /// The smallest value indexed for the field.
    pub min: i64,
    /// The largest value indexed for the field.
    pub max: i64,
    /// The number of documents that have a value for the field.
    pub doc_count: i32,
}

/// Reads the statistics of a numeric field from the index structures that carry
/// them.
///
/// Equivalent to the `final` utility class
/// `org.apache.lucene.search.NumericFieldStats`, whose constructor is private.
#[derive(Debug, Clone, Copy)]
pub struct NumericFieldStats;

impl NumericFieldStats {
    /// Returns the statistics of `field`, from the points index when it has
    /// one and from the doc-values skip index otherwise, or `None` when neither
    /// can provide them.
    ///
    /// Equivalent to `NumericFieldStats.getStats(IndexReader, String)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the index structures.
    pub fn get_stats(reader: Arc<dyn IndexReader>, field: &str) -> Result<Option<Stats>> {
        if let Some(stats) = Self::get_stats_from_points(Arc::clone(&reader), field)? {
            return Ok(Some(stats));
        }
        Self::get_stats_from_skipper(reader, field)
    }

    /// Equivalent to the private
    /// `NumericFieldStats.getStatsFromPoints(IndexReader, String)`.
    fn get_stats_from_points(reader: Arc<dyn IndexReader>, field: &str) -> Result<Option<Stats>> {
        let min_packed = min_packed_value(Arc::clone(&reader), field)?;
        let max_packed = max_packed_value(Arc::clone(&reader), field)?;
        let (Some(min_packed), Some(max_packed)) = (min_packed, max_packed) else {
            return Ok(None);
        };
        if min_packed.len() > 8 || max_packed.len() > 8 {
            return Ok(None);
        }
        let doc_count = doc_count(reader, field)?;
        Ok(Some(Stats {
            min: decode_long(&min_packed),
            max: decode_long(&max_packed),
            doc_count,
        }))
    }

    /// Equivalent to the private
    /// `NumericFieldStats.getStatsFromSkipper(IndexReader, String)`.
    fn get_stats_from_skipper(reader: Arc<dyn IndexReader>, field: &str) -> Result<Option<Stats>> {
        let mut min: Option<i64> = None;
        let mut max: Option<i64> = None;
        let mut doc_count: i32 = 0;
        for ctx in Arc::clone(&reader).leaves() {
            let leaf_reader = ctx.leaf_reader();
            if leaf_reader.get_field_infos().field_info(field).is_none() {
                continue;
            }
            let Some(skipper) = leaf_reader.get_doc_values_skipper(field)? else {
                return Ok(None);
            };
            match (min, max) {
                (None, None) => {
                    min = Some(skipper.global_min_value());
                    max = Some(skipper.global_max_value());
                }
                _ => {
                    min = min.map(|value| value.min(skipper.global_min_value()));
                    max = max.map(|value| value.max(skipper.global_max_value()));
                }
            }
            doc_count += skipper.global_doc_count();
        }
        let (Some(min), Some(max)) = (min, max) else {
            return Ok(None);
        };
        Ok(Some(Stats {
            min,
            max,
            doc_count,
        }))
    }
}

/// Decodes a packed point value of at most eight bytes into a `long`.
///
/// Equivalent to the private `NumericFieldStats.decodeLong(byte[])`, which
/// undoes the sign flip of the leading byte and then accumulates the rest.
fn decode_long(packed: &[u8]) -> i64 {
    debug_assert!(!packed.is_empty() && packed.len() <= 8);
    let mut result = i64::from((packed[0] ^ 0x80) as i8);
    for &byte in &packed[1..] {
        result = (result << 8) | i64::from(byte);
    }
    result
}
