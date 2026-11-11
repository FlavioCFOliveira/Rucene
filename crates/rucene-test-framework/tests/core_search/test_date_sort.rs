//! Port of `lucene/core/src/test/org/apache/lucene/search/TestDateSort.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{DateTools, Document, Resolution, SortedDocValuesField, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{
    close_dir, close_reader, fixture_class, ftest, set_up, tear_down, tq,
};
use crate::core_support::{assert_equals, random};

const TEXT_FIELD: &str = "text";
const DATE_TIME_FIELD: &str = "dateTime";

/// The instance of `TestDateSort`.
#[derive(Default)]
pub struct DateSort {
    directory: Option<TestDirectory>,
    reader: Option<Arc<dyn DirectoryReader>>,
}

/// `TestDateSort`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<DateSort>(
        "org.apache.lucene.search.TestDateSort",
        vec![
            set_up(date_sort_set_up),
            tear_down(|f: &mut DateSort| {
                close_reader(f.reader.take())?;
                close_dir(f.directory.take())
            }),
            ftest("testReverseDateSort", test_reverse_date_sort),
        ],
    )
}

fn date_sort_set_up(f: &mut DateSort) -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;

    // oldest doc:
    // Add the first document.  text = "Document 1"  dateTime = Oct 10 03:25:22 EDT 2007
    writer.add_document(&create_document("Document 1", 1192001122000)?)?;
    // Add the second document.  text = "Document 2"  dateTime = Oct 10 03:25:26 EDT 2007
    writer.add_document(&create_document("Document 2", 1192001126000)?)?;
    // Add the third document.  text = "Document 3"  dateTime = Oct 11 07:12:13 EDT 2007
    writer.add_document(&create_document("Document 3", 1192101133000)?)?;
    // Add the fourth document.  text = "Document 4"  dateTime = Oct 11 08:02:09 EDT 2007
    writer.add_document(&create_document("Document 4", 1192104129000)?)?;
    // latest doc:
    // Add the fifth document.  text = "Document 5"  dateTime = Oct 12 13:25:43 EDT 2007
    writer.add_document(&create_document("Document 5", 1192209943000)?)?;

    f.reader = Some(writer.get_reader()?);
    f.directory = Some(directory);
    writer.close()?;
    Ok(())
}

fn test_reverse_date_sort(f: &mut DateSort) -> Result<()> {
    let reader = f.reader.as_ref().expect("setUp ran");
    let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));

    let sort = Sort::new_fields(vec![SortField::new_reverse(
        Some(DATE_TIME_FIELD.to_string()),
        SortFieldType::String,
        true,
    )?])?;
    let query = tq(TEXT_FIELD, "document");

    // Execute the search and process the search results.
    let mut actual_order: Vec<Option<String>> = vec![None; 5];
    let hits = searcher.search_with_sort(query, 1000, &sort)?.score_docs;
    let stored_fields = searcher.stored_fields()?;
    for (i, hit) in hits.iter().enumerate() {
        let document = stored_fields.document(hit.score_doc.doc)?;
        let text = document.get(TEXT_FIELD);
        actual_order[i] = text;
    }

    // Set up the expected order (i.e. Document 5, 4, 3, 2, 1).
    let expected_order: Vec<Option<String>> = [
        "Document 5",
        "Document 4",
        "Document 3",
        "Document 2",
        "Document 1",
    ]
    .iter()
    .map(|s| Some(s.to_string()))
    .collect();

    assert_equals(&expected_order, &actual_order);
    Ok(())
}

fn create_document(text: &str, time: i64) -> Result<Document> {
    let mut document = Document::new();

    // Add the text field.
    let text_field = LuceneTestCase::new_text_field(TEXT_FIELD, text, Store::YES);
    document.add(Box::new(text_field));

    // Add the date/time field.
    let date_time_string = DateTools::time_to_string(time, Resolution::SECOND);
    let date_time_field =
        LuceneTestCase::new_string_field(DATE_TIME_FIELD, &date_time_string, Store::YES);
    document.add(Box::new(date_time_field));
    document.add(Box::new(SortedDocValuesField::new(
        DATE_TIME_FIELD,
        LuceneTestCase::new_bytes_ref_bytes(date_time_string.as_bytes()),
    )));

    Ok(document)
}
