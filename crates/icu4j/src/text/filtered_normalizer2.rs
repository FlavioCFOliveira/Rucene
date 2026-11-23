// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.FilteredNormalizer2`.

use std::sync::Arc;

use rucene::error::Result;

use crate::text::normalizer::QuickCheckResult;
use crate::text::normalizer2::Normalizer2;
use crate::text::unicode_set::{SpanCondition, UnicodeSet};

/// Normalization filtered by a UnicodeSet. Normalizes portions of the text
/// contained in the filter set and leaves portions not contained in the
/// filter set unchanged.
///
/// Equivalent to `com.ibm.icu.text.FilteredNormalizer2`. Java holds
/// references to the normalizer and the set; here they are shared `Arc`s.
#[derive(Debug, Clone)]
pub struct FilteredNormalizer2 {
    norm2: Arc<dyn Normalizer2>,
    set: Arc<UnicodeSet>,
}

impl FilteredNormalizer2 {
    /// `FilteredNormalizer2(Normalizer2 n2, UnicodeSet filterSet)`.
    pub fn new(n2: Arc<dyn Normalizer2>, filter_set: Arc<UnicodeSet>) -> Self {
        Self { norm2: n2, set: filter_set }
    }

    // Internal: No argument checking, and appends to dest.
    // Pass as input spanCondition the one that is likely to yield a non-zero
    // span length at the start of src.
    // For set=[:age=3.2:], since almost all common characters were in Unicode 3.2,
    // UnicodeSet.SpanCondition.SIMPLE should be passed in for the start of src
    // and UnicodeSet.SpanCondition.NOT_CONTAINED should be passed in if we continue after
    // an in-filter prefix.
    fn normalize_cond(&self, src: &[u16], dest: &mut Vec<u16>, mut span_condition: SpanCondition) -> Result<()> {
        let mut temp_dest: Vec<u16> = Vec::new();
        let mut prev_span_limit = 0;
        while prev_span_limit < src.len() as i32 {
            let span_limit = self.set.span(src, prev_span_limit, span_condition);
            let span_length = span_limit - prev_span_limit;
            if span_condition == SpanCondition::NotContained {
                if span_length != 0 {
                    dest.extend_from_slice(&src[prev_span_limit as usize..span_limit as usize]);
                }
                span_condition = SpanCondition::Simple;
            } else {
                if span_length != 0 {
                    // Not norm2.normalizeSecondAndAppend() because we do not want
                    // to modify the non-filter part of dest.
                    self.norm2.normalize(&src[prev_span_limit as usize..span_limit as usize], &mut temp_dest)?;
                    dest.extend_from_slice(&temp_dest);
                }
                span_condition = SpanCondition::NotContained;
            }
            prev_span_limit = span_limit;
        }
        Ok(())
    }

    fn normalize_second_and_append_do(&self, first: &mut Vec<u16>, second: &[u16], do_normalize: bool) -> Result<()> {
        if first.is_empty() {
            if do_normalize {
                return self.normalize(second, first);
            } else {
                first.extend_from_slice(second);
                return Ok(());
            }
        }
        // merge the in-filter suffix of the first string with the in-filter prefix of the second
        let prefix_limit = self.set.span(second, 0, SpanCondition::Simple);
        if prefix_limit != 0 {
            let prefix = &second[..prefix_limit as usize];
            let suffix_start = self.set.span_back(first, 0x7fffffff, SpanCondition::Simple);
            if suffix_start == 0 {
                if do_normalize {
                    self.norm2.normalize_second_and_append(first, prefix)?;
                } else {
                    self.norm2.append(first, prefix)?;
                }
            } else {
                let mut middle: Vec<u16> = first[suffix_start as usize..].to_vec();
                if do_normalize {
                    self.norm2.normalize_second_and_append(&mut middle, prefix)?;
                } else {
                    self.norm2.append(&mut middle, prefix)?;
                }
                first.truncate(suffix_start as usize);
                first.extend_from_slice(&middle);
            }
        }
        if prefix_limit < second.len() as i32 {
            let rest = &second[prefix_limit as usize..];
            if do_normalize {
                self.normalize_cond(rest, first, SpanCondition::NotContained)?;
            } else {
                first.extend_from_slice(rest);
            }
        }
        Ok(())
    }
}

impl Normalizer2 for FilteredNormalizer2 {
    fn normalize(&self, src: &[u16], dest: &mut Vec<u16>) -> Result<()> {
        dest.clear();
        self.normalize_cond(src, dest, SpanCondition::Simple)
    }

    fn normalize_append(&self, src: &[u16], dest: &mut Vec<u16>) -> Result<()> {
        self.normalize_cond(src, dest, SpanCondition::Simple)
    }

    fn normalize_second_and_append(&self, first: &mut Vec<u16>, second: &[u16]) -> Result<()> {
        self.normalize_second_and_append_do(first, second, true)
    }

    fn append(&self, first: &mut Vec<u16>, second: &[u16]) -> Result<()> {
        self.normalize_second_and_append_do(first, second, false)
    }

    fn get_decomposition(&self, c: i32) -> Result<Option<Vec<u16>>> {
        if self.set.contains(c)? {
            self.norm2.get_decomposition(c)
        } else {
            Ok(None)
        }
    }

    fn get_raw_decomposition(&self, c: i32) -> Result<Option<Vec<u16>>> {
        if self.set.contains(c)? {
            self.norm2.get_raw_decomposition(c)
        } else {
            Ok(None)
        }
    }

    fn compose_pair(&self, a: i32, b: i32) -> Result<i32> {
        if self.set.contains(a)? && self.set.contains(b)? {
            self.norm2.compose_pair(a, b)
        } else {
            Ok(-1)
        }
    }

    fn get_combining_class(&self, c: i32) -> Result<i32> {
        if self.set.contains(c)? {
            self.norm2.get_combining_class(c)
        } else {
            Ok(0)
        }
    }

    fn is_normalized(&self, s: &[u16]) -> Result<bool> {
        let mut span_condition = SpanCondition::Simple;
        let mut prev_span_limit = 0;
        while prev_span_limit < s.len() as i32 {
            let span_limit = self.set.span(s, prev_span_limit, span_condition);
            if span_condition == SpanCondition::NotContained {
                span_condition = SpanCondition::Simple;
            } else {
                if !self.norm2.is_normalized(&s[prev_span_limit as usize..span_limit as usize])? {
                    return Ok(false);
                }
                span_condition = SpanCondition::NotContained;
            }
            prev_span_limit = span_limit;
        }
        Ok(true)
    }

    fn quick_check(&self, s: &[u16]) -> Result<QuickCheckResult> {
        let mut result = QuickCheckResult::Yes;
        let mut span_condition = SpanCondition::Simple;
        let mut prev_span_limit = 0;
        while prev_span_limit < s.len() as i32 {
            let span_limit = self.set.span(s, prev_span_limit, span_condition);
            if span_condition == SpanCondition::NotContained {
                span_condition = SpanCondition::Simple;
            } else {
                let qc_result = self.norm2.quick_check(&s[prev_span_limit as usize..span_limit as usize])?;
                if qc_result == QuickCheckResult::No {
                    return Ok(qc_result);
                } else if qc_result == QuickCheckResult::Maybe {
                    result = qc_result;
                }
                span_condition = SpanCondition::NotContained;
            }
            prev_span_limit = span_limit;
        }
        Ok(result)
    }

    fn span_quick_check_yes(&self, s: &[u16]) -> Result<i32> {
        let mut span_condition = SpanCondition::Simple;
        let mut prev_span_limit = 0;
        while prev_span_limit < s.len() as i32 {
            let span_limit = self.set.span(s, prev_span_limit, span_condition);
            if span_condition == SpanCondition::NotContained {
                span_condition = SpanCondition::Simple;
            } else {
                let yes_limit = prev_span_limit + self.norm2.span_quick_check_yes(&s[prev_span_limit as usize..span_limit as usize])?;
                if yes_limit < span_limit {
                    return Ok(yes_limit);
                }
                span_condition = SpanCondition::NotContained;
            }
            prev_span_limit = span_limit;
        }
        Ok(s.len() as i32)
    }

    fn has_boundary_before(&self, c: i32) -> Result<bool> {
        Ok(!self.set.contains(c)? || self.norm2.has_boundary_before(c)?)
    }

    fn has_boundary_after(&self, c: i32) -> Result<bool> {
        Ok(!self.set.contains(c)? || self.norm2.has_boundary_after(c)?)
    }

    fn is_inert(&self, c: i32) -> Result<bool> {
        Ok(!self.set.contains(c)? || self.norm2.is_inert(c)?)
    }
}
