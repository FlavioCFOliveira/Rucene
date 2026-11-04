//! Oracle parity of `org.apache.lucene.codecs.uniformsplit` and its
//! `sharedterms` package (rmp #336: `UniformSplitPostingsFormat`,
//! `STUniformSplitPostingsFormat`) against Apache Lucene 10.5.0.
//!
//! The Java oracle (`PostingsFormatOracle`, in the workspace harness
//! `crates/randomizedtesting/tests/fixtures/oracle/`) writes the corpus of
//! `SimpleTextOracle` with `Lucene104Codec` routing every field but `e_id` to
//! the format under test, five times: `UniformSplit` and
//! `SharedTermsUniformSplit` with their default settings, `UniformSplit(32,
//! 3, dictionaryOnHeap=true)`, `UniformSplit(8, 2, false)` and
//! `SharedTermsUniformSplit(8, 2, dictionaryOnHeap=true)`. The tests write
//! the same corpus with Rucene and compare every `.ustb`/`.ustd`
//! (`.stustb`/`.stustd`) file and the postings files the format writes
//! under its name byte for byte, Lucene's random segment id replaced by
//! Rucene's — the corpus' `forceMerge(2)` and `forceMergeDeletes()` run the
//! shared-terms format's own merge (`STMergingBlockReader`,
//! `STMergingTermsEnum`); read both sides' files with Rucene; run Lucene's
//! reader on Rucene's files; replay the enum trace (term states and
//! `intersect`); compare the errors of corrupted copies; and resolve the
//! formats' service names.

mod oracle;

use std::sync::Arc;

use oracle::*;
use rucene::codecs::Codec;
use rucene::util::resource_loader::ClassLoader;
use rucene_codecs::uniformsplit::sharedterms::STUniformSplitPostingsFormat;
use rucene_codecs::uniformsplit::UniformSplitPostingsFormat;

/// `lucene-codecs`' loader, plus a child that registers the identity block
/// codec formats of [`block_codec_round_trip_reads_what_lucene_reads`] under
/// their own names (as Lucene's test framework registers its `Rot13` ones):
/// a reader finds a postings format by the name its segment records.
fn test_class_loader() -> &'static Arc<ClassLoader> {
    use rucene::codecs::PostingsFormat;
    use rucene::util::resource_loader::ServiceProvider;
    use rucene::util::spi::NamedSPIProvider;
    static LOADER: std::sync::LazyLock<Arc<ClassLoader>> = std::sync::LazyLock::new(|| {
        ClassLoader::with_services(
            "uniformsplit-oracle-tests",
            Some(Arc::clone(rucene_codecs::class_loader())),
            vec![
                ServiceProvider::new(
                    "org.apache.lucene.codecs.PostingsFormat",
                    "IdentityUniformSplit",
                    NamedSPIProvider::new(|| encoded_format() as Arc<dyn PostingsFormat>),
                ),
                ServiceProvider::new(
                    "org.apache.lucene.codecs.PostingsFormat",
                    "IdentitySharedTermsUniformSplit",
                    NamedSPIProvider::new(|| encoded_shared_format() as Arc<dyn PostingsFormat>),
                ),
            ],
        )
    });
    &LOADER
}

fn uniform_split_codec() -> Arc<dyn Codec> {
    per_field_postings_codec(Arc::new(UniformSplitPostingsFormat::new()))
}

fn uniform_split_heap_codec() -> Arc<dyn Codec> {
    per_field_postings_codec(Arc::new(
        UniformSplitPostingsFormat::with_settings(32, 3, None, None, true).unwrap(),
    ))
}

fn uniform_split_small_codec() -> Arc<dyn Codec> {
    per_field_postings_codec(Arc::new(
        UniformSplitPostingsFormat::with_settings(8, 2, None, None, false).unwrap(),
    ))
}

fn shared_terms_codec() -> Arc<dyn Codec> {
    per_field_postings_codec(Arc::new(STUniformSplitPostingsFormat::new()))
}

fn shared_terms_small_codec() -> Arc<dyn Codec> {
    per_field_postings_codec(Arc::new(
        STUniformSplitPostingsFormat::with_settings(8, 2, None, None, true).unwrap(),
    ))
}

/// The formats read postings with `Lucene104PostingsReader`, whose `impacts`
/// (core defect 1) the dump's `live` lines meet.
const fn format(
    prefix: &'static str,
    format_name: &'static str,
    extensions: &'static [&'static str],
    codec: fn() -> Arc<dyn Codec>,
) -> FormatOracle {
    FormatOracle {
        prefix,
        format_name,
        extensions,
        codec,
        extra_jars: &[],
        live_impacts_differ: true,
    }
}

const UNIFORM_SPLIT: FormatOracle = format(
    "uniformsplit",
    "UniformSplit",
    &["ustb", "ustd"],
    uniform_split_codec,
);
const UNIFORM_SPLIT_HEAP: FormatOracle = format(
    "uniformsplit_heap",
    "UniformSplit",
    &["ustb", "ustd"],
    uniform_split_heap_codec,
);
const UNIFORM_SPLIT_SMALL: FormatOracle = format(
    "uniformsplit_small",
    "UniformSplit",
    &["ustb", "ustd"],
    uniform_split_small_codec,
);
const SHARED_TERMS: FormatOracle = format(
    "stuniformsplit",
    "SharedTermsUniformSplit",
    &["stustb", "stustd"],
    shared_terms_codec,
);
const SHARED_TERMS_SMALL: FormatOracle = format(
    "stuniformsplit_small",
    "SharedTermsUniformSplit",
    &["stustb", "stustd"],
    shared_terms_small_codec,
);

const ALL: [&FormatOracle; 5] = [
    &UNIFORM_SPLIT,
    &UNIFORM_SPLIT_HEAP,
    &UNIFORM_SPLIT_SMALL,
    &SHARED_TERMS,
    &SHARED_TERMS_SMALL,
];

#[test]
fn uniform_split_writes_lucenes_bytes() {
    for f in [&UNIFORM_SPLIT, &UNIFORM_SPLIT_HEAP, &UNIFORM_SPLIT_SMALL] {
        assert!(f.assert_writes_lucenes_bytes() > 0, "{}", f.prefix);
    }
}

#[test]
fn shared_terms_writes_lucenes_bytes() {
    for f in [&SHARED_TERMS, &SHARED_TERMS_SMALL] {
        assert!(f.assert_writes_lucenes_bytes() > 0, "{}", f.prefix);
    }
}

#[test]
fn uniform_split_reads_what_lucene_reads() {
    for f in [&UNIFORM_SPLIT, &UNIFORM_SPLIT_HEAP, &UNIFORM_SPLIT_SMALL] {
        f.assert_reads_what_lucene_reads();
    }
}

#[test]
fn shared_terms_reads_what_lucene_reads() {
    for f in [&SHARED_TERMS, &SHARED_TERMS_SMALL] {
        f.assert_reads_what_lucene_reads();
    }
}

#[test]
fn enum_trace_matches_lucenes() {
    for f in ALL {
        f.assert_enum_trace_matches();
    }
}

#[test]
fn lucene_reads_what_rucene_writes() {
    for f in ALL {
        f.assert_lucene_reads_what_rucene_writes();
    }
}

#[test]
fn corrupted_files_raise_lucenes_errors() {
    for f in ALL {
        assert!(
            f.assert_corrupted_files_raise_lucenes_errors().is_empty(),
            "{}",
            f.prefix
        );
    }
}

/// `PostingsFormat.forName("UniformSplit")` and
/// `("SharedTermsUniformSplit")` resolve once the crate's service files are
/// loaded; neither format overrides `toString()`.
#[test]
fn formats_resolve_by_name() {
    install_codecs();
    for (f, class) in [
        (
            &UNIFORM_SPLIT,
            "org.apache.lucene.codecs.uniformsplit.UniformSplitPostingsFormat",
        ),
        (
            &SHARED_TERMS,
            "org.apache.lucene.codecs.uniformsplit.sharedterms.STUniformSplitPostingsFormat",
        ),
    ] {
        let spi = f.spi();
        let found = rucene::codecs::postings_for_name(&spi["getName"]).expect("resolves");
        assert_eq!(found.name(), spi["getName"]);
        assert_eq!(found.to_string(), spi["toString"]);
        assert_eq!(found.to_string(), spi["forName.toString"]);
        assert_eq!(spi["forName.getClass"], class);
    }
}

// ---------------------------------------------------------------------------
// The `shared` case: terms in several fields (`PostingsFormatOracle.writeShared`)
// ---------------------------------------------------------------------------

use std::path::Path;

use rucene::document::{Document, Field, FieldType, Store, StringField};
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_policy::{MergePolicy, TieredMergePolicy};
use rucene::index::IndexOptions;

/// `PostingsFormatOracle.sharedType(IndexOptions)`.
fn shared_type(options: IndexOptions) -> FieldType {
    field_type(|t| {
        t.set_tokenized(false).unwrap();
        t.set_omit_norms(true).unwrap();
        t.set_index_options(options).unwrap();
    })
}

/// `PostingsFormatOracle.writeShared(Directory, Codec)`.
fn write_shared(path: &Path, codec: Arc<dyn Codec>) {
    let mut config = new_config(codec);
    let policy = TieredMergePolicy::new();
    policy.set_no_cfs_ratio(0.0).unwrap();
    config.set_merge_policy(Arc::new(policy)).unwrap();
    config.set_use_compound_file(false).unwrap();
    let docs = shared_type(IndexOptions::DOCS);
    let freqs = shared_type(IndexOptions::DOCS_AND_FREQS);
    let positions = shared_type(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
    let offsets = shared_type(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
    let w = IndexWriter::new(open_dir(path), config).unwrap();
    for batch in 0..3 {
        for i in batch * 100..batch * 100 + 100 {
            let mut d = Document::new();
            d.add(Box::new(
                StringField::new("s_id", format!("{i:04}"), Store::NO).unwrap(),
            ));
            d.add(Box::new(
                Field::new("sa", format!("t{}", i % 37), docs.clone()).unwrap(),
            ));
            d.add(Box::new(
                Field::new("sb", format!("t{}", i % 23), positions.clone()).unwrap(),
            ));
            d.add(Box::new(
                Field::new("sc", format!("t{}", i % 11), offsets.clone()).unwrap(),
            ));
            d.add(Box::new(
                Field::new("sc", format!("u{}", i % 53), offsets.clone()).unwrap(),
            ));
            d.add(Box::new(
                Field::new("sd", format!("t{}", i % 37), freqs.clone()).unwrap(),
            ));
            if i % 3 == 0 {
                d.add(Box::new(
                    Field::new("se", format!("t{}", i % 7), docs.clone()).unwrap(),
                ));
            }
            w.add_document(&d).unwrap();
        }
        w.commit().unwrap();
        if batch == 1 {
            for i in (0..200).step_by(9) {
                w.delete_documents_by_terms(vec![term("s_id", &format!("{i:04}"))])
                    .unwrap();
            }
        }
    }
    w.force_merge(1).unwrap();
    w.commit().unwrap();
    w.close().unwrap();
}

/// The `shared` case of every format: Rucene writes Lucene's bytes (the
/// shared-terms format's multi-field lines and its merge of them included),
/// reads Lucene's files and its own as Lucene does, replays the enum trace,
/// and Lucene reads Rucene's files.
#[test]
fn shared_case_matches_lucene() {
    for f in ALL {
        let write = |path: &Path| write_shared(path, (f.codec)());
        assert!(
            f.assert_case_writes_lucenes_bytes("shared", write, |_, java, _| java) > 0,
            "{}",
            f.prefix
        );
        f.assert_case_reads_what_lucene_reads("shared", write);
        f.assert_case_enum_trace_matches("shared", write);
        f.assert_case_lucene_reads_what_rucene_writes("shared", write);
    }
}

// ---------------------------------------------------------------------------
// The block encoder / decoder extension points (`BlockEncoder`,
// `BlockDecoder`) on an identity codec: what Lucene reads in the default
// format's files is what Rucene reads in its own files encoded block by block.
// ---------------------------------------------------------------------------

use rucene::store::{DataInput, DataOutput};
use rucene::util::BytesRef;
use rucene_codecs::uniformsplit::{BlockDecoder, BlockEncoder, WritableBytes};

struct IdentityCodec;

impl std::fmt::Display for IdentityCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IdentityCodec")
    }
}

struct Bytes(Vec<u8>);

impl WritableBytes for Bytes {
    fn size(&self) -> i64 {
        self.0.len() as i64
    }

    fn write_to(&self, data_output: &mut dyn DataOutput) -> rucene::error::Result<()> {
        data_output.write_bytes(&self.0, 0, self.0.len())
    }
}

fn read_all(input: &mut dyn DataInput, length: i64) -> rucene::error::Result<Vec<u8>> {
    let mut bytes = vec![0u8; length as usize];
    input.read_bytes(&mut bytes, 0, length as usize)?;
    Ok(bytes)
}

impl BlockEncoder for IdentityCodec {
    fn encode(
        &self,
        block_bytes: &mut dyn DataInput,
        length: i64,
    ) -> rucene::error::Result<Box<dyn WritableBytes>> {
        Ok(Box::new(Bytes(read_all(block_bytes, length)?)))
    }
}

impl BlockDecoder for IdentityCodec {
    fn decode(
        &self,
        block_bytes: &mut dyn DataInput,
        length: i64,
    ) -> rucene::error::Result<BytesRef> {
        Ok(BytesRef::new(read_all(block_bytes, length)?))
    }
}

fn encoded_format() -> Arc<UniformSplitPostingsFormat> {
    let c = Arc::new(IdentityCodec);
    Arc::new(
        UniformSplitPostingsFormat::with_name(
            "IdentityUniformSplit",
            32,
            3,
            Some(c.clone()),
            Some(c),
            false,
        )
        .unwrap(),
    )
}

fn encoded_shared_format() -> Arc<STUniformSplitPostingsFormat> {
    let c = Arc::new(IdentityCodec);
    Arc::new(
        STUniformSplitPostingsFormat::with_name(
            "IdentitySharedTermsUniformSplit",
            32,
            3,
            Some(c.clone()),
            Some(c),
            false,
        )
        .unwrap(),
    )
}

fn encoded_codec() -> Arc<dyn Codec> {
    per_field_postings_codec(encoded_format())
}

fn encoded_shared_codec() -> Arc<dyn Codec> {
    per_field_postings_codec(encoded_shared_format())
}

/// Rucene's files written through a block encoder, which also encodes the
/// FST dictionary and the fields metadata, read through its decoder (the
/// dictionary then on-heap) as Lucene reads the default format's files.
#[test]
fn block_codec_round_trip_reads_what_lucene_reads() {
    for (prefix, codec) in [
        ("uniformsplit", encoded_codec as fn() -> Arc<dyn Codec>),
        ("stuniformsplit", encoded_shared_codec),
    ] {
        for case in ["nocfs", "edge", "shared"] {
            let dir = TempDir::new(&format!("{prefix}-encoded-{case}"));
            match case {
                "nocfs" => write_corpus(&dir.0, false, codec()),
                "edge" => write_edge(&dir.0, codec()),
                _ => write_shared(&dir.0, codec()),
            }
            assert_same_lines_but_live(
                &format!("{prefix}: {case}: encoded"),
                &dump(&dir.0),
                &java_reads(prefix, case),
                true,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// `STUniformSplitTermsWriter.merge` on a sorted index: the custom merge does
// not support index sorting and falls back to the default merge.
// ---------------------------------------------------------------------------

use rucene::document::NumericDocValuesField;
use rucene::search::{Sort, SortField, SortFieldType};

/// `java -cp <jars> SimpleTextReadOracle.java <dir>`: what Lucene reads in
/// `dir`.
fn lucene_dump(dir: &Path) -> Vec<String> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../randomizedtesting/tests/fixtures/oracle/src/org/apache/lucene/codecs/simpletext/\
         SimpleTextReadOracle.java",
    );
    let classpath = format!(
        "{}:{}",
        lucene_jar("lucene-core").display(),
        lucene_jar("lucene-codecs").display()
    );
    let output = std::process::Command::new("java")
        .arg("-ea")
        .arg("-cp")
        .arg(&classpath)
        .arg(&source)
        .arg(dir)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "SimpleTextReadOracle failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

/// A sorted index of three segments, merged into one: with the shared-terms
/// format, the merge falls back to the default one (`mergeState.needsIndexSort`),
/// whose documents follow the sort; Lucene reads the merged files as Rucene
/// does.
#[test]
fn shared_terms_sorted_merge_falls_back_to_the_default_merge() {
    let dir = TempDir::new("stuniformsplit-sorted");
    let mut config = new_config(shared_terms_small_codec());
    config
        .set_index_sort(
            Sort::new_fields(vec![SortField::new(
                Some("rank".to_string()),
                SortFieldType::Long,
            )
            .unwrap()])
            .unwrap(),
        )
        .unwrap();
    let docs = shared_type(IndexOptions::DOCS);
    let positions = shared_type(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
    let w = IndexWriter::new(open_dir(&dir.0), config).unwrap();
    for batch in 0..3 {
        for i in batch * 50..batch * 50 + 50 {
            let mut d = Document::new();
            d.add(Box::new(NumericDocValuesField::new(
                "rank",
                (i * 7919) % 151,
            )));
            d.add(Box::new(
                Field::new("sa", format!("t{}", i % 13), docs.clone()).unwrap(),
            ));
            d.add(Box::new(
                Field::new("sb", format!("t{}", i % 17), positions.clone()).unwrap(),
            ));
            w.add_document(&d).unwrap();
        }
        w.commit().unwrap();
    }
    w.force_merge(1).unwrap();
    w.commit().unwrap();
    w.close().unwrap();
    let rucene = dump(&dir.0);
    assert!(rucene.len() > 50, "{} lines", rucene.len());
    // Rucene's `live` lines meet core defect 1 (`Lucene104PostingsReader.impacts`).
    assert_same_lines_but_live(
        "stuniformsplit: sorted merge: Rucene and Lucene on Rucene's files",
        &rucene,
        &lucene_dump(&dir.0),
        true,
    );
}
