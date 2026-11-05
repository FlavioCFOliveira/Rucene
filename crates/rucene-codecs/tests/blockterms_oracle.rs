//! Oracle parity of `org.apache.lucene.codecs.blockterms` (rmp #334)
//! against Apache Lucene 10.5.0.
//!
//! The package registers no service; Lucene's users of it are
//! `lucene-test-framework`'s `LuceneFixedGap`, `LuceneVarGapFixedInterval`
//! and `LuceneVarGapDocFreqInterval`, which assemble `BlockTermsWriter` (or
//! `BlockTermsReader`), a terms index and `Lucene104PostingsWriter` (or
//! reader). The Java oracle (`PostingsFormatOracle`, in the workspace harness
//! `crates/randomizedtesting/tests/fixtures/oracle/`) writes the corpus of
//! `SimpleTextOracle` with each of the three; this file assembles the same
//! three formats with the ported classes, under the same names, and
//! registers them in a class loader of its own, as `lucene-test-framework`'s
//! service file does.
//!
//! For each format the tests write the corpus with Rucene and compare every
//! file of the format (`_<segment>_<name>_<n>.<ext>`, compound files
//! included) byte for byte with Lucene's, once Lucene's random segment id in
//! the header is replaced by Rucene's (and the footer's CRC-32, which covers
//! it, recomputed); read Lucene's files and Rucene's with Rucene and compare
//! what is read with what Lucene read; run Lucene's reader on Rucene's files;
//! and compare the errors of corrupted copies.

mod oracle;

use std::sync::{Arc, LazyLock};

use oracle::*;
use rucene::codecs::{
    Codec, FieldsConsumer, FieldsProducer, Lucene104PostingsReader, Lucene104PostingsWriter,
    PostingsFormat, SegmentReadState, SegmentWriteState,
};
use rucene::error::Result;
use rucene::util::resource_loader::{ClassLoader, ServiceProvider};
use rucene::util::spi::NamedSPIProvider;
use rucene_codecs::blockterms::{
    BlockTermsReader, BlockTermsWriter, EveryNOrDocFreqTermSelector, EveryNTermSelector,
    FixedGapTermsIndexReader, FixedGapTermsIndexWriter, TermsIndexReaderBase, TermsIndexWriterBase,
    VariableGapTermsIndexReader, VariableGapTermsIndexWriter, DEFAULT_TERM_INDEX_INTERVAL,
};

/// Which terms index a [`BlockTermsAssembly`] uses.
#[derive(Debug, Clone, Copy)]
enum Index {
    /// `LuceneFixedGap`: `FixedGapTermsIndexWriter(state, termIndexInterval)`.
    FixedGap,
    /// `LuceneVarGapFixedInterval`:
    /// `VariableGapTermsIndexWriter(state, new EveryNTermSelector(termIndexInterval))`.
    VarGapFixedInterval,
    /// `LuceneVarGapDocFreqInterval`:
    /// `VariableGapTermsIndexWriter(state, new EveryNOrDocFreqTermSelector(docFreqThreshold, termIndexInterval))`.
    VarGapDocFreqInterval,
}

/// `lucene-test-framework`'s three blockterms postings formats, with their
/// default parameters (`termIndexInterval = 32`, `docFreqThreshold =
/// 1000000`).
#[derive(Debug)]
struct BlockTermsAssembly {
    name: &'static str,
    index: Index,
}

impl PostingsFormat for BlockTermsAssembly {
    fn name(&self) -> &str {
        self.name
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        let mut docs = Box::new(Lucene104PostingsWriter::new(state)?);
        let index_writer: Result<Box<dyn TermsIndexWriterBase>> = match self.index {
            Index::FixedGap => {
                FixedGapTermsIndexWriter::with_interval(state, DEFAULT_TERM_INDEX_INTERVAL)
                    .map(|w| Box::new(w) as Box<dyn TermsIndexWriterBase>)
            }
            Index::VarGapFixedInterval => VariableGapTermsIndexWriter::new(
                state,
                Box::new(EveryNTermSelector::new(DEFAULT_TERM_INDEX_INTERVAL)),
            )
            .map(|w| Box::new(w) as Box<dyn TermsIndexWriterBase>),
            Index::VarGapDocFreqInterval => VariableGapTermsIndexWriter::new(
                state,
                Box::new(EveryNOrDocFreqTermSelector::new(
                    1_000_000,
                    DEFAULT_TERM_INDEX_INTERVAL,
                )),
            )
            .map(|w| Box::new(w) as Box<dyn TermsIndexWriterBase>),
        };
        let index_writer = match index_writer {
            Ok(w) => w,
            Err(e) => {
                rucene::codecs::PostingsWriterBase::close(docs.as_mut())?;
                return Err(e);
            }
        };
        // `BlockTermsWriter::new` closes both when it fails.
        Ok(Box::new(BlockTermsWriter::new(index_writer, state, docs)?))
    }

    fn fields_producer<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn FieldsProducer>> {
        let mut postings = Box::new(Lucene104PostingsReader::new(state)?);
        let index_reader: Result<Box<dyn TermsIndexReaderBase>> = match self.index {
            Index::FixedGap => FixedGapTermsIndexReader::new(state)
                .map(|r| Box::new(r) as Box<dyn TermsIndexReaderBase>),
            Index::VarGapFixedInterval | Index::VarGapDocFreqInterval => {
                VariableGapTermsIndexReader::new(state)
                    .map(|r| Box::new(r) as Box<dyn TermsIndexReaderBase>)
            }
        };
        let index_reader = match index_reader {
            Ok(r) => r,
            Err(e) => {
                rucene::codecs::PostingsReaderBase::close(postings.as_mut())?;
                return Err(e);
            }
        };
        // `BlockTermsReader::new` closes both when it fails.
        Ok(Box::new(BlockTermsReader::new(
            index_reader,
            postings,
            state,
        )?))
    }
}

const FIXED_GAP: BlockTermsAssembly = BlockTermsAssembly {
    name: "LuceneFixedGap",
    index: Index::FixedGap,
};
const VAR_GAP_FIXED: BlockTermsAssembly = BlockTermsAssembly {
    name: "LuceneVarGapFixedInterval",
    index: Index::VarGapFixedInterval,
};
const VAR_GAP_DOC_FREQ: BlockTermsAssembly = BlockTermsAssembly {
    name: "LuceneVarGapDocFreqInterval",
    index: Index::VarGapDocFreqInterval,
};

fn provider(class: &'static str, format: BlockTermsAssembly) -> ServiceProvider {
    let format = Arc::new(format);
    ServiceProvider::new(
        "org.apache.lucene.codecs.PostingsFormat",
        class,
        NamedSPIProvider::new(move || Arc::clone(&format) as Arc<dyn PostingsFormat>),
    )
}

/// `lucene-codecs` and the blockterms entries of `lucene-test-framework`'s
/// `META-INF/services/org.apache.lucene.codecs.PostingsFormat`.
fn test_class_loader() -> &'static Arc<ClassLoader> {
    static LOADER: LazyLock<Arc<ClassLoader>> = LazyLock::new(|| {
        ClassLoader::with_services(
            "lucene-test-framework",
            Some(Arc::clone(rucene_codecs::class_loader())),
            vec![
                provider(
                    "org.apache.lucene.tests.codecs.blockterms.LuceneFixedGap",
                    FIXED_GAP,
                ),
                provider(
                    "org.apache.lucene.tests.codecs.blockterms.LuceneVarGapFixedInterval",
                    VAR_GAP_FIXED,
                ),
                provider(
                    "org.apache.lucene.tests.codecs.blockterms.LuceneVarGapDocFreqInterval",
                    VAR_GAP_DOC_FREQ,
                ),
            ],
        )
    });
    &LOADER
}

fn fixed_gap_codec() -> Arc<dyn Codec> {
    per_field_postings_codec(Arc::new(FIXED_GAP))
}
fn var_gap_fixed_codec() -> Arc<dyn Codec> {
    per_field_postings_codec(Arc::new(VAR_GAP_FIXED))
}
fn var_gap_doc_freq_codec() -> Arc<dyn Codec> {
    per_field_postings_codec(Arc::new(VAR_GAP_DOC_FREQ))
}

const ORACLES: [FormatOracle; 3] = [
    FormatOracle {
        prefix: "fixedgap",
        format_name: "LuceneFixedGap",
        extensions: &["tib", "tii"],
        codec: fixed_gap_codec,
        extra_jars: TEST_FRAMEWORK_JARS,
        live_impacts_differ: true,
    },
    FormatOracle {
        prefix: "vargap_fixed",
        format_name: "LuceneVarGapFixedInterval",
        extensions: &["tib", "tiv", "tmv"],
        codec: var_gap_fixed_codec,
        extra_jars: TEST_FRAMEWORK_JARS,
        live_impacts_differ: true,
    },
    FormatOracle {
        prefix: "vargap_docfreq",
        format_name: "LuceneVarGapDocFreqInterval",
        extensions: &["tib", "tiv", "tmv"],
        codec: var_gap_doc_freq_codec,
        extra_jars: TEST_FRAMEWORK_JARS,
        live_impacts_differ: true,
    },
];

#[test]
fn rucene_writes_lucenes_bytes() {
    for oracle in &ORACLES {
        oracle.assert_writes_lucenes_bytes();
    }
}

#[test]
fn rucene_reads_what_lucene_reads() {
    for oracle in &ORACLES {
        oracle.assert_reads_what_lucene_reads();
    }
}

#[test]
fn enum_trace_matches_lucenes() {
    for oracle in &ORACLES {
        oracle.assert_enum_trace_matches();
    }
}

#[test]
fn lucene_reads_what_rucene_writes() {
    for oracle in &ORACLES {
        oracle.assert_lucene_reads_what_rucene_writes();
    }
}

/// The variable-gap readers check `.tmv`'s footer with the prior error of
/// its corrupted header (`CodecUtil.checkFooter(metaIn, priorE)`), whose
/// message is Lucene's: the prior error is suppressed, not appended.
#[test]
fn corrupted_files_raise_lucenes_errors() {
    for oracle in &ORACLES {
        oracle.assert_corrupted_files_raise_lucenes_errors();
    }
}

/// The assemblies resolve by name once the class loader is installed, as
/// Lucene's do from `lucene-test-framework`'s service file.
#[test]
fn assemblies_resolve_by_name() {
    install_codecs();
    for oracle in &ORACLES {
        let spi = oracle.spi();
        let format = rucene::codecs::postings_for_name(oracle.format_name)
            .unwrap_or_else(|| panic!("{} resolves", oracle.format_name));
        assert_eq!(format.name(), spi["getName"]);
        assert_eq!(format.to_string(), spi["toString"]);
        assert_eq!(format.to_string(), spi["forName.toString"]);
    }
}
