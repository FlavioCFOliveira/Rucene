//! rmp #228: `PerFieldKnnVectorsFormat.FieldsWriter` keys its writers by the
//! format **instance** (`Map<KnnVectorsFormat, WriterAndSuffix> formats`,
//! `PerFieldKnnVectorsFormat.java:103, 144-177`), whose `equals` is
//! identity: a codec answering a new `Lucene99HnswVectorsFormat` for each of
//! four fields writes four suffixes, `_0` to `_3`; one instance shared by
//! every field writes one. `PerFieldKnnSuffixFixture`
//! (`tests/fixtures/java-codec-harness`) writes the same segment with Lucene
//! 10.5.0 and prints its vector files; Rucene's must be the same names.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use rucene::codecs::lucene99::hnsw_vectors_format::Lucene99HnswVectorsFormat;
use rucene::codecs::per_field::PerFieldKnnVectorsFormat;
use rucene::codecs::{
    Codec, CompoundFormat, DocValuesFormat, FieldInfosFormat, KnnVectorsFormat, LiveDocsFormat,
    Lucene104Codec, NormsFormat, PointsFormat, PostingsFormat, SegmentInfoFormat,
    StoredFieldsFormat, TermVectorsFormat,
};
use rucene::document::{Document, KnnFloatVectorField};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::store::{Directory, FSDirectory};

/// `Lucene104Codec` overriding `getKnnVectorsFormatForField`.
#[derive(Debug)]
struct PerFieldCodec {
    inner: Lucene104Codec,
    knn: PerFieldKnnVectorsFormat,
}

impl Codec for PerFieldCodec {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn postings_format(&self) -> &dyn PostingsFormat {
        self.inner.postings_format()
    }
    fn doc_values_format(&self) -> &dyn DocValuesFormat {
        self.inner.doc_values_format()
    }
    fn stored_fields_format(&self) -> &dyn StoredFieldsFormat {
        self.inner.stored_fields_format()
    }
    fn term_vectors_format(&self) -> &dyn TermVectorsFormat {
        self.inner.term_vectors_format()
    }
    fn field_infos_format(&self) -> &dyn FieldInfosFormat {
        self.inner.field_infos_format()
    }
    fn segment_info_format(&self) -> &dyn SegmentInfoFormat {
        self.inner.segment_info_format()
    }
    fn norms_format(&self) -> &dyn NormsFormat {
        self.inner.norms_format()
    }
    fn live_docs_format(&self) -> &dyn LiveDocsFormat {
        self.inner.live_docs_format()
    }
    fn compound_format(&self) -> &dyn CompoundFormat {
        self.inner.compound_format()
    }
    fn points_format(&self) -> &dyn PointsFormat {
        self.inner.points_format()
    }
    fn knn_vectors_format(&self) -> &dyn KnnVectorsFormat {
        &self.knn
    }
}

fn lucene_jar() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME is set");
    let jar = Path::new(&home)
        .join(".m2/repository/org/apache/lucene/lucene-core/10.5.0/lucene-core-10.5.0.jar");
    assert!(
        jar.is_file(),
        "the Lucene 10.5.0 jar is required: {}",
        jar.display()
    );
    jar
}

/// The `_0_*` files `PerFieldKnnSuffixFixture` writes.
fn lucene_files(shared: bool) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         PerFieldKnnSuffixFixture.java",
    );
    let output = Command::new("java")
        .arg("-cp")
        .arg(lucene_jar())
        .arg(&fixture)
        .arg(dir.path().join("idx"))
        .arg(shared.to_string())
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "PerFieldKnnSuffixFixture failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

/// The `_0_*` files Rucene writes for the same segment.
fn rucene_files(shared: bool) -> Vec<String> {
    let _ = rucene::codecs::register_codec("Lucene104", Lucene104Codec::new());
    let tmp = tempfile::tempdir().unwrap();
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(tmp.path()).unwrap());
    let one: Arc<dyn KnnVectorsFormat> = Arc::new(Lucene99HnswVectorsFormat::new());
    let knn = PerFieldKnnVectorsFormat::new(move |_field| {
        if shared {
            Arc::clone(&one)
        } else {
            Arc::new(Lucene99HnswVectorsFormat::new()) as Arc<dyn KnnVectorsFormat>
        }
    });
    let mut config = IndexWriterConfig::new();
    config.set_use_compound_file(false).unwrap();
    config
        .set_merge_policy(Some(Arc::new(NoMergePolicy::new())))
        .unwrap();
    config
        .set_codec(Arc::new(PerFieldCodec {
            inner: Lucene104Codec::new(),
            knn,
        }))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    let mut doc = Document::new();
    for i in 0..4 {
        doc.add(Box::new(
            KnnFloatVectorField::with_euclidean(&format!("v{i}"), &[i as f32, (i + 1) as f32])
                .unwrap(),
        ));
    }
    writer.add_document(&doc).unwrap();
    writer.commit().unwrap();
    let mut files: Vec<String> = dir
        .list_all()
        .unwrap()
        .into_iter()
        .filter(|f| f.starts_with("_0_"))
        .collect();
    files.sort();
    writer.close().unwrap();
    files
}

#[test]
fn one_format_instance_per_field_writes_one_suffix_per_field() {
    let lucene = lucene_files(false);
    assert_eq!(
        lucene.len(),
        16,
        "four files for each of four suffixes: {lucene:?}"
    );
    assert_eq!(rucene_files(false), lucene);
}

#[test]
fn one_shared_format_instance_writes_one_suffix() {
    let lucene = lucene_files(true);
    assert_eq!(lucene.len(), 4, "{lucene:?}");
    assert_eq!(rucene_files(true), lucene);
}
