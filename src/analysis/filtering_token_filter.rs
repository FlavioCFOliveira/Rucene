//! Base class for token filters that remove tokens, ported from
//! `org.apache.lucene.analysis.FilteringTokenFilter`.
//!
//! Implementations supply an [`accept`](FilteringTokenFilterLogic::accept)
//! predicate. The adapter folds the position increments of skipped tokens into
//! the next accepted token, matching Lucene's behavior.

#![deny(unsafe_code)]

use std::fmt::Debug;

use crate::analysis::tokenattributes::PositionIncrementAttribute;
use crate::analysis::{TokenFilter, TokenFilterLogic, TokenStream};
use crate::error::Result;
use crate::util::attribute::AttributeSource;

/// User-supplied accept/reject predicate for a [`FilteringTokenFilter`].
pub trait FilteringTokenFilterLogic: Debug {
    /// Returns `true` if the current token should be kept.
    fn accept(&mut self, source: &AttributeSource) -> Result<bool>;
}

/// Adapter that turns a [`FilteringTokenFilterLogic`] into a [`TokenFilterLogic`].
///
/// It handles skipping rejected tokens and folding their position increments
/// into the next accepted token, matching Lucene's behavior.
#[derive(Debug)]
pub struct FilteringTokenFilterAdapter<T: FilteringTokenFilterLogic> {
    logic: T,
    skipped_positions: i32,
}

impl<T: FilteringTokenFilterLogic> FilteringTokenFilterAdapter<T> {
    /// Creates the adapter around the supplied accept predicate.
    pub fn new(logic: T) -> Self {
        Self {
            logic,
            skipped_positions: 0,
        }
    }
}

impl<T: FilteringTokenFilterLogic> TokenFilterLogic for FilteringTokenFilterAdapter<T> {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        self.skipped_positions = 0;
        while input.increment_token()? {
            if self.logic.accept(source)? {
                if self.skipped_positions != 0 {
                    let mut pos_incr_att = pos_incr_att_mut(source);
                    let pos_incr = pos_incr_att.get_position_increment();
                    pos_incr_att.set_position_increment(pos_incr + self.skipped_positions);
                }
                return Ok(true);
            }
            self.skipped_positions += pos_incr_att_mut(source).get_position_increment();
        }
        Ok(false)
    }

    fn end(&mut self, source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.end()?;
        let mut pos_incr_att = pos_incr_att_mut(source);
        let pos_incr = pos_incr_att.get_position_increment();
        pos_incr_att.set_position_increment(pos_incr + self.skipped_positions);
        Ok(())
    }

    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.skipped_positions = 0;
        Ok(())
    }
}

/// The `PositionIncrementAttribute` the constructor added
/// (`FilteringTokenFilter.java:29-30`).
fn pos_incr_att_mut(
    source: &AttributeSource,
) -> std::cell::RefMut<'_, dyn PositionIncrementAttribute> {
    source
        .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
        .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
}

/// A [`TokenFilter`] that removes tokens according to an
/// [`accept`](FilteringTokenFilterLogic::accept) predicate.
pub type FilteringTokenFilter<T> = TokenFilter<FilteringTokenFilterAdapter<T>>;

/// Creates a [`FilteringTokenFilter`] over `input` using `logic`.
///
/// Equivalent to the constructor `FilteringTokenFilter(TokenStream)`, which a
/// subclass such as `StopFilter` calls as `super(in)`; `logic` is the
/// subclass's `accept`. It adds the `PositionIncrementAttribute`
/// (`FilteringTokenFilter.java:29-30`).
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_filtering_token_filter<T: FilteringTokenFilterLogic>(
    input: Box<dyn TokenStream>,
    logic: T,
) -> Result<FilteringTokenFilter<T>> {
    let mut filter = TokenFilter::new(input, FilteringTokenFilterAdapter::new(logic));
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    Ok(filter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::tokenattributes::CharTermAttribute;

    #[derive(Debug, Default)]
    struct RejectShortLogic {
        min_len: usize,
    }

    impl FilteringTokenFilterLogic for RejectShortLogic {
        fn accept(&mut self, source: &AttributeSource) -> Result<bool> {
            let len = source
                .get_attribute_interface::<dyn CharTermAttribute>()
                .unwrap()
                .length();
            Ok(len >= self.min_len)
        }
    }

    #[test]
    fn adapter_is_debug() {
        let adapter = FilteringTokenFilterAdapter::new(RejectShortLogic { min_len: 2 });
        assert!(format!("{:?}", adapter).contains("FilteringTokenFilterAdapter"));
    }
}
