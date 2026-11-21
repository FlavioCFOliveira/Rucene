//! The index lifecycle shared by every entity: where an index lives, and the
//! write commands `create`, `add`, `update`, `delete`, plus `get` and
//! `stats`.
//!
//! # Layout
//!
//! Every command takes a **root** directory and works only in
//! `<root>/<entity>`, the fixed subdirectory named after the command
//! ([`index_dir`]). `create` creates that subdirectory (and the root when
//! missing) and nothing else, so the ten indices never share or overwrite
//! each other's files, and no file is ever written in the root itself.
//!
//! # Writer lifecycle
//!
//! Each write command opens an `IndexWriter` on an `FSDirectory` with the
//! pt-PT [`PortugueseFoldingAnalyzer`], does its work, commits and closes the
//! writer: the lifecycle of Lucene's `IndexFiles` demo. A failure rolls the
//! writer back, so the index keeps its last commit.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::{IndexReader, Term};
use rucene::search::{IndexSearcher, Query, TermQuery};
use rucene::store::{Directory, FSDirectory};
use rucene::util::BytesRef;

use crate::analysis::PortugueseFoldingAnalyzer;
use crate::error::{CliError, Result};
use crate::schema::{document_to_json, EntitySchema, Record, KEY_FIELD};

/// The directory of the index of `schema` under `root`:
/// `<root>/<schema.name>`.
#[must_use]
pub fn index_dir(root: &Path, schema: &EntitySchema) -> PathBuf {
    root.join(schema.name)
}

/// The `Term` that identifies the document with key `key`.
#[must_use]
pub fn key_term(key: &str) -> Term {
    Term::new(KEY_FIELD, BytesRef::new(key.as_bytes().to_vec()))
}

fn writer_config() -> IndexWriterConfig {
    let analyzer: Arc<dyn Analyzer> = Arc::new(PortugueseFoldingAnalyzer::new());
    IndexWriterConfig::with_analyzer(Some(analyzer))
}

/// Creates an empty index of `schema` in `<root>/<entity>`, replacing any
/// index already there; returns its directory.
///
/// Equivalent to opening an `IndexWriter` with `OpenMode.CREATE` and
/// committing. Only the entity's own subdirectory (and the root, when
/// missing) is created.
///
/// # Errors
///
/// Returns the I/O or `rucene` error that prevented the commit.
pub fn create_index(root: &Path, schema: &EntitySchema) -> Result<PathBuf> {
    let dir = index_dir(root, schema);
    std::fs::create_dir_all(&dir)?;
    let directory: Arc<dyn Directory> = Arc::new(FSDirectory::open(&dir)?);
    let config = writer_config();
    config.set_open_mode(OpenMode::CREATE)?;
    let writer = IndexWriter::new(directory, config)?;
    finish(&writer, Ok(()))?;
    Ok(dir)
}

/// Adds every record as a new document; returns the number added.
///
/// Uses `IndexWriter.addDocument`, which ignores the key: a record whose key
/// is already indexed becomes a second document. Use [`update_documents`] to
/// replace. Records are consumed one at a time; the first `Err` fails the
/// command and nothing of it is committed.
///
/// # Errors
///
/// The first error of `records`, or as [`with_writer`].
pub fn add_documents(
    root: &Path,
    schema: &EntitySchema,
    records: impl IntoIterator<Item = Result<Record>>,
) -> Result<usize> {
    with_writer(root, schema, |writer| {
        let mut count = 0;
        for record in records {
            let record = record?;
            writer
                .add_document(&record.to_document()?)
                .map_err(|error| {
                    CliError::from(error).context(&format!("key {:?}", record.key()))
                })?;
            count += 1;
        }
        Ok(count)
    })
}

/// Replaces, by key, the document of every record; returns the number of
/// records written.
///
/// Uses `IndexWriter.updateDocument(Term("_key", key), doc)` per record,
/// which atomically deletes every document holding the key, including one
/// added earlier in the same command, and adds the new one. Repeated keys in
/// the input therefore leave one document, the last. A key not indexed yet is
/// simply added, as in Lucene.
///
/// # Errors
///
/// The first error of `records`, or as [`with_writer`].
pub fn update_documents(
    root: &Path,
    schema: &EntitySchema,
    records: impl IntoIterator<Item = Result<Record>>,
) -> Result<usize> {
    with_writer(root, schema, |writer| {
        let mut count = 0;
        for record in records {
            let record = record?;
            writer
                .update_document(key_term(record.key()), &record.to_document()?)
                .map_err(|error| {
                    CliError::from(error).context(&format!("key {:?}", record.key()))
                })?;
            count += 1;
        }
        Ok(count)
    })
}

/// Deletes every document whose key is one of `keys`, with one
/// `IndexWriter.deleteDocuments(Term...)` call; returns the number of keys
/// given. As in Lucene, a key that matches nothing is not an error.
///
/// # Errors
///
/// As [`with_writer`].
pub fn delete_documents(root: &Path, schema: &EntitySchema, keys: &[String]) -> Result<usize> {
    with_writer(root, schema, |writer| {
        writer.delete_documents_by_terms(keys.iter().map(|key| key_term(key)).collect())?;
        Ok(keys.len())
    })
}

/// Opens an `IndexWriter` in `OpenMode.APPEND` on the existing index of
/// `schema` under `root`, runs `work`, then commits and closes the writer.
///
/// # Errors
///
/// Returns [`CliError::Input`] when the index does not exist; otherwise the
/// error of `work` or of the commit, after rolling the writer back.
pub fn with_writer<T>(
    root: &Path,
    schema: &EntitySchema,
    work: impl FnOnce(&IndexWriter) -> Result<T>,
) -> Result<T> {
    let directory = open_existing(root, schema)?;
    let config = writer_config();
    config.set_open_mode(OpenMode::APPEND)?;
    let writer = IndexWriter::new(directory, config)?;
    let outcome = work(&writer);
    finish(&writer, outcome)
}

/// Opens the directory of an index that must already exist.
///
/// # Errors
///
/// Returns [`CliError::Input`] when `<root>/<entity>` holds no index.
pub fn open_existing(root: &Path, schema: &EntitySchema) -> Result<Arc<dyn Directory>> {
    let dir = index_dir(root, schema);
    let missing = || {
        CliError::Input(format!(
            "no {} index found at {}; run `rucene-base {} create {}` first",
            schema.name,
            dir.display(),
            schema.name,
            root.display()
        ))
    };
    if !dir.is_dir() {
        return Err(missing());
    }
    let directory: Arc<dyn Directory> = Arc::new(FSDirectory::open(&dir)?);
    if !directory_reader::index_exists(directory.as_ref())? {
        return Err(missing());
    }
    Ok(directory)
}

/// Opens an `IndexSearcher` over the latest commit of the index.
///
/// # Errors
///
/// As [`open_existing`], and any `rucene` error opening the reader.
pub fn open_searcher(root: &Path, schema: &EntitySchema) -> Result<IndexSearcher> {
    let directory = open_existing(root, schema)?;
    let reader = directory_reader::open(directory)?;
    Ok(IndexSearcher::new(reader as Arc<dyn IndexReader>)?)
}

/// Commits and closes `writer` when `outcome` is a success; rolls it back
/// when it is a failure.
fn finish<T>(writer: &IndexWriter, outcome: Result<T>) -> Result<T> {
    match outcome {
        Ok(value) => {
            if let Err(error) = writer.commit() {
                let _ = writer.rollback();
                return Err(error.into());
            }
            writer.close()?;
            Ok(value)
        }
        Err(error) => {
            // The work's error is the one to report; a rollback failure on
            // top of it would only hide it.
            let _ = writer.rollback();
            Err(error)
        }
    }
}

/// The documents found for each requested key by [`get_documents`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GetResult {
    /// One JSON object (bulk input format) per document found, in key order,
    /// then index order.
    pub documents: Vec<String>,
    /// The keys that matched no document.
    pub missing: Vec<String>,
}

/// Looks up the documents of `keys` with a `TermQuery` on the key field and
/// returns their stored fields as JSON objects.
///
/// # Errors
///
/// As [`open_searcher`], and any `rucene` error while searching or loading
/// stored fields.
pub fn get_documents(root: &Path, schema: &EntitySchema, keys: &[String]) -> Result<GetResult> {
    let searcher = open_searcher(root, schema)?;
    let stored = searcher.stored_fields()?;
    let mut result = GetResult {
        documents: Vec::new(),
        missing: Vec::new(),
    };
    for key in keys {
        let query: Arc<dyn Query> = Arc::new(TermQuery::new(key_term(key)));
        let count = searcher.count(Arc::clone(&query))?;
        if count == 0 {
            result.missing.push(key.clone());
            continue;
        }
        let top = searcher.search(query, count)?;
        let mut docs: Vec<i32> = top.score_docs.iter().map(|hit| hit.doc).collect();
        docs.sort_unstable();
        for doc in docs {
            result
                .documents
                .push(document_to_json(schema, &stored.document(doc)?));
        }
    }
    Ok(result)
}

/// The figures `stats` prints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexStats {
    /// The index directory.
    pub directory: PathBuf,
    /// Live documents (`IndexReader.numDocs`).
    pub documents: i32,
    /// Deleted documents not yet merged away (`maxDoc - numDocs`).
    pub deleted: i32,
    /// `IndexReader.maxDoc`.
    pub max_doc: i32,
    /// Segments in the latest commit (`IndexReader.leaves().size()`).
    pub segments: usize,
}

/// Reads the [`IndexStats`] of the index of `schema`.
///
/// # Errors
///
/// As [`open_existing`], and any `rucene` error opening the reader.
pub fn index_stats(root: &Path, schema: &EntitySchema) -> Result<IndexStats> {
    let directory = open_existing(root, schema)?;
    let reader = directory_reader::open(directory)?;
    let reader = reader as Arc<dyn IndexReader>;
    let documents = reader.num_docs();
    let max_doc = reader.max_doc();
    let segments = Arc::clone(&reader).leaves()?.len();
    Ok(IndexStats {
        directory: index_dir(root, schema),
        documents,
        deleted: max_doc - documents,
        max_doc,
        segments,
    })
}
