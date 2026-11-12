//! The write commands: create an index, add, update and delete documents.
//!
//! Each operation opens an `IndexWriter` on an `FSDirectory`, does its work,
//! commits and closes the writer, the lifecycle of Lucene's `IndexFiles`
//! demo. A failure rolls the writer back, so the index keeps its last commit.

use std::path::Path;
use std::sync::Arc;

use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::Term;
use rucene::store::{Directory, FSDirectory};
use rucene::util::BytesRef;

use crate::error::{CliError, Result};
use crate::schema::{Record, ID_FIELD};

/// Opens the index directory at `path` as an `FSDirectory`.
///
/// # Errors
///
/// Propagates the error `FSDirectory::open` raises.
pub fn open_directory(path: &Path) -> Result<Arc<dyn Directory>> {
    Ok(Arc::new(FSDirectory::open(path)?))
}

/// The `Term` that identifies the document with key `id`.
#[must_use]
pub fn id_term(id: &str) -> Term {
    Term::new(ID_FIELD, BytesRef::new(id.as_bytes().to_vec()))
}

/// Creates an empty index at `path`, replacing any index already there.
///
/// Equivalent to opening an `IndexWriter` with `OpenMode.CREATE` and
/// committing; the directory is created when missing.
///
/// # Errors
///
/// Returns the I/O or `rucene` error that prevented the commit.
pub fn create_index(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    let directory = open_directory(path)?;
    let config = IndexWriterConfig::new();
    config.set_open_mode(OpenMode::CREATE)?;
    let writer = IndexWriter::new(directory, config)?;
    finish(&writer, Ok(()))
}

/// Adds every record as a new document; returns the number added.
///
/// Uses `IndexWriter.addDocument`, which does not look at `id`: adding a
/// record whose `id` is already indexed creates a second document with that
/// key. Use [`update_documents`] to replace.
///
/// # Errors
///
/// As [`with_writer`].
pub fn add_documents(path: &Path, records: &[Record]) -> Result<usize> {
    with_writer(path, |writer| {
        for record in records {
            writer
                .add_document(&record.to_document()?)
                .map_err(|error| CliError::from(error).context(&format!("id {:?}", record.id())))?;
        }
        Ok(records.len())
    })
}

/// Replaces, by `id`, the document of every record; returns the number of
/// records written.
///
/// Uses `IndexWriter.updateDocument(Term("id", id), doc)` once per record,
/// which atomically deletes every document holding the term and adds the new
/// one. As in Lucene, a record whose `id` is not indexed yet is simply
/// added.
///
/// # Errors
///
/// As [`with_writer`].
pub fn update_documents(path: &Path, records: &[Record]) -> Result<usize> {
    with_writer(path, |writer| {
        for record in records {
            writer
                .update_document(id_term(record.id()), &record.to_document()?)
                .map_err(|error| CliError::from(error).context(&format!("id {:?}", record.id())))?;
        }
        Ok(records.len())
    })
}

/// Deletes every document whose `id` is one of `ids`, with one
/// `IndexWriter.deleteDocuments(Term...)` call; returns the number of ids
/// given.
///
/// As in Lucene, an id that matches no document is not an error.
///
/// # Errors
///
/// As [`with_writer`].
pub fn delete_documents(path: &Path, ids: &[String]) -> Result<usize> {
    with_writer(path, |writer| {
        writer.delete_documents_by_terms(ids.iter().map(|id| id_term(id)).collect())?;
        Ok(ids.len())
    })
}

/// Opens an `IndexWriter` in `OpenMode.APPEND` on the existing index at
/// `path`, runs `work`, then commits and closes the writer.
///
/// # Errors
///
/// Returns [`CliError::Input`] when no index exists at `path`; otherwise the
/// error of `work` or of the commit, after rolling the writer back.
pub fn with_writer<T>(path: &Path, work: impl FnOnce(&IndexWriter) -> Result<T>) -> Result<T> {
    let directory = open_existing(path)?;
    let config = IndexWriterConfig::new();
    config.set_open_mode(OpenMode::APPEND)?;
    let writer = IndexWriter::new(directory, config)?;
    let outcome = work(&writer);
    finish(&writer, outcome)
}

/// Opens the directory of an index that must already exist.
///
/// # Errors
///
/// Returns [`CliError::Input`] when `path` holds no index.
pub fn open_existing(path: &Path) -> Result<Arc<dyn Directory>> {
    let missing = || {
        CliError::Input(format!(
            "no index found at {}; run `rucene-cli create {}` first",
            path.display(),
            path.display()
        ))
    };
    if !path.is_dir() {
        return Err(missing());
    }
    let directory = open_directory(path)?;
    if !directory_reader::index_exists(directory.as_ref())? {
        return Err(missing());
    }
    Ok(directory)
}

/// Commits and closes `writer` when `outcome` is a success; rolls it back
/// (discarding the uncommitted changes) when it is a failure.
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
