/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `PassageFormatter.java`, of the Java package
//! `org.apache.lucene.search.matchhighlight`.

use std::fmt;
use std::sync::Arc;

use rucene::error::{LuceneError, LuceneErrorKind, Result};

use crate::java::units;
use crate::matchhighlight::offset_range::OffsetRange;
use crate::matchhighlight::passage::Passage;
use crate::matchhighlight::passage_selector::PassageSelector;

/// `Function<OffsetRange, String>`: the marker text of a range.
pub type MarkerFunction = Arc<dyn Fn(&OffsetRange) -> String + Send + Sync>;

/// Formats a collection of passages over a given string, cleanly
/// leaving highlight markers as tags (or other formatting).
///
/// Equivalent to `org.apache.lucene.search.matchhighlight.PassageFormatter`.
/// Java keeps its marker stack in a field, cleared on every use; the port
/// keeps it local to the call.
pub struct PassageFormatter {
    ellipsis: String,
    marker_start: MarkerFunction,
    marker_end: MarkerFunction,
}

impl fmt::Debug for PassageFormatter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PassageFormatter")
            .field("ellipsis", &self.ellipsis)
            .finish_non_exhaustive()
    }
}

impl PassageFormatter {
    /// Equivalent to `new PassageFormatter(String, String, String)`.
    pub fn new(ellipsis: &str, marker_start: &str, marker_end: &str) -> Self {
        let start = marker_start.to_string();
        let end = marker_end.to_string();
        Self::with_functions(
            ellipsis,
            Arc::new(move |_: &OffsetRange| start.clone()),
            Arc::new(move |_: &OffsetRange| end.clone()),
        )
    }

    /// Equivalent to `new PassageFormatter(String, Function, Function)`.
    pub fn with_functions(
        ellipsis: &str,
        marker_start: MarkerFunction,
        marker_end: MarkerFunction,
    ) -> Self {
        Self {
            ellipsis: ellipsis.to_string(),
            marker_start,
            marker_end,
        }
    }

    /// Equivalent to `format(CharSequence, List, List)`.
    ///
    /// # Errors
    ///
    /// Java's `AssertionError` (under `debug_assertions`) for unsorted
    /// passages or ranges, or passages outside the value.
    pub fn format(
        &self,
        value: &str,
        passages: &[Passage],
        ranges: &[OffsetRange],
    ) -> Result<Vec<String>> {
        let value = units(value);
        if cfg!(debug_assertions) {
            PassageSelector::sorted_and_non_overlapping(passages)?;
            PassageSelector::sorted_and_non_overlapping(ranges)?;
            Self::within_range(&OffsetRange::new(0, value.len() as i32), passages)?;
        }

        if ranges.is_empty() {
            return Ok(Vec::new());
        }

        let mut result: Vec<String> = Vec::new();
        let mut buf: Vec<u16> = Vec::new();

        let mut range_index = 0;
        let mut range = &ranges[range_index];
        'passage_formatting: for passage in passages {
            // Move to the range of the current passage.
            while passage.from >= range.to {
                range_index += 1;
                if range_index == ranges.len() {
                    break 'passage_formatting;
                }
                range = &ranges[range_index];
            }

            debug_assert!(
                range.from <= passage.from && range.to >= passage.to,
                "{range} ? {passage}"
            );

            buf.clear();
            if range.from < passage.from {
                buf.extend(self.ellipsis.encode_utf16());
            }
            self.format_passage_units(&mut buf, &value, passage)?;
            if range.to > passage.to {
                buf.extend(self.ellipsis.encode_utf16());
            }
            result.push(String::from_utf16_lossy(&buf));
        }
        Ok(result)
    }

    fn within_range(limits: &OffsetRange, contained: &[Passage]) -> Result<bool> {
        for r in contained {
            if r.from < limits.from || r.to > limits.to {
                return Err(LuceneError::from(LuceneErrorKind::AssertionError(format!(
                    "Range outside of the permitted limit (limit = {limits}): {r}"
                ))));
            }
        }
        Ok(true)
    }

    /// Equivalent to `format(StringBuilder, CharSequence, Passage)`: appends
    /// the formatted passage to `buf`.
    ///
    /// # Errors
    ///
    /// Java's `StringIndexOutOfBoundsException` for a passage or marker
    /// outside the value.
    pub fn format_passage(&self, buf: &mut String, value: &str, passage: &Passage) -> Result<()> {
        let mut out: Vec<u16> = Vec::new();
        self.format_passage_units(&mut out, &units(value), passage)?;
        buf.push_str(&String::from_utf16_lossy(&out));
        Ok(())
    }

    fn append(buf: &mut Vec<u16>, value: &[u16], start: i32, end: i32) -> Result<()> {
        if start < 0 || start > end || end > value.len() as i32 {
            return Err(LuceneError::from(LuceneErrorKind::StringIndexOutOfBounds(
                format!("start {start}, end {end}, length {}", value.len()),
            )));
        }
        buf.extend_from_slice(&value[start as usize..end as usize]);
        Ok(())
    }

    fn format_passage_units(
        &self,
        buf: &mut Vec<u16>,
        value: &[u16],
        passage: &Passage,
    ) -> Result<()> {
        match passage.markers.len() {
            0 => {
                // No markers, full passage appended.
                Self::append(buf, value, passage.from, passage.to)?;
            }
            1 => {
                // One marker, trivial and frequent case so it's handled
                // separately.
                let m = &passage.markers[0];
                Self::append(buf, value, passage.from, m.from)?;
                buf.extend((self.marker_start)(m).encode_utf16());
                Self::append(buf, value, m.from, m.to)?;
                buf.extend((self.marker_end)(m).encode_utf16());
                Self::append(buf, value, m.to, passage.to)?;
            }
            _ => {
                // Multiple markers, possibly overlapping or nested.
                let mut marker_stack: Vec<OffsetRange> = Vec::new();
                self.multiple_markers(value, passage, buf, &mut marker_stack)?;
            }
        }
        Ok(())
    }

    /// Handle multiple markers that may have overlapping ranges.
    fn multiple_markers(
        &self,
        value: &[u16],
        p: &Passage,
        b: &mut Vec<u16>,
        marker_stack: &mut Vec<OffsetRange>,
    ) -> Result<()> {
        let mut at = p.from;
        let max = p.to;
        let slice_points = Self::slice_points(p);
        for slice_point in &slice_points {
            Self::append(b, value, at, slice_point.offset)?;
            let current_marker = &slice_point.marker;
            match slice_point.r#type {
                Type::START => {
                    marker_stack.push(current_marker.clone());
                    b.extend((self.marker_start)(current_marker).encode_utf16());
                }
                Type::END => {
                    let marker_index = marker_stack
                        .iter()
                        .rposition(|m| m == current_marker)
                        .ok_or_else(|| {
                            LuceneError::unchecked(
                                "java.lang.ArrayIndexOutOfBoundsException",
                                "Index -1 out of bounds",
                            )
                        })?;
                    for k in marker_index..marker_stack.len() {
                        b.extend((self.marker_end)(&marker_stack[k]).encode_utf16());
                    }
                    marker_stack.remove(marker_index);
                    for k in marker_index..marker_stack.len() {
                        b.extend((self.marker_start)(&marker_stack[k]).encode_utf16());
                    }
                }
            }
            at = slice_point.offset;
        }

        if at < max {
            Self::append(b, value, at, max)?;
        }
        Ok(())
    }

    fn slice_points(p: &Passage) -> Vec<SlicePoint> {
        let mut slice_points: Vec<SlicePoint> = Vec::with_capacity(p.markers.len() * 2);
        for m in &p.markers {
            slice_points.push(SlicePoint::new(Type::START, m.from, m.clone()));
            slice_points.push(SlicePoint::new(Type::END, m.to, m.clone()));
        }

        // Order slice points by their offset
        slice_points.sort_by(|a, b| {
            a.offset
                .cmp(&b.offset)
                .then(a.r#type.ordering().cmp(&b.r#type.ordering()))
                .then_with(|| {
                    if a.r#type == Type::START {
                        b.marker.to.cmp(&a.marker.to)
                    } else {
                        b.marker.from.cmp(&a.marker.from)
                    }
                })
        });

        slice_points
    }
}

/// Equivalent to the package-private nested class
/// `PassageFormatter.SlicePoint`.
#[derive(Debug, Clone)]
pub(crate) struct SlicePoint {
    pub(crate) offset: i32,
    pub(crate) r#type: Type,
    pub(crate) marker: OffsetRange,
}

impl SlicePoint {
    fn new(t: Type, offset: i32, m: OffsetRange) -> Self {
        Self {
            r#type: t,
            offset,
            marker: m,
        }
    }
}

impl fmt::Display for SlicePoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({:?}, {})", self.r#type, self.marker)
    }
}

/// Equivalent to the nested enum `PassageFormatter.SlicePoint.Type`.
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Type {
    START,
    END,
}

impl Type {
    fn ordering(self) -> i32 {
        match self {
            Type::START => 2,
            Type::END => 1,
        }
    }
}
