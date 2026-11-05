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
//! **Declared divergence.** Java's fields (`textField1`, `fields`, the maps
//! `all`, `indexed`, …) are `static` instances shared by every document
//! `setupDoc` fills. A Rust [`Document`] owns its fields, so each accessor here
//! builds new instances with the same name, value and type; a test that
//! mutates a Java static field and expects another document to see it has no
//! counterpart.

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

fn frozen(mut ft: FieldType) -> FieldType {
    ft.freeze();
    ft
}

/// `Field.setXxx` on a type that is not frozen cannot fail.
fn set(r: Result<()>) {
    r.expect("INVARIANT: the field type is not frozen yet");
}

impl DocHelper {
    /// `customType`: `new FieldType(TextField.TYPE_STORED)`.
    pub fn custom_type() -> FieldType {
        FieldType::new_from(TextField::type_stored())
    }

    /// `TEXT_TYPE_STORED_WITH_TVS`: `TextField.TYPE_STORED` with term vectors,
    /// their positions and offsets, frozen.
    pub fn text_type_stored_with_tvs() -> FieldType {
        let mut ft = FieldType::new_from(TextField::type_stored());
        set(ft.set_store_term_vectors(true));
        set(ft.set_store_term_vector_positions(true));
        set(ft.set_store_term_vector_offsets(true));
        frozen(ft)
    }

    /// `customType3`: `TextField.TYPE_STORED` omitting norms.
    pub fn custom_type3() -> FieldType {
        let mut ft = FieldType::new_from(TextField::type_stored());
        set(ft.set_omit_norms(true));
        ft
    }

    /// `customType5`: `TextField.TYPE_STORED` omitting norms, not tokenized.
    pub fn custom_type5() -> FieldType {
        let mut ft = FieldType::new_from(TextField::type_stored());
        set(ft.set_omit_norms(true));
        set(ft.set_tokenized(false));
        ft
    }

    /// `customType6`: `TextField.TYPE_STORED` indexing `DOCS` only.
    pub fn custom_type6() -> FieldType {
        let mut ft = FieldType::new_from(TextField::type_stored());
        set(ft.set_index_options(IndexOptions::DOCS));
        ft
    }

    /// `customType7`: a stored, unindexed `new FieldType()`.
    pub fn custom_type7() -> FieldType {
        let mut ft = FieldType::new();
        set(ft.set_stored(true));
        ft
    }

    /// `STRING_TYPE_STORED_WITH_TVS`: `StringField.TYPE_STORED` with term
    /// vectors, their positions and offsets, frozen.
    pub fn string_type_stored_with_tvs() -> FieldType {
        let string_field = StringField::new("", String::new(), Store::YES)
            .expect("INVARIANT: a StringField of an empty value is valid");
        let mut ft = FieldType::new_from(string_field.field_type());
        set(ft.set_store_term_vectors(true));
        set(ft.set_store_term_vector_positions(true));
        set(ft.set_store_term_vector_offsets(true));
        frozen(ft)
    }

    /// `customType8`: `TextField.TYPE_NOT_STORED` with term vectors.
    pub fn custom_type8() -> FieldType {
        let mut ft = FieldType::new_from(TextField::type_not_stored());
        set(ft.set_store_term_vectors(true));
        ft
    }

    fn field(name: &str, value: &str, ft: FieldType) -> Box<dyn IndexableField> {
        Box::new(
            Field::new(name, value.to_string(), ft)
                .expect("INVARIANT: DocHelper's fields are valid"),
        )
    }

    /// `textField1`.
    pub fn text_field1() -> Box<dyn IndexableField> {
        Self::field(TEXT_FIELD_1_KEY, FIELD_1_TEXT, Self::custom_type())
    }

    /// `textField2`.
    pub fn text_field2() -> Box<dyn IndexableField> {
        Self::field(
            TEXT_FIELD_2_KEY,
            FIELD_2_TEXT,
            Self::text_type_stored_with_tvs(),
        )
    }

    /// `textField3`.
    pub fn text_field3() -> Box<dyn IndexableField> {
        Self::field(TEXT_FIELD_3_KEY, FIELD_3_TEXT, Self::custom_type3())
    }

    /// `keyField`: `new StringField(KEYWORD_FIELD_KEY, KEYWORD_TEXT,
    /// Field.Store.YES)`.
    pub fn key_field() -> Box<dyn IndexableField> {
        Box::new(
            StringField::new(KEYWORD_FIELD_KEY, KEYWORD_TEXT.to_string(), Store::YES)
                .expect("INVARIANT: DocHelper's fields are valid"),
        )
    }

    /// `noNormsField`.
    pub fn no_norms_field() -> Box<dyn IndexableField> {
        Self::field(NO_NORMS_KEY, NO_NORMS_TEXT, Self::custom_type5())
    }

    /// `noTFField`.
    pub fn no_tf_field() -> Box<dyn IndexableField> {
        Self::field(NO_TF_KEY, NO_TF_TEXT, Self::custom_type6())
    }

    /// `unIndField`.
    pub fn un_ind_field() -> Box<dyn IndexableField> {
        Self::field(
            UNINDEXED_FIELD_KEY,
            UNINDEXED_FIELD_TEXT,
            Self::custom_type7(),
        )
    }

    /// `unStoredField1`: `new TextField(UNSTORED_FIELD_1_KEY,
    /// UNSTORED_1_FIELD_TEXT, Field.Store.NO)`.
    pub fn un_stored_field1() -> Box<dyn IndexableField> {
        Box::new(
            TextField::new(
                UNSTORED_FIELD_1_KEY,
                UNSTORED_1_FIELD_TEXT.to_string(),
                Store::NO,
            )
            .expect("INVARIANT: DocHelper's fields are valid"),
        )
    }

    /// `unStoredField2`.
    pub fn un_stored_field2() -> Box<dyn IndexableField> {
        Self::field(
            UNSTORED_FIELD_2_KEY,
            UNSTORED_2_FIELD_TEXT,
            Self::custom_type8(),
        )
    }

    /// `lazyFieldBinary`: `new StoredField(LAZY_FIELD_BINARY_KEY,
    /// LAZY_FIELD_BINARY_BYTES)`.
    pub fn lazy_field_binary() -> Box<dyn IndexableField> {
        Box::new(
            StoredField::new_bytes(
                LAZY_FIELD_BINARY_KEY,
                BytesRef::new(LAZY_FIELD_BINARY_BYTES.to_vec()),
            )
            .expect("INVARIANT: DocHelper's fields are valid"),
        )
    }

    /// `lazyField`.
    pub fn lazy_field() -> Box<dyn IndexableField> {
        Self::field(LAZY_FIELD_KEY, LAZY_FIELD_TEXT, Self::custom_type())
    }

    /// `largeLazyField`.
    pub fn large_lazy_field() -> Box<dyn IndexableField> {
        Self::field(
            LARGE_LAZY_FIELD_KEY,
            &LARGE_LAZY_FIELD_TEXT,
            Self::custom_type(),
        )
    }

    /// `textUtfField1`.
    pub fn text_utf_field1() -> Box<dyn IndexableField> {
        Self::field(TEXT_FIELD_UTF1_KEY, FIELD_UTF1_TEXT, Self::custom_type())
    }

    /// `textUtfField2`.
    pub fn text_utf_field2() -> Box<dyn IndexableField> {
        Self::field(
            TEXT_FIELD_UTF2_KEY,
            FIELD_UTF2_TEXT,
            Self::text_type_stored_with_tvs(),
        )
    }

    /// `fields`: "ordered list of all the fields...", the binary field second
    /// to last and the large one last.
    pub fn fields() -> Vec<Box<dyn IndexableField>> {
        vec![
            Self::text_field1(),
            Self::text_field2(),
            Self::text_field3(),
            Self::key_field(),
            Self::no_norms_field(),
            Self::no_tf_field(),
            Self::un_ind_field(),
            Self::un_stored_field1(),
            Self::un_stored_field2(),
            Self::text_utf_field1(),
            Self::text_utf_field2(),
            Self::lazy_field(),
            Self::lazy_field_binary(),
            Self::large_lazy_field(),
        ]
    }

    /// The `HashMap` of the fields of `fields` that `keep` accepts, keyed by
    /// name: the static initializer's `add(map, f)` calls.
    fn map_of(keep: impl Fn(&dyn IndexableField) -> bool) -> JavaHashMap<Box<dyn IndexableField>> {
        let mut map = JavaHashMap::new();
        for f in Self::fields() {
            if keep(&*f) {
                map.insert(f.name().to_string(), f);
            }
        }
        map
    }

    /// `all`.
    pub fn all() -> JavaHashMap<Box<dyn IndexableField>> {
        Self::map_of(|_| true)
    }

    /// `indexed`: the fields whose index options are not `NONE`.
    pub fn indexed() -> JavaHashMap<Box<dyn IndexableField>> {
        Self::map_of(|f| f.field_type().index_options() != IndexOptions::NONE)
    }

    /// `unindexed`.
    pub fn unindexed() -> JavaHashMap<Box<dyn IndexableField>> {
        Self::map_of(|f| f.field_type().index_options() == IndexOptions::NONE)
    }

    /// `termvector`.
    pub fn termvector() -> JavaHashMap<Box<dyn IndexableField>> {
        Self::map_of(|f| f.field_type().store_term_vectors())
    }

    /// `notermvector`: indexed fields without term vectors.
    pub fn notermvector() -> JavaHashMap<Box<dyn IndexableField>> {
        Self::map_of(|f| {
            f.field_type().index_options() != IndexOptions::NONE
                && !f.field_type().store_term_vectors()
        })
    }

    /// `stored`.
    pub fn stored() -> JavaHashMap<Box<dyn IndexableField>> {
        Self::map_of(|f| f.field_type().stored())
    }

    /// `unstored`.
    pub fn unstored() -> JavaHashMap<Box<dyn IndexableField>> {
        Self::map_of(|f| !f.field_type().stored())
    }

    /// `noTf`: the fields indexing `DOCS` only.
    pub fn no_tf() -> JavaHashMap<Box<dyn IndexableField>> {
        Self::map_of(|f| f.field_type().index_options() == IndexOptions::DOCS)
    }

    /// `noNorms`.
    pub fn no_norms() -> JavaHashMap<Box<dyn IndexableField>> {
        Self::map_of(|f| f.field_type().omit_norms())
    }

    /// `lazy`: always empty (Java's `if (f.isLazy()) add(lazy, f)` is
    /// commented out).
    pub fn lazy() -> JavaHashMap<Box<dyn IndexableField>> {
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
        for field in Self::fields() {
            doc.add(field);
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
        let mut config = IndexWriterConfig::with_analyzer(analyzer);
        config.set_similarity(similarity.unwrap_or_else(IndexSearcher::get_default_similarity))?;
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
        doc.add(Self::field(
            "id",
            &n.to_string(),
            Self::string_type_stored_with_tvs(),
        ));
        doc.add(Self::field(
            "indexname",
            index_name,
            Self::string_type_stored_with_tvs(),
        ));
        sb.push('a');
        sb.push_str(&n.to_string());
        doc.add(Self::field(
            "field1",
            &sb,
            Self::text_type_stored_with_tvs(),
        ));
        sb.push_str(" b");
        sb.push_str(&n.to_string());
        for i in 1..num_fields {
            doc.add(Self::field(
                &format!("field{}", i + 1),
                &sb,
                Self::text_type_stored_with_tvs(),
            ));
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
