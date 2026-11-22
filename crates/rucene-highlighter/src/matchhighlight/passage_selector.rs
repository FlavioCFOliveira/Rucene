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

//! Port of `PassageSelector.java`, of the Java package
//! `org.apache.lucene.search.matchhighlight`.

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::util::extra::{PriorityQueue, PriorityQueueComparator};

use crate::java::utf16_len;
use crate::matchhighlight::offset_range::{HasOffsetRange, OffsetRange};
use crate::matchhighlight::passage::Passage;
use crate::matchhighlight::passage_adjuster::PassageAdjuster;

/// `Comparator<Passage>`: the passage scorer.
pub type PassageScorer = Arc<dyn Fn(&Passage, &Passage) -> Ordering + Send + Sync>;

/// Equivalent to `PassageSelector.DEFAULT_SCORER`.
pub fn default_scorer() -> PassageScorer {
    Arc::new(|a: &Passage, b: &Passage| {
        // Compare the number of highlights first.
        let v = a.markers.len().cmp(&b.markers.len());
        if v != Ordering::Equal {
            return v;
        }

        // Total number of characters covered by the highlights.
        let mut len1 = 0i32;
        let mut len2 = 0i32;
        for o in &a.markers {
            len1 = len1.wrapping_add(o.length());
        }
        for o in &b.markers {
            len2 = len2.wrapping_add(o.length());
        }
        if len1 != len2 {
            return len1.cmp(&len2);
        }

        b.from.cmp(&a.from)
    })
}

/// The anonymous `PriorityQueue<Passage>` order of `pickBest`.
struct ScorerLessThan(PassageScorer);

impl PriorityQueueComparator<Passage> for ScorerLessThan {
    fn less_than(&self, a: &Passage, b: &Passage) -> bool {
        (self.0)(a, b) == Ordering::Less
    }
}

/// Selects fragments of text that score best for the given set of
/// highlight markers.
///
/// Equivalent to `org.apache.lucene.search.matchhighlight.PassageSelector`.
pub struct PassageSelector {
    passage_scorer: PassageScorer,
    passage_adjuster: Option<Arc<dyn PassageAdjuster>>,
}

impl fmt::Debug for PassageSelector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PassageSelector")
            .field("passage_adjuster", &self.passage_adjuster)
            .finish_non_exhaustive()
    }
}

impl Default for PassageSelector {
    fn default() -> Self {
        Self::new()
    }
}

fn to_int_exact(v: i64) -> Result<i32> {
    i32::try_from(v).map_err(|_| LuceneError::from(rucene::error::LuceneErrorKind::Arithmetic("integer overflow".to_string())))
}

impl PassageSelector {
    /// Equivalent to `new PassageSelector()`.
    pub fn new() -> Self {
        Self::with_scorer(default_scorer(), None)
    }

    /// Equivalent to `new PassageSelector(Comparator, PassageAdjuster)`;
    /// `passage_adjuster` is `None` for Java's `null`.
    pub fn with_scorer(
        passage_scorer: PassageScorer,
        passage_adjuster: Option<Arc<dyn PassageAdjuster>>,
    ) -> Self {
        Self {
            passage_scorer,
            passage_adjuster,
        }
    }

    /// Equivalent to `pickBest(CharSequence, List, int, int)`.
    ///
    /// # Errors
    ///
    /// As [`pick_best_in_ranges`](Self::pick_best_in_ranges).
    pub fn pick_best(
        &self,
        value: &str,
        markers: &[OffsetRange],
        max_passage_window: i32,
        max_passages: i32,
    ) -> Result<Vec<Passage>> {
        self.pick_best_in_ranges(
            value,
            markers,
            max_passage_window,
            max_passages,
            &[OffsetRange::new(0, utf16_len(value))],
        )
    }

    /// Equivalent to `pickBest(CharSequence, List, int, int, List)`:
    /// `value` is the input text; `markers` the offset ranges to highlight;
    /// `max_passage_window` the maximum passage size; `max_passages` the
    /// number of passages to return; `permitted_passage_ranges` the
    /// sorted, non-overlapping ranges passages may come from.
    ///
    /// # Errors
    ///
    /// Java's `AssertionError` for unsorted or overlapping permitted ranges
    /// (under `debug_assertions`), `ArithmeticException` on an offset
    /// overflow, and what the adjuster raises.
    pub fn pick_best_in_ranges(
        &self,
        value: &str,
        markers: &[OffsetRange],
        max_passage_window: i32,
        max_passages: i32,
        permitted_passage_ranges: &[OffsetRange],
    ) -> Result<Vec<Passage>> {
        if cfg!(debug_assertions) {
            Self::sorted_and_non_overlapping(permitted_passage_ranges)?;
        }

        let value_length = utf16_len(value);
        // If there are no markers, just return the default passage.
        if value_length == 0 || max_passage_window == 0 {
            return Ok(Vec::new());
        }

        // Best passages so far.
        let pq_size = 16.max(max_passages).max(0) as usize;
        let mut pq: PriorityQueue<Passage, ScorerLessThan> =
            PriorityQueue::new(pq_size, ScorerLessThan(Arc::clone(&self.passage_scorer)))?;

        let mut markers = Self::split_or_truncate_to_windows(markers, max_passage_window, permitted_passage_ranges);

        // Sort markers by their start offset, shortest first.
        markers.sort_by(|a, b| a.from.cmp(&b.from).then(a.to.cmp(&b.to)));

        let max = markers.len();
        let mut marker_index = 0;
        'next_range: for range in permitted_passage_ranges {
            let range_to = range.to.min(value_length);
            if range.from >= range_to {
                continue;
            }

            while marker_index < max {
                let m = &markers[marker_index];

                // Markers are sorted so if the current marker's start is
                // past the range, we can advance, but we need to check the
                // same marker against the new range.
                if m.from >= range_to {
                    continue 'next_range;
                }

                // Check if current marker falls within the range and is
                // smaller than the largest allowed passage size.
                if m.from >= range.from && m.to <= range_to && m.length() <= max_passage_window {
                    // Adjust the window range to center the highlight
                    // marker.
                    let mut from = to_int_exact(
                        (i64::from(m.from) + i64::from(m.to) - i64::from(max_passage_window)) / 2,
                    )?;
                    let mut to = to_int_exact(
                        (i64::from(m.from) + i64::from(m.to) + i64::from(max_passage_window)) / 2,
                    )?;
                    if from < range.from {
                        to += range.from - from;
                        from = range.from;
                    }
                    if to > range_to {
                        from -= to - range_to;
                        to = range_to;
                        if from < range.from {
                            from = range.from;
                        }
                    }

                    if from < to && to <= value_length {
                        // Find other markers that are completely inside the
                        // passage.
                        let mut inside: Vec<OffsetRange> = Vec::new();
                        let mut i = marker_index;
                        while i > 0 && markers[i - 1].from >= from {
                            i -= 1;
                        }

                        while i < max {
                            let c = &markers[i];
                            if c.from >= to {
                                break;
                            }
                            if c.to <= to {
                                inside.push(c.clone());
                            }
                            i += 1;
                        }

                        if !inside.is_empty() {
                            pq.insert_with_overflow(Passage::new(from, to, inside));
                        }
                    }
                }

                // Advance to the next marker.
                marker_index += 1;
            }
        }

        // Collect from the priority queue (reverse the order so that we're
        // right-to-left).
        let mut passages: Vec<Option<Passage>> = if pq.size() > 0 {
            let mut passages: Vec<Option<Passage>> = vec![None; pq.size()];
            for i in (0..pq.size()).rev() {
                passages[i] = pq.pop();
            }
            passages
        } else {
            // Handle the default, no highlighting markers case.
            self.pick_default_passage(value, max_passage_window, max_passages, permitted_passage_ranges)
                .into_iter()
                .map(Some)
                .collect()
        };

        // Correct passage boundaries from maxExclusive window. Typically
        // shrink boundaries until we're on a proper word/sentence boundary.
        if let Some(adjuster) = &self.passage_adjuster {
            adjuster.current_value(value);
            for slot in passages.iter_mut() {
                let p = slot.as_ref().expect("INVARIANT: no passage is removed yet");
                let new_range = adjuster.adjust(p)?;
                if new_range.from != p.from || new_range.to != p.to {
                    debug_assert!(
                        new_range.from >= p.from && new_range.to <= p.to,
                        "Adjusters must not expand the passage's range: was {p} => changed to {new_range}"
                    );
                    let markers = p.markers.clone();
                    *slot = Some(Passage::new(new_range.from, new_range.to, markers));
                }
            }
        }

        // Ensure there are no overlaps on passages. In case of conflicts,
        // better score wins.
        let mut last = 0;
        for i in 0..passages.len() {
            let Some(a) = passages[i].clone() else {
                continue;
            };
            if a.length() > 0 {
                passages[last] = Some(a.clone());
                last += 1;
                for j in i + 1..passages.len() {
                    if let Some(b) = &passages[j] {
                        if Self::adjecent_or_overlapping(&a, b) {
                            passages[j] = None;
                        }
                    }
                }
            }
        }

        // Remove nullified slots.
        let last = last.min(max_passages.max(0) as usize);
        passages.truncate(last);

        // Sort in the offset order again.
        let mut passages: Vec<Passage> = passages
            .into_iter()
            .map(|p| p.expect("INVARIANT: the kept passages are set"))
            .collect();
        passages.sort_by_key(|a| a.from);

        Ok(passages)
    }

    /// Truncate or split highlight markers that cross permitted value
    /// boundaries.
    fn split_or_truncate_to_windows(
        markers: &[OffsetRange],
        max_passage_window: i32,
        permitted_passage_ranges: &[OffsetRange],
    ) -> Vec<OffsetRange> {
        // Process markers overlapping with each permitted window.
        let mut processed_markers: Vec<OffsetRange> = Vec::with_capacity(markers.len());
        for marker in markers {
            for permitted in permitted_passage_ranges {
                let mut new_slice = false;

                let mut from = marker.from;
                if from < permitted.from {
                    from = permitted.from;
                    new_slice = true;
                }

                let mut to = marker.to;
                if to > permitted.to {
                    to = permitted.to;
                    new_slice = true;
                }

                if from >= to {
                    continue;
                }

                if to - from > max_passage_window {
                    to = from + max_passage_window;
                    new_slice = true;
                }

                processed_markers.push(if new_slice {
                    marker.slice(from, to)
                } else {
                    marker.clone()
                });
            }
        }

        processed_markers
    }

    /// Equivalent to the package-private static
    /// `sortedAndNonOverlapping(List)`.
    ///
    /// # Errors
    ///
    /// Java's `AssertionError` when the ranges are not sorted and
    /// non-overlapping.
    pub(crate) fn sorted_and_non_overlapping<R: HasOffsetRange + fmt::Display>(
        permitted_passage_ranges: &[R],
    ) -> Result<bool> {
        if permitted_passage_ranges.len() > 1 {
            for w in permitted_passage_ranges.windows(2) {
                let (previous, next) = (&w[0], &w[1]);
                if previous.range_to() > next.range_from() {
                    let list: Vec<String> =
                        permitted_passage_ranges.iter().map(|r| r.to_string()).collect();
                    return Err(LuceneError::from(rucene::error::LuceneErrorKind::AssertionError(
                        format!(
                            "Ranges must be sorted and non-overlapping: [{}]",
                            list.join(", ")
                        ),
                    )));
                }
            }
        }
        Ok(true)
    }

    /// Invoked when no passages could be selected (due to constraints or
    /// lack of highlight markers).
    ///
    /// Equivalent to the protected `pickDefaultPassage(CharSequence, int,
    /// int, List)`.
    pub fn pick_default_passage(
        &self,
        value: &str,
        max_character_window: i32,
        max_passages: i32,
        permitted_passage_ranges: &[OffsetRange],
    ) -> Vec<Passage> {
        // Search for the first range that is not empty.
        let mut default_passages: Vec<Passage> = Vec::new();
        let value_length = utf16_len(value);
        for o in permitted_passage_ranges {
            if default_passages.len() as i64 >= max_passages as i64 {
                break;
            }

            let to = value_length.min(o.to);
            if o.from < to {
                default_passages.push(Passage::new(
                    o.from,
                    o.from + max_character_window.min(o.length()),
                    Vec::new(),
                ));
            }
        }

        default_passages
    }

    fn adjecent_or_overlapping(a: &Passage, b: &Passage) -> bool {
        if a.from >= b.from {
            a.from <= b.to - 1
        } else {
            a.to - 1 >= b.from
        }
    }
}