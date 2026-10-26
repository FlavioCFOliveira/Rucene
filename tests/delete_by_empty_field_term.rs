//! Delete-by-term on a field whose name is empty.
//!
//! Lucene accepts an empty field name (`TestIndexWriter.java:640-680` indexes
//! and searches one), and `PrefixCodedTerms.TermIterator.field()` returns
//! `null` only once the iteration is exhausted (`PrefixCodedTerms.java:126-140`).
//! The port encoded exhaustion as an empty field name, so a frozen delete
//! packet holding a term of the empty field looked exhausted and applying it
//! after a commit panicked.

use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{Document, Store, StringField};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::Term;
use rucene::store::{Directory, RamDirectory};
use rucene::util::BytesRef;

#[test]
fn a_term_of_the_empty_field_deletes_its_document_after_a_commit() {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    for value in ["a", "b"] {
        let mut doc = Document::new();
        doc.add(Box::new(
            StringField::new("", value.to_string(), Store::NO).unwrap(),
        ));
        writer.add_document(&doc).unwrap();
    }
    writer.commit().unwrap();
    writer
        .delete_documents_by_terms(vec![Term::new("", BytesRef::new(b"a".to_vec()))])
        .unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();

    // The merge policy may rewrite the half-deleted segment on close, so only
    // the live documents are counted.
    let reader = directory_reader::open(dir).unwrap();
    assert_eq!(
        reader.num_docs(),
        1,
        "the document of the empty field's term is deleted"
    );
}
