//! Tests proving the behaviour closed by the sprint 7 census gaps
//! (`census-d2a623c`) that is reachable through the crate's public API and
//! needs an index. Each test names its gap (`gap_gNNN_...`) and cites the
//! Lucene 10.5.0 source that defines the expected behaviour.

use std::sync::Arc;

use crate::codecs::{register_codec, Lucene104Codec};
use crate::document::Document;
use crate::error::{LuceneError, LuceneErrorKind};
use crate::index::directory_reader::DirectoryReader;
use crate::index::index_writer::IndexWriter;
use crate::index::index_writer_config::IndexWriterConfig;
use crate::search::IndexSearcher;
use crate::store::{ByteBuffersDirectory, Directory};

/// An in-memory index of `docs`, flushed into one segment, with its writer
/// still open.
struct TestIndex {
    writer: Arc<IndexWriter>,
    reader: Arc<dyn DirectoryReader>,
}

impl TestIndex {
    fn new(docs: Vec<Document>) -> Self {
        Self::with_config(docs, IndexWriterConfig::new())
    }

    fn with_config(docs: Vec<Document>, config: IndexWriterConfig) -> Self {
        let _ = register_codec("Lucene104", Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        for doc in &docs {
            writer.add_document(doc).unwrap();
        }
        writer.force_merge(1).unwrap();
        let reader = crate::index::directory_reader::open_with_writer(&*writer).unwrap();
        Self { writer, reader }
    }

    fn searcher(&self) -> IndexSearcher {
        IndexSearcher::new(Arc::clone(&self.reader) as Arc<dyn crate::index::IndexReader>).unwrap()
    }
}

impl Drop for TestIndex {
    fn drop(&mut self) {
        let _ = self.reader.close();
        let _ = self.writer.close();
    }
}

fn illegal_state_message(error: &LuceneError) -> &str {
    match &error.kind {
        LuceneErrorKind::IllegalState { message, .. } => message,
        other => panic!("expected IllegalStateException, got {other:?}"),
    }
}

mod search {
    use super::*;
    use crate::document::{DoubleDocValuesField, NumericDocValuesField, SortedDocValuesField};
    use crate::index::Term;
    use crate::search::doc_values_rewrite_method::DocValuesRewriteMethod;
    use crate::search::sort::MissingValue;
    use crate::search::{MatchAllDocsQuery, PrefixQuery, Query, Sort};
    use crate::util::BytesRef;

    /// G-012: `DoubleValuesSortField.setMissingValue(Number)` forwards the
    /// value to its `DoubleValuesComparatorSource`
    /// (`DoubleValuesSource.java:532-541`), and `toString()` is
    /// `"<" + getField() + ">"` plus `!` when reversed
    /// (`DoubleValuesSource.java:549-554`); the same holds for
    /// `LongValuesSortField` (`LongValuesSource.java:293-315`).
    #[test]
    fn gap_g012_values_sort_field_missing_value_and_to_string() {
        let mut with_value = Document::new();
        with_value.add(Box::new(DoubleDocValuesField::new("d", 1.0)));
        with_value.add(Box::new(NumericDocValuesField::new("l", 1)));
        let index = TestIndex::new(vec![with_value, Document::new()]);
        let searcher = index.searcher();
        let order = |sort_field: crate::search::SortField| -> Vec<i32> {
            let sort = Sort::new_fields(vec![sort_field]).unwrap();
            searcher
                .search_with_sort(Arc::new(MatchAllDocsQuery), 10, &sort)
                .unwrap()
                .score_docs
                .iter()
                .map(|d| d.score_doc.doc)
                .collect()
        };

        let source = crate::search::double_values_source::from_double_field("d");
        let mut field =
            crate::search::double_values_source::get_sort_field(Arc::clone(&source), true).unwrap();
        assert_eq!(
            field.to_string(),
            format!("<{}>!", source.to_source_string())
        );
        field
            .set_missing_value(Some(MissingValue::Double(5.0)))
            .unwrap();
        // Reversed: the missing document (5.0) sorts before the 1.0 one.
        assert_eq!(order(field.clone()), vec![1, 0]);
        field
            .set_missing_value(Some(MissingValue::Double(-5.0)))
            .unwrap();
        assert_eq!(order(field), vec![0, 1]);

        let source = crate::search::long_values_source::from_long_field("l");
        let mut field =
            crate::search::long_values_source::get_sort_field(Arc::clone(&source), false).unwrap();
        assert_eq!(
            field.to_string(),
            format!("<{}>", source.to_source_string())
        );
        field
            .set_missing_value(Some(MissingValue::Long(-7)))
            .unwrap();
        assert_eq!(order(field.clone()), vec![1, 0]);
        field
            .set_missing_value(Some(MissingValue::Long(7)))
            .unwrap();
        assert_eq!(order(field), vec![0, 1]);
    }

    fn prefix_dv_query(field: &str, prefix: &str) -> Arc<dyn Query> {
        Arc::new(
            PrefixQuery::with_rewrite_method(
                Term::from_text(field, prefix),
                Arc::new(DocValuesRewriteMethod),
            )
            .unwrap(),
        )
    }

    /// G-013: on a single-valued (`SORTED`) field, `DocValuesRewriteMethod`
    /// unwraps the singleton and builds `DocValuesRangeIterator.forOrdinalSet`
    /// over the `SortedDocValues` (`DocValuesRewriteMethod.java:166-174`),
    /// here with a doc-values skipper.
    #[test]
    fn gap_g013_doc_values_rewrite_takes_the_singleton_branch() {
        let docs = ["apple", "apricot", "banana", "avocado", "cherry"]
            .iter()
            .map(|v| {
                let mut doc = Document::new();
                doc.add(Box::new(SortedDocValuesField::indexed_field(
                    "s",
                    BytesRef::from_char_sequence(v),
                )));
                doc
            })
            .collect();
        let index = TestIndex::new(docs);
        let leaves = Arc::clone(&index.reader).leaves().unwrap();
        let values = crate::index::doc_values::DocValues::get_sorted_set(
            leaves[0].leaf_reader().as_ref(),
            "s",
        )
        .unwrap();
        assert!(
            values.is_singleton(),
            "the singleton branch is the one taken"
        );
        assert!(leaves[0]
            .leaf_reader()
            .get_doc_values_skipper("s")
            .unwrap()
            .is_some());
        let hits = index
            .searcher()
            .search(prefix_dv_query("s", "a"), 10)
            .unwrap();
        let mut docs: Vec<i32> = hits.score_docs.iter().map(|d| d.doc).collect();
        docs.sort_unstable();
        assert_eq!(docs, vec![0, 1, 3]);
    }

    /// G-019: `DocValues.getSortedSet` runs `checkField`, so a field indexed
    /// with another doc-values type raises `IllegalStateException`
    /// (`DocValues.java:206-226, 317-330`) instead of matching nothing.
    #[test]
    fn gap_g019_doc_values_rewrite_rejects_another_doc_values_type() {
        let mut doc = Document::new();
        doc.add(Box::new(NumericDocValuesField::new("n", 3)));
        let index = TestIndex::new(vec![doc]);
        let error = index
            .searcher()
            .search(prefix_dv_query("n", "a"), 10)
            .unwrap_err();
        assert_eq!(
            illegal_state_message(&error),
            "unexpected docvalues type NUMERIC for field 'n' (expected one of [SORTED, \
             SORTED_SET]). Re-index with correct docvalues type."
        );
    }
}

/// Runs one case of the Java oracle `CensusGapFixture` against
/// `lucene-core-10.5.0.jar` and returns its standard output.
pub(crate) fn run_oracle(jvm_args: &[&str], case_args: &[&str]) -> String {
    let home = std::env::var("HOME").expect("HOME is set");
    let jar = std::path::Path::new(&home)
        .join(".m2/repository/org/apache/lucene/lucene-core/10.5.0/lucene-core-10.5.0.jar");
    assert!(
        jar.is_file(),
        "the Lucene 10.5.0 jar is required: {}",
        jar.display()
    );
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         CensusGapFixture.java",
    );
    let output = std::process::Command::new("java")
        .arg("--enable-native-access=ALL-UNNAMED")
        .args(jvm_args)
        .arg("-cp")
        .arg(jar)
        .arg(fixture)
        .args(case_args)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "CensusGapFixture {case_args:?} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// `Throwable.toString()` of the Java exception `error` stands for, for the
/// kinds the oracle cases compare.
fn java_to_string(error: &LuceneError) -> String {
    match &error.kind {
        LuceneErrorKind::Io(io) => crate::store::java_nio_file::java_to_string(io),
        LuceneErrorKind::Unchecked {
            class,
            message: Some(m),
            ..
        } => format!("{class}: {m}"),
        LuceneErrorKind::Unchecked {
            class,
            message: None,
            ..
        } => class.to_string(),
        LuceneErrorKind::AlreadyClosed { message, .. } => {
            format!("org.apache.lucene.store.AlreadyClosedException: {message}")
        }
        other => panic!("no Java rendering for {other:?}"),
    }
}

fn outcome(result: crate::error::Result<()>) -> String {
    match result {
        Ok(()) => "ok".to_string(),
        Err(e) => java_to_string(&e),
    }
}

mod oracle {
    use super::*;

    /// G-022: `QueryVisitor.termCollector(Set<Term>)` fills the caller's set;
    /// over a `java.util.HashSet` the terms iterate in `HashSet` order
    /// (`QueryVisitor.java:79-86`), which `JavaObjectHashSet` reproduces.
    /// `Term.hashCode` hashes the bytes with `StringHelper.GOOD_FAST_HASH_SEED`,
    /// derived from `tests.seed` (`StringHelper.java:142-151`), so both sides
    /// run under the same seed.
    #[test]
    fn gap_g022_term_collector_iterates_in_java_hash_set_order() {
        std::env::set_var("tests.seed", "C0FFEE");
        use crate::index::Term;
        use crate::search::boolean_clause::Occur;
        use crate::search::phrase_query::PhraseQuery;
        use crate::search::query_visitor::term_collector;
        use crate::search::{BooleanQueryBuilder, Query, TermQuery};
        let mut builder = BooleanQueryBuilder::new();
        for (field, text) in [
            ("body", "zebra"),
            ("title", "apple"),
            ("body", "mango"),
            ("id", "42"),
            ("body", "kiwi"),
            ("title", "pear"),
            ("tag", "x"),
            ("body", "banana"),
            ("title", "zebra"),
        ] {
            builder
                .add(
                    Arc::new(TermQuery::new(Term::from_text(field, text))),
                    Occur::SHOULD,
                )
                .unwrap();
        }
        let phrase = PhraseQuery::new(
            0,
            ["quick", "brown", "fox"]
                .iter()
                .map(|t| Term::from_text("body", t))
                .collect(),
            vec![0, 1, 2],
        )
        .unwrap();
        builder.add(Arc::new(phrase), Occur::MUST).unwrap();
        let query = builder.build();
        let mut collected = crate::util::java_collections::JavaObjectHashSet::new();
        query.visit(&mut term_collector(&mut collected));
        let rust: Vec<String> = collected
            .iter()
            .map(|t: &Term| format!("{}:{}", t.field(), t.bytes().utf8_to_string().unwrap()))
            .collect();
        let java: Vec<String> = run_oracle(&["-Dtests.seed=C0FFEE"], &["termCollector"])
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(rust, java);
    }

    fn fma_lines() -> Vec<String> {
        let vector = |seed: i32, length: usize| -> Vec<f32> {
            (0..length)
                .map(|i| {
                    (f64::from(seed * 31) + i as f64 * 7.3).sin() as f32 * 1000.5 + i as f32 / 3.0
                })
                .collect()
        };
        let mut lines = vec![format!(
            "HAS_FAST_SCALAR_FMA={}",
            crate::util::vector_util::use_fma()
        )];
        for length in [3usize, 37, 64] {
            let (a, b) = (vector(1, length), vector(2, length));
            lines.push(format!(
                "len={length} dot={:x} square={:x} cosine={:x}",
                crate::util::vector_util::dot_product_f32(&a, &b)
                    .unwrap()
                    .to_bits(),
                crate::util::vector_util::square_distance_f32(&a, &b)
                    .unwrap()
                    .to_bits(),
                crate::util::vector_util::cosine_f32(&a, &b)
                    .unwrap()
                    .to_bits(),
            ));
        }
        lines
    }

    fn java_lines(output: &str) -> Vec<String> {
        output.lines().map(str::to_string).collect()
    }

    /// G-031: `Constants.HAS_FAST_SCALAR_FMA` decides whether
    /// `DefaultVectorUtilSupport` fuses the float accumulation
    /// (`Constants.java:112, 143-166`); with Lucene's default (`auto`) the
    /// flag and the scores agree with the JVM, bit for bit, on this host.
    #[test]
    fn gap_g031_scalar_fma_auto_detection_matches_lucene() {
        assert_eq!(fma_lines(), java_lines(&run_oracle(&[], &["fma"])));
    }

    /// Environment variable that turns [`gap_g031_hotspot_cpu_options_match_lucene`]
    /// into the child process printing this process's `Constants` values.
    const G031_CHILD: &str = "RUCENE_G031_CHILD";

    /// The `constants` oracle case's lines for this process.
    fn constants_lines() -> Vec<String> {
        use crate::util::Constants;
        let mut lines = vec![
            format!("HAS_FMA={}", Constants::has_fma()),
            format!("HAS_AVX2={}", Constants::has_avx2()),
            format!("HAS_SVE={}", Constants::has_sve()),
            format!("HAS_SSE4A={}", Constants::has_sse4a()),
            format!("MAX_VECTOR_SIZE={}", Constants::max_vector_size()),
            format!("HAS_FAST_VECTOR_FMA={}", Constants::has_fast_vector_fma()),
            format!("HAS_FAST_SCALAR_FMA={}", Constants::has_fast_scalar_fma()),
        ];
        lines.extend(fma_lines());
        lines
    }

    /// G-031: `Constants.HAS_FMA`, `HAS_AVX2`, `HAS_SVE`, `HAS_SSE4A` and
    /// `MAX_VECTOR_SIZE` read HotSpot's `UseFMA`, `UseAVX`, `UseSVE`,
    /// `UseXmmI2F` and `MaxVectorSize` (`Constants.java:89-106`), so with
    /// `-Dlucene.useScalarFMA=true` the scalar similarity functions fuse their
    /// accumulation (`DefaultVectorUtilSupport.java:29-36`). The values and the
    /// score bits equal a default JVM's on this host, with Lucene's default
    /// (`auto`) and with the flag. `vector_util` reads the flag once per
    /// process, so each configuration runs in a child process.
    #[test]
    fn gap_g031_hotspot_cpu_options_match_lucene() {
        if std::env::var_os(G031_CHILD).is_some() {
            for line in constants_lines() {
                println!("g031 {line}");
            }
            return;
        }
        let test_name = concat!(
            module_path!(),
            "::gap_g031_hotspot_cpu_options_match_lucene"
        )
        .split_once("::")
        .expect("INVARIANT: module_path! starts with the crate name")
        .1;
        for flag in [None, Some("true")] {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child
                .args([test_name, "--exact", "--nocapture", "--test-threads=1"])
                .env(G031_CHILD, "1")
                .env_remove("lucene.useScalarFMA");
            let mut jvm_args = Vec::new();
            if let Some(flag) = flag {
                child.env("lucene.useScalarFMA", flag);
                jvm_args.push("-Dlucene.useScalarFMA=true");
            }
            let output = child.output().unwrap();
            assert!(output.status.success(), "{output:?}");
            // With `--nocapture` the harness may write `test <name> ... `
            // before the test's own output, on the same line.
            let rust: Vec<String> = String::from_utf8(output.stdout)
                .unwrap()
                .lines()
                .filter_map(|line| line.find("g031 ").map(|at| line[at + 5..].to_string()))
                .collect();
            let java = java_lines(&run_oracle(&jvm_args, &["constants"]));
            assert_eq!(rust, java, "lucene.useScalarFMA={flag:?}");
        }
    }

    /// A fired timeout whose `toString()` is `"T"`, as the oracle's.
    #[derive(Debug)]
    struct FiredTimeout;

    impl crate::index::QueryTimeout for FiredTimeout {
        fn should_exit(&self) -> bool {
            true
        }

        fn java_to_string(&self) -> String {
            "T".to_string()
        }
    }

    /// `Throwable.toString()` of an `ExitingReaderException`, or `ok`.
    fn exiting_outcome<T>(result: crate::error::Result<T>) -> String {
        match result {
            Ok(_) => "ok".to_string(),
            Err(error) => match &error.kind {
                LuceneErrorKind::ExitingReader(message) => format!(
                    "org.apache.lucene.index.ExitableDirectoryReader$ExitingReaderException: \
                     {message}"
                ),
                other => panic!("expected ExitingReaderException, got {other:?}"),
            },
        }
    }

    /// Drops the identity-hash digits of every `name@hex`, which the JVM draws
    /// at random.
    fn without_identity_hashes(line: &str) -> String {
        let mut out = String::with_capacity(line.len());
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            out.push(c);
            if c == '@' {
                while chars.peek().is_some_and(char::is_ascii_hexdigit) {
                    chars.next();
                }
            }
        }
        out
    }

    /// A timeout that never fires, whose `toString()` is `"N"`, as the
    /// oracle's.
    #[derive(Debug)]
    struct NeverTimeout;

    impl crate::index::QueryTimeout for NeverTimeout {
        fn should_exit(&self) -> bool {
            false
        }

        fn java_to_string(&self) -> String {
            "N".to_string()
        }
    }

    /// A timeout fired on demand, whose `toString()` is `"S"`, as the
    /// oracle's.
    #[derive(Debug, Default)]
    struct SwitchableTimeout(std::sync::atomic::AtomicBool);

    impl crate::index::QueryTimeout for SwitchableTimeout {
        fn should_exit(&self) -> bool {
            self.0.load(std::sync::atomic::Ordering::SeqCst)
        }

        fn java_to_string(&self) -> String {
            "S".to_string()
        }
    }

    /// The oracle's `CensusGapFixture.InterruptingVisitor`: it interrupts its
    /// own thread on its first callback.
    #[derive(Default)]
    struct InterruptingVisitor(std::cell::Cell<bool>);

    impl InterruptingVisitor {
        fn interrupt_once(&self) {
            if !self.0.replace(true) {
                crate::util::InterruptHandle::current().interrupt();
            }
        }
    }

    impl crate::index::point_values::IntersectVisitor for InterruptingVisitor {
        fn java_to_string(&self) -> String {
            crate::index::index_reader::java_object_to_string_stand_in(
                "org.apache.lucene.rucene.codec.CensusGapFixture$InterruptingVisitor",
            )
        }

        fn visit(&mut self, _doc_id: i32) -> crate::error::Result<()> {
            self.interrupt_once();
            Ok(())
        }

        fn visit_with_value(&mut self, _doc_id: i32, _packed: &[u8]) -> crate::error::Result<()> {
            self.interrupt_once();
            Ok(())
        }

        fn compare(
            &self,
            _min: &[u8],
            _max: &[u8],
        ) -> crate::error::Result<crate::index::point_values::Relation> {
            self.interrupt_once();
            Ok(crate::index::point_values::Relation::CellCrossesQuery)
        }

        fn grow(&mut self, _count: i32) -> crate::error::Result<()> {
            self.interrupt_once();
            Ok(())
        }
    }

    /// `CensusGapFixture.exitableCalls(leaf).get(call)`: each call reaches one
    /// `ExitableDirectoryReader` check.
    fn exitable_call(leaf: &dyn crate::index::LeafReader, call: usize) -> String {
        use crate::search::knn::TopKnnCollector;
        match call {
            0 => exiting_outcome(
                leaf.get_numeric_doc_values("n")
                    .and_then(|v| v.unwrap().next_doc()),
            ),
            1 => exiting_outcome(
                leaf.get_sorted_doc_values("sd")
                    .and_then(|v| v.unwrap().next_doc()),
            ),
            2 => exiting_outcome(
                leaf.get_binary_doc_values("b")
                    .and_then(|v| v.unwrap().next_doc()),
            ),
            3 => exiting_outcome(
                leaf.get_sorted_numeric_doc_values("sn")
                    .and_then(|v| v.unwrap().next_doc()),
            ),
            4 => exiting_outcome(
                leaf.get_sorted_set_doc_values("ss")
                    .and_then(|v| v.unwrap().next_doc()),
            ),
            5 => exiting_outcome(
                leaf.terms("s")
                    .and_then(|t| t.unwrap().iterator())
                    .and_then(|mut e| e.next()),
            ),
            6 => exiting_outcome(leaf.get_point_values("p")),
            7 => exiting_outcome(
                leaf.get_float_vector_values("fv")
                    .and_then(|v| v.unwrap().iterator())
                    .and_then(|mut i| i.next_doc()),
            ),
            8 => exiting_outcome(
                leaf.get_byte_vector_values("bv")
                    .and_then(|v| v.unwrap().iterator())
                    .and_then(|mut i| i.next_doc()),
            ),
            _ => {
                let mut collector = TopKnnCollector::new(2, 100);
                let mut accept_docs = crate::search::from_live_docs(None, 3).unwrap();
                exiting_outcome(leaf.search_nearest_vectors(
                    "fv",
                    &[1.0, 1.0],
                    &mut collector,
                    &mut accept_docs,
                ))
            }
        }
    }

    /// G-028: every `ExitingReaderException` of `ExitableDirectoryReader`
    /// names the timeout and the iterated object: `". Timeout: " +
    /// queryTimeout + ", DocValues=" + in` and so on, and, when the timeout
    /// has not fired but the thread is interrupted, `"Interrupted while ...
    /// DocValues=" + in` (`ExitableDirectoryReader.java:439-914`). The
    /// messages equal Lucene's, class names included, up to the identity-hash
    /// digits of the objects that declare no `toString()`; the port's digits
    /// are the stable stand-in. The oracle also covers an exitable leaf over
    /// an exitable leaf, a `SortingCodecReader` view, `ExitablePointTree`
    /// (its timeout names the point values, its interrupt the tree's
    /// `nodeID=` rendering) and `ExitableIntersectVisitor`.
    #[test]
    fn gap_g028_exiting_reader_messages_match_lucene() {
        use crate::document::{
            BinaryDocValuesField, IntPoint, KnnByteVectorField, KnnFloatVectorField,
            NumericDocValuesField, SortedDocValuesField, SortedNumericDocValuesField,
            SortedSetDocValuesField, Store, StringField,
        };
        use crate::index::filter_directory_reader::{
            DirectoryReaderWrapper, ExitableDirectoryReaderWrapper, ExitableFilterLeafReader,
        };
        use crate::index::sorting_codec_reader::SortingCodecReader;
        use crate::index::LeafReader;
        use crate::index::VectorSimilarityFunction::EUCLIDEAN;
        use crate::search::{Sort, SortField, SortFieldType};
        use crate::util::BytesRef;
        use crate::util::InterruptHandle;
        let docs = (0..3)
            .map(|i| {
                let mut d = Document::new();
                d.add(Box::new(
                    StringField::new("s", format!("t{i}"), Store::NO).unwrap(),
                ));
                d.add(Box::new(NumericDocValuesField::new("n", i)));
                let value = || BytesRef::new(format!("v{i}").into_bytes());
                d.add(Box::new(SortedDocValuesField::new("sd", value())));
                d.add(Box::new(BinaryDocValuesField::new("b", value())));
                d.add(Box::new(SortedNumericDocValuesField::new("sn", i)));
                d.add(Box::new(SortedSetDocValuesField::new("ss", value())));
                d.add(Box::new(IntPoint::new("p", &[i as i32]).unwrap()));
                d.add(Box::new(
                    KnnFloatVectorField::new("fv", &[i as f32, 1.0], EUCLIDEAN).unwrap(),
                ));
                d.add(Box::new(
                    KnnByteVectorField::new("bv", &[i as u8, 1], EUCLIDEAN).unwrap(),
                ));
                d
            })
            .collect();
        let index = TestIndex::new(docs);
        let fired: Arc<dyn crate::index::QueryTimeout> = Arc::new(FiredTimeout);
        let reader = ExitableDirectoryReaderWrapper::new(Arc::clone(&fired))
            .wrap(Arc::clone(&index.reader))
            .unwrap();
        let leaves = Arc::clone(&reader).leaves().unwrap();
        let leaf = leaves[0].leaf_reader();
        let mut rust: Vec<String> = (0..10).map(|call| exitable_call(&*leaf, call)).collect();
        // The digits of an object without `toString()` are the stand-in's.
        let numeric = "org.apache.lucene.codecs.lucene90.Lucene90DocValuesProducer$4";
        assert!(
            rust[0].ends_with(&format!(
                "DocValues={numeric}@{:x}",
                crate::util::java_collections::java_identity_hash_code_stand_in(numeric) as u32
            )),
            "{}",
            rust[0]
        );
        let raw_leaves = Arc::clone(&index.reader).leaves().unwrap();
        let raw_leaf = raw_leaves[0].leaf_reader();
        // The Thread.interrupted() branch, with a timeout that never fires.
        let calm: Arc<dyn LeafReader> = Arc::new(
            ExitableFilterLeafReader::new(Arc::clone(&raw_leaf), Arc::new(NeverTimeout)).unwrap(),
        );
        for call in 0..10 {
            InterruptHandle::current().interrupt();
            rust.push(exitable_call(&*calm, call));
            InterruptHandle::interrupted();
        }
        // An exitable leaf over an exitable leaf: the messages name the inner
        // wrappers.
        let nested = ExitableFilterLeafReader::new(Arc::clone(&calm), Arc::clone(&fired)).unwrap();
        rust.extend((0..9).map(|call| exitable_call(&nested, call)));
        // A sorted view: the messages name the SortingCodecReader values.
        let sort = Sort::new_fields(vec![SortField::new_reverse(
            Some("n".to_string()),
            SortFieldType::Long,
            true,
        )
        .unwrap()])
        .unwrap();
        let codec_reader = Arc::clone(&raw_leaf).as_codec_reader().unwrap();
        let sorted_view: Arc<dyn LeafReader> =
            SortingCodecReader::wrap_sort(codec_reader, sort).unwrap();
        let sorted = ExitableFilterLeafReader::new(sorted_view, Arc::clone(&fired)).unwrap();
        rust.extend([0, 1, 2, 3, 4, 6, 7, 8].map(|call| exitable_call(&sorted, call)));
        // ExitablePointTree: the timeout names the point values, the
        // interrupt the tree.
        let switchable = Arc::new(SwitchableTimeout::default());
        let switched = ExitableFilterLeafReader::new(
            Arc::clone(&raw_leaf),
            Arc::clone(&switchable) as Arc<dyn crate::index::QueryTimeout>,
        )
        .unwrap();
        let tree = switched
            .get_point_values("p")
            .unwrap()
            .unwrap()
            .point_tree()
            .unwrap();
        switchable
            .0
            .store(true, std::sync::atomic::Ordering::SeqCst);
        rust.push(exiting_outcome(tree.size()));
        switchable
            .0
            .store(false, std::sync::atomic::Ordering::SeqCst);
        InterruptHandle::current().interrupt();
        rust.push(exiting_outcome(tree.size()));
        InterruptHandle::interrupted();
        // ExitableIntersectVisitor: the visitor interrupts its own thread.
        let mut visitor = InterruptingVisitor::default();
        rust.push(exiting_outcome(
            calm.get_point_values("p")
                .unwrap()
                .unwrap()
                .point_tree()
                .unwrap()
                .visit_doc_values(&mut visitor),
        ));
        InterruptHandle::interrupted();
        let java: Vec<String> = run_oracle(&[], &["exitable"])
            .lines()
            .map(without_identity_hashes)
            .collect();
        let rust: Vec<String> = rust.iter().map(|l| without_identity_hashes(l)).collect();
        drop(tree);
        drop(leaves);
        drop(raw_leaves);
        let _ = reader.close();
        assert_eq!(rust, java);
    }

    /// G-032: `SloppyMath` tables are seeded from `StrictMath` (fdlibm), so
    /// `cos`, `sin`, `asin` and `haversinMeters` agree with Lucene bit for bit
    /// (`SloppyMath.java:92-230`).
    #[test]
    fn gap_g032_sloppy_math_bits_match_lucene() {
        use crate::util::SloppyMath;
        let mut bits: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut rust = Vec::new();
        for _ in 0..2000 {
            bits = bits
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let unit = (bits >> 11) as f64 * f64::powi(2.0, -53);
            let x = (unit - 0.5) * 40.0;
            let y = unit * 2.0 - 1.0;
            let lat = y * 90.0;
            let lon = ((bits >> 7) % 360_000) as f64 / 1000.0 - 180.0;
            rust.push(format!(
                "{:x} {:x} {:x} {:x}",
                SloppyMath::cos(x).to_bits(),
                SloppyMath::sin(x).to_bits(),
                SloppyMath::asin(y).to_bits(),
                SloppyMath::haversin_meters(lat, lon, -lat / 2.0, lon / 3.0).to_bits()
            ));
        }
        assert_eq!(rust, java_lines(&run_oracle(&[], &["sloppy"])));
    }

    /// G-429 and G-430: `MemorySegmentIndexInput` lets `MemorySegment.copy`
    /// raise `IndexOutOfBoundsException` for a destination out of range
    /// (sequential `readBytes`, `MemorySegmentIndexInput.java:195-228`) or
    /// reports it as `EOFException` through `handlePositionalIOOBE`
    /// (positional), and `getFilePointer()` on a closed input raises
    /// `AlreadyClosedException` (`MemorySegmentIndexInput.java:123-127, 330-333`).
    #[test]
    fn gap_g429_g430_mmap_input_failures_match_lucene() {
        use crate::store::mmap::MMapDirectory;
        use crate::store::Directory as _;
        let base = std::env::var_os("TMPDIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let path = base.join(format!("census_mmap_{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("f.bin"), [0u8; 100]).unwrap();
        let path = path.canonicalize().unwrap();
        let dir = MMapDirectory::open(path.clone()).unwrap();
        let mut input = dir
            .open_input("f.bin", &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        let mut rust = Vec::new();
        let mut small = [0u8; 8];
        rust.push(format!(
            "readBytes={}",
            outcome(input.read_bytes(Some(&mut small), 5, 10))
        ));
        rust.push(format!(
            "readBytesNegative={}",
            outcome(crate::store::read_bytes_java(
                &mut *input,
                &mut small,
                -1,
                2
            ))
        ));
        let mut slice = input.random_access_slice(0, input.length()).unwrap();
        rust.push(format!(
            "readBytesPositional={}",
            outcome(slice.read_bytes_at(3, &mut small, 5, 10))
        ));
        rust.push(format!("pointer={}", input.file_pointer().unwrap()));
        input.close().unwrap();
        rust.push(format!(
            "closedPointer={}",
            outcome(input.file_pointer().map(|_| ()))
        ));
        let java = java_lines(&run_oracle(&[], &["mmap", path.to_str().unwrap()]));
        drop(slice);
        let _ = std::fs::remove_dir_all(&path);
        assert_eq!(rust, java);
    }

    /// G-413, G-414 and G-415: `FST.ramBytesUsed()` with a 24-byte shallow
    /// size, `FST.toString()` and `Arc.toString()` with Java's renderings,
    /// and `readMetadata` failing where Java's does (`FST.java:237-273,
    /// 455-503`).
    #[test]
    fn gap_g413_g414_g415_fst_matches_lucene() {
        use crate::store::ByteBuffersDataOutput;
        use crate::store::DataOutput;
        use crate::util::fst::{Arc as FstArc, Builder, InputType, PositiveIntOutputs, Util, FST};
        use crate::util::{Accountable, BytesRef, IntsRefBuilder};
        let mut compiler = Builder::new(InputType::Byte1, PositiveIntOutputs::get_singleton())
            .build()
            .unwrap();
        let mut scratch = IntsRefBuilder::new();
        for (i, term) in ["ab", "ac", "b"].iter().enumerate() {
            let input = Util::to_ints_ref(&BytesRef::new(term.as_bytes().to_vec()), &mut scratch);
            compiler.add(&input, i as i64 + 5).unwrap();
        }
        let metadata = compiler.compile().unwrap();
        let fst = FST::from_fst_reader(metadata, compiler.into_fst_reader().unwrap()).unwrap();
        let mut rust = vec![
            format!("ram={}", fst.ram_bytes_used()),
            format!("toString={fst}"),
        ];
        let mut first = FstArc::default();
        fst.get_first_arc(&mut first);
        rust.push(format!("first={first}"));
        let mut target = FstArc::default();
        fst.read_first_target_arc(&first, &mut target, &mut *fst.get_bytes_reader().unwrap())
            .unwrap();
        rust.push(format!("target={target}"));

        let mut negative = ByteBuffersDataOutput::new();
        crate::codecs::codec_util::write_header(
            &mut negative,
            "FST",
            crate::util::fst::VERSION_CURRENT,
        )
        .unwrap();
        negative.write_byte(1).unwrap();
        negative.write_v_int(-1).unwrap();
        negative.write_byte(0).unwrap();
        negative.write_v_long(0).unwrap();
        negative.write_v_long(0).unwrap();
        let bytes = negative.to_array_copy().unwrap();
        let result = FST::read_metadata(
            &mut crate::store::ByteArrayDataInput::new(&bytes),
            PositiveIntOutputs::get_singleton(),
        )
        .map(|_| ());
        rust.push(format!("negativeEmpty={}", outcome(result)));

        let mut bad = ByteBuffersDataOutput::new();
        crate::codecs::codec_util::write_header(&mut bad, "FST", crate::util::fst::VERSION_CURRENT)
            .unwrap();
        bad.write_byte(0).unwrap();
        bad.write_byte(0xFF).unwrap();
        let bytes = bad.to_array_copy().unwrap();
        let error = FST::read_metadata(
            &mut crate::store::ByteArrayDataInput::new(&bytes),
            PositiveIntOutputs::get_singleton(),
        )
        .expect_err("an unknown input type is corrupt");
        rust.push(format!("badType={}", error.original_message().unwrap()));
        assert_eq!(rust, java_lines(&run_oracle(&[], &["fst"])));
    }
}

/// The message of an `IllegalArgumentException`.
fn illegal_argument_message(error: &LuceneError) -> &str {
    match &error.kind {
        LuceneErrorKind::IllegalArgument { message, .. } => message,
        other => panic!("expected IllegalArgumentException, got {other:?}"),
    }
}

/// The text of a panic raised for a Java runtime exception.
fn panic_text(result: std::thread::Result<()>) -> String {
    let payload = result.expect_err("a Java runtime exception");
    if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else {
        panic!("unexpected panic payload")
    }
}

/// G-109: exception texts at existing checks, as Lucene words them.
mod messages {
    use super::*;
    use crate::util::BytesRef;

    /// `NumericUtils.bigIntToSortableBytes` (`NumericUtils.java:239-243`):
    /// `"BigInteger: " + bigInt + " requires more than " + size + " bytes storage"`.
    #[test]
    fn gap_g109_numeric_utils_big_integer_message() {
        let text = panic_text(std::panic::catch_unwind(|| {
            let mut result = [0u8; 2];
            crate::util::NumericUtils::big_int_to_sortable_bytes(&[1, 0, 0], 2, &mut result, 0);
        }));
        assert_eq!(
            text,
            "java.lang.IllegalArgumentException: BigInteger: 65536 requires more than 2 bytes storage"
        );
    }

    /// `ClasspathResourceLoader.openResource` (`ClasspathResourceLoader.java:64-68`).
    #[test]
    fn gap_g109_classpath_resource_loader_message() {
        use crate::util::resource_loader::{ClasspathResourceLoader, ResourceLoader};
        let loader = ClasspathResourceLoader::new(std::collections::HashMap::new());
        let error = match loader.open_resource("stop.txt") {
            Ok(_) => panic!("an absent resource"),
            Err(e) => e,
        };
        assert_eq!(
            java_to_string(&error),
            "java.io.IOException: Resource not found (if you use Java Module System, make sure \
             to open module and package containing resources to 'org.apache.lucene.core' \
             module): stop.txt"
        );
    }

    /// `PackedInts.checkVersion` (`PackedInts.java:57-63`) and
    /// `PackedInts.numBlocks` (`PackedInts.java:903-909`).
    #[test]
    fn gap_g109_packed_ints_messages() {
        use crate::util::packed::PackedInts;
        let error = PackedInts::check_version(-1).unwrap_err();
        assert_eq!(
            illegal_argument_message(&error),
            format!(
                "Version is too old, should be at least {} (got -1)",
                PackedInts::VERSION_START
            )
        );
        let error = PackedInts::num_blocks(i64::MAX, 2).unwrap_err();
        assert_eq!(
            illegal_argument_message(&error),
            "size is too large for this block size"
        );
    }

    /// `DirectMonotonicWriter.add` (`DirectMonotonicWriter.java:124-128`).
    #[test]
    fn gap_g109_direct_monotonic_writer_order_message() {
        use crate::store::MockIndexOutput;
        let mut meta = MockIndexOutput::new("meta", "meta");
        let mut data = MockIndexOutput::new("data", "data");
        let mut writer =
            crate::util::packed::DirectMonotonicWriter::new(&mut meta, &mut data, 4, 2).unwrap();
        writer.add(5).unwrap();
        let error = writer.add(3).unwrap_err();
        assert_eq!(
            illegal_argument_message(&error),
            "Values do not come in order: 5, 3"
        );
    }

    /// `DefaultIOContext(Set)` (`DefaultIOContext.java:27-38`) prints the
    /// offending `Map.Entry<Class, List<FileOpenHint>>`; the list order comes
    /// from `Set.of`, so only its members are fixed.
    #[test]
    fn gap_g109_default_io_context_multiple_hints_message() {
        use crate::store::{DataAccessHint, DefaultIOContext, FileOpenHint};
        let hints: Vec<Arc<dyn FileOpenHint>> = vec![
            Arc::new(DataAccessHint::Random),
            Arc::new(DataAccessHint::Sequential),
        ];
        let error = DefaultIOContext::new(hints).unwrap_err();
        let message = illegal_argument_message(&error);
        assert!(
            message == "Multiple hints of type class org.apache.lucene.store.DataAccessHint=[RANDOM, SEQUENTIAL] specified"
                || message == "Multiple hints of type class org.apache.lucene.store.DataAccessHint=[SEQUENTIAL, RANDOM] specified",
            "{message}"
        );
    }

    /// `FieldInfos.fieldInfo(int)` (`FieldInfos.java:341-345`) and
    /// `FieldInfo.setOmitsNorms` (`FieldInfo.java:629-634`).
    #[test]
    fn gap_g109_field_infos_messages() {
        use crate::index::{FieldInfo, FieldInfos};
        let infos = FieldInfos::new(vec![FieldInfo::new("f", 0)]).unwrap();
        let error = infos.field_info_checked(-1).unwrap_err();
        assert_eq!(illegal_argument_message(&error), "Illegal field number: -1");
        let mut info = FieldInfo::new("g", 1);
        let error = info.set_omits_norms().unwrap_err();
        assert_eq!(
            illegal_state_message(&error),
            "cannot omit norms: this field is not indexed"
        );
    }

    /// `Polygon`, `XYPolygon` and `XYLine` constructors (`Polygon.java:73-76`,
    /// `XYPolygon.java:57-60`, `XYLine.java:53-56`).
    #[test]
    fn gap_g109_geometry_equal_length_messages() {
        use crate::geo::geometry::{Polygon, XYLine, XYPolygon};
        let error = Polygon::new(vec![0.0, 1.0, 0.0], vec![0.0, 1.0], vec![]).unwrap_err();
        assert_eq!(
            illegal_argument_message(&error),
            "polyLats and polyLons must be equal length"
        );
        let error = XYPolygon::new(vec![0.0f32, 1.0, 0.0], vec![0.0f32, 1.0], vec![]).unwrap_err();
        assert_eq!(
            illegal_argument_message(&error),
            "x and y must be equal length"
        );
        let error = XYLine::new(vec![0.0f32, 1.0, 2.0], vec![0.0f32, 1.0]).unwrap_err();
        assert_eq!(
            illegal_argument_message(&error),
            "x and y must be equal length"
        );
    }

    /// `BinaryPoint(String, byte[], IndexableFieldType)` (`BinaryPoint.java:136-144`).
    #[test]
    fn gap_g109_binary_point_packed_length_message() {
        use crate::document::{BinaryPoint, FieldType};
        let field_type = FieldType::new();
        field_type.set_dimensions(2, 4).unwrap();
        let error = BinaryPoint::new_with_packed("p", BytesRef::new(vec![0; 5]), field_type)
            .expect_err("a packed value of the wrong length");
        assert_eq!(
            illegal_argument_message(&error),
            "packedPoint is length=5 but type.pointDimensionCount()=2 and type.pointNumBytes()=4"
        );
    }

    /// `Lucene90PointsFormat(int)` (`Lucene90PointsFormat.java:80-84`).
    #[test]
    fn gap_g109_points_format_version_message() {
        let error = crate::codecs::lucene90::points::Lucene90PointsFormat::with_version(99)
            .expect_err("an unknown version");
        assert_eq!(illegal_argument_message(&error), "Invalid version: 99");
    }

    /// `IndexedDISI.writeBitSet` and the `IndexedDISI` constructor
    /// (`IndexedDISI.java:195-203, 332-340`).
    #[test]
    fn gap_g109_indexed_disi_dense_rank_power_messages() {
        use crate::codecs::lucene90::indexed_disi::{write_bit_set, IndexedDISI};
        use crate::store::{MockIndexInput, MockIndexOutput};
        let mut out = MockIndexOutput::new("disi", "disi");
        let mut it = crate::search::doc_id_set_iterator::all(4).unwrap();
        let error = write_bit_set(&mut it, &mut out, 5).unwrap_err();
        assert_eq!(
            illegal_argument_message(&error),
            "Acceptable values for denseRankPower are 7-15 (every 128-32768 docIDs). The \
             provided power was 5 (every 32 docIDs)"
        );
        let error = IndexedDISI::with_slices(
            Box::new(MockIndexInput::new(Vec::new(), "disi")),
            None,
            0,
            16,
            0,
        )
        .err()
        .expect("an invalid power");
        assert_eq!(
            illegal_argument_message(&error),
            "Acceptable values for denseRankPower are 7-15 (every 128-32768 docIDs). The \
             provided power was 16 (every 65536 docIDs). "
        );
    }

    /// `CompressionAlgorithm.byCode` (`CompressionAlgorithm.java:56-62`).
    #[test]
    fn gap_g109_blocktree_compression_code_message() {
        let error = crate::codecs::lucene103::blocktree::CompressionAlgorithm::by_code(3)
            .expect_err("an unknown code");
        assert_eq!(
            illegal_argument_message(&error),
            "Illegal code for a compression algorithm: 3"
        );
    }

    /// `PackedTokenAttributeImpl.setPositionIncrement`
    /// (`PackedTokenAttributeImpl.java:57-61`).
    #[test]
    fn gap_g109_packed_token_position_increment_message() {
        use crate::analysis::tokenattributes::{
            PackedTokenAttributeImpl, PositionIncrementAttribute,
        };
        let text = panic_text(std::panic::catch_unwind(|| {
            let mut attribute = PackedTokenAttributeImpl::default();
            attribute.set_position_increment(-1);
        }));
        assert_eq!(text, "Increment must be zero or greater: -1");
    }
}

/// G-111: reachable Lucene throws the port lacked.
mod absent_throws {
    use super::*;
    use crate::util::BytesRef;

    /// `GroupVIntUtil.readGroupVInt(DataInput, long, IntReader, long, int[],
    /// int)` is deprecated and throws `UnsupportedOperationException("No
    /// longer implemented.")` (`GroupVIntUtil.java:160-167`).
    #[test]
    #[allow(deprecated)]
    fn gap_g111_group_vint_deprecated_method_throws() {
        let mut input = crate::store::MockIndexInput::new(vec![0; 16], "in");
        struct Zero;
        impl crate::util::IntReader for Zero {
            fn read(&self, _v: i64) -> i32 {
                0
            }
        }
        let reader: &dyn crate::util::IntReader = &Zero;
        let mut dst = [0i32; 4];
        let error = crate::util::GroupVIntUtil::read_group_vint_reader(
            &mut input, 16, reader, 0, &mut dst, 0,
        )
        .unwrap_err();
        assert!(
            matches!(
                &error.kind,
                LuceneErrorKind::UnsupportedOperation { message, .. } if message == "No longer implemented."
            ),
            "{error:?}"
        );
    }

    /// `ConcurrentApproximatePriorityQueue(int)` checks the concurrency
    /// range (`ConcurrentApproximatePriorityQueue.java:50-57`).
    #[test]
    fn gap_g111_concurrent_approximate_priority_queue_concurrency_range() {
        let error = crate::index::index_utilities::ConcurrentApproximatePriorityQueue::<i32>::with_concurrency(0)
            .err()
            .expect("concurrency 0");
        assert_eq!(
            illegal_argument_message(&error),
            "concurrency must be in [1, 256], got 0"
        );
    }

    /// `BinaryPoint` refuses 0-length dimensions in the field constructor
    /// (`BinaryPoint.java:63-65`) and in `pack` (`BinaryPoint.java:101-104`).
    #[test]
    fn gap_g111_binary_point_zero_length_values() {
        let error = crate::document::BinaryPoint::new("p", &[BytesRef::new(vec![])])
            .expect_err("a 0-length value");
        assert_eq!(
            illegal_argument_message(&error),
            "point must not have 0-length values"
        );
        let error =
            crate::document::pack_binary_point(&[BytesRef::new(vec![]), BytesRef::new(vec![])])
                .unwrap_err();
        assert_eq!(
            illegal_argument_message(&error),
            "point must not have 0-length values"
        );
    }
}

/// Codec files patched by hand, checksum kept valid, so that reading them
/// reaches the check under test.
mod corrupt_files {
    use super::*;
    use crate::document::{IntPoint, Store, StringField};

    /// `CodecUtil.indexHeaderLength(codec, suffix)`.
    fn index_header_length(codec: &str, suffix: &str) -> usize {
        4 + 1 + codec.len() + 4 + 16 + 1 + suffix.len()
    }

    /// Commits one document (a string field and an int point) without
    /// compound files, patches the file with extension `ext` through
    /// `patch(name, bytes)`, rewrites its checksum and opens a reader.
    fn open_patched(ext: &str, patch: &dyn Fn(&str, &mut Vec<u8>)) -> LuceneError {
        let _ = register_codec("Lucene104", Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let config = IndexWriterConfig::new();
        config.set_use_compound_file(false).unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        let mut doc = Document::new();
        doc.add(Box::new(
            StringField::new("id", "a".to_string(), Store::NO).unwrap(),
        ));
        doc.add(Box::new(IntPoint::new("p", &[7]).unwrap()));
        writer.add_document(&doc).unwrap();
        writer.commit().unwrap();
        writer.close().unwrap();
        let name = dir
            .list_all()
            .unwrap()
            .into_iter()
            .find(|n| n.ends_with(&format!(".{ext}")))
            .unwrap_or_else(|| panic!("no .{ext} file"));
        let mut bytes = {
            let mut input = dir
                .open_input(&name, &*crate::store::DEFAULT_IO_CONTEXT)
                .unwrap();
            let len = input.length() as usize;
            let mut buf = vec![0u8; len];
            input.read_bytes(Some(&mut buf), 0, len).unwrap();
            buf
        };
        patch(&name, &mut bytes);
        let body = bytes.len() - 8;
        let crc = i64::from(crc32fast::hash(&bytes[..body]));
        bytes[body..].copy_from_slice(&crc.to_be_bytes());
        dir.delete_file(&name).unwrap();
        let mut out = dir
            .create_output(&name, &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        out.write_bytes(&bytes, 0, bytes.len()).unwrap();
        out.close().unwrap();
        match crate::index::directory_reader::open(dir) {
            Ok(_) => panic!("the patched .{ext} must be refused"),
            Err(e) => e,
        }
    }

    /// The `CorruptIndexException` original message of `error` or of the
    /// cause it wraps (`SegmentCoreReaders` wraps codec failures only for
    /// missing files, so this is the error itself).
    fn corrupt_message(error: &LuceneError) -> String {
        error
            .original_message()
            .unwrap_or_else(|| panic!("expected CorruptIndexException, got {error:?}"))
            .to_string()
    }

    /// G-109: `Lucene99SegmentInfoFormat.read` (`Lucene99SegmentInfoFormat.java:116-125`):
    /// `"Illegal boolean value " + hasMinVersion`.
    #[test]
    fn gap_g109_segment_info_illegal_boolean_message() {
        let error = open_patched("si", &|_, bytes| {
            // header, then major, minor and bugfix ints, then the boolean.
            let at = index_header_length("Lucene90SegmentInfo", "") + 12;
            bytes[at] = 2;
        });
        assert_eq!(corrupt_message(&error), "Illegal boolean value 2");
    }

    /// G-109: `Lucene90PointsReader` (`Lucene90PointsReader.java:100-107`):
    /// `"Illegal field number: " + fieldNumber`.
    #[test]
    fn gap_g109_points_reader_illegal_field_number_message() {
        let error = open_patched("kdm", &|_, bytes| {
            let at = index_header_length("Lucene90PointsFormatMeta", "");
            bytes[at..at + 4].copy_from_slice(&(-2i32).to_le_bytes());
        });
        assert_eq!(corrupt_message(&error), "Illegal field number: -2");
    }

    /// G-109: `Lucene103BlockTreeTermsReader` (`Lucene103BlockTreeTermsReader.java:160-166`):
    /// `"Illegal numTerms for field number: " + field`.
    #[test]
    fn gap_g109_blocktree_illegal_num_terms_message() {
        let error = open_patched("tmd", &|name, bytes| {
            // `_0_<suffix>.tmd`: the terms-meta header, the postings header
            // (both with the segment suffix), the block size vInt 128 (two
            // bytes), numFields 1, field number 0, then numTerms.
            let suffix = name
                .trim_end_matches(".tmd")
                .split_once('_')
                .and_then(|(_, rest)| rest.split_once('_'))
                .map(|(_, s)| s)
                .unwrap_or("");
            let postings_codec = "Lucene104PostingsWriterTerms";
            let at = index_header_length("BlockTreeTermsMeta", suffix)
                + index_header_length(postings_codec, suffix)
                + 2
                + 1
                + 1;
            assert_eq!(bytes[at], 1, "numTerms of the single term");
            bytes[at] = 0;
        });
        assert_eq!(
            corrupt_message(&error),
            "Illegal numTerms for field number: 0"
        );
    }
}

/// G-111 throws on the index read path.
mod index_read_path {
    use super::*;
    use crate::document::{NumericDocValuesField, SortedDocValuesField, Store, StringField};
    use crate::index::doc_values::DocValues;
    use crate::util::BytesRef;

    /// `SingletonSortedSetDocValues` and `SingletonSortedNumericDocValues`
    /// refuse an already advanced iterator, in the constructor and in the
    /// unwrapping accessor (`SingletonSortedSetDocValues.java:34-47`,
    /// `SingletonSortedNumericDocValues.java:31-44`).
    #[test]
    fn gap_g111_singleton_doc_values_refuse_a_used_iterator() {
        let docs = (0..2)
            .map(|i| {
                let mut doc = Document::new();
                doc.add(Box::new(SortedDocValuesField::new(
                    "s",
                    BytesRef::from_char_sequence(&i.to_string()),
                )));
                doc.add(Box::new(NumericDocValuesField::new("n", i)));
                doc
            })
            .collect();
        let index = TestIndex::new(docs);
        let leaves = Arc::clone(&index.reader).leaves().unwrap();
        let leaf = leaves[0].leaf_reader();

        let mut sorted = leaf.get_sorted_doc_values("s").unwrap().unwrap();
        sorted.next_doc().unwrap();
        let error = DocValues::singleton_sorted(sorted)
            .err()
            .expect("used iterator");
        assert_eq!(
            illegal_state_message(&error),
            "iterator has already been used: docID=0"
        );
        let mut set =
            DocValues::singleton_sorted(leaf.get_sorted_doc_values("s").unwrap().unwrap()).unwrap();
        assert!(set.get_sorted_doc_values().is_ok());
        crate::search::DocIdSetIterator::next_doc(&mut set).unwrap();
        let error = set.get_sorted_doc_values().err().expect("used iterator");
        assert_eq!(
            illegal_state_message(&error),
            "iterator has already been used: docID=0"
        );

        let mut numeric = leaf.get_numeric_doc_values("n").unwrap().unwrap();
        numeric.next_doc().unwrap();
        let error = DocValues::singleton_numeric(numeric)
            .err()
            .expect("used iterator");
        assert_eq!(
            illegal_state_message(&error),
            "iterator has already been used: docID=0"
        );
        let mut sorted_numeric =
            DocValues::singleton_numeric(leaf.get_numeric_doc_values("n").unwrap().unwrap())
                .unwrap();
        crate::search::DocIdSetIterator::next_doc(&mut sorted_numeric).unwrap();
        let error = sorted_numeric
            .get_numeric_doc_values()
            .err()
            .expect("used iterator");
        assert_eq!(
            illegal_state_message(&error),
            "iterator has already been used: docID=0"
        );
    }

    /// Commits one document without compound files and returns the
    /// directory.
    fn committed_index() -> Arc<dyn Directory> {
        let _ = register_codec("Lucene104", Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let config = IndexWriterConfig::new();
        config.set_use_compound_file(false).unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        let mut doc = Document::new();
        doc.add(Box::new(
            StringField::new("id", "a".to_string(), Store::NO).unwrap(),
        ));
        writer.add_document(&doc).unwrap();
        writer.commit().unwrap();
        writer.close().unwrap();
        dir
    }

    fn truncate(dir: &Arc<dyn Directory>, name: &str, len: usize) {
        let mut bytes = vec![0u8; len];
        let mut input = dir
            .open_input(name, &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        input.read_bytes(Some(&mut bytes), 0, len).unwrap();
        drop(input);
        dir.delete_file(name).unwrap();
        let mut out = dir
            .create_output(name, &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        out.write_bytes(&bytes, 0, len).unwrap();
        out.close().unwrap();
    }

    fn file_with(dir: &Arc<dyn Directory>, suffix: &str) -> String {
        dir.list_all()
            .unwrap()
            .into_iter()
            .find(|n| n.ends_with(suffix))
            .unwrap_or_else(|| panic!("no file ending in {suffix}"))
    }

    /// `SegmentInfos.readCommit` turns an `EOFException` into
    /// `CorruptIndexException("Unexpected file read error while reading
    /// index.", input, e)` (`SegmentInfos.java:300-306`).
    #[test]
    fn gap_g111_read_commit_wraps_eof() {
        let dir = committed_index();
        let segments = file_with(&dir, "segments_1");
        truncate(&dir, &segments, 10);
        let error = crate::index::SegmentInfos::read_latest_commit(Arc::clone(&dir))
            .expect_err("a truncated commit");
        assert_eq!(
            error.original_message(),
            Some("Unexpected file read error while reading index.")
        );
        match &error.kind {
            LuceneErrorKind::CorruptIndex {
                cause: Some(cause), ..
            } => {
                assert!(cause.is_eof_or_missing_file(), "{cause:?}")
            }
            other => panic!("expected a cause, got {other:?}"),
        }
    }

    /// `SegmentCoreReaders` turns a `NoSuchFileException` into
    /// `CorruptIndexException("Problem reading index.", e.getFile(), e)` and
    /// an `EOFException` into `CorruptIndexException("Problem reading index
    /// from " + dir, dir.toString(), e)` (`SegmentCoreReaders.java:140-147`).
    #[test]
    fn gap_g111_segment_core_readers_wrap_missing_and_short_files() {
        let dir = committed_index();
        let tip = file_with(&dir, ".tip");
        dir.delete_file(&tip).unwrap();
        let error = crate::index::directory_reader::open(Arc::clone(&dir))
            .expect_err("a missing terms index");
        assert_eq!(error.original_message(), Some("Problem reading index."));
        assert_eq!(error.resource_description(), Some(tip.as_str()));

        let dir = committed_index();
        let tip = file_with(&dir, ".tip");
        truncate(&dir, &tip, 3);
        let error = crate::index::directory_reader::open(Arc::clone(&dir))
            .expect_err("a truncated terms index");
        let message = error.original_message().expect("CorruptIndexException");
        assert!(
            message.starts_with("Problem reading index from "),
            "{message}"
        );
        match &error.kind {
            LuceneErrorKind::CorruptIndex {
                cause: Some(cause), ..
            } => {
                assert!(cause.is_eof_or_missing_file(), "{cause:?}")
            }
            other => panic!("expected a cause, got {other:?}"),
        }
    }
}

/// G-111: the term-vectors positions enum checks it was started.
mod term_vectors {
    use super::*;
    use crate::document::{Field, FieldType};
    use crate::index::postings_enum::POSTINGS_ENUM_ALL;
    use crate::index::IndexOptions;

    /// `TVPostingsEnum.checkPosition()` throws `IllegalStateException("Position
    /// enum not started")` when an offset is read before `nextPosition()`
    /// (`Lucene90CompressingTermVectorsReader.java:1253-1262`).
    #[test]
    fn gap_g111_term_vectors_position_enum_not_started() {
        let field_type = FieldType::new();
        field_type
            .set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
            .unwrap();
        field_type.set_tokenized(true).unwrap();
        field_type.set_store_term_vectors(true).unwrap();
        field_type.set_store_term_vector_positions(true).unwrap();
        field_type.set_store_term_vector_offsets(true).unwrap();
        let mut doc = Document::new();
        doc.add(Box::new(
            Field::new("body", "alpha beta".to_string(), field_type).unwrap(),
        ));
        let index = TestIndex::new(vec![doc]);
        let leaves = Arc::clone(&index.reader).leaves().unwrap();
        let vectors = leaves[0].leaf_reader().term_vectors().unwrap();
        let fields = vectors.get(0).unwrap().expect("the document's vectors");
        let terms = fields.terms("body").unwrap().expect("the field's vectors");
        let mut terms_enum = terms.iterator().unwrap();
        terms_enum.next().unwrap().expect("alpha");
        let mut postings = terms_enum
            .postings(None, POSTINGS_ENUM_ALL)
            .unwrap()
            .unwrap();
        postings.next_doc().unwrap();
        let text = panic_text(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || {
                let _ = postings.start_offset();
            },
        )));
        assert!(text.contains("Position enum not started"), "{text}");
    }
}

/// B17: `CheckIndex` output and status.
mod check_index {
    use super::*;
    use crate::document::{Store, StringField};
    use crate::index::check_index::CheckIndex;
    use crate::util::print_stream::ByteArrayPrintStream;

    fn checked_index() -> (
        (),
        Arc<ByteArrayPrintStream>,
        crate::index::check_index::Status,
    ) {
        let docs = ["a", "b", "c"]
            .iter()
            .map(|v| {
                let mut doc = Document::new();
                doc.add(Box::new(
                    StringField::new("id", v.to_string(), Store::YES).unwrap(),
                ));
                doc
            })
            .collect();
        let index = TestIndex::new(docs);
        index.writer.commit().unwrap();
        let dir = Arc::clone(index.writer.get_directory());
        drop(index);
        let out = Arc::new(ByteArrayPrintStream::new());
        let mut checker = CheckIndex::new(dir).unwrap();
        checker.set_level(3).unwrap();
        checker
            .set_info_stream(Arc::clone(&out) as Arc<dyn crate::util::print_stream::PrintStream>);
        let status = checker.check_index().unwrap();
        ((), out, status)
    }

    /// G-206: `CheckIndex` writes raw lines to its `PrintStream`
    /// (`CheckIndex.java` `msg`/`infoStream.print`), every per-area header
    /// indented by four spaces (`"    test: field infos........."`), with no
    /// `InfoStream` prefix.
    #[test]
    fn gap_g206_check_index_output_lines_as_java() {
        let (_index, out, status) = checked_index();
        assert!(status.clean);
        let text = out.to_string_utf8();
        for header in [
            "    test: open reader.........",
            "    test: check integrity.....",
            "    test: check live docs.....",
            "    test: field infos.........",
            "    test: field norms.........",
            "    test: terms, freq, prox...",
            "    test: stored fields.......",
            "    test: docvalues...........",
        ] {
            assert!(
                text.lines().any(|l| l.starts_with(header)),
                "missing {header:?} in:\n{text}"
            );
        }
        assert!(
            !text.lines().any(|l| l.starts_with("CheckIndex ")),
            "{text}"
        );
        assert!(text.contains("Segments file=segments_"), "{text}");
    }

    /// G-207: `TermIndexStatus.blockTreeStats` holds each field's
    /// `Terms.getStats()` object (`CheckIndex.java:317`, `testPostings`), the
    /// blocktree `Stats` for the default codec.
    #[test]
    fn gap_g207_block_tree_stats_are_stats_objects() {
        let (_index, _out, status) = checked_index();
        let term_status = status.segment_infos[0]
            .term_index_status
            .as_ref()
            .expect("the postings were checked");
        let stats = term_status
            .block_tree_stats
            .as_ref()
            .expect("the default codec publishes its stats");
        let id = stats.get("id").expect("the id field's stats");
        let blocktree = id
            .as_any()
            .downcast_ref::<crate::codecs::lucene103::blocktree::Stats>()
            .expect("a blocktree Stats object");
        assert_eq!(blocktree.total_term_count, 3);
    }
}

/// B18-B22: `IndexWriter` behaviour closed by the census.
mod index_writer {
    use super::*;
    use crate::document::NumericValue;
    use crate::document::{Field, FieldType, NumericDocValuesField, Store, StringField};
    use crate::index::{DocValuesType, Term};

    fn id_doc(id: &str) -> Document {
        let mut doc = Document::new();
        doc.add(Box::new(
            StringField::new("id", id.to_string(), Store::NO).unwrap(),
        ));
        doc
    }

    fn new_writer(config: IndexWriterConfig) -> (Arc<dyn Directory>, Arc<IndexWriter>) {
        let _ = register_codec("Lucene104", Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        (dir, writer)
    }

    /// G-301: `advanceSegmentInfosVersion` and `advanceSegmentInfosCounter`
    /// call `changed()`, which bumps `changeCount`
    /// (`IndexWriter.java:1424-1444`): the writer then has uncommitted
    /// changes, and the next commit writes a new generation.
    #[test]
    fn gap_g301_advancing_segment_infos_is_a_change() {
        let (_dir, writer) = new_writer(IndexWriterConfig::new());
        writer.add_document(&id_doc("a")).unwrap();
        writer.commit().unwrap();
        assert!(!writer.has_uncommitted_changes().unwrap());
        let before = writer.clone_segment_infos().unwrap().generation();
        writer.advance_segment_infos_version(1_000_000).unwrap();
        assert!(writer.has_uncommitted_changes().unwrap());
        writer.commit().unwrap();
        let infos = writer.clone_segment_infos().unwrap();
        assert_eq!(infos.generation(), before + 1);
        assert!(infos.version() > 1_000_000);
        assert!(!writer.has_uncommitted_changes().unwrap());
        writer.advance_segment_infos_counter(1_000).unwrap();
        assert!(writer.has_uncommitted_changes().unwrap());
        writer.close().unwrap();
    }

    /// G-302: `IndexWriter(Directory, IndexWriterConfig)` attaches the config
    /// first (`conf.setIndexWriter(this)`, `IndexWriter.java:951`), so a
    /// constructor that then fails to obtain the lock leaves it attached and
    /// the config cannot be reused (`IndexWriterConfig.setIndexWriter`).
    #[test]
    fn gap_g302_a_failed_constructor_leaves_the_config_attached() {
        let (dir, writer) = new_writer(IndexWriterConfig::new());
        let config = Arc::new(IndexWriterConfig::new());
        let locked = IndexWriter::new(Arc::clone(&dir), Arc::clone(&config));
        assert!(
            matches!(&locked, Err(e) if matches!(e.kind, LuceneErrorKind::LockObtainFailed { .. })),
            "the second writer on a locked directory fails"
        );
        let other: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let error = match IndexWriter::new(other, config) {
            Ok(_) => panic!("the config is attached to the failed writer"),
            Err(e) => e,
        };
        assert_eq!(
            illegal_state_message(&error),
            "do not share IndexWriterConfig instances across IndexWriters"
        );
        writer.close().unwrap();
    }

    /// G-304: `buildDocValuesUpdate` casts a NUMERIC field's value with
    /// `(Long) f.numericValue()` (`IndexWriter.java:2071`): an `Integer`
    /// value is a `ClassCastException`.
    #[test]
    fn gap_g304_numeric_update_with_an_int_value_is_a_class_cast() {
        let (_dir, writer) = new_writer(IndexWriterConfig::new());
        let mut doc = id_doc("a");
        doc.add(Box::new(NumericDocValuesField::new("n", 1)));
        writer.add_document(&doc).unwrap();
        let field_type = FieldType::new();
        field_type
            .set_doc_values_type(DocValuesType::NUMERIC)
            .unwrap();
        let int_valued = Field::new_with_number("n", NumericValue::Int(5), field_type).unwrap();
        let error = writer
            .update_doc_values(Term::from_text("id", "a"), &[int_valued])
            .unwrap_err();
        assert!(
            matches!(
                &error.kind,
                LuceneErrorKind::Unchecked {
                    class: "java.lang.ClassCastException",
                    ..
                }
            ),
            "{error:?}"
        );
        writer.close().unwrap();
    }

    /// G-305: `softUpdateDocument` builds the doc-values updates before any
    /// `ensureOpen` (`IndexWriter.java:1949-1959`): on a closed writer an
    /// invalid soft-delete field is reported, not the closed writer.
    #[test]
    fn gap_g305_soft_update_validates_fields_before_ensure_open() {
        let (_dir, writer) = new_writer(IndexWriterConfig::new());
        writer.close().unwrap();
        let field_type = FieldType::new();
        field_type
            .set_doc_values_type(DocValuesType::SORTED)
            .unwrap();
        let sorted = Field::new_with_bytes(
            "soft",
            crate::util::BytesRef::from_char_sequence("x"),
            field_type,
        )
        .unwrap();
        let error = writer
            .soft_update_document(Term::from_text("id", "a"), &id_doc("a"), &[sorted])
            .unwrap_err();
        assert!(error.is_illegal_argument_exception(), "{error:?}");
    }

    /// G-306 (b): `tryModifyDocument` on a reader without leaves:
    /// `ReaderUtil.subIndex` answers `-1` and `leaves.get(-1)` throws
    /// `IndexOutOfBoundsException` (`IndexWriter.java:1812-1813`).
    #[test]
    fn gap_g306_try_delete_on_an_empty_reader_is_out_of_bounds() {
        let (_dir, writer) = new_writer(IndexWriterConfig::new());
        writer.commit().unwrap();
        let reader = crate::index::directory_reader::open_with_writer(&*writer).unwrap();
        let as_index_reader: Arc<dyn crate::index::IndexReader> = reader.clone();
        let error = writer.try_delete_document(&as_index_reader, 0).unwrap_err();
        assert!(
            matches!(
                &error.kind,
                LuceneErrorKind::Unchecked { class, .. } if class.ends_with("IndexOutOfBoundsException")
            ),
            "{error:?}"
        );
        reader.close().unwrap();
        writer.close().unwrap();
    }

    /// G-312: `IndexWriter.numDeletedDocs` (its `MergeContext`
    /// implementation) runs `ensureOpen(false)` and counts soft deletes
    /// through `ReadersAndUpdates` (`IndexWriter.java:895-908`).
    #[test]
    fn gap_g312_num_deleted_docs_counts_soft_deletes_and_checks_open() {
        let config = IndexWriterConfig::new();
        config.set_soft_deletes_field(Some("soft".to_string()));
        config
            .set_merge_policy(Some(crate::index::merge_policy::NoMergePolicy::instance()))
            .unwrap();
        let (_dir, writer) = new_writer(config);
        writer.add_document(&id_doc("a")).unwrap();
        writer.add_document(&id_doc("b")).unwrap();
        writer.commit().unwrap();
        writer
            .soft_update_document(
                Term::from_text("id", "a"),
                &id_doc("a"),
                &[{
                    let field_type = FieldType::new();
                    field_type
                        .set_doc_values_type(DocValuesType::NUMERIC)
                        .unwrap();
                    Field::new_with_number("soft", NumericValue::Long(1), field_type).unwrap()
                }],
            )
            .unwrap();
        writer.commit().unwrap();
        let infos = writer.clone_segment_infos().unwrap();
        let total: i32 = (0..infos.size())
            .map(|i| writer.num_deleted_docs(infos.info(i)).unwrap())
            .sum();
        assert_eq!(total, 1);
        writer.close().unwrap();
        let error = writer.num_deleted_docs(infos.info(0)).unwrap_err();
        assert!(
            matches!(error.kind, LuceneErrorKind::AlreadyClosed { .. }),
            "{error:?}"
        );
    }

    /// Records every message of the components it is asked about.
    #[derive(Default)]
    struct Recording(std::sync::Mutex<Vec<String>>);

    impl crate::util::InfoStream for Recording {
        fn java_class_name(&self) -> &'static str {
            "org.apache.lucene.util.InfoStream"
        }
        fn message(&self, component: &str, message: &str) {
            self.0
                .lock()
                .unwrap()
                .push(format!("{component}: {message}"));
        }
        fn is_enabled(&self, _component: &str) -> bool {
            true
        }
        fn close(&self) {}
    }

    /// G-209 and G-313: `IndexWriter` infoStream messages carry Lucene's
    /// text: `commit: took %.1f msec`, `incRefDeleter`/`decRefDeleter`
    /// with ` segments=`, `now apply all deletes ... reader pool bytesUsed=`,
    /// `drop 100% deleted segments: `, `  index before flush `,
    /// `addIndexes: process directory `, `merged segment size=`
    /// (`IndexWriter.java` `infoStream.message("IW", ...)` sites).
    #[test]
    fn gap_g209_g313_index_writer_info_stream_messages() {
        let recording = Arc::new(Recording::default());
        let config = IndexWriterConfig::new();
        config
            .set_info_stream(Arc::clone(&recording) as Arc<dyn crate::util::InfoStream>)
            .unwrap();
        let (_dir, writer) = new_writer(config);
        writer.add_document(&id_doc("a")).unwrap();
        writer.add_document(&id_doc("b")).unwrap();
        writer.commit().unwrap();
        writer.add_document(&id_doc("c")).unwrap();
        writer
            .delete_documents_by_terms(vec![Term::from_text("id", "a")])
            .unwrap();
        writer.commit().unwrap();
        let source = {
            let (dir, w) = new_writer(IndexWriterConfig::new());
            w.add_document(&id_doc("x")).unwrap();
            w.commit().unwrap();
            w.close().unwrap();
            dir
        };
        writer.add_indexes_dirs(&[source]).unwrap();
        let nrt = crate::index::directory_reader::open_with_writer(&*writer).unwrap();
        nrt.close().unwrap();
        writer
            .delete_documents_by_terms(vec![Term::from_text("id", "x")])
            .unwrap();
        writer.force_merge(1).unwrap();
        writer.commit().unwrap();
        writer.close().unwrap();
        let lines = recording.0.lock().unwrap().clone();
        let has = |prefix: &str, contains: &str| {
            lines
                .iter()
                .any(|l| l.starts_with(prefix) && l.contains(contains))
        };
        assert!(has("IW: commit: took ", " msec"), "commit: took");
        assert!(
            has("IW: incRefDeleter for NRT reader version=", " segments="),
            "incRefDeleter"
        );
        assert!(
            has("IW: decRefDeleter for NRT reader version=", " segments="),
            "decRefDeleter"
        );
        assert!(
            has(
                "IW: now apply all deletes for all segments",
                " reader pool bytesUsed="
            ),
            "now apply all deletes"
        );
        assert!(has("IW:   index before flush ", ""), "index before flush");
        assert!(
            has("IW: addIndexes: process directory ", ""),
            "process directory"
        );
        assert!(
            has("IW: merged segment size=", " MB vs estimate="),
            "merged segment size"
        );
    }
}

/// B14 and B16: block-tree terms enums, `RegExp` intervals, `BooleanQuery`
/// clause-set order.
mod terms_and_queries {
    use super::*;
    use crate::document::{Store, StringField};
    use crate::index::terms::Terms;
    use crate::index::Term;
    use crate::util::automaton::compiled_automaton::CompiledAutomaton;
    use crate::util::automaton::reg_exp::RegExp;
    use crate::util::BytesRef;

    /// An index of the 3000 terms `t0000` .. `t2999` in field `f`, enough
    /// for the block tree to write floor blocks.
    fn floor_block_index() -> TestIndex {
        let docs = (0..3000)
            .map(|i| {
                let mut doc = Document::new();
                doc.add(Box::new(
                    StringField::new("f", format!("t{i:04}"), Store::NO).unwrap(),
                ));
                doc
            })
            .collect();
        TestIndex::new(docs)
    }

    fn intersect(terms: &dyn Terms, regexp: &str) -> Box<dyn crate::index::terms::TermsEnum> {
        let automaton = RegExp::new(regexp).unwrap().to_automaton().unwrap();
        let compiled = CompiledAutomaton::new(automaton, false, true, false).unwrap();
        terms.intersect(&compiled, None).unwrap()
    }

    fn unsupported(error: LuceneError) {
        assert!(
            matches!(
                &error.kind,
                LuceneErrorKind::UnsupportedOperation { message, .. } if message.is_empty()
            ),
            "{error:?}"
        );
    }

    /// G-112 and G-115: `IntersectTermsEnum` skips the floor blocks whose
    /// labels the automaton cannot reach (`IntersectTermsEnumFrame.load`,
    /// `loadNextFloorBlock`) and reads the floor data from a seeked input
    /// with no size cap, returning exactly the matching terms.
    #[test]
    fn gap_g112_g115_intersect_across_floor_blocks() {
        let index = floor_block_index();
        let leaves = Arc::clone(&index.reader).leaves().unwrap();
        let terms = leaves[0].leaf_reader().terms("f").unwrap().unwrap();
        for (regexp, expected) in [
            ("t1.*", (1000..2000).collect::<Vec<i32>>()),
            (
                "t2[5-9]9[0-3]",
                (25..30)
                    .flat_map(|h| (0..4).map(move |u| h * 100 + 90 + u))
                    .collect(),
            ),
            ("t0007|t2999", vec![7, 2999]),
        ] {
            let mut terms_enum = intersect(terms.as_ref(), regexp);
            let mut found = Vec::new();
            while let Some(term) = terms_enum.next().unwrap() {
                found.push(term.utf8_to_string().unwrap()[1..].parse::<i32>().unwrap());
            }
            assert_eq!(found, expected, "{regexp}");
        }
    }

    /// G-113: `CompressionAlgorithm.byCode`'s `IllegalArgumentException` is
    /// rethrown as `CorruptIndexException(e.getMessage(), in, e)`
    /// (`SegmentTermsEnumFrame.java:197-201`, `IntersectTermsEnumFrame.java:182-187`).
    #[test]
    fn gap_g113_compression_code_failure_is_corrupt_index_with_cause() {
        let by_code = crate::codecs::lucene103::blocktree::CompressionAlgorithm::by_code(3)
            .expect_err("code 3 is unknown");
        let input = crate::store::MockIndexInput::new(Vec::new(), "_0.tim");
        let error =
            crate::codecs::lucene103::segment_terms_enum::corrupt_from_by_code(by_code, &input);
        assert_eq!(
            error.original_message(),
            Some("Illegal code for a compression algorithm: 3")
        );
        assert_eq!(error.resource_description(), Some("_0.tim"));
        match &error.kind {
            LuceneErrorKind::CorruptIndex {
                cause: Some(cause), ..
            } => {
                assert!(cause.is_illegal_argument_exception())
            }
            other => panic!("expected a cause, got {other:?}"),
        }
    }

    /// G-114 and G-202: the block-tree enums throw
    /// `UnsupportedOperationException()` — no message — for the operations
    /// they do not support (`IntersectTermsEnum.java` `seekExact`, `seekCeil`,
    /// `seekExact(long)`, `ord`; `SegmentTermsEnum.java` `seekExact(long)`,
    /// `ord`), and `IntersectTermsEnum.next()` after exhaustion dereferences
    /// the `null` current frame (`NullPointerException`).
    #[test]
    fn gap_g114_g202_block_tree_enum_unsupported_and_exhausted() {
        let index = floor_block_index();
        let leaves = Arc::clone(&index.reader).leaves().unwrap();
        let terms = leaves[0].leaf_reader().terms("f").unwrap().unwrap();

        let mut segment_enum = terms.iterator().unwrap();
        unsupported(segment_enum.seek_ord(0).unwrap_err());
        unsupported(segment_enum.ord().unwrap_err());

        let mut intersect_enum = intersect(terms.as_ref(), "t000[0-2]");
        unsupported(
            intersect_enum
                .seek_exact(&BytesRef::from_char_sequence("t0001"))
                .unwrap_err(),
        );
        unsupported(
            intersect_enum
                .seek_ceil(&BytesRef::from_char_sequence("t0001"))
                .expect_err("unsupported"),
        );
        unsupported(intersect_enum.seek_ord(0).unwrap_err());
        unsupported(intersect_enum.ord().unwrap_err());
        let mut count = 0;
        while intersect_enum.next().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 3);
        let error = intersect_enum.next().unwrap_err();
        assert!(
            matches!(
                &error.kind,
                LuceneErrorKind::Unchecked {
                    class: "java.lang.NullPointerException",
                    ..
                }
            ),
            "{error:?}"
        );
    }

    /// G-204: `RegExp` parses interval bounds with `Integer.parseInt`, which
    /// accepts any Unicode decimal digit, and counts the digits in UTF-16
    /// units (`RegExp.java` `parseSimpleExp`): `<٠١-٩٩>` is `<01-99>`. An
    /// overflowing repeat count is `NumberFormatException`.
    #[test]
    fn gap_g204_regexp_interval_unicode_digits() {
        let regexp = RegExp::new("<\u{660}\u{661}-\u{669}\u{669}>").unwrap();
        assert_eq!(regexp.to_string(), "<01-99>");
        let run = crate::util::automaton::character_run_automaton::CharacterRunAutomaton::new(
            regexp.to_automaton().unwrap(),
        )
        .unwrap();
        assert!(run.run("42"));
        assert!(run.run("01"));
        assert!(!run.run("4"));
        assert!(!run.run("100"));
        let error = RegExp::new("a{99999999999}").expect_err("an overflowing count");
        assert!(error.is_number_format_exception(), "{error:?}");
        assert!(
            error
                .to_string()
                .contains("For input string: \"99999999999\""),
            "{error}"
        );
    }

    /// Java's `HashSet` iteration order for elements inserted in `hashes`
    /// order into the default table of 16 buckets: by bucket
    /// `(h ^ (h >>> 16)) & 15`, then by insertion within a bucket.
    fn java_hash_set_order(hashes: &[i32]) -> Vec<usize> {
        let mut distinct: Vec<(usize, i32)> = Vec::new();
        for (i, &h) in hashes.iter().enumerate() {
            if !distinct.iter().any(|&(_, seen)| seen == h) {
                distinct.push((i, h));
            }
        }
        assert!(distinct.len() <= 12, "no resize");
        let bucket = |h: i32| ((h ^ ((h as u32) >> 16) as i32) & 15) as usize;
        let mut order: Vec<(usize, usize)> = distinct
            .iter()
            .enumerate()
            .map(|(rank, &(i, h))| (bucket(h) * 100 + rank, i))
            .collect();
        order.sort_unstable();
        order.into_iter().map(|(_, i)| i).collect()
    }

    /// G-201: `BooleanQuery.rewrite` rebuilds the query from `clauseSets`
    /// when FILTER clauses repeat, iterating the FILTER `HashSet` in Java's
    /// order of `hashCode()` (`BooleanQuery.java` "remove duplicate FILTER and
    /// MUST_NOT clauses").
    #[test]
    fn gap_g201_boolean_rewrite_iterates_filter_clauses_in_java_hash_set_order() {
        use crate::search::boolean_clause::Occur;
        use crate::search::{BooleanQuery, BooleanQueryBuilder, Query, TermQuery};
        std::env::set_var("tests.seed", "C0FFEE");
        let texts = [
            "alpha", "bravo", "charlie", "delta", "bravo", "echo", "foxtrot", "golf", "alpha",
        ];
        let queries: Vec<Arc<dyn Query>> = texts
            .iter()
            .map(|t| Arc::new(TermQuery::new(Term::from_text("f", t))) as Arc<dyn Query>)
            .collect();
        let mut builder = BooleanQueryBuilder::new();
        for query in &queries {
            builder.add(Arc::clone(query), Occur::FILTER).unwrap();
        }
        let query = builder.build();
        let index = TestIndex::new(vec![Document::new()]);
        let searcher = index.searcher();
        let rewritten = query
            .rewrite(&searcher)
            .unwrap()
            .expect("duplicates are removed");
        let rewritten = rewritten
            .as_any()
            .downcast_ref::<BooleanQuery>()
            .expect("a BooleanQuery");
        let actual: Vec<String> = rewritten
            .clauses()
            .iter()
            .map(|c| crate::search::query::query_to_string(c.query().as_ref()))
            .collect();
        let hashes: Vec<i32> = queries.iter().map(|q| q.query_hash()).collect();
        let expected: Vec<String> = java_hash_set_order(&hashes)
            .into_iter()
            .map(|i| crate::search::query::query_to_string(queries[i].as_ref()))
            .collect();
        assert_eq!(actual, expected);
    }
}

/// B10-B13 and B24-B26: members restored with Lucene's shapes.
mod members {
    use super::*;

    /// G-102: `StoredField(String, FieldType)` and `StoredField(String,
    /// BytesRef, FieldType)` (`StoredField.java:47-50, 79-82`).
    #[test]
    fn gap_g102_stored_field_type_constructors() {
        use crate::document::{FieldType, StoredField};
        use crate::index::IndexableField;
        let field_type = FieldType::new();
        field_type.set_stored(true).unwrap();
        let typed = StoredField::new_type_only("t", field_type.clone()).unwrap();
        assert_eq!(typed.name(), "t");
        assert!(typed.field_type().stored());
        let bytes = StoredField::new_bytes_with_field_type(
            "b",
            crate::util::BytesRef::new(vec![1, 2, 3]),
            field_type,
        )
        .unwrap();
        assert_eq!(bytes.binary_value().unwrap().slice(), &[1, 2, 3]);
        assert!(bytes.field_type().stored());
    }

    /// G-104: the public `StandardDirectoryReader.open(Directory,
    /// SegmentInfos, List, Comparator, ExecutorService)`
    /// (`StandardDirectoryReader.java:175-185`).
    #[test]
    fn gap_g104_standard_directory_reader_open_with_segment_infos() {
        use crate::document::{Store, StringField};
        let docs = ["a", "b"]
            .iter()
            .map(|v| {
                let mut doc = Document::new();
                doc.add(Box::new(
                    StringField::new("id", v.to_string(), Store::NO).unwrap(),
                ));
                doc
            })
            .collect();
        let index = TestIndex::new(docs);
        index.writer.commit().unwrap();
        let dir = Arc::clone(index.writer.get_directory());
        let infos = crate::index::SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
        let reader =
            crate::index::directory_reader::StandardDirectoryReader::open_with_segment_infos(
                dir, infos, None, None, None,
            )
            .unwrap();
        assert_eq!(reader.num_docs(), 2);
        reader.close().unwrap();
    }

    /// G-105: the public `Axiomatic` base constructors validate `s`, `k` and
    /// `queryLen` with Lucene's messages (`Axiomatic.java:66-77`).
    #[test]
    fn gap_g105_axiomatic_base_constructor_checks() {
        use crate::search::similarities::axiomatic::AxiomaticParams;
        let message = |r: crate::error::Result<AxiomaticParams>| {
            illegal_argument_message(&r.err().expect("refused")).to_string()
        };
        assert_eq!(
            message(AxiomaticParams::new(true, 1.5, 1, 0.35)),
            "illegal s value: 1.5, must be between 0 and 1"
        );
        assert_eq!(
            message(AxiomaticParams::new(true, 0.25, 1, -0.5)),
            "illegal k value: -0.5, must be between 0 and 1"
        );
        assert_eq!(
            message(AxiomaticParams::new(true, 0.25, -1, 0.35)),
            "illegal query length value: -1, must be larger 0"
        );
        assert!(AxiomaticParams::new(true, 0.25, 1, 0.35).is_ok());
    }

    /// G-106: `IOUtils.deleteFilesIgnoringExceptions(Directory, ...)` deletes
    /// each file and ignores the failures (`IOUtils.java:223-237`).
    #[test]
    fn gap_g106_delete_files_ignoring_exceptions_in_a_directory() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        for name in ["a", "b"] {
            let mut out = dir
                .create_output(name, &*crate::store::DEFAULT_IO_CONTEXT)
                .unwrap();
            out.write_byte(1).unwrap();
            out.close().unwrap();
        }
        crate::util::IOUtils::delete_files_ignoring_exceptions_in(&*dir, ["missing", "a", "b"]);
        assert!(dir.list_all().unwrap().is_empty());
    }

    /// G-107: `PriorityQueue(int, Supplier)` pre-populates the queue with
    /// sentinels, so `size() == maxSize` (`PriorityQueue.java:96-112`).
    #[test]
    fn gap_g107_priority_queue_sentinels_fill_the_queue() {
        struct Less;
        impl crate::util::extra::PriorityQueueComparator<i32> for Less {
            fn less_than(&self, a: &i32, b: &i32) -> bool {
                a < b
            }
        }
        let queue =
            crate::util::extra::PriorityQueue::with_sentinels(4, Less, || Some(i32::MIN)).unwrap();
        assert_eq!(queue.size(), 4);
        assert_eq!(queue.top(), Some(&i32::MIN));
    }

    /// G-108: `RamUsageEstimator.humanReadableUnits(long, DecimalFormat)`
    /// (`RamUsageEstimator.java:606-616`); expected values printed by
    /// `lucene-core-10.5.0.jar` with `new DecimalFormat("#,##0.0#",
    /// DecimalFormatSymbols.getInstance(Locale.ROOT))`.
    #[test]
    fn gap_g108_human_readable_units_with_a_decimal_format() {
        let df = crate::util::java_fmt::DecimalFormat::new("#,##0.0#").unwrap();
        let rendered: Vec<String> = [512i64, 1536, 5_000_000, 3_221_225_472, 1_234_567_890_123]
            .iter()
            .map(|&b| crate::util::RamUsageEstimator::human_readable_units_with(b, &df))
            .collect();
        assert_eq!(
            rendered,
            ["512 bytes", "1.5 KB", "4.77 MB", "3.0 GB", "1,149.78 GB"]
        );
    }

    /// G-416: `FSTCompiler.NullFSTReader` throws
    /// `UnsupportedOperationException("FST was not constructed with
    /// getOnHeapReaderWriter()")` (`FSTCompiler.java:205-217`).
    #[test]
    fn gap_g416_null_fst_reader_message() {
        use crate::util::fst::{FSTReader, NullFSTReader};
        let reader = NullFSTReader;
        for error in [
            reader
                .get_reverse_bytes_reader()
                .err()
                .expect("unsupported"),
            reader
                .write_to(&mut crate::store::ByteBuffersDataOutput::new())
                .unwrap_err(),
        ] {
            assert!(
                matches!(
                    &error.kind,
                    LuceneErrorKind::UnsupportedOperation { message, .. }
                        if message == "FST was not constructed with getOnHeapReaderWriter()"
                ),
                "{error:?}"
            );
        }
    }
}

/// G-203: the block-tree frames handle corrupt lengths as Lucene does, with
/// no `.max(0)` clamp.
mod blocktree_lengths {
    use super::*;
    use crate::codecs::lucene103::segment_terms_enum::{java_grow_buffer, java_read_len};

    /// A negative `numBytes` reaches `ArrayUtil.growNoCopy` /
    /// `ArrayUtil.oversize`, which throws `IllegalArgumentException("invalid
    /// array size -1")` (`ArrayUtil.java` `oversize`), and a negative length
    /// handed to `readBytes(byte[], 0, len)` is the JDK's
    /// `IndexOutOfBoundsException` (`SegmentTermsEnumFrame.loadBlock`,
    /// `nextLeaf`, `scanToTermLeaf`, `binarySearchTermLeaf`).
    #[test]
    fn gap_g203_negative_lengths_fail_as_java() {
        let mut buf = Vec::new();
        let mut is_null = true;
        let error = java_grow_buffer(&mut buf, &mut is_null, -1).unwrap_err();
        assert_eq!(illegal_argument_message(&error), "invalid array size -1");
        java_grow_buffer(&mut buf, &mut is_null, 5).unwrap();
        assert!(buf.len() >= 5 && !is_null);
        let error = java_read_len(-3, 8).unwrap_err();
        assert_eq!(
            java_to_string(&error),
            "java.lang.IndexOutOfBoundsException: Range [0, 0 + -3) out of bounds for length 8"
        );
        assert_eq!(java_read_len(4, 8).unwrap(), 4);
    }
}
