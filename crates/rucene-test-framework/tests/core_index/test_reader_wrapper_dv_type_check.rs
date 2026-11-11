//! Port of `lucene/core/src/test/org/apache/lucene/index/TestReaderWrapperDVTypeCheck.java`.

use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, SortedDocValuesField, SortedSetDocValuesField, Store};
use rucene::error::Result;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::IndexReader;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{class, random, test};
use crate::support::assert_null_msg;

/// `TestReaderWrapperDVTypeCheck`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestReaderWrapperDVTypeCheck",
        vec![test("testNoDVFieldOnSegment", test_no_dv_field_on_segment)],
    )
}

/// Port of `TestReaderWrapperDVTypeCheck.testNoDVFieldOnSegment`.
fn test_no_dv_field_on_segment() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let cfg = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    cfg.set_codec(TestUtil::always_doc_values_format(
        TestUtil::get_default_doc_values_format(),
    ))?;
    let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), cfg)?;

    let mut sdv_exist = false;
    let mut ssdv_exist = false;

    let seed = random().next_long();
    {
        let index_random = JdkRandom::new(seed);
        let docs = TestUtil::next_int(&index_random, 1, 4);
        // System.out.println("docs:"+docs);

        for i in 0..docs {
            let mut d = Document::new();
            d.add(Box::new(LuceneTestCase::new_string_field(
                "id",
                &i.to_string(),
                Store::NO,
            )));
            if LuceneTestCase::rarely_with(&index_random) {
                // System.out.println("on:"+i+" rarely: true");
                d.add(Box::new(SortedDocValuesField::new(
                    "sdv",
                    BytesRef::new(i.to_string().into_bytes()),
                )));
                sdv_exist = true;
            } else {
                // System.out.println("on:"+i+" rarely: false");
            }
            let num_sorted_set = index_random.next_int_bound(5) - 3;
            for j in 0..num_sorted_set {
                // System.out.println("on:"+i+" add ssdv:"+j);
                d.add(Box::new(SortedSetDocValuesField::new(
                    "ssdv",
                    BytesRef::new(j.to_string().into_bytes()),
                )));
                ssdv_exist = true;
            }
            iw.add_document(&d)?;
            iw.commit()?;
        }
    }
    iw.force_merge(1)?;
    let reader = iw.get_reader()?;

    // System.out.println("sdv:"+ sdvExist+ " ssdv:"+ssdvExist+", segs: "+reader.leaves().size() +",
    // "+reader.leaves());

    iw.close()?;
    let wrapper =
        LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as Arc<dyn IndexReader>)?;

    {
        // final Random indexRandom = new Random(seed);
        let sdv = wrapper.get_sorted_doc_values("sdv")?;
        let ssdv = wrapper.get_sorted_set_doc_values("ssdv")?;

        assert_null_msg(
            "confusing DV type",
            &wrapper
                .get_sorted_doc_values("ssdv")?
                .map(|_| "SortedDocValues"),
        );
        assert_null_msg(
            "confusing DV type",
            &wrapper
                .get_sorted_set_doc_values("sdv")?
                .map(|_| "SortedSetDocValues"),
        );

        assert_null_msg(
            "absent field",
            &wrapper
                .get_sorted_doc_values("NOssdv")?
                .map(|_| "SortedDocValues"),
        );
        assert_null_msg(
            "absent field",
            &wrapper
                .get_sorted_set_doc_values("NOsdv")?
                .map(|_| "SortedSetDocValues"),
        );

        Assert::assert_true_msg("optional sdv field", sdv_exist == sdv.is_some());
        Assert::assert_true_msg("optional ssdv field", ssdv_exist == ssdv.is_some());
    }
    reader.close()?;

    dir.close()
}
