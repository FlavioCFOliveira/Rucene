//! Port of `org.apache.lucene.analysis.TestAbstractAnalysisFactory`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::factories::{
    find_char_filter_spi_name, find_token_filter_spi_name, find_tokenizer_spi_name,
    AnalysisSPILoader, CharFilterFactory, TokenFilterFactory, TokenizerFactory,
};
use rucene::analysis::standard::StandardTokenizerFactory;
use rucene::error::Result;

use super::fake_factories::{FakeCharFilterFactory, FakeTokenFilterFactory};
use crate::lucene_core_shims::{lucene_class, t};

fn test_lookup_tokenizer_spi_name() -> Result<()> {
    Assert::assert_equals_str(
        None,
        Some("standard"),
        Some(AnalysisSPILoader::<dyn TokenizerFactory>::lookup_spi_name(
            &StandardTokenizerFactory::class(),
        )?),
    );
    Assert::assert_equals_str(
        None,
        Some("standard"),
        Some(find_tokenizer_spi_name(&StandardTokenizerFactory::class())?),
    );
    Ok(())
}

fn test_lookup_char_filter_spi_name() -> Result<()> {
    Assert::assert_equals_str(
        None,
        Some("fake"),
        Some(AnalysisSPILoader::<dyn CharFilterFactory>::lookup_spi_name(
            &FakeCharFilterFactory::class(),
        )?),
    );
    Assert::assert_equals_str(
        None,
        Some("fake"),
        Some(find_char_filter_spi_name(&FakeCharFilterFactory::class())?),
    );
    Ok(())
}

fn test_lookup_token_filter_spi_name() -> Result<()> {
    Assert::assert_equals_str(
        None,
        Some("fake"),
        Some(
            AnalysisSPILoader::<dyn TokenFilterFactory>::lookup_spi_name(
                &FakeTokenFilterFactory::class(),
            )?,
        ),
    );
    Assert::assert_equals_str(
        None,
        Some("fake"),
        Some(find_token_filter_spi_name(&FakeTokenFilterFactory::class())?),
    );
    Ok(())
}

/// The class `org.apache.lucene.analysis.TestAbstractAnalysisFactory`.
pub fn class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.TestAbstractAnalysisFactory")
        .method(t(
            "testLookupTokenizerSPIName",
            test_lookup_tokenizer_spi_name,
        ))
        .method(t(
            "testLookupCharFilterSPIName",
            test_lookup_char_filter_spi_name,
        ))
        .method(t(
            "testLookupTokenFilterSPIName",
            test_lookup_token_filter_spi_name,
        ))
        .build()
}
