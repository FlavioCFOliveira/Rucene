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

//! Port of `org.apache.lucene.tests.index.DocHelper`.
//!
//! Java's field types and fields (`customType`, `textField1`, `fields`, …) are
//! `static` instances shared by every document `setupDoc` fills; here each is
//! a lazily built `static` shared through an [`Arc`], and `setupDoc` adds the
//! same objects ([`Document::add_shared`]).

use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::random::Random;
use rucene::analysis::Analyzer;
use rucene::document::{Document, Field, FieldType, Store, StoredField, StringField, TextField};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::{IndexOptions, IndexableField, SegmentCommitInfo};
use rucene::internal::tests_hooks::{IndexWriterAccess, TestSecrets};
use rucene::search::similarities::Similarity;
use rucene::search::IndexSearcher;
use rucene::store::Directory;
use rucene::util::java_collections::JavaHashMap;
use rucene::util::BytesRef;

use crate::analysis::mock_tokenizer::WHITESPACE;
use crate::analysis::MockAnalyzer;

/// `INDEX_WRITER_ACCESS = TestSecrets.getIndexWriterAccess()`.
fn index_writer_access() -> Arc<dyn IndexWriterAccess> {
    TestSecrets::get_index_writer_access()
        .expect("IndexWriter registers its IndexWriterAccess with TestSecrets")
}

/// A value of `DocHelper.nameValues`: a `String` or the `byte[]` of the
/// binary field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameValue {
    /// A `String` value.
    Text(String),
    /// A `byte[]` value.
    Bytes(Vec<u8>),
}

/// Port of `org.apache.lucene.tests.index.DocHelper`: "Helper functions for
/// tests that handles documents".
#[derive(Debug)]
pub struct DocHelper;

/// `FIELD_1_TEXT`.
pub const FIELD_1_TEXT: &str = "field one text";
/// `TEXT_FIELD_1_KEY`.
pub const TEXT_FIELD_1_KEY: &str = "textField1";
/// `FIELD_2_TEXT`.
pub const FIELD_2_TEXT: &str = "field field field two text";
/// `FIELD_2_FREQS`: "Fields will be lexicographically sorted. So, the order
/// is: field, text, two".
pub const FIELD_2_FREQS: [i32; 3] = [3, 1, 1];
/// `TEXT_FIELD_2_KEY`.
pub const TEXT_FIELD_2_KEY: &str = "textField2";
/// `FIELD_3_TEXT`.
pub const FIELD_3_TEXT: &str = "aaaNoNorms aaaNoNorms bbbNoNorms";
/// `TEXT_FIELD_3_KEY`.
pub const TEXT_FIELD_3_KEY: &str = "textField3";
/// `KEYWORD_TEXT`.
pub const KEYWORD_TEXT: &str = "Keyword";
/// `KEYWORD_FIELD_KEY`.
pub const KEYWORD_FIELD_KEY: &str = "keyField";
/// `NO_NORMS_TEXT`.
pub const NO_NORMS_TEXT: &str = "omitNormsText";
/// `NO_NORMS_KEY`.
pub const NO_NORMS_KEY: &str = "omitNorms";
/// `NO_TF_TEXT`.
pub const NO_TF_TEXT: &str = "analyzed with no tf and positions";
/// `NO_TF_KEY`.
pub const NO_TF_KEY: &str = "omitTermFreqAndPositions";
/// `UNINDEXED_FIELD_TEXT`.
pub const UNINDEXED_FIELD_TEXT: &str = "unindexed field text";
/// `UNINDEXED_FIELD_KEY`.
pub const UNINDEXED_FIELD_KEY: &str = "unIndField";
/// `UNSTORED_1_FIELD_TEXT`.
pub const UNSTORED_1_FIELD_TEXT: &str = "unstored field text";
/// `UNSTORED_FIELD_1_KEY`.
pub const UNSTORED_FIELD_1_KEY: &str = "unStoredField1";
/// `UNSTORED_2_FIELD_TEXT`.
pub const UNSTORED_2_FIELD_TEXT: &str = "unstored field text";
/// `UNSTORED_FIELD_2_KEY`.
pub const UNSTORED_FIELD_2_KEY: &str = "unStoredField2";
/// `LAZY_FIELD_BINARY_KEY`.
pub const LAZY_FIELD_BINARY_KEY: &str = "lazyFieldBinary";
/// `LAZY_FIELD_KEY`.
pub const LAZY_FIELD_KEY: &str = "lazyField";
/// `LAZY_FIELD_TEXT`.
pub const LAZY_FIELD_TEXT: &str = "These are some field bytes";
/// `LARGE_LAZY_FIELD_KEY`.
pub const LARGE_LAZY_FIELD_KEY: &str = "largeLazyField";
/// `FIELD_UTF1_TEXT` ("From Issue 509").
pub const FIELD_UTF1_TEXT: &str = "field one \u{4e00}text";
/// `TEXT_FIELD_UTF1_KEY`.
pub const TEXT_FIELD_UTF1_KEY: &str = "textField1Utf8";
/// `FIELD_UTF2_TEXT`.
pub const FIELD_UTF2_TEXT: &str = "field field field \u{4e00}two text";
/// `FIELD_UTF2_FREQS`: "Fields will be lexicographically sorted. So, the
/// order is: field, text, two".
pub const FIELD_UTF2_FREQS: [i32; 3] = [3, 1, 1];
/// `TEXT_FIELD_UTF2_KEY`.
pub const TEXT_FIELD_UTF2_KEY: &str = "textField2Utf8";

/// `LAZY_FIELD_BINARY_BYTES`: `"These are some binary field
/// bytes".getBytes(UTF_8)`.
pub const LAZY_FIELD_BINARY_BYTES: &[u8] = b"These are some binary field bytes";

/// `LARGE_LAZY_FIELD_TEXT`: "Initialize the large Lazy Field".
pub static LARGE_LAZY_FIELD_TEXT: LazyLock<String> =
    LazyLock::new(|| "Lazily loading lengths of language in lieu of laughing ".repeat(10000));

fn frozen(ft: FieldType) -> FieldType {
    ft.freeze();
    ft
}

/// A setter call on a field type that is not frozen yet.
fn set(r: Result<()>) {
    r.expect("INVARIANT: the field type is not frozen yet");
}

/// A `static final` of `DocHelper`: one instance, built on first use and
/// shared by every caller, as Java's static initializer builds it once.
type Static<T> = LazyLock<Arc<T>>;

/// `new Field(name, value, type)` for DocHelper's fields.
fn new_field(name: &str, value: &str, ft: &Arc<FieldType>) -> Field {
    Field::new(name, value.to_string(), Arc::clone(ft))
        .expect("INVARIANT: DocHelper's fields are valid")
}

static CUSTOM_TYPE: Static<FieldType> =
    LazyLock::new(|| Arc::new(FieldType::new_from(TextField::type_stored())));

static TEXT_TYPE_STORED_WITH_TVS: Static<FieldType> = LazyLock::new(|| {
    let ft = FieldType::new_from(TextField::type_stored());
    set(ft.set_store_term_vectors(true));
    set(ft.set_store_term_vector_positions(true));
    set(ft.set_store_term_vector_offsets(true));
    Arc::new(frozen(ft))
});

static CUSTOM_TYPE3: Static<FieldType> = LazyLock::new(|| {
    let ft = FieldType::new_from(TextField::type_stored());
    set(ft.set_omit_norms(true));
    Arc::new(ft)
});

static CUSTOM_TYPE5: Static<FieldType> = LazyLock::new(|| {
    let ft = FieldType::new_from(TextField::type_stored());
    set(ft.set_omit_norms(true));
    set(ft.set_tokenized(false));
    Arc::new(ft)
});

static CUSTOM_TYPE6: Static<FieldType> = LazyLock::new(|| {
    let ft = FieldType::new_from(TextField::type_stored());
    set(ft.set_index_options(IndexOptions::DOCS));
    Arc::new(ft)
});

static CUSTOM_TYPE7: Static<FieldType> = LazyLock::new(|| {
    let ft = FieldType::new();
    set(ft.set_stored(true));
    Arc::new(ft)
});

static STRING_TYPE_STORED_WITH_TVS: Static<FieldType> = LazyLock::new(|| {
    let ft = FieldType::new_from(StringField::type_stored());
    set(ft.set_store_term_vectors(true));
    set(ft.set_store_term_vector_positions(true));
    set(ft.set_store_term_vector_offsets(true));
    Arc::new(frozen(ft))
});

static CUSTOM_TYPE8: Static<FieldType> = LazyLock::new(|| {
    let ft = FieldType::new_from(TextField::type_not_stored());
    set(ft.set_store_term_vectors(true));
    Arc::new(ft)
});

static TEXT_FIELD1: Static<Field> =
    LazyLock::new(|| Arc::new(new_field(TEXT_FIELD_1_KEY, FIELD_1_TEXT, &CUSTOM_TYPE)));

static TEXT_FIELD2: Static<Field> = LazyLock::new(|| {
    Arc::new(new_field(
        TEXT_FIELD_2_KEY,
        FIELD_2_TEXT,
        &TEXT_TYPE_STORED_WITH_TVS,
    ))
});

static TEXT_FIELD3: Static<Field> =
    LazyLock::new(|| Arc::new(new_field(TEXT_FIELD_3_KEY, FIELD_3_TEXT, &CUSTOM_TYPE3)));

static KEY_FIELD: Static<StringField> = LazyLock::new(|| {
    Arc::new(
        StringField::new(KEYWORD_FIELD_KEY, KEYWORD_TEXT.to_string(), Store::YES)
            .expect("INVARIANT: DocHelper's fields are valid"),
    )
});

static NO_NORMS_FIELD: Static<Field> =
    LazyLock::new(|| Arc::new(new_field(NO_NORMS_KEY, NO_NORMS_TEXT, &CUSTOM_TYPE5)));

static NO_TF_FIELD: Static<Field> =
    LazyLock::new(|| Arc::new(new_field(NO_TF_KEY, NO_TF_TEXT, &CUSTOM_TYPE6)));

static UN_IND_FIELD: Static<Field> = LazyLock::new(|| {
    Arc::new(new_field(
        UNINDEXED_FIELD_KEY,
        UNINDEXED_FIELD_TEXT,
        &CUSTOM_TYPE7,
    ))
});

static UN_STORED_FIELD1: Static<TextField> = LazyLock::new(|| {
    Arc::new(
        TextField::new(
            UNSTORED_FIELD_1_KEY,
            UNSTORED_1_FIELD_TEXT.to_string(),
            Store::NO,
        )
        .expect("INVARIANT: DocHelper's fields are valid"),
    )
});

static UN_STORED_FIELD2: Static<Field> = LazyLock::new(|| {
    Arc::new(new_field(
        UNSTORED_FIELD_2_KEY,
        UNSTORED_2_FIELD_TEXT,
        &CUSTOM_TYPE8,
    ))
});

static LAZY_FIELD_BINARY: Static<StoredField> = LazyLock::new(|| {
    Arc::new(
        StoredField::new_bytes(
            LAZY_FIELD_BINARY_KEY,
            BytesRef::new(LAZY_FIELD_BINARY_BYTES.to_vec()),
        )
        .expect("INVARIANT: DocHelper's fields are valid"),
    )
});

static LAZY_FIELD: Static<Field> =
    LazyLock::new(|| Arc::new(new_field(LAZY_FIELD_KEY, LAZY_FIELD_TEXT, &CUSTOM_TYPE)));

static LARGE_LAZY_FIELD: Static<Field> = LazyLock::new(|| {
    Arc::new(new_field(
        LARGE_LAZY_FIELD_KEY,
        &LARGE_LAZY_FIELD_TEXT,
        &CUSTOM_TYPE,
    ))
});

static TEXT_UTF_FIELD1: Static<Field> = LazyLock::new(|| {
    Arc::new(new_field(
        TEXT_FIELD_UTF1_KEY,
        FIELD_UTF1_TEXT,
        &CUSTOM_TYPE,
    ))
});

static TEXT_UTF_FIELD2: Static<Field> = LazyLock::new(|| {
    Arc::new(new_field(
        TEXT_FIELD_UTF2_KEY,
        FIELD_UTF2_TEXT,
        &TEXT_TYPE_STORED_WITH_TVS,
    ))
});

/// `fields`, in Java's order.
static FIELDS: LazyLock<Vec<Arc<dyn IndexableField>>> = LazyLock::new(|| {
    vec![
        DocHelper::text_field1(),
        DocHelper::text_field2(),
        DocHelper::text_field3(),
        DocHelper::key_field(),
        DocHelper::no_norms_field(),
        DocHelper::no_tf_field(),
        DocHelper::un_ind_field(),
        DocHelper::un_stored_field1(),
        DocHelper::un_stored_field2(),
        DocHelper::text_utf_field1(),
        DocHelper::text_utf_field2(),
        DocHelper::lazy_field(),
        DocHelper::lazy_field_binary(),
        DocHelper::large_lazy_field(),
    ]
});

impl DocHelper {
    /// `customType`: `new FieldType(TextField.TYPE_STORED)`.
    pub fn custom_type() -> Arc<FieldType> {
        Arc::clone(&CUSTOM_TYPE)
    }

    /// `TEXT_TYPE_STORED_WITH_TVS`: `TextField.TYPE_STORED` with term vectors,
    /// their positions and offsets, frozen.
    pub fn text_type_stored_with_tvs() -> Arc<FieldType> {
        Arc::clone(&TEXT_TYPE_STORED_WITH_TVS)
    }

    /// `customType3`: `TextField.TYPE_STORED` omitting norms.
    pub fn custom_type3() -> Arc<FieldType> {
        Arc::clone(&CUSTOM_TYPE3)
    }

    /// `customType5`: `TextField.TYPE_STORED` omitting norms, not tokenized.
    pub fn custom_type5() -> Arc<FieldType> {
        Arc::clone(&CUSTOM_TYPE5)
    }

    /// `customType6`: `TextField.TYPE_STORED` indexing `DOCS` only.
    pub fn custom_type6() -> Arc<FieldType> {
        Arc::clone(&CUSTOM_TYPE6)
    }

    /// `customType7`: a stored, unindexed `new FieldType()`.
    pub fn custom_type7() -> Arc<FieldType> {
        Arc::clone(&CUSTOM_TYPE7)
    }

    /// `STRING_TYPE_STORED_WITH_TVS`: `StringField.TYPE_STORED` with term
    /// vectors, their positions and offsets, frozen.
    pub fn string_type_stored_with_tvs() -> Arc<FieldType> {
        Arc::clone(&STRING_TYPE_STORED_WITH_TVS)
    }

    /// `customType8`: `TextField.TYPE_NOT_STORED` with term vectors.
    pub fn custom_type8() -> Arc<FieldType> {
        Arc::clone(&CUSTOM_TYPE8)
    }

    /// `textField1`.
    pub fn text_field1() -> Arc<Field> {
        Arc::clone(&TEXT_FIELD1)
    }

    /// `textField2`.
    pub fn text_field2() -> Arc<Field> {
        Arc::clone(&TEXT_FIELD2)
    }

    /// `textField3`.
    pub fn text_field3() -> Arc<Field> {
        Arc::clone(&TEXT_FIELD3)
    }

    /// `keyField`: `new StringField(KEYWORD_FIELD_KEY, KEYWORD_TEXT,
    /// Field.Store.YES)`.
    pub fn key_field() -> Arc<StringField> {
        Arc::clone(&KEY_FIELD)
    }

    /// `noNormsField`.
    pub fn no_norms_field() -> Arc<Field> {
        Arc::clone(&NO_NORMS_FIELD)
    }

    /// `noTFField`.
    pub fn no_tf_field() -> Arc<Field> {
        Arc::clone(&NO_TF_FIELD)
    }

    /// `unIndField`.
    pub fn un_ind_field() -> Arc<Field> {
        Arc::clone(&UN_IND_FIELD)
    }

    /// `unStoredField1`: `new TextField(UNSTORED_FIELD_1_KEY,
    /// UNSTORED_1_FIELD_TEXT, Field.Store.NO)`.
    pub fn un_stored_field1() -> Arc<TextField> {
        Arc::clone(&UN_STORED_FIELD1)
    }

    /// `unStoredField2`.
    pub fn un_stored_field2() -> Arc<Field> {
        Arc::clone(&UN_STORED_FIELD2)
    }

    /// `lazyFieldBinary`: `new StoredField(LAZY_FIELD_BINARY_KEY,
    /// LAZY_FIELD_BINARY_BYTES)`.
    pub fn lazy_field_binary() -> Arc<StoredField> {
        Arc::clone(&LAZY_FIELD_BINARY)
    }

    /// `lazyField`.
    pub fn lazy_field() -> Arc<Field> {
        Arc::clone(&LAZY_FIELD)
    }

    /// `largeLazyField`.
    pub fn large_lazy_field() -> Arc<Field> {
        Arc::clone(&LARGE_LAZY_FIELD)
    }

    /// `textUtfField1`.
    pub fn text_utf_field1() -> Arc<Field> {
        Arc::clone(&TEXT_UTF_FIELD1)
    }

    /// `textUtfField2`.
    pub fn text_utf_field2() -> Arc<Field> {
        Arc::clone(&TEXT_UTF_FIELD2)
    }

    /// `fields`: "ordered list of all the fields...", the binary field second
    /// to last and the large one last; the same field objects every document
    /// [`setup_doc`](Self::setup_doc) fills holds.
    pub fn fields() -> Vec<Arc<dyn IndexableField>> {
        FIELDS.clone()
    }

    /// The `HashMap` of the fields of `fields` that `keep` accepts, keyed by
    /// name: the static initializer's `add(map, f)` calls.
    fn map_of(keep: impl Fn(&dyn IndexableField) -> bool) -> JavaHashMap<Arc<dyn IndexableField>> {
        let mut map = JavaHashMap::new();
        for f in FIELDS.iter() {
            if keep(&**f) {
                map.insert(f.name().to_string(), Arc::clone(f));
            }
        }
        map
    }

    /// `all`.
    pub fn all() -> JavaHashMap<Arc<dyn IndexableField>> {
        Self::map_of(|_| true)
    }

    /// `indexed`: the fields whose index options are not `NONE`.
    pub fn indexed() -> JavaHashMap<Arc<dyn IndexableField>> {
        Self::map_of(|f| f.field_type().index_options() != IndexOptions::NONE)
    }

    /// `unindexed`.
    pub fn unindexed() -> JavaHashMap<Arc<dyn IndexableField>> {
        Self::map_of(|f| f.field_type().index_options() == IndexOptions::NONE)
    }

    /// `termvector`.
    pub fn termvector() -> JavaHashMap<Arc<dyn IndexableField>> {
        Self::map_of(|f| f.field_type().store_term_vectors())
    }

    /// `notermvector`: indexed fields without term vectors.
    pub fn notermvector() -> JavaHashMap<Arc<dyn IndexableField>> {
        Self::map_of(|f| {
            f.field_type().index_options() != IndexOptions::NONE
                && !f.field_type().store_term_vectors()
        })
    }

    /// `stored`.
    pub fn stored() -> JavaHashMap<Arc<dyn IndexableField>> {
        Self::map_of(|f| f.field_type().stored())
    }

    /// `unstored`.
    pub fn unstored() -> JavaHashMap<Arc<dyn IndexableField>> {
        Self::map_of(|f| !f.field_type().stored())
    }

    /// `noTf`: the fields indexing `DOCS` only.
    pub fn no_tf() -> JavaHashMap<Arc<dyn IndexableField>> {
        Self::map_of(|f| f.field_type().index_options() == IndexOptions::DOCS)
    }

    /// `noNorms`.
    pub fn no_norms() -> JavaHashMap<Arc<dyn IndexableField>> {
        Self::map_of(|f| f.field_type().omit_norms())
    }

    /// `lazy`: always empty (Java's `if (f.isLazy()) add(lazy, f)` is
    /// commented out).
    pub fn lazy() -> JavaHashMap<Arc<dyn IndexableField>> {
        JavaHashMap::new()
    }

    /// `nameValues`.
    pub fn name_values() -> JavaHashMap<NameValue> {
        let text = |s: &str| NameValue::Text(s.to_string());
        let mut name_values = JavaHashMap::new();
        name_values.insert(TEXT_FIELD_1_KEY.to_string(), text(FIELD_1_TEXT));
        name_values.insert(TEXT_FIELD_2_KEY.to_string(), text(FIELD_2_TEXT));
        name_values.insert(TEXT_FIELD_3_KEY.to_string(), text(FIELD_3_TEXT));
        name_values.insert(KEYWORD_FIELD_KEY.to_string(), text(KEYWORD_TEXT));
        name_values.insert(NO_NORMS_KEY.to_string(), text(NO_NORMS_TEXT));
        name_values.insert(NO_TF_KEY.to_string(), text(NO_TF_TEXT));
        name_values.insert(UNINDEXED_FIELD_KEY.to_string(), text(UNINDEXED_FIELD_TEXT));
        name_values.insert(
            UNSTORED_FIELD_1_KEY.to_string(),
            text(UNSTORED_1_FIELD_TEXT),
        );
        name_values.insert(
            UNSTORED_FIELD_2_KEY.to_string(),
            text(UNSTORED_2_FIELD_TEXT),
        );
        name_values.insert(LAZY_FIELD_KEY.to_string(), text(LAZY_FIELD_TEXT));
        name_values.insert(
            LAZY_FIELD_BINARY_KEY.to_string(),
            NameValue::Bytes(LAZY_FIELD_BINARY_BYTES.to_vec()),
        );
        name_values.insert(
            LARGE_LAZY_FIELD_KEY.to_string(),
            text(&LARGE_LAZY_FIELD_TEXT),
        );
        name_values.insert(TEXT_FIELD_UTF1_KEY.to_string(), text(FIELD_UTF1_TEXT));
        name_values.insert(TEXT_FIELD_UTF2_KEY.to_string(), text(FIELD_UTF2_TEXT));
        name_values
    }

    /// `setupDoc(doc)`: "Adds the fields above to a document".
    pub fn setup_doc(doc: &mut Document) {
        for field in FIELDS.iter() {
            doc.add_shared(Arc::clone(field));
        }
    }

    /// `writeDoc(random, dir, doc)`: "Writes the document to the directory
    /// using a segment named "test"; returns the SegmentInfo describing the
    /// new segment", with `new MockAnalyzer(random, MockTokenizer.WHITESPACE,
    /// false)` and the default similarity.
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn write_doc(
        random: &dyn Random,
        dir: Arc<dyn Directory>,
        doc: &Document,
    ) -> Result<Option<SegmentCommitInfo>> {
        let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::with_automaton(
            random,
            Arc::clone(&WHITESPACE),
            false,
        ));
        Self::write_doc_with(dir, analyzer, None, doc)
    }

    /// `writeDoc(dir, analyzer, similarity, doc)`: "Writes the document to the
    /// directory using the analyzer and the similarity score; returns the
    /// SegmentInfo describing the new segment".
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn write_doc_with(
        dir: Arc<dyn Directory>,
        analyzer: Arc<dyn Analyzer>,
        similarity: Option<Arc<dyn Similarity>>,
        doc: &Document,
    ) -> Result<Option<SegmentCommitInfo>> {
        let config = IndexWriterConfig::with_analyzer(Some(analyzer));
        config.set_similarity(Some(
            similarity.unwrap_or_else(IndexSearcher::get_default_similarity),
        ))?;
        let writer = IndexWriter::new(dir, config)?;
        // writer.setNoCFSRatio(0.0);
        writer.add_document(doc)?;
        writer.commit()?;
        let info = index_writer_access().newest_segment(&writer)?;
        writer.close()?;
        Ok(info)
    }

    /// `numFields(doc)`.
    pub fn num_fields(doc: &Document) -> usize {
        doc.get_fields().len()
    }

    /// `createDocument(n, indexName, numFields)`.
    pub fn create_document(n: i32, index_name: &str, num_fields: i32) -> Document {
        let mut sb = String::new();

        let mut doc = Document::new();
        doc.add(Box::new(new_field(
            "id",
            &n.to_string(),
            &STRING_TYPE_STORED_WITH_TVS,
        )));
        doc.add(Box::new(new_field(
            "indexname",
            index_name,
            &STRING_TYPE_STORED_WITH_TVS,
        )));
        sb.push('a');
        sb.push_str(&n.to_string());
        doc.add(Box::new(new_field(
            "field1",
            &sb,
            &TEXT_TYPE_STORED_WITH_TVS,
        )));
        sb.push_str(" b");
        sb.push_str(&n.to_string());
        for i in 1..num_fields {
            doc.add(Box::new(new_field(
                &format!("field{}", i + 1),
                &sb,
                &TEXT_TYPE_STORED_WITH_TVS,
            )));
        }
        doc
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rucene::analysis::standard::StandardAnalyzer;
    use rucene::store::ByteBuffersDirectory;

    /// `writeDoc` commits one segment holding the document. (The
    /// `MockAnalyzer` overload needs a randomized context, as in Java.)
    #[test]
    fn write_doc_returns_the_new_segment() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let mut doc = Document::new();
        DocHelper::setup_doc(&mut doc);
        assert_eq!(DocHelper::num_fields(&doc), 14);
        let info = DocHelper::write_doc_with(dir, Arc::new(StandardAnalyzer::new()), None, &doc)
            .unwrap()
            .expect("the newest segment");
        assert_eq!(info.info.max_doc().unwrap(), 1);
    }
}
