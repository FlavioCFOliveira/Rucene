//! Base class for token filters that remove tokens, ported from
//! `org.apache.lucene.analysis.FilteringTokenFilter`.
//!
//! Implementations supply an [`accept`](FilteringTokenFilterLogic::accept)
//! predicate. The adapter folds the position increments of skipped tokens into
//! the next accepted token, matching Lucene's behavior.

#![deny(unsafe_code)]

use std::fmt::Debug;

use crate::analysis::tokenattributes::PositionIncrementAttribute;
use crate::analysis::{AttributeHandle, TokenFilter, TokenFilterLogic, TokenStream};
use crate::error::Result;
use crate::util::attribute::AttributeSource;

/// User-supplied accept/reject predicate for a [`FilteringTokenFilter`].
pub trait FilteringTokenFilterLogic: Debug + Send + Sync {
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
    /// `private final PositionIncrementAttribute posIncrAtt =
    /// addAttribute(PositionIncrementAttribute.class)`
    /// (`FilteringTokenFilter.java:29-30`), set by
    /// [`new_filtering_token_filter`]; an adapter built with
    /// [`new`](Self::new), whose caller adds the attribute itself, takes the
    /// source's instance the first time it needs it.
    pos_incr_att: Option<AttributeHandle<dyn PositionIncrementAttribute>>,
}

impl<T: FilteringTokenFilterLogic> FilteringTokenFilterAdapter<T> {
    /// Creates the adapter around the supplied accept predicate.
    pub fn new(logic: T) -> Self {
        Self {
            logic,
            skipped_positions: 0,
            pos_incr_att: None,
        }
    }

    /// The `PositionIncrementAttribute` the constructor added.
    fn pos_incr_att(
        &mut self,
        source: &AttributeSource,
    ) -> &AttributeHandle<dyn PositionIncrementAttribute> {
        self.pos_incr_att.get_or_insert_with(|| {
            AttributeHandle::get(source)
                .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
        })
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
                    let skipped = self.skipped_positions;
                    let mut pos_incr_att = self.pos_incr_att(source).borrow_mut();
                    let pos_incr = pos_incr_att.get_position_increment();
                    pos_incr_att.set_position_increment(pos_incr + skipped);
                }
                return Ok(true);
            }
            let increment = self.pos_incr_att(source).borrow().get_position_increment();
            self.skipped_positions += increment;
        }
        Ok(false)
    }

    fn end(&mut self, source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.end()?;
        let skipped = self.skipped_positions;
        let mut pos_incr_att = self.pos_incr_att(source).borrow_mut();
        let pos_incr = pos_incr_att.get_position_increment();
        pos_incr_att.set_position_increment(pos_incr + skipped);
        Ok(())
    }

    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.skipped_positions = 0;
        Ok(())
    }
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
    new_filtering_token_filter_with(input, |_| Ok(logic))
}

/// Creates a [`FilteringTokenFilter`] over `input` whose predicate is built
/// from the filter's source once the `PositionIncrementAttribute` is added.
///
/// Equivalent to a subclass constructor: `super(in)` (which adds the
/// `PositionIncrementAttribute`, `FilteringTokenFilter.java:29-30`), then
/// the subclass's field initializers, which `make_logic` runs on the source
/// (`termAtt = addAttribute(CharTermAttribute.class)` in `StopFilter`).
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws, and what `make_logic` returns.
pub fn new_filtering_token_filter_with<T: FilteringTokenFilterLogic>(
    input: Box<dyn TokenStream>,
    make_logic: impl FnOnce(&mut AttributeSource) -> Result<T>,
) -> Result<FilteringTokenFilter<T>> {
    TokenFilter::with_logic(input, |source| {
        let pos_incr_att = AttributeHandle::add(source)?;
        Ok(FilteringTokenFilterAdapter {
            logic: make_logic(source)?,
            skipped_positions: 0,
            pos_incr_att: Some(pos_incr_att),
        })
    })
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
