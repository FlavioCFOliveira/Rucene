/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! rmp #513: an HNSW merge reuses the graph of every source reader that is an
//! `HnswGraphProvider`, `AssertingKnnVectorsReader` included, exactly as
//! Lucene 10.5.0 does (`IncrementalHnswGraphMerger.addReader`,
//! `IncrementalHnswGraphMerger.java:85-121`).
//!
//! Two segments of 300 and 200 two-dimensional float vectors are written and
//! force-merged, once by Rucene and once by Lucene 10.5.0 (the program
//! [`FIXTURE`], run on the local JDK 25 with the Lucene 10.5.0 jars of the
//! local Maven repository, in a work directory of its own), under two codecs:
//!
//! * **direct** — a `FilterCodec` whose `knnVectorsFormat()` is an
//!   `AssertingKnnVectorsFormat` itself. The merge-state readers are
//!   `AssertingKnnVectorsReader`s, which are providers, so Lucene merges the
//!   two source graphs (`build graph from merging 2 graphs of 500 vectors,
//!   graph sizes:300 200 `). Before #513 Rucene's merger never saw a provider
//!   and rebuilt the graph from scratch (`build graph from 500 vectors`),
//!   writing a different `.vex`/`.vem`.
//! * **per-field** — `AssertingCodec`, whose `knnVectorsFormat()` is a
//!   `PerFieldKnnVectorsFormat`. `addReader` tests the per-field reader it is
//!   handed, which is not a provider, so Lucene 10.5.0 rebuilds the graph
//!   (`build graph from 500 vectors`); the port must too.
//!
//! In both cases the `HNSW` info-stream messages and every byte of the merged
//! vector files must be Lucene's, once Lucene's random segment id is replaced
//! by Rucene's (each side's footer checksum is checked on its own).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once};

use rucene::codecs::{
    Codec, CompoundFormat, DocValuesFormat, FieldInfosFormat, KnnVectorsFormat, LiveDocsFormat,
    Lucene104Codec, NormsFormat, PointsFormat, PostingsFormat, SegmentInfoFormat,
    StoredFieldsFormat, TermVectorsFormat,
};
use rucene::document::{Document, KnnFloatVectorField};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::SegmentInfos;
use rucene::store::{Directory, FSDirectory};
use rucene::util::InfoStream;
use rucene_test_framework::codecs::asserting::{AssertingCodec, AssertingKnnVectorsFormat};

/// The documents of the first and of the second source segment.
const SEGMENT_SIZES: [i32; 2] = [300, 200];

/// The Lucene 10.5.0 side: the same documents, codec and merge, printing the
/// `HNSW` `build graph` messages, the merged segment's name and id, and every
/// byte of its vector files.
const FIXTURE: &str = r#"
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.HexFormat;
import java.util.TreeSet;
import org.apache.lucene.codecs.Codec;
import org.apache.lucene.codecs.FilterCodec;
import org.apache.lucene.codecs.KnnVectorsFormat;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.KnnFloatVectorField;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.index.VectorSimilarityFunction;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.tests.codecs.asserting.AssertingCodec;
import org.apache.lucene.tests.codecs.asserting.AssertingKnnVectorsFormat;
import org.apache.lucene.util.InfoStream;

public class AssertingGraphMerge {
  public static void main(String[] args) throws Exception {
    Path out = Paths.get(args[0]);
    Codec codec =
        switch (args[1]) {
          case "direct" ->
              new FilterCodec("Lucene104", new Lucene104Codec()) {
                final KnnVectorsFormat format = new AssertingKnnVectorsFormat();

                @Override
                public KnnVectorsFormat knnVectorsFormat() {
                  return format;
                }
              };
          case "per-field" -> new AssertingCodec();
          default -> throw new IllegalArgumentException(args[1]);
        };
    IndexWriterConfig config = new IndexWriterConfig().setCodec(codec).setUseCompoundFile(false);
    config.setInfoStream(
        new InfoStream() {
          @Override
          public void message(String component, String message) {
            if (message.startsWith("build graph")) {
              System.out.println("hnsw " + message);
            }
          }

          @Override
          public boolean isEnabled(String component) {
            return component.equals("HNSW");
          }

          @Override
          public void close() {}
        });
    try (FSDirectory dir = FSDirectory.open(out)) {
      try (IndexWriter writer = new IndexWriter(dir, config)) {
        int doc = 0;
        for (int size : new int[] {300, 200}) {
          for (int i = 0; i < size; i++, doc++) {
            Document d = new Document();
            d.add(
                new KnnFloatVectorField(
                    "f",
                    new float[] {((doc * 7919) % 1009) / 7f, ((doc * 104729) % 1013) / 11f},
                    VectorSimilarityFunction.EUCLIDEAN));
            writer.addDocument(d);
          }
          writer.commit();
        }
        writer.forceMerge(1);
        writer.commit();
      }
      SegmentCommitInfo merged = SegmentInfos.readLatestCommit(dir).info(0);
      System.out.println("name " + merged.info.name);
      System.out.println("id " + HexFormat.of().formatHex(merged.info.getId()));
      for (String file : new TreeSet<>(merged.files())) {
        if (file.matches(".*\\.(vec|vem|vemf|vex)")) {
          System.out.println(
              "file " + file + " " + HexFormat.of().formatHex(Files.readAllBytes(out.resolve(file))));
        }
      }
    }
  }
}
"#;

/// The class path of [`FIXTURE`]: `lucene-core`, `lucene-codecs` (which
/// `AssertingCodec` loads) and `lucene-test-framework` with its runtime
/// dependencies, from the local Maven repository.
const JARS: [&str; 6] = [
    "org/apache/lucene/lucene-core/10.5.0/lucene-core-10.5.0.jar",
    "org/apache/lucene/lucene-codecs/10.5.0/lucene-codecs-10.5.0.jar",
    "org/apache/lucene/lucene-test-framework/10.5.0/lucene-test-framework-10.5.0.jar",
    "com/carrotsearch/randomizedtesting/randomizedtesting-runner/2.8.1/randomizedtesting-runner-2.8.1.jar",
    "junit/junit/4.13.1/junit-4.13.1.jar",
    "org/hamcrest/hamcrest/2.2/hamcrest-2.2.jar",
];

/// A directory of its own under `TMPDIR`, removed on drop, so that the tests
/// of this file run in parallel safely.
struct WorkDir(PathBuf);

impl WorkDir {
    fn new(label: &str) -> WorkDir {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "rucene-asserting-graph-merge-{label}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create the work directory");
        WorkDir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What one merge produced: the `HNSW` `build graph` messages, the merged
/// segment's name and id, and its vector files with their bytes.
#[derive(Debug)]
struct Merged {
    hnsw: Vec<String>,
    name: String,
    id: [u8; 16],
    files: Vec<(String, Vec<u8>)>,
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hexadecimal"))
        .collect()
}

/// Runs [`FIXTURE`] for `case` and parses what it printed.
fn lucene(case: &str) -> Merged {
    let work = WorkDir::new(&format!("java-{case}"));
    let source = work.path().join("AssertingGraphMerge.java");
    std::fs::write(&source, FIXTURE).expect("write the Java source");
    let index = work.path().join("index");
    std::fs::create_dir_all(&index).expect("create the index directory");
    let home = std::env::var("HOME").expect("HOME is set");
    let jars: Vec<PathBuf> = JARS
        .iter()
        .map(|jar| {
            let path = Path::new(&home).join(".m2/repository").join(jar);
            assert!(path.is_file(), "the jar is required: {}", path.display());
            path
        })
        .collect();
    let output = Command::new("java")
        .current_dir(work.path())
        .arg(format!("-Djava.io.tmpdir={}", work.path().display()))
        .arg("-Xmx512m")
        .arg("-cp")
        .arg(std::env::join_paths(&jars).expect("a class path"))
        .arg(&source)
        .arg(&index)
        .arg(case)
        .output()
        .expect("a JDK 25 is required");
    assert!(
        output.status.success(),
        "AssertingGraphMerge.java {case} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("the JVM prints UTF-8");
    let mut merged = Merged {
        hnsw: Vec::new(),
        name: String::new(),
        id: [0; 16],
        files: Vec::new(),
    };
    for line in stdout.lines() {
        if let Some(message) = line.strip_prefix("hnsw ") {
            merged.hnsw.push(message.to_string());
        } else if let Some(name) = line.strip_prefix("name ") {
            merged.name = name.to_string();
        } else if let Some(id) = line.strip_prefix("id ") {
            merged.id = unhex(id).try_into().expect("a 16-byte id");
        } else if let Some(file) = line.strip_prefix("file ") {
            let (name, bytes) = file.split_once(' ').expect("name and bytes");
            merged.files.push((name.to_string(), unhex(bytes)));
        }
    }
    merged
}

/// The info stream of [`FIXTURE`]: records the `HNSW` `build graph` messages.
#[derive(Default)]
struct BuildGraphMessages(Mutex<Vec<String>>);

impl InfoStream for BuildGraphMessages {
    fn java_class_name(&self) -> &'static str {
        "AssertingGraphMerge$1"
    }

    fn message(&self, _component: &str, message: &str) {
        if message.starts_with("build graph") {
            self.0.lock().unwrap().push(message.to_string());
        }
    }

    fn is_enabled(&self, component: &str) -> bool {
        component == "HNSW"
    }

    fn close(&self) {}
}

/// `new FilterCodec("Lucene104", new Lucene104Codec()) { knnVectorsFormat()
/// { return new AssertingKnnVectorsFormat(); } }`.
#[derive(Debug)]
struct DirectAssertingCodec {
    inner: Lucene104Codec,
    knn: AssertingKnnVectorsFormat,
}

impl Codec for DirectAssertingCodec {
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

/// The service files of `lucene-test-framework` on the class path, as on
/// [`FIXTURE`]'s: the segments of the per-field case name the `Asserting`
/// vectors format, which a merge resolves by name.
fn install_services() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let loader = rucene_test_framework::class_loader();
        rucene::codecs::reload_codecs(loader).unwrap();
        rucene::codecs::knn_vectors::reload_knn_vectors_format(loader).unwrap();
    });
}

fn codec(case: &str) -> Arc<dyn Codec> {
    install_services();
    match case {
        "direct" => Arc::new(DirectAssertingCodec {
            inner: Lucene104Codec::new(),
            knn: AssertingKnnVectorsFormat::new(),
        }),
        "per-field" => Arc::new(AssertingCodec::new()),
        _ => unreachable!("{case}"),
    }
}

/// Rucene's side of [`FIXTURE`].
fn rucene(case: &str) -> Merged {
    let work = WorkDir::new(&format!("rust-{case}"));
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(work.path()).unwrap());
    let messages = Arc::new(BuildGraphMessages::default());
    let config = IndexWriterConfig::new();
    config.set_codec(codec(case)).unwrap();
    config.set_use_compound_file(false).unwrap();
    config
        .set_info_stream(Arc::clone(&messages) as Arc<dyn InfoStream>)
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    let mut doc = 0;
    for size in SEGMENT_SIZES {
        for _ in 0..size {
            let mut d = Document::new();
            d.add(Box::new(
                KnnFloatVectorField::with_euclidean(
                    "f",
                    &[
                        ((doc * 7919) % 1009) as f32 / 7.0,
                        ((doc * 104729) % 1013) as f32 / 11.0,
                    ],
                )
                .unwrap(),
            ));
            writer.add_document(&d).unwrap();
            doc += 1;
        }
        writer.commit().unwrap();
    }
    writer.force_merge(1).unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();

    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    assert_eq!(infos.size(), 1);
    let merged = &infos.info(0).info;
    let mut names: Vec<String> = infos
        .info(0)
        .files()
        .unwrap()
        .into_iter()
        .filter(|f| {
            [".vec", ".vem", ".vemf", ".vex"]
                .iter()
                .any(|e| f.ends_with(e))
        })
        .collect();
    names.sort();
    let files = names
        .into_iter()
        .map(|name| {
            let bytes = std::fs::read(work.path().join(&name)).unwrap();
            (name, bytes)
        })
        .collect();
    let hnsw = messages.0.lock().unwrap().clone();
    Merged {
        hnsw,
        name: merged.name.clone(),
        id: merged.id(),
        files,
    }
}

/// `java.util.zip.CRC32` of `bytes` (CRC-32/ISO-HDLC, reflected polynomial
/// `0xEDB88320`), computed bit by bit.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Asserts that the footer's checksum is the CRC-32 of everything before it,
/// as `CodecUtil.writeFooter` writes it.
fn assert_valid_footer(side: &str, name: &str, file: &[u8]) {
    let (body, footer) = file.split_at(file.len() - 8);
    assert_eq!(
        u64::from_be_bytes(footer.try_into().unwrap()),
        u64::from(crc32(body)),
        "{side} {name}: the footer checksum must cover the file"
    );
}

/// Replaces every occurrence of `from` in `bytes` by `to`.
fn replace_id(bytes: &[u8], from: &[u8; 16], to: &[u8; 16]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let mut i = 0;
    while i + 16 <= out.len() {
        if &out[i..i + 16] == from {
            out[i..i + 16].copy_from_slice(to);
            i += 16;
        } else {
            i += 1;
        }
    }
    out
}

/// Merges under `case` on both sides and asserts that Rucene took Lucene's
/// route (`expected_hnsw`) and wrote Lucene's vector files.
fn assert_merges_like_lucene(case: &str, expected_hnsw: &str) {
    let java = lucene(case);
    let rust = rucene(case);
    assert_eq!(java.hnsw, [expected_hnsw], "Lucene's merge route");
    assert_eq!(rust.hnsw, java.hnsw, "the merge route");
    assert_eq!(rust.name, java.name, "the merged segment");
    let names = |m: &Merged| m.files.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
    assert_eq!(names(&rust), names(&java), "the merged vector files");
    assert_eq!(java.files.len(), 4, "{:?}", names(&java));
    for ((name, rust_bytes), (_, java_bytes)) in rust.files.iter().zip(&java.files) {
        assert_valid_footer("Rucene", name, rust_bytes);
        assert_valid_footer("Lucene", name, java_bytes);
        let java_bytes = replace_id(java_bytes, &java.id, &rust.id);
        assert_eq!(rust_bytes.len(), java_bytes.len(), "{name}: length");
        let body = rust_bytes.len() - 8;
        if rust_bytes[..body] != java_bytes[..body] {
            let at = (0..body)
                .find(|&i| rust_bytes[i] != java_bytes[i])
                .expect("a differing byte");
            panic!("{name}: first differing byte at {at} of {body}");
        }
    }
}

/// The regression of #513: an `AssertingKnnVectorsReader` in the merge state
/// hands out its delegate's graph, so the merge starts from the two source
/// graphs (the largest first) instead of rebuilding, and writes Lucene's
/// files.
#[test]
fn a_direct_asserting_merge_starts_from_the_source_graphs() {
    assert_merges_like_lucene(
        "direct",
        "build graph from merging 2 graphs of 500 vectors, graph sizes:300 200 ",
    );
}

/// Lucene 10.5.0 tests the per-field reader it is handed, which is not a
/// provider (`IncrementalHnswGraphMerger.java:89-95`), so under
/// `AssertingCodec` the merge rebuilds the graph, and so does Rucene.
#[test]
fn a_per_field_asserting_merge_rebuilds_the_graph_as_lucene_does() {
    assert_merges_like_lucene("per-field", "build graph from 500 vectors");
}
