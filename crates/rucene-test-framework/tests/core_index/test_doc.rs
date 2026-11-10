//! Port of `lucene/core/src/test/org/apache/lucene/index/TestDoc.java`.
//!
//! JUnit adaptation of an older test case DocTest.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use rucene::analysis::StringCharReader;
use rucene::document::{Document, TextField};
use rucene::error::Result;
use rucene::index::documents_writer::SharedDirectory;
use rucene::index::field_infos::FieldNumbers;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::OpenMode;
use rucene::index::postings_enum::POSTINGS_ENUM_POSITIONS;
use rucene::index::segment_merger::SegmentMerger;
use rucene::index::{IndexOptions, LeafReader, SegmentCommitInfo, SegmentInfo};
use rucene::search::{Sort, NO_MORE_DOCS};
use rucene::store::java_nio_file::{Files, Path};
use rucene::store::{merge_io_context, IOContext, MergeInfo, TrackingDirectoryWrapper};
use rucene::util::concurrent::SameThreadExecutorService;
use rucene::util::string_helper::StringHelper;
use rucene::util::{default_info_stream, Version};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::VERBOSE;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_not_null, random};

/// `TestDoc`: its fields `workDir`, `indexDir` and `files`, set by
/// `setUp()`.
#[derive(Default)]
struct TestDocInstance {
    work_dir: Option<Path>,
    index_dir: Option<Path>,
    files: Vec<Path>,
}

impl TestDocInstance {
    fn work_dir(&self) -> &Path {
        self.work_dir
            .as_ref()
            .unwrap_or_else(|| crate::support::npe("workDir"))
    }

    fn index_dir(&self) -> &Path {
        self.index_dir
            .as_ref()
            .unwrap_or_else(|| crate::support::npe("indexDir"))
    }
}

/// `TestDoc`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestDoc")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(TestDocInstance::default))
        .method(
            Method::new("setUp").throws(&["java.lang.Exception"]).body(
                |inst: &mut TestDocInstance| set_up(inst).unwrap_or_else(|e| rethrow_java(e)),
            ),
        )
        .method(
            Method::new("testIndexAndMerge")
                .throws(&["java.lang.Exception"])
                .body(|inst: &mut TestDocInstance| {
                    test_index_and_merge(inst).unwrap_or_else(|e| rethrow_java(e))
                }),
        )
        .build()
}

/// Set the test case. This test case needs a few text files created in the
/// current working directory.
fn set_up(inst: &mut TestDocInstance) -> Result<()> {
    LuceneTestCase::super_set_up();
    if *VERBOSE {
        println!("TEST: setUp");
    }
    inst.work_dir = Some(LuceneTestCase::create_temp_dir_prefix("TestDoc"));
    inst.index_dir = Some(LuceneTestCase::create_temp_dir_prefix("testIndex"));

    let mut directory = LuceneTestCase::new_fs_directory(inst.index_dir());
    directory.close()?;

    inst.files = Vec::new();
    let first = create_output(inst, "test.txt", "This is the first test file")?;
    inst.files.push(first);

    let second = create_output(inst, "test2.txt", "This is the second test file")?;
    inst.files.push(second);
    Ok(())
}

/// `private Path createOutput(String name, String text)`: the file holds
/// `pw.println(text)`, UTF-8.
fn create_output(inst: &TestDocInstance, name: &str, text: &str) -> Result<Path> {
    let path = inst.work_dir().resolve_str(name);
    Files::delete_if_exists(&path)?;

    let mut fw = Files::new_output_stream(&path, &[])?;
    let line = format!("{text}\n");
    fw.write(line.as_bytes(), 0, line.len())?;
    fw.close()?;
    Ok(path)
}

/// This test executes a number of merges and compares the contents of the
/// segments created when using compound file or not using one.
///
/// TODO: the original test used to print the segment contents to
/// System.out for visual validation. To have the same effect, a new method
/// checkSegment(String name, ...) should be created that would assert
/// various things about the segment.
///
/// Port of `TestDoc.testIndexAndMerge`.
fn test_index_and_merge(inst: &mut TestDocInstance) -> Result<()> {
    let multi_file_output = index_and_merge(inst, false)?;
    // System.out.println(multiFileOutput);

    let single_file_output = index_and_merge(inst, true)?;

    assert_equals(&multi_file_output, &single_file_output);
    Ok(())
}

/// One half of `testIndexAndMerge`: index both files, merge them three
/// times with `useCompoundFile`, and return what `printSegment` printed.
fn index_and_merge(inst: &TestDocInstance, use_compound_file: bool) -> Result<String> {
    let mut out = String::new();

    let mut directory = LuceneTestCase::new_fs_directory(inst.index_dir());

    if let Some(mock) = directory.as_mock() {
        // We create unreferenced files (we don't even write
        // a segments file):
        mock.set_assert_no_unrefenced_files_on_close(false);
    }

    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    iwc.set_open_mode(OpenMode::CREATE)?;
    iwc.set_max_buffered_docs(-1)?;
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(10)?,
    )))?;
    let writer = IndexWriter::new(directory.as_directory(), iwc)?;

    let si1 = index_doc(inst, &writer, "test.txt")?;
    print_segment(&mut out, &si1)?;

    let si2 = index_doc(inst, &writer, "test2.txt")?;
    print_segment(&mut out, &si2)?;
    writer.close()?;

    let si_merge = merge(&si1, &si2, "_merge", use_compound_file)?;
    print_segment(&mut out, &si_merge)?;

    let si_merge2 = merge(&si1, &si2, "_merge2", use_compound_file)?;
    print_segment(&mut out, &si_merge2)?;

    let si_merge3 = merge(&si_merge, &si_merge2, "_merge3", use_compound_file)?;
    print_segment(&mut out, &si_merge3)?;

    directory.close()?;
    Ok(out)
}

/// `private SegmentCommitInfo indexDoc(IndexWriter writer, String fileName)`.
///
/// Java reads the file through an `InputStreamReader` over
/// `Files.newInputStream(path)`, UTF-8; the port reads the same bytes
/// through `Files` and hands the field a reader over their UTF-8 text.
fn index_doc(
    inst: &TestDocInstance,
    writer: &IndexWriter,
    file_name: &str,
) -> Result<SegmentCommitInfo> {
    let path = inst.work_dir().resolve_str(file_name);
    let mut doc = Document::new();
    let bytes = Files::read_all_bytes(&path)?;
    let is = StringCharReader::new(String::from_utf8_lossy(&bytes).into_owned());
    doc.add(Box::new(TextField::new_with_reader(
        "contents",
        Box::new(is),
    )?));
    writer.add_document(&doc)?;
    writer.commit()?;
    Ok(writer
        .newest_segment()?
        .unwrap_or_else(|| crate::support::npe("writer.newestSegment()")))
}

/// `private SegmentCommitInfo merge(Directory dir, SegmentCommitInfo si1,
/// SegmentCommitInfo si2, String merged, boolean useCompoundFile)`; `dir`
/// is `si1.info.dir`, the directory every call passes.
fn merge(
    si1: &SegmentCommitInfo,
    si2: &SegmentCommitInfo,
    merged: &str,
    use_compound_file: bool,
) -> Result<SegmentCommitInfo> {
    let context: Arc<dyn IOContext> = Arc::from(LuceneTestCase::new_io_context_from(
        &*random(),
        &*merge_io_context(MergeInfo::new(-1, -1, false, -1)),
    ));
    let r1: Arc<dyn rucene::index::codec_reader::CodecReader> =
        Arc::new(rucene::index::segment_reader::SegmentReader::new(
            si1.clone(),
            i32::from(Version::LATEST.major),
            &*context,
        )?);
    let r2: Arc<dyn rucene::index::codec_reader::CodecReader> =
        Arc::new(rucene::index::segment_reader::SegmentReader::new(
            si2.clone(),
            i32::from(Version::LATEST.major),
            &*context,
        )?);

    let codec =
        rucene::codecs::get_default().unwrap_or_else(|| crate::support::npe("Codec.getDefault()"));
    let dir = Arc::clone(&si1.info.directory);
    let tracking_dir = Arc::new(TrackingDirectoryWrapper::new(Box::new(SharedDirectory(
        Arc::clone(&dir),
    ))));
    let si = SegmentInfo::new(
        Arc::clone(&dir),
        Version::LATEST,
        None,
        merged.to_string(),
        -1,
        false,
        false,
        Arc::clone(&codec),
        HashMap::new(),
        StringHelper::random_id(),
        HashMap::new(),
        Sort::default(),
    )?;

    let mut merger = SegmentMerger::new(
        &[Arc::clone(&r1), Arc::clone(&r2)],
        si,
        Arc::new(FieldNumbers::new(None, None)?),
        Arc::clone(&codec),
        Arc::clone(&tracking_dir) as Arc<dyn rucene::store::Directory>,
        Arc::clone(&context),
        default_info_stream(),
        Some(Arc::new(SameThreadExecutorService::new())),
        None,
    )?;

    merger.merge()?;
    let mut si = merger.into_segment_info();
    Arc::clone(&r1).as_index_reader().close()?;
    Arc::clone(&r2).as_index_reader().close()?;
    si.set_files(
        tracking_dir
            .get_created_files()
            .into_iter()
            .collect::<HashSet<_>>(),
    )?;

    if use_compound_file {
        let files_to_delete = si.files()?;
        codec.compound_format().write(&*dir, &si, &*context)?;
        si.set_use_compound_file(true);
        for name in &files_to_delete {
            si1.info.directory.delete_file(name)?;
        }
    }

    SegmentCommitInfo::new(si, 0, 0, -1, -1, -1, StringHelper::random_id())
}

/// `private void printSegment(PrintWriter out, SegmentCommitInfo si)`.
fn print_segment(out: &mut String, si: &SegmentCommitInfo) -> Result<()> {
    let context = LuceneTestCase::new_io_context(&*random());
    let reader: Arc<dyn LeafReader> = Arc::new(rucene::index::segment_reader::SegmentReader::new(
        si.clone(),
        i32::from(Version::LATEST.major),
        &*context,
    )?);

    let stored_fields = reader.stored_fields()?;
    for i in 0..reader.num_docs() {
        let _ = writeln!(out, "{}", stored_fields.document(i)?);
    }

    for field_info in reader.get_field_infos().iter() {
        if field_info.get_index_options() == IndexOptions::NONE {
            continue;
        }
        let terms = reader.terms(&field_info.name)?;
        assert_not_null(&terms);
        let terms = terms.expect("checked by assertNotNull");
        let mut tis = terms.iterator()?;
        while tis.next()?.is_some() {
            let _ = write!(out, "  term={}:{}", field_info.name, tis.term()?);
            let _ = writeln!(out, "    DF={}", tis.doc_freq()?);

            let mut positions = tis.postings(None, POSTINGS_ENUM_POSITIONS)?;

            let live_docs = reader.get_live_docs();
            while positions.next_doc()? != NO_MORE_DOCS {
                if live_docs
                    .as_ref()
                    .is_some_and(|l| !l.get(positions.doc_id() as usize))
                {
                    continue;
                }
                let _ = write!(out, " doc={}", positions.doc_id());
                let _ = write!(out, " TF={}", positions.freq()?);
                let _ = write!(out, " pos=");
                let _ = write!(out, "{}", positions.next_position()?);
                for _ in 1..positions.freq()? {
                    let _ = write!(out, ",{}", positions.next_position()?);
                }
                let _ = writeln!(out);
            }
        }
    }
    Arc::clone(&reader).as_index_reader().close()?;
    Ok(())
}
