//! Token filter that caches all tokens so the stream can be consumed multiple
//! times, ported from `org.apache.lucene.analysis.CachingTokenFilter`.

#![deny(unsafe_code)]

use std::fmt::Debug;

use crate::analysis::TokenStream;
use crate::error::Result;
use crate::util::attribute::{AttributeSource, CapturedState};

/// Caches all token attribute states on the first pass and replays them on
/// subsequent passes.
///
/// Equivalent to `org.apache.lucene.analysis.CachingTokenFilter`.
#[derive(Debug)]
pub struct CachingTokenFilter {
    source: AttributeSource,
    input: Box<dyn TokenStream>,
    cache: Option<Vec<CapturedState>>,
    cache_index: usize,
    final_state: Option<CapturedState>,
}

/// Creates a [`CachingTokenFilter`] around `input`.
pub fn new_caching_token_filter(input: Box<dyn TokenStream>) -> CachingTokenFilter {
    CachingTokenFilter::new(input)
}

impl CachingTokenFilter {
    /// Creates a caching filter around `input`.
    pub fn new(input: Box<dyn TokenStream>) -> Self {
        let source = AttributeSource::new_from(input.attribute_source());
        Self {
            source,
            input,
            cache: None,
            cache_index: 0,
            final_state: None,
        }
    }

    /// Returns `true` once the underlying token stream has been consumed and
    /// cached.
    pub fn is_cached(&self) -> bool {
        self.cache.is_some()
    }

    fn fill_cache(&mut self) -> Result<()> {
        let mut cache = Vec::with_capacity(64);
        while self.input.increment_token()? {
            if let Some(state) = self.input.attribute_source().capture_state() {
                cache.push(state);
            }
        }
        self.input.end()?;
        self.final_state = self.input.attribute_source().capture_state();
        self.cache = Some(cache);
        Ok(())
    }
}

impl TokenStream for CachingTokenFilter {
    fn increment_token(&mut self) -> Result<bool> {
        if self.cache.is_none() {
            self.fill_cache()?;
            self.cache_index = 0;
        }

        let cache = self.cache.as_ref().unwrap();
        if self.cache_index < cache.len() {
            let state = &cache[self.cache_index];
            self.source.restore_state(state)?;
            self.cache_index += 1;
            Ok(true)
        } else {
            Ok(false)
        }
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
            self.input.reset()?;
        } else {
            self.cache_index = 0;
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
    use crate::analysis::{
        new_standard_tokenizer, PackedTokenAttributeImpl, ReusableStringReader, Tokenizer,
    };

    fn collect_terms(stream: &mut dyn TokenStream) -> Vec<String> {
        let mut terms = Vec::new();
        stream.reset().unwrap();
        while stream.increment_token().unwrap() {
            let term = stream
                .attribute_source()
                .get_attribute::<PackedTokenAttributeImpl>()
                .unwrap()
                .term();
            terms.push(term);
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
