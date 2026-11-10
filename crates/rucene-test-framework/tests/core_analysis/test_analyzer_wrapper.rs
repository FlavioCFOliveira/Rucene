//! Port of `org.apache.lucene.analysis.TestAnalyzerWrapper`.

use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{
    new_lower_case_filter, Analyzer, AnalyzerState, AnalyzerWrapper, AnalyzerWrapperLogic,
    GlobalReuseStrategy, SharedTokenStream, TokenStream, TokenStreamComponents,
};
use rucene::error::Result;
use rucene_test_framework::analysis::canned_token_stream::CannedTokenStream;

use crate::lucene_core_shims::{lucene_class, t};

/// The anonymous `Analyzer` of `testSourceDelegation`, whose components'
/// source records that it was called.
#[derive(Debug)]
struct SourceRecordingAnalyzer {
    state: AnalyzerState,
    source_called: Arc<AtomicBool>,
}

impl Analyzer for SourceRecordingAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.TestAnalyzerWrapper$1"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let source_called = Arc::clone(&self.source_called);
        let sink: Arc<AtomicRefCell<dyn TokenStream>> =
            Arc::new(AtomicRefCell::new(CannedTokenStream::new(Vec::new())));
        Ok(TokenStreamComponents::new(
            Box::new(move |_r| {
                source_called.store(true, Ordering::SeqCst);
                Ok(())
            }),
            sink,
        ))
    }
}

/// The anonymous `AnalyzerWrapper` of `testSourceDelegation`.
#[derive(Debug)]
struct LowerCasingWrapper {
    analyzer: SourceRecordingAnalyzer,
}

impl AnalyzerWrapperLogic for LowerCasingWrapper {
    fn get_wrapped_analyzer(&self, _field_name: &str) -> &dyn Analyzer {
        &self.analyzer
    }

    fn wrap_components(
        &self,
        _field_name: &str,
        components: TokenStreamComponents,
    ) -> TokenStreamComponents {
        // `new TokenStreamComponents(components.getSource(), new
        // LowerCaseFilter(components.getTokenStream()))`
        let filter =
            new_lower_case_filter(Box::new(SharedTokenStream::new(components.token_stream())))
                .expect("new LowerCaseFilter(...)");
        components.with_sink(Arc::new(AtomicRefCell::new(filter)))
    }
}

/// `TestAnalyzerWrapper.testSourceDelegation()`.
///
/// `new AnalyzerWrapper(analyzer.getReuseStrategy())`: the anonymous
/// analyzer's strategy is `Analyzer.GLOBAL_REUSE_STRATEGY`, which a Rust
/// `Box<dyn ReuseStrategy>` cannot share, so the wrapper gets its own
/// instance of the same stateless strategy.
fn test_source_delegation() -> Result<()> {
    let source_called = Arc::new(AtomicBool::new(false));

    let analyzer = SourceRecordingAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        source_called: Arc::clone(&source_called),
    };

    let wrapped = AnalyzerWrapper::new(
        Box::new(LowerCasingWrapper { analyzer }),
        Box::new(GlobalReuseStrategy),
    );

    let ts = wrapped.token_stream_from_str("", "text")?;
    Assert::assert_true(source_called.load(Ordering::SeqCst));
    let closed = ts.borrow_mut().close();
    closed
}

/// The class `org.apache.lucene.analysis.TestAnalyzerWrapper`.
pub fn class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.TestAnalyzerWrapper")
        .method(t("testSourceDelegation", test_source_delegation))
        .build()
}
