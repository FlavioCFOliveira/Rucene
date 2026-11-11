//! Port of `lucene/core/src/test/org/apache/lucene/index/TestStressIndexing2.java`.
//!
//! The instance fields (`maxFields`, `bigFieldSize`, `sameFieldOrder`,
//! `mergeFactor`, `maxBufferedDocs`, `seed`, `fieldTypes`) are a
//! [`TestStressIndexing2`] value each test method creates, as JUnit creates
//! one instance per method. An `IndexingThread` shares its `docs` with the
//! test, which reads them after `join` (`synchronized (th)`); every
//! `HashMap<String, Document>` iterates in Java's order (`JavaHashMap`). A document
//! kept in `docs` and re-indexed by `indexSerial` shares its fields, as the
//! Java `Field` objects are shared.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, FieldType, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::field_infos::FieldInfos;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::OpenMode;
use rucene::index::merge_policy::{LogMergePolicy, MergePolicy};
use rucene::index::multi_bits::MultiBits;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::postings_enum::{
    PostingsEnum, POSTINGS_ENUM_ALL, POSTINGS_ENUM_FREQS, POSTINGS_ENUM_NONE,
};
use rucene::index::terms::{Fields, Term, TermsEnum};
use rucene::index::{IndexOptions, IndexReader, IndexableField};
use rucene::search::term_query::TermQuery;
use rucene::search::NO_MORE_DOCS;
use rucene::store::Directory;
use rucene::util::java_collections::JavaHashMap;
use rucene::util::Bits;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_null, assertion_error, class, random, test,
    verbose,
};
use crate::support::npe;

/// `TestStressIndexing2`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestStressIndexing2",
        vec![
            test("testRandomIWReader", test_random_iw_reader),
            test("testRandom", test_random),
            test("testMultiConfig", test_multi_config),
        ],
    )
}

/// The instance fields of `TestStressIndexing2`.
struct TestStressIndexing2 {
    max_fields: i32,
    big_field_size: i32,
    same_field_order: bool,
    merge_factor: i32,
    max_buffered_docs: i32,
    seed: i32,
    /// `private final Map<String, FieldType> fieldTypes = new ConcurrentHashMap<>()`.
    field_types: Arc<Mutex<HashMap<String, Arc<FieldType>>>>,
}

impl Default for TestStressIndexing2 {
    fn default() -> Self {
        Self {
            max_fields: 4,
            big_field_size: 10,
            same_field_order: false,
            merge_factor: 3,
            max_buffered_docs: 3,
            seed: 0,
            field_types: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

/// `Comparator.comparing(IndexableField::name)`, sorted stably as
/// `Collections.sort` does.
fn sort_by_name(fields: &mut [Arc<dyn IndexableField>]) {
    fields.sort_by(|a, b| a.name().cmp(b.name()));
}

/// `List.toString()` of fields.
fn fields_to_string(fields: &[Arc<dyn IndexableField>]) -> String {
    let parts: Vec<String> = fields.iter().map(|f| f.to_string()).collect();
    format!("[{}]", parts.join(", "))
}

/// Port of `TestStressIndexing2.testRandomIWReader`.
fn test_random_iw_reader() -> Result<()> {
    let mut this = TestStressIndexing2::default();
    let mut dir = LuceneTestCase::new_maybe_virus_checking_directory();

    // TODO: verify equals using IW.getReader
    let dw = this.index_random_iw_reader(5, 3, 100, &dir.as_directory())?;
    let reader = directory_reader::open_with_writer(&*dw.writer)?;
    dw.writer.commit()?;
    this.verify_equals_reader_dir(&*random(), &reader, &dir.as_directory(), "id")?;
    reader.close()?;
    dw.writer.close()?;
    dir.close()
}

/// Port of `TestStressIndexing2.testRandom`.
fn test_random() -> Result<()> {
    let mut this = TestStressIndexing2::default();
    let mut dir1 = LuceneTestCase::new_maybe_virus_checking_directory();
    let mut dir2 = LuceneTestCase::new_maybe_virus_checking_directory();
    // mergeFactor=2; maxBufferedDocs=2; Map docs = indexRandom(1, 3, 2, dir1);
    let do_reader_pooling = random().next_boolean();
    let docs = this.index_random(5, 3, 100, &dir1.as_directory(), do_reader_pooling)?;
    index_serial(&*random(), &docs, &dir2.as_directory())?;

    // verifying verify
    // verifyEquals(dir1, dir1, "id");
    // verifyEquals(dir2, dir2, "id");

    this.verify_equals_dirs(&dir1.as_directory(), &dir2.as_directory(), "id")?;
    dir1.close()?;
    dir2.close()
}

/// Port of `TestStressIndexing2.testMultiConfig`.
fn test_multi_config() -> Result<()> {
    let mut this = TestStressIndexing2::default();
    // test lots of smaller different params together

    let num = LuceneTestCase::at_least(3);
    for i in 0..num {
        // increase iterations for better testing
        if verbose() {
            println!("\n\nTEST: top iter={i}");
        }
        this.same_field_order = random().next_boolean();
        this.merge_factor = random().next_int_bound(3) + 2;
        this.max_buffered_docs = random().next_int_bound(3) + 2;
        let do_reader_pooling = random().next_boolean();
        this.seed += 1;

        let n_threads = random().next_int_bound(5) + 1;
        let iter = random().next_int_bound(5) + 1;
        let range = random().next_int_bound(20) + 1;
        let mut dir1 = LuceneTestCase::new_directory();
        let mut dir2 = LuceneTestCase::new_directory();
        if verbose() {
            println!(
                "  nThreads={n_threads} iter={iter} range={range} doPooling={do_reader_pooling} \
                 sameFieldOrder={} mergeFactor={} maxBufferedDocs={}",
                this.same_field_order, this.merge_factor, this.max_buffered_docs
            );
        }
        let docs = this.index_random(
            n_threads,
            iter,
            range,
            &dir1.as_directory(),
            do_reader_pooling,
        )?;
        if verbose() {
            println!("TEST: index serial");
        }
        index_serial(&*random(), &docs, &dir2.as_directory())?;
        if verbose() {
            println!("TEST: verify");
        }
        this.verify_equals_dirs(&dir1.as_directory(), &dir2.as_directory(), "id")?;
        dir1.close()?;
        dir2.close()?;
    }
    Ok(())
}

/// `public static class DocsAndWriter`.
struct DocsAndWriter {
    #[allow(dead_code)]
    docs: JavaHashMap<Arc<Document>>,
    writer: Arc<IndexWriter>,
}

/// `(LogMergePolicy) w.getConfig().getMergePolicy()`.
fn as_log_merge_policy(w: &IndexWriter) -> Arc<dyn MergePolicy> {
    let mp = w.get_config().merge_policy();
    if mp
        .as_any()
        .and_then(|a| a.downcast_ref::<LogMergePolicy>())
        .is_none()
    {
        Throwable::new("java.lang.ClassCastException", "not a LogMergePolicy").throw()
    }
    mp
}

/// `lmp.setNoCFSRatio(0.0); lmp.setMergeFactor(mergeFactor);`.
fn configure_lmp(w: &IndexWriter, merge_factor: i32) -> Result<()> {
    let mp = as_log_merge_policy(w);
    let lmp = mp
        .as_any()
        .and_then(|a| a.downcast_ref::<LogMergePolicy>())
        .expect("checked");
    mp.set_no_cfs_ratio(0.0)?;
    lmp.set_merge_factor(merge_factor)?;
    Ok(())
}

impl TestStressIndexing2 {
    /// `public DocsAndWriter indexRandomIWReader(int nThreads, int iterations, int range,
    /// Directory dir)`.
    fn index_random_iw_reader(
        &mut self,
        n_threads: i32,
        iterations: i32,
        range: i32,
        dir: &Arc<dyn Directory>,
    ) -> Result<DocsAndWriter> {
        let mut docs = JavaHashMap::new();
        let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
            MockAnalyzer::new(&*random()),
        )));
        iwc.set_open_mode(OpenMode::CREATE)?;
        iwc.set_ram_buffer_size_mb(0.1)?;
        iwc.set_max_buffered_docs(self.max_buffered_docs)?;
        iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
        let w = RandomIndexWriter::mock_index_writer(Arc::clone(dir), iwc, &*random())?;
        w.commit()?;
        configure_lmp(&w, self.merge_factor)?;
        /*
         * w.setMaxMergeDocs(Integer.MAX_VALUE);
         * w.setMaxFieldLength(10000);
         * w.setRAMBufferSizeMB(1);
         * w.setMergeFactor(10);
         */

        let threads = self.run_threads(&w, n_threads, iterations, range);

        // w.forceMerge(1);
        // w.close();

        for th in &threads {
            for (k, v) in th.take_docs().iter() {
                docs.insert(k.clone(), Arc::clone(v));
            }
        }

        TestUtil::check_index(Arc::clone(dir))?;
        Ok(DocsAndWriter { docs, writer: w })
    }

    /// `public Map<String, Document> indexRandom(int nThreads, int iterations, int range,
    /// Directory dir, boolean doReaderPooling)`.
    fn index_random(
        &mut self,
        n_threads: i32,
        iterations: i32,
        range: i32,
        dir: &Arc<dyn Directory>,
        do_reader_pooling: bool,
    ) -> Result<JavaHashMap<Arc<Document>>> {
        let mut docs = JavaHashMap::new();
        let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
            MockAnalyzer::new(&*random()),
        )));
        iwc.set_open_mode(OpenMode::CREATE)?;
        iwc.set_ram_buffer_size_mb(0.1)?;
        iwc.set_max_buffered_docs(self.max_buffered_docs)?;
        iwc.set_reader_pooling(do_reader_pooling);
        iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
        let w = RandomIndexWriter::mock_index_writer(Arc::clone(dir), iwc, &*random())?;
        configure_lmp(&w, self.merge_factor)?;

        let threads = self.run_threads(&w, n_threads, iterations, range);

        // w.forceMerge(1);
        w.close()?;

        for th in &threads {
            for (k, v) in th.take_docs().iter() {
                docs.insert(k.clone(), Arc::clone(v));
            }
        }

        // System.out.println("TEST: checkindex");
        TestUtil::check_index(Arc::clone(dir))?;

        Ok(docs)
    }

    /// Creates, starts and joins the `IndexingThread`s.
    fn run_threads(
        &self,
        w: &Arc<IndexWriter>,
        n_threads: i32,
        iterations: i32,
        range: i32,
    ) -> Vec<IndexingThreadHandle> {
        let mut threads = Vec::new();
        for i in 0..n_threads {
            let th = IndexingThread {
                w: Arc::clone(w),
                base: 1_000_000 * i,
                range,
                iterations,
                docs: JavaHashMap::new(),
                r: None,
                buffer: vec![0u16; 100],
                max_fields: self.max_fields,
                big_field_size: self.big_field_size,
                same_field_order: self.same_field_order,
                seed: self.seed,
                field_types: Arc::clone(&self.field_types),
            };
            threads.push(IndexingThreadHandle::new(th));
        }

        for th in &threads {
            th.thread.start();
        }
        for th in &threads {
            let _ = th.thread.join();
        }
        threads
    }

    /// `public void verifyEquals(Random r, DirectoryReader r1, Directory dir2, String idField)`.
    fn verify_equals_reader_dir(
        &self,
        _r: &dyn Random,
        r1: &Arc<dyn DirectoryReader>,
        dir2: &Arc<dyn Directory>,
        id_field: &str,
    ) -> Result<()> {
        let r2 = directory_reader::open(Arc::clone(dir2))?;
        verify_equals_readers(r1, &r2, id_field)?;
        r2.close()
    }

    /// `public void verifyEquals(Directory dir1, Directory dir2, String idField)`.
    fn verify_equals_dirs(
        &self,
        dir1: &Arc<dyn Directory>,
        dir2: &Arc<dyn Directory>,
        id_field: &str,
    ) -> Result<()> {
        let r1 = directory_reader::open(Arc::clone(dir1))?;
        let r2 = directory_reader::open(Arc::clone(dir2))?;
        verify_equals_readers(&r1, &r2, id_field)?;
        r1.close()?;
        r2.close()
    }
}

/// `public static void indexSerial(Random random, Map<String, Document> docs, Directory dir)`.
fn index_serial(
    random: &dyn Random,
    docs: &JavaHashMap<Arc<Document>>,
    dir: &Arc<dyn Directory>,
) -> Result<()> {
    let iwc = LuceneTestCase::new_index_writer_config_with(
        random,
        Some(Arc::new(MockAnalyzer::new(random))),
    );
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let w = IndexWriter::new(Arc::clone(dir), iwc)?;

    // index all docs in a single thread
    for d in docs.values() {
        let mut fields: Vec<Arc<dyn IndexableField>> = d.get_fields().to_vec();
        // put fields in same order each time
        sort_by_name(&mut fields);

        let mut d1 = Document::new();
        for field in fields {
            d1.add_shared(field);
        }
        w.add_document(&d1)?;
        // System.out.println("indexing "+d1);
    }

    w.close()
}

/// `private static void printDocs(DirectoryReader r)`.
fn print_docs(r: &Arc<dyn DirectoryReader>) -> Result<()> {
    for ctx in (Arc::clone(r) as Arc<dyn IndexReader>).leaves()? {
        // TODO: improve this
        let sub = ctx.leaf_reader();
        let live_docs = sub.get_live_docs();
        let stored_fields = sub.stored_fields()?;
        let si = sub
            .as_segment_reader()
            .unwrap_or_else(|| {
                Throwable::new("java.lang.ClassCastException", "not a SegmentReader").throw()
            })
            .get_segment_info();
        println!("  {si}");
        for doc_id in 0..sub.max_doc() {
            let doc = stored_fields.document(doc_id)?;
            let id = doc.get("id").unwrap_or_else(|| "null".to_string());
            if live_docs.as_ref().is_none_or(|l| l.get(doc_id as usize)) {
                println!("    docID={doc_id} id:{id}");
            } else {
                println!("    DEL docID={doc_id} id:{id}");
            }
        }
    }
    Ok(())
}

/// `private static int nextNonDeletedDoc(PostingsEnum it, Bits liveDocs)`.
fn next_non_deleted_doc(it: &mut dyn PostingsEnum, live_docs: Option<&dyn Bits>) -> Result<i32> {
    let mut doc = it.next_doc()?;
    while doc != NO_MORE_DOCS && live_docs.is_some_and(|l| !l.get(doc as usize)) {
        doc = it.next_doc()?;
    }
    Ok(doc)
}

/// The sorted indexed fields of a reader.
fn sorted_indexed_fields(r: &Arc<dyn DirectoryReader>) -> Result<std::vec::IntoIter<String>> {
    let mut fields: Vec<String> =
        FieldInfos::get_indexed_fields(Arc::clone(r) as Arc<dyn IndexReader>)?
            .into_iter()
            .collect();
    fields.sort();
    Ok(fields.into_iter())
}

/// Prints the term vectors of one document, as the `catch` of `verifyEquals` does.
fn print_term_vectors(tv: Option<&dyn Fields>, check_has_positions: bool) -> Result<()> {
    if let Some(tv) = tv {
        let mut dp_enum: Option<Box<dyn PostingsEnum>> = None;
        let mut d_enum: Option<Box<dyn PostingsEnum>> = None;
        let names: Vec<String> = tv.iterator().collect();
        for field in names {
            println!("    {field}:");
            let terms3 = tv.terms(&field)?;
            assert_not_null(&terms3);
            let terms3 = terms3.expect("checked");
            let mut terms_enum3 = terms3.iterator()?;
            while let Some(term2) = terms_enum3.next()? {
                println!(
                    "      {}: freq={}",
                    term2.utf8_to_string()?,
                    terms_enum3.total_term_freq()?
                );
                dp_enum = terms_enum3.postings(dp_enum.take(), POSTINGS_ENUM_ALL)?;
                // tv1: `if (terms3.hasPositions())`; tv2: `if (dpEnum != null)`.
                let has_positions = if check_has_positions {
                    terms3.has_positions()
                } else {
                    dp_enum.is_some()
                };
                if has_positions {
                    let dp = dp_enum
                        .as_mut()
                        .ok_or_else(rucene::error::LuceneError::null_pointer)?;
                    Assert::assert_true(dp.next_doc()? != NO_MORE_DOCS);
                    let freq = dp.freq()?;
                    println!("        doc={} freq={freq}", dp.doc_id());
                    for _ in 0..freq {
                        println!("          pos={}", dp.next_position()?);
                    }
                } else {
                    let d = TestUtil::docs_of(
                        &*random(),
                        &mut *terms_enum3,
                        d_enum.take(),
                        POSTINGS_ENUM_FREQS,
                    )?;
                    assert_not_null(&d);
                    let mut d = d.ok_or_else(rucene::error::LuceneError::null_pointer)?;
                    Assert::assert_true(d.next_doc()? != NO_MORE_DOCS);
                    let freq = d.freq()?;
                    println!("        doc={} freq={freq}", d.doc_id());
                    d_enum = Some(d);
                }
            }
        }
    }
    Ok(())
}

/// `public void verifyEquals(DirectoryReader r1, DirectoryReader r2, String idField)`.
fn verify_equals_readers(
    r1: &Arc<dyn DirectoryReader>,
    r2: &Arc<dyn DirectoryReader>,
    id_field: &str,
) -> Result<()> {
    if verbose() {
        println!("\nr1 docs:");
        print_docs(r1)?;
        println!("\nr2 docs:");
        print_docs(r2)?;
    }
    if r1.num_docs() != r2.num_docs() {
        return Err(assertion_error(format!(
            "r1.numDocs()={} vs r2.numDocs()={}",
            r1.num_docs(),
            r2.num_docs()
        )));
    }
    let has_deletes = !(r1.max_doc() == r2.max_doc() && r1.num_docs() == r1.max_doc());

    let mut r2r1 = vec![0i32; r2.max_doc() as usize]; // r2 id to r1 id mapping

    let ir1: Arc<dyn IndexReader> = Arc::clone(r1) as Arc<dyn IndexReader>;
    let ir2: Arc<dyn IndexReader> = Arc::clone(r2) as Arc<dyn IndexReader>;

    // create mapping from id2 space to id2 based on idField
    if FieldInfos::get_indexed_fields(Arc::clone(&ir1))?.is_empty() {
        Assert::assert_true(FieldInfos::get_indexed_fields(Arc::clone(&ir2))?.is_empty());
        return Ok(());
    }
    let Some(terms1) = MultiTerms::get_terms(&ir1, id_field)? else {
        Assert::assert_true(MultiTerms::get_terms(&ir2, id_field)?.is_none());
        return Ok(());
    };
    let mut terms_enum = terms1.iterator()?;

    let live_docs1 = MultiBits::get_live_docs(&ir1)?;
    let live_docs2 = MultiBits::get_live_docs(&ir2)?;

    let Some(terms2) = MultiTerms::get_terms(&ir2, id_field)? else {
        // make sure r1 is in fact empty (eg has only all
        // deleted docs):
        let live_docs = MultiBits::get_live_docs(&ir1)?;
        let mut docs: Option<Box<dyn PostingsEnum>> = None;
        while terms_enum.next()?.is_some() {
            let mut d = TestUtil::docs_of(
                &*random(),
                &mut *terms_enum,
                docs.take(),
                POSTINGS_ENUM_NONE,
            )?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?;
            while next_non_deleted_doc(&mut *d, live_docs.as_deref())? != NO_MORE_DOCS {
                Assert::fail("r1 is not empty but r2 is");
            }
            docs = Some(d);
        }
        return Ok(());
    };
    let mut terms_enum2 = terms2.iterator()?;

    let mut term_docs1: Option<Box<dyn PostingsEnum>> = None;
    let mut term_docs2: Option<Box<dyn PostingsEnum>> = None;

    loop {
        let Some(term) = terms_enum.next()? else {
            break;
        };
        // System.out.println("TEST: match id term=" + term);

        let mut td1 = TestUtil::docs_of(
            &*random(),
            &mut *terms_enum,
            term_docs1.take(),
            POSTINGS_ENUM_NONE,
        )?
        .ok_or_else(rucene::error::LuceneError::null_pointer)?;
        let mut td2 = if terms_enum2.seek_exact(&term)? {
            TestUtil::docs_of(
                &*random(),
                &mut *terms_enum2,
                term_docs2.take(),
                POSTINGS_ENUM_NONE,
            )?
        } else {
            None
        };

        if next_non_deleted_doc(&mut *td1, live_docs1.as_deref())? == NO_MORE_DOCS {
            // This doc is deleted and wasn't replaced
            let ok = match td2.as_mut() {
                None => true,
                Some(td2) => {
                    next_non_deleted_doc(&mut **td2, live_docs2.as_deref())? == NO_MORE_DOCS
                }
            };
            Assert::assert_true(ok);
            term_docs1 = Some(td1);
            term_docs2 = td2;
            continue;
        }

        let id1 = td1.doc_id();
        assert_equals(
            &NO_MORE_DOCS,
            &next_non_deleted_doc(&mut *td1, live_docs1.as_deref())?,
        );

        let mut td2_box = td2.unwrap_or_else(|| npe("termDocs2"));
        Assert::assert_true(
            next_non_deleted_doc(&mut *td2_box, live_docs2.as_deref())? != NO_MORE_DOCS,
        );
        let id2 = td2_box.doc_id();
        assert_equals(
            &NO_MORE_DOCS,
            &next_non_deleted_doc(&mut *td2_box, live_docs2.as_deref())?,
        );

        r2r1[id2 as usize] = id1;
        term_docs1 = Some(td1);
        term_docs2 = Some(td2_box);

        // verify stored fields are equivalent
        let d1 = r1.stored_fields()?.document(id1)?;
        let d2 = r2.stored_fields()?.document(id2)?;
        let stored = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            verify_equals_documents(&d1, &d2)
        }));
        if let Err(t) = stored {
            println!("FAILED id={term} id1={id1} id2={id2} term={term}");
            println!("  d1={}", r1.stored_fields()?.document(id1)?);
            println!("  d2={}", r2.stored_fields()?.document(id2)?);
            std::panic::resume_unwind(t);
        }

        // verify term vectors are equivalent
        let tv1 = r1.term_vectors()?.get(id1)?;
        let tv2 = r2.term_vectors()?.get(id2)?;
        let vectors = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            verify_equals_fields(tv1.as_deref(), tv2.as_deref())
        }));
        let failure = match vectors {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(Err(e)),
            Err(panic) => Some(Ok(panic)),
        };
        if let Some(failure) = failure {
            println!("FAILED id={term} id1={id1} id2={id2}");
            let tv1 = r1.term_vectors()?.get(id1)?;
            println!("  d1={}", fields_display(tv1.as_deref()));
            print_term_vectors(tv1.as_deref(), true)?;

            let tv2 = r2.term_vectors()?.get(id2)?;
            println!("  d2={}", fields_display(tv2.as_deref()));
            print_term_vectors(tv2.as_deref(), false)?;

            match failure {
                Err(e) => return Err(e),
                Ok(panic) => std::panic::resume_unwind(panic),
            }
        }
    }

    // System.out.println("TEST: done match id");

    // Verify postings
    // System.out.println("TEST: create te1");
    let mut fields1_enum = sorted_indexed_fields(r1)?;
    let mut fields2_enum = sorted_indexed_fields(r2)?;

    let mut field1: Option<String> = None;
    let mut field2: Option<String> = None;
    let mut terms_enum1: Option<Box<dyn TermsEnum>> = None;
    let mut terms_enum2: Option<Box<dyn TermsEnum>> = None;
    let mut docs1: Option<Box<dyn PostingsEnum>> = None;
    let mut docs2: Option<Box<dyn PostingsEnum>> = None;

    // pack both doc and freq into single element for easy sorting
    let mut info1 = vec![0i64; r1.num_docs() as usize];
    let mut info2 = vec![0i64; r2.num_docs() as usize];

    loop {
        let mut term1 = None;
        let mut term2 = None;

        // iterate until we get some docs
        let mut len1;
        loop {
            len1 = 0;
            if terms_enum1.is_none() {
                let Some(f) = fields1_enum.next() else {
                    break;
                };
                field1 = Some(f);
                let Some(terms) = MultiTerms::get_terms(&ir1, field1.as_deref().expect("set"))?
                else {
                    continue;
                };
                terms_enum1 = Some(terms.iterator()?);
            }
            let te1 = terms_enum1.as_mut().expect("set");
            term1 = te1.next()?;
            if term1.is_none() {
                // no more terms in this field
                terms_enum1 = None;
                continue;
            }

            // System.out.println("TEST: term1=" + term1);
            let mut d1 =
                TestUtil::docs_of(&*random(), &mut **te1, docs1.take(), POSTINGS_ENUM_FREQS)?
                    .ok_or_else(rucene::error::LuceneError::null_pointer)?;
            while d1.next_doc()? != NO_MORE_DOCS {
                let d = d1.doc_id();
                if live_docs1.as_ref().is_some_and(|l| !l.get(d as usize)) {
                    continue;
                }
                let f = d1.freq()?;
                info1[len1] = (i64::from(d) << 32) | i64::from(f);
                len1 += 1;
            }
            docs1 = Some(d1);
            if len1 > 0 {
                break;
            }
        }

        // iterate until we get some docs
        let mut len2;
        loop {
            len2 = 0;
            if terms_enum2.is_none() {
                let Some(f) = fields2_enum.next() else {
                    break;
                };
                field2 = Some(f);
                let Some(terms) = MultiTerms::get_terms(&ir2, field2.as_deref().expect("set"))?
                else {
                    continue;
                };
                terms_enum2 = Some(terms.iterator()?);
            }
            let te2 = terms_enum2.as_mut().expect("set");
            term2 = te2.next()?;
            if term2.is_none() {
                // no more terms in this field
                terms_enum2 = None;
                continue;
            }

            // System.out.println("TEST: term1=" + term1);
            let mut d2 =
                TestUtil::docs_of(&*random(), &mut **te2, docs2.take(), POSTINGS_ENUM_FREQS)?
                    .ok_or_else(rucene::error::LuceneError::null_pointer)?;
            while d2.next_doc()? != NO_MORE_DOCS {
                if live_docs2
                    .as_ref()
                    .is_some_and(|l| !l.get(d2.doc_id() as usize))
                {
                    continue;
                }
                let d = r2r1[d2.doc_id() as usize];
                let f = d2.freq()?;
                info2[len2] = (i64::from(d) << 32) | i64::from(f);
                len2 += 1;
            }
            docs2 = Some(d2);
            if len2 > 0 {
                break;
            }
        }

        assert_equals(&len1, &len2);
        if len1 == 0 {
            break; // no more terms
        }

        assert_equals(&field1, &field2);
        assert_equals(&term1, &term2);

        if !has_deletes {
            assert_equals(
                &terms_enum1.as_mut().expect("set").doc_freq()?,
                &terms_enum2.as_mut().expect("set").doc_freq()?,
            );
        }

        assert_equals_msg(
            Some(&format!("len1={len1} len2={len2} deletes?={has_deletes}")),
            &term1,
            &term2,
        );

        // sort info2 to get it into ascending docid
        info2[..len2].sort_unstable();

        // now compare
        for i in 0..len1 {
            assert_equals_msg(
                Some(&format!(
                    "i={i} len={len1} d1={} f1={} d2={} f2={} field={} term={}",
                    (info1[i] as u64) >> 32,
                    info1[i] & i64::from(i32::MAX),
                    (info2[i] as u64) >> 32,
                    info2[i] & i64::from(i32::MAX),
                    field1.as_deref().unwrap_or("null"),
                    term1.as_ref().expect("checked").utf8_to_string()?
                )),
                &info1[i],
                &info2[i],
            );
        }
    }
    Ok(())
}

/// `Fields.toString()` of a term vectors `Fields`, `null` when absent.
fn fields_display(tv: Option<&dyn Fields>) -> String {
    match tv {
        None => "null".to_string(),
        // `Object.toString()`: the class name and the identity hash.
        Some(tv) => format!("Fields@{:x}", tv as *const dyn Fields as *const () as usize),
    }
}

/// `public static void verifyEquals(Document d1, Document d2)`.
fn verify_equals_documents(d1: &Document, d2: &Document) {
    let mut ff1: Vec<Arc<dyn IndexableField>> = d1.get_fields().to_vec();
    let mut ff2: Vec<Arc<dyn IndexableField>> = d2.get_fields().to_vec();

    sort_by_name(&mut ff1);
    sort_by_name(&mut ff2);

    let msg = format!("{} : {}", fields_to_string(&ff1), fields_to_string(&ff2));
    assert_equals_msg(Some(&msg), &ff1.len(), &ff2.len());

    for i in 0..ff1.len() {
        let f1 = &ff1[i];
        let f2 = &ff2[i];
        if f1.binary_value().is_some() {
            if f2.binary_value().is_none() {
                Throwable::new("java.lang.AssertionError", "").throw()
            }
        } else {
            let s1 = f1.string_value();
            let s2 = f2.string_value();
            assert_equals_msg(Some(&msg), &s1, &s2);
        }
    }
}

/// `public static void verifyEquals(Fields d1, Fields d2)`.
fn verify_equals_fields(d1: Option<&dyn Fields>, d2: Option<&dyn Fields>) -> Result<()> {
    let Some(d1) = d1 else {
        Assert::assert_true(d2.is_none_or(|d2| d2.size() == 0));
        return Ok(());
    };
    Assert::assert_true(d2.is_some());
    let d2 = d2.expect("checked");

    let mut fields_enum2 = d2.iterator();

    let names1: Vec<String> = d1.iterator().collect();
    for field1 in names1 {
        let field2 = fields_enum2
            .next()
            .unwrap_or_else(|| Throwable::new("java.util.NoSuchElementException", "").throw());
        assert_equals(&field1, &field2);

        let terms1 = d1.terms(&field1)?;
        assert_not_null(&terms1);
        let terms1 = terms1.expect("checked");
        let mut terms_enum1 = terms1.iterator()?;

        let terms2 = d2.terms(&field2)?;
        assert_not_null(&terms2);
        let terms2 = terms2.expect("checked");
        let mut terms_enum2 = terms2.iterator()?;

        let mut dp_enum1: Option<Box<dyn PostingsEnum>> = None;
        let mut dp_enum2: Option<Box<dyn PostingsEnum>> = None;
        let mut d_enum1: Option<Box<dyn PostingsEnum>> = None;
        let mut d_enum2: Option<Box<dyn PostingsEnum>> = None;

        while let Some(term1) = terms_enum1.next()? {
            let term2 = terms_enum2.next()?;
            assert_equals(&Some(term1), &term2);
            assert_equals(
                &terms_enum1.total_term_freq()?,
                &terms_enum2.total_term_freq()?,
            );

            dp_enum1 = terms_enum1.postings(dp_enum1.take(), POSTINGS_ENUM_ALL)?;
            dp_enum2 = terms_enum2.postings(dp_enum2.take(), POSTINGS_ENUM_ALL)?;

            if terms1.has_positions() {
                Assert::assert_true(terms2.has_positions());
                let dp1 = dp_enum1
                    .as_mut()
                    .ok_or_else(rucene::error::LuceneError::null_pointer)?;
                let dp2 = dp_enum2
                    .as_mut()
                    .ok_or_else(rucene::error::LuceneError::null_pointer)?;
                let doc_id1 = dp1.next_doc()?;
                dp2.next_doc()?;
                // docIDs are not supposed to be equal
                // int docID2 = dpEnum2.nextDoc();
                // assertEquals(docID1, docID2);
                Assert::assert_true(doc_id1 != NO_MORE_DOCS);

                let freq1 = dp1.freq()?;
                let freq2 = dp2.freq()?;
                assert_equals(&freq1, &freq2);

                for _ in 0..freq1 {
                    let pos1 = dp1.next_position()?;
                    let pos2 = dp2.next_position()?;
                    assert_equals(&pos1, &pos2);
                    if terms1.has_offsets() {
                        Assert::assert_true(terms2.has_offsets());
                        assert_equals(&dp1.start_offset(), &dp2.start_offset());
                        assert_equals(&dp1.end_offset(), &dp2.end_offset());
                    }
                }
                assert_equals(&NO_MORE_DOCS, &dp1.next_doc()?);
                assert_equals(&NO_MORE_DOCS, &dp2.next_doc()?);
            } else {
                let de1 = TestUtil::docs_of(
                    &*random(),
                    &mut *terms_enum1,
                    d_enum1.take(),
                    POSTINGS_ENUM_FREQS,
                )?;
                let de2 = TestUtil::docs_of(
                    &*random(),
                    &mut *terms_enum2,
                    d_enum2.take(),
                    POSTINGS_ENUM_FREQS,
                )?;
                assert_not_null(&de1);
                assert_not_null(&de2);
                let mut de1 = de1.ok_or_else(rucene::error::LuceneError::null_pointer)?;
                let mut de2 = de2.ok_or_else(rucene::error::LuceneError::null_pointer)?;
                let doc_id1 = de1.next_doc()?;
                de2.next_doc()?;
                // docIDs are not supposed to be equal
                // int docID2 = dEnum2.nextDoc();
                // assertEquals(docID1, docID2);
                Assert::assert_true(doc_id1 != NO_MORE_DOCS);
                let freq1 = de1.freq()?;
                let freq2 = de2.freq()?;
                assert_equals(&freq1, &freq2);
                assert_equals(&NO_MORE_DOCS, &de1.next_doc()?);
                assert_equals(&NO_MORE_DOCS, &de2.next_doc()?);
                d_enum1 = Some(de1);
                d_enum2 = Some(de2);
            }
        }

        Assert::assert_true(terms_enum2.next()?.is_none());
    }
    Assert::assert_false(fields_enum2.next().is_some());
    Ok(())
}

/// `private class IndexingThread extends Thread`: its state, which the
/// thread body owns while it runs.
struct IndexingThread {
    w: Arc<IndexWriter>,
    base: i32,
    range: i32,
    iterations: i32,
    docs: JavaHashMap<Arc<Document>>,
    r: Option<JdkRandom>,
    buffer: Vec<u16>,
    // The enclosing instance's fields the inner class reads.
    max_fields: i32,
    big_field_size: i32,
    same_field_order: bool,
    seed: i32,
    field_types: Arc<Mutex<HashMap<String, Arc<FieldType>>>>,
}

/// A started `IndexingThread`: the thread, and its `docs` after it ran.
struct IndexingThreadHandle {
    thread: Thread,
    docs: Arc<Mutex<JavaHashMap<Arc<Document>>>>,
}

impl IndexingThreadHandle {
    fn new(mut th: IndexingThread) -> Self {
        let docs = Arc::new(Mutex::new(JavaHashMap::new()));
        let out = Arc::clone(&docs);
        let thread = Thread::unnamed(move || {
            let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| th.run()));
            // synchronized (this) { docs.size(); }: the test reads `th.docs`.
            *out.lock().unwrap_or_else(|e| e.into_inner()) = std::mem::take(&mut th.docs);
            if let Err(payload) = ran {
                std::panic::resume_unwind(payload);
            }
        });
        Self { thread, docs }
    }

    /// `synchronized (th) { docs.putAll(th.docs); }`.
    fn take_docs(&self) -> JavaHashMap<Arc<Document>> {
        std::mem::take(&mut *self.docs.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl IndexingThread {
    fn r(&self) -> &JdkRandom {
        self.r.as_ref().unwrap_or_else(|| npe("r"))
    }

    fn next_int(&self, lim: i32) -> i32 {
        self.r().next_int_bound(lim)
    }

    // start is inclusive and end is exclusive
    fn next_int_range(&self, start: i32, end: i32) -> i32 {
        start + self.r().next_int_bound(end - start)
    }

    fn add_utf8_token(&mut self, start: usize) -> usize {
        let end = start + self.next_int(20) as usize;
        if self.buffer.len() < 1 + end {
            let mut new_buffer = vec![0u16; ((1 + end) as f64 * 1.25) as usize];
            new_buffer[..self.buffer.len()].copy_from_slice(&self.buffer);
            self.buffer = new_buffer;
        }

        let mut i = start;
        while i < end {
            let t = self.next_int(5);
            if 0 == t && i < end - 1 {
                // Make a surrogate pair
                // High surrogate
                self.buffer[i] = self.next_int_range(0xd800, 0xdc00) as u16;
                i += 1;
                // Low surrogate
                self.buffer[i] = self.next_int_range(0xdc00, 0xe000) as u16;
            } else if t <= 1 {
                self.buffer[i] = self.next_int(0x80) as u16;
            } else if 2 == t {
                self.buffer[i] = self.next_int_range(0x80, 0x800) as u16;
            } else if 3 == t {
                self.buffer[i] = self.next_int_range(0x800, 0xd800) as u16;
            } else if 4 == t {
                self.buffer[i] = self.next_int_range(0xe000, 0xffff) as u16;
            }
            i += 1;
        }
        self.buffer[end] = u16::from(b' ');
        1 + end
    }

    fn get_string(&mut self, n_tokens: i32) -> String {
        let n_tokens = if n_tokens != 0 {
            n_tokens
        } else {
            self.r().next_int_bound(4) + 1
        };

        // Half the time make a random UTF8 string
        if self.r().next_boolean() {
            return self.get_utf8_string(n_tokens);
        }

        // avoid StringBuffer because it adds extra synchronization.
        let mut arr = vec![0u16; (n_tokens * 2) as usize];
        for i in 0..n_tokens as usize {
            arr[i * 2] = u16::from(b'A') + self.r().next_int_bound(10) as u16;
            arr[i * 2 + 1] = u16::from(b' ');
        }
        String::from_utf16_lossy(&arr)
    }

    fn get_utf8_string(&mut self, n_tokens: i32) -> String {
        let mut upto = 0;
        self.buffer.fill(0);
        for _ in 0..n_tokens {
            upto = self.add_utf8_token(upto);
        }
        String::from_utf16_lossy(&self.buffer[..upto])
    }

    fn get_id_string(&self) -> String {
        (self.base + self.next_int(self.range)).to_string()
    }

    /// The `computeIfAbsent` mapping function of `indexDoc`.
    fn new_field_type(&self) -> Result<FieldType> {
        let ft = FieldType::new();
        match self.next_int(4) {
            0 => {}
            1 => {
                ft.set_store_term_vectors(true)?;
            }
            2 => {
                ft.set_store_term_vectors(true)?;
                ft.set_store_term_vector_positions(true)?;
            }
            3 => {
                ft.set_store_term_vectors(true)?;
                ft.set_store_term_vector_offsets(true)?;
            }
            _ => unreachable!("nextInt(4)"),
        }
        match self.next_int(4) {
            0 => {
                ft.set_stored(true)?;
                ft.set_omit_norms(true)?;
                ft.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)?;
            }
            1 => {
                ft.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)?;
                ft.set_tokenized(true)?;
            }
            2 => {
                ft.set_stored(true)?;
                ft.set_store_term_vectors(false)?;
                ft.set_store_term_vector_offsets(false)?;
                ft.set_store_term_vector_positions(false)?;
            }
            3 => {
                ft.set_stored(true)?;
                ft.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)?;
                ft.set_tokenized(true)?;
            }
            _ => unreachable!("nextInt(4)"),
        }
        ft.freeze();
        Ok(ft)
    }

    fn index_doc(&mut self) -> Result<()> {
        let mut d = Document::new();

        let custom_type1 = FieldType::new_from(TextField::type_stored());
        custom_type1.set_tokenized(false)?;
        custom_type1.set_omit_norms(true)?;

        let mut fields: Vec<Arc<dyn IndexableField>> = Vec::new();
        let id_string = self.get_id_string();
        let id_field = LuceneTestCase::new_field("id", &id_string, &custom_type1);
        fields.push(Arc::new(id_field));

        let n_fields = self.next_int(self.max_fields);
        for _ in 0..n_fields {
            let field_name = format!("f{}", self.next_int(100));
            // Use the same field type if we already added this field to the index
            let field_type = {
                let field_types = Arc::clone(&self.field_types);
                let mut map = field_types.lock().unwrap_or_else(|e| e.into_inner());
                match map.get(&field_name) {
                    Some(ft) => Arc::clone(ft),
                    None => {
                        let ft = Arc::new(self.new_field_type()?);
                        map.insert(field_name.clone(), Arc::clone(&ft));
                        ft
                    }
                }
            };
            let n_tokens = self.next_int(3);
            let n_tokens = if n_tokens < 2 {
                n_tokens
            } else {
                self.big_field_size
            };
            let value = self.get_string(n_tokens);
            fields.push(Arc::new(LuceneTestCase::new_field(
                &field_name,
                &value,
                &field_type,
            )));
        }

        if self.same_field_order {
            sort_by_name(&mut fields);
        } else {
            // random placement of id field also
            let j = self.next_int(fields.len() as i32) as usize;
            fields.swap(j, 0);
        }

        for field in fields {
            d.add_shared(field);
        }
        if verbose() {
            println!("{}: indexing id:{id_string}", Thread::current().name());
        }
        self.w
            .update_document(Term::from_text("id", &id_string), &d)?;
        // System.out.println(Thread.currentThread().getName() + ": indexing "+d);
        self.docs.insert(id_string, Arc::new(d));
        Ok(())
    }

    fn delete_doc(&mut self) -> Result<()> {
        let id_string = self.get_id_string();
        if verbose() {
            println!("{}: del id:{id_string}", Thread::current().name());
        }
        self.w
            .delete_documents_by_terms(vec![Term::from_text("id", &id_string)])?;
        self.docs.remove(&id_string);
        Ok(())
    }

    fn delete_by_query(&mut self) -> Result<()> {
        let id_string = self.get_id_string();
        if verbose() {
            println!("{}: del query id:{id_string}", Thread::current().name());
        }
        self.w
            .delete_documents_by_queries(vec![Arc::new(TermQuery::new(Term::from_text(
                "id", &id_string,
            )))])?;
        self.docs.remove(&id_string);
        Ok(())
    }

    fn run(&mut self) {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
            self.r = Some(JdkRandom::new(i64::from(
                self.base.wrapping_add(self.range).wrapping_add(self.seed),
            )));
            for _ in 0..self.iterations {
                let what = self.next_int(100);
                if what < 5 {
                    self.delete_doc()?;
                } else if what < 10 {
                    self.delete_by_query()?;
                } else {
                    self.index_doc()?;
                }
            }
            Ok(())
        }));
        let e = match outcome {
            Ok(Ok(())) => return,
            Ok(Err(e)) => java_throwable(&e),
            Err(payload) => Throwable::from_panic(payload),
        };
        Throwable::from_cause("java.lang.RuntimeException", e).throw()
    }
}
