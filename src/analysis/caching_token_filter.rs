//! Token filter that caches all tokens so the stream can be consumed multiple
//! times, ported from `org.apache.lucene.analysis.CachingTokenFilter`.

#![deny(unsafe_code)]

use std::fmt::Debug;

use crate::analysis::TokenStream;
use crate::error::{LuceneError, Result};
use crate::util::attribute::{AttributeSource, CapturedState};

/// Caches all token attribute states on the first pass and replays them on
/// subsequent passes.
///
/// Equivalent to `org.apache.lucene.analysis.CachingTokenFilter`.
#[derive(Debug)]
pub struct CachingTokenFilter {
    source: AttributeSource,
    input: Box<dyn TokenStream>,
    /// Equivalent to `private List<AttributeSource.State> cache`, `null`
    /// (`None`) until the first `incrementToken()`.
    cache: Option<Vec<CapturedState>>,
    /// Equivalent to `private Iterator<AttributeSource.State> iterator`: the
    /// index of the next state to replay, `None` for Java's `null` (before
    /// the cache is filled, and after a fill that failed).
    iterator: Option<usize>,
    final_state: Option<CapturedState>,
}

impl CachingTokenFilter {
    /// Creates a caching filter around `input`.
    pub fn new(input: Box<dyn TokenStream>) -> Self {
        let source = AttributeSource::new_from(input.attribute_source());
        Self {
            source,
            input,
            cache: None,
            iterator: None,
            final_state: None,
        }
    }

    /// Returns `true` once the underlying token stream has been consumed and
    /// cached.
    ///
    /// Equivalent to `CachingTokenFilter.isCached()`.
    pub fn is_cached(&self) -> bool {
        self.cache.is_some()
    }

    /// Reads every token of the input into the cache, then the final state.
    ///
    /// Equivalent to the private `fillCache()` (`CachingTokenFilter.java:90-97`),
    /// called after `incrementToken()` has already assigned
    /// `cache = new ArrayList<>(64)` (`:69`): when the input fails, the
    /// states captured before the failure stay cached, and the filter counts
    /// as cached.
    fn fill_cache(&mut self) -> Result<()> {
        while self.input.increment_token()? {
            if let Some(state) = self.input.attribute_source().capture_state() {
                self.cache
                    .as_mut()
                    .expect("INVARIANT: incrementToken assigns the cache before filling it")
                    .push(state);
            }
        }
        // capture final state
        self.input.end()?;
        self.final_state = self.input.attribute_source().capture_state();
        Ok(())
    }
}

impl TokenStream for CachingTokenFilter {
    /// The first call reads and caches every token of the input; every call
    /// replays the next cached state.
    ///
    /// Equivalent to `CachingTokenFilter.incrementToken()`
    /// (`CachingTokenFilter.java:66-81`). After a first call whose fill
    /// failed, the cache is set but the iterator is still `null`, so a
    /// further call without a `reset()` throws Java's
    /// `NullPointerException` on `iterator.hasNext()`.
    fn increment_token(&mut self) -> Result<bool> {
        if self.cache.is_none() {
            // first-time: fill cache lazily
            self.cache = Some(Vec::with_capacity(64));
            self.fill_cache()?;
            self.iterator = Some(0);
        }

        let Some(next) = self.iterator else {
            return Err(LuceneError::null_pointer_with_message(
                "Cannot invoke \"java.util.Iterator.hasNext()\" because \"this.iterator\" is null",
            ));
        };
        let cache = self
            .cache
            .as_ref()
            .expect("INVARIANT: the cache is set before the iterator");
        if next >= cache.len() {
            // the cache is exhausted, return false
            return Ok(false);
        }
        // Since the TokenFilter can be reset, the tokens need to be preserved
        // as immutable.
        self.source.restore_state(&cache[next])?;
        self.iterator = Some(next + 1);
        Ok(true)
    }

    fn end(&mut self) -> Result<()> {
        if let Some(ref final_state) = self.final_state {
            self.source.restore_state(final_state)?;
        } else {
            self.source.end_attributes();
        }
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        if self.cache.is_none() {
            // first time
            self.input.reset()?;
        } else {
            self.iterator = Some(0);
        }
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        self.input.close()
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{new_standard_tokenizer, CharTermAttribute, ReusableStringReader};

    fn collect_terms(stream: &mut dyn TokenStream) -> Vec<String> {
        let mut terms = Vec::new();
        stream.reset().unwrap();
        while stream.increment_token().unwrap() {
            let term_att = stream
                .attribute_source()
                .get_attribute_interface::<dyn CharTermAttribute>()
                .unwrap();
            terms.push(term_att.buffer()[..term_att.length()].iter().collect());
        }
        stream.end().unwrap();
        terms
    }

    #[test]
    fn caches_and_replays_tokens() {
        let mut tokenizer = new_standard_tokenizer();
        let mut reader = ReusableStringReader::new();
        reader.set_value("Hello world");
        tokenizer.set_reader(Box::new(reader)).unwrap();

        let mut cached = CachingTokenFilter::new(Box::new(tokenizer));
        let first = collect_terms(&mut cached);
        assert!(cached.is_cached());
        let second = collect_terms(&mut cached);
        assert_eq!(first, second);
        assert_eq!(first, vec!["Hello".to_string(), "world".to_string()]);
    }
}
