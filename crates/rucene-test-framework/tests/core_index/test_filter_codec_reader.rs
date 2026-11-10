//! Port of `lucene/core/src/test/org/apache/lucene/index/TestFilterCodecReader.java`.
//!
//! * Java reflects on `CodecReader` and `FilterCodecReader`
//!   (`getDeclaredMethods`, modifiers, return types). Rucene has no run-time
//!   reflection, so both method sets are declared lists: the non-private,
//!   non-final, non-static methods `CodecReader.java` declares, and the
//!   methods Rucene's `impl CodecReader for FilterCodecReader` and `impl
//!   LeafReader for FilterCodecReader` (`src/index/filter_reader.rs`) define,
//!   under their Java names. A Rust override has the trait's signature, so
//!   the return types are equal by construction.

use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::codec_reader::CodecReader;
use rucene::index::directory_reader;
use rucene::index::filter_reader::FilterCodecReader;
use rucene::index::index_writer::IndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals_msg, class, test};

/// `TestFilterCodecReader`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestFilterCodecReader",
        vec![
            test(
                "testDeclaredMethodsOverridden",
                test_declared_methods_overridden,
            ),
            test("testGetDelegate", test_get_delegate),
        ],
    )
}

/// A declared method: name, parameter types, return type, `private`,
/// `final`, `static`.
struct DeclaredMethod {
    name: &'static str,
    parameter_types: &'static [&'static str],
    return_type: &'static str,
    is_private: bool,
    is_final: bool,
    is_static: bool,
}

const fn m(
    name: &'static str,
    parameter_types: &'static [&'static str],
    return_type: &'static str,
    is_final: bool,
) -> DeclaredMethod {
    DeclaredMethod {
        name,
        parameter_types,
        return_type,
        is_private: false,
        is_final,
        is_static: false,
    }
}

const S: &str = "java.lang.String";

/// `CodecReader.class.getDeclaredMethods()` (`CodecReader.java:42-300`).
const CODEC_READER_METHODS: &[DeclaredMethod] = &[
    m(
        "getFieldsReader",
        &[],
        "org.apache.lucene.codecs.StoredFieldsReader",
        false,
    ),
    m(
        "getTermVectorsReader",
        &[],
        "org.apache.lucene.codecs.TermVectorsReader",
        false,
    ),
    m(
        "getNormsReader",
        &[],
        "org.apache.lucene.codecs.NormsProducer",
        false,
    ),
    m(
        "getDocValuesReader",
        &[],
        "org.apache.lucene.codecs.DocValuesProducer",
        false,
    ),
    m(
        "getPostingsReader",
        &[],
        "org.apache.lucene.codecs.FieldsProducer",
        false,
    ),
    m(
        "getPointsReader",
        &[],
        "org.apache.lucene.codecs.PointsReader",
        false,
    ),
    m(
        "getVectorReader",
        &[],
        "org.apache.lucene.codecs.KnnVectorsReader",
        false,
    ),
    m(
        "storedFields",
        &[],
        "org.apache.lucene.index.StoredFields",
        true,
    ),
    m(
        "termVectors",
        &[],
        "org.apache.lucene.index.TermVectors",
        true,
    ),
    m("terms", &[S], "org.apache.lucene.index.Terms", true),
    m(
        "getNumericDocValues",
        &[S],
        "org.apache.lucene.index.NumericDocValues",
        true,
    ),
    m(
        "getBinaryDocValues",
        &[S],
        "org.apache.lucene.index.BinaryDocValues",
        true,
    ),
    m(
        "getSortedDocValues",
        &[S],
        "org.apache.lucene.index.SortedDocValues",
        true,
    ),
    m(
        "getSortedNumericDocValues",
        &[S],
        "org.apache.lucene.index.SortedNumericDocValues",
        true,
    ),
    m(
        "getSortedSetDocValues",
        &[S],
        "org.apache.lucene.index.SortedSetDocValues",
        true,
    ),
    m(
        "getDocValuesSkipper",
        &[S],
        "org.apache.lucene.index.DocValuesSkipper",
        true,
    ),
    m(
        "getNormValues",
        &[S],
        "org.apache.lucene.index.NumericDocValues",
        true,
    ),
    m(
        "getPointValues",
        &[S],
        "org.apache.lucene.index.PointValues",
        true,
    ),
    m(
        "getFloatVectorValues",
        &[S],
        "org.apache.lucene.index.FloatVectorValues",
        true,
    ),
    m(
        "getByteVectorValues",
        &[S],
        "org.apache.lucene.index.ByteVectorValues",
        true,
    ),
    m(
        "searchNearestVectors",
        &[
            S,
            "float[]",
            "org.apache.lucene.search.KnnCollector",
            "org.apache.lucene.search.AcceptDocs",
        ],
        "void",
        true,
    ),
    m(
        "searchNearestVectors",
        &[
            S,
            "byte[]",
            "org.apache.lucene.search.KnnCollector",
            "org.apache.lucene.search.AcceptDocs",
        ],
        "void",
        true,
    ),
    m("doClose", &[], "void", false),
    m("checkIntegrity", &[], "void", false),
];

/// `FilterCodecReader.class.getDeclaredMethod(...)`: the methods `impl
/// CodecReader for FilterCodecReader` and `impl LeafReader for
/// FilterCodecReader` define.
const FILTER_CODEC_READER_METHODS: &[DeclaredMethod] = &[
    m(
        "getFieldsReader",
        &[],
        "org.apache.lucene.codecs.StoredFieldsReader",
        false,
    ),
    m(
        "getTermVectorsReader",
        &[],
        "org.apache.lucene.codecs.TermVectorsReader",
        false,
    ),
    m(
        "getNormsReader",
        &[],
        "org.apache.lucene.codecs.NormsProducer",
        false,
    ),
    m(
        "getDocValuesReader",
        &[],
        "org.apache.lucene.codecs.DocValuesProducer",
        false,
    ),
    m(
        "getPostingsReader",
        &[],
        "org.apache.lucene.codecs.FieldsProducer",
        false,
    ),
    m("getLiveDocs", &[], "org.apache.lucene.util.Bits", false),
    m(
        "getFieldInfos",
        &[],
        "org.apache.lucene.index.FieldInfos",
        false,
    ),
    m(
        "getPointsReader",
        &[],
        "org.apache.lucene.codecs.PointsReader",
        false,
    ),
    m(
        "getVectorReader",
        &[],
        "org.apache.lucene.codecs.KnnVectorsReader",
        false,
    ),
    m("numDocs", &[], "int", false),
    m("maxDoc", &[], "int", false),
    m(
        "getMetaData",
        &[],
        "org.apache.lucene.index.LeafMetaData",
        false,
    ),
    m("doClose", &[], "void", false),
    m("checkIntegrity", &[], "void", false),
    m(
        "getDelegate",
        &[],
        "org.apache.lucene.index.CodecReader",
        false,
    ),
];

/// Port of `TestFilterCodecReader.testDeclaredMethodsOverridden`.
fn test_declared_methods_overridden() -> Result<()> {
    // `FilterCodecReader.class.getSuperclass()` is `CodecReader`.
    impl_test_declared_methods_overridden(
        ("org.apache.lucene.index.CodecReader", CODEC_READER_METHODS),
        (
            "org.apache.lucene.index.FilterCodecReader",
            FILTER_CODEC_READER_METHODS,
        ),
    );
    Ok(())
}

/// The data address of a reader, for `assertSame`.
fn addr<T: ?Sized>(r: &Arc<T>) -> *const () {
    Arc::as_ptr(r) as *const ()
}

/// Port of `TestFilterCodecReader.testGetDelegate`.
fn test_get_delegate() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let w = IndexWriter::new(
            dir.as_directory(),
            LuceneTestCase::new_index_writer_config(),
        )?;
        let inner = (|| -> Result<()> {
            w.add_document(&Document::new())?;
            let reader = directory_reader::open_with_writer(&*w)?;
            let r = (|| -> Result<()> {
                // `(CodecReader) reader.getSequentialSubReaders().get(0)`.
                let sub = Arc::clone(&reader)
                    .leaves()?
                    .first()
                    .map(|c| c.leaf_reader())
                    .unwrap_or_else(|| crate::support::npe("sub reader"));
                let sub: Arc<dyn CodecReader> =
                    Arc::clone(&sub).as_codec_reader().unwrap_or_else(|| {
                        Throwable::new("java.lang.ClassCastException", "not a CodecReader").throw()
                    });
                let r: Arc<dyn CodecReader> =
                    Arc::new(FilterCodecReader::wrap_live_docs(Arc::clone(&sub), None, 1));

                Assert::assert_true_msg(
                    "expected same",
                    addr(&FilterCodecReader::unwrap(&r)) == addr(&sub),
                );
                Assert::assert_true_msg(
                    "expected same",
                    r.filter_codec_reader_delegate()
                        .is_some_and(|d| addr(d) == addr(&sub)),
                );
                Ok(())
            })();
            let closed = reader.close();
            r?;
            closed
        })();
        let closed = w.close();
        inner?;
        closed
    })();
    let closed = dir.close();
    result?;
    closed
}

fn impl_test_declared_methods_overridden(
    super_class: (&str, &[DeclaredMethod]),
    sub_class: (&str, &[DeclaredMethod]),
) {
    for super_class_method in super_class.1 {
        if super_class_method.is_private {
            continue;
        }
        if super_class_method.is_final {
            continue;
        }
        if super_class_method.is_static {
            continue;
        }
        match sub_class.1.iter().find(|s| {
            s.name == super_class_method.name
                && s.parameter_types == super_class_method.parameter_types
        }) {
            Some(sub_class_method) => assert_equals_msg(
                Some("getReturnType() difference"),
                super_class_method.return_type,
                sub_class_method.return_type,
            ),
            None => Assert::fail(&format!(
                "class {} needs to override '{} {}.{}({})'",
                sub_class.0,
                super_class_method.return_type,
                super_class.0,
                super_class_method.name,
                super_class_method.parameter_types.join(",")
            )),
        }
    }
}
