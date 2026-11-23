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

//! The types declared in `PerfRunData.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask`.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use rucene::analysis::Analyzer;
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::index_reader::IndexReader;
use rucene::index::index_writer::IndexWriter;
use rucene::search::index_searcher::IndexSearcher;
use rucene::store::{ByteBuffersDirectory, Directory, FSDirectory};
use rucene::util::IOUtils;
use rucene_analysis_common::jdk::locale::Locale;
use rucene_facet::taxonomy::taxonomy_reader::TaxonomyReader;
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;

use crate::by_task::feeds::content_source::ContentSource;
use crate::by_task::feeds::doc_maker::DocMakerClass;
use crate::by_task::feeds::facet_source::FacetSource;
use crate::by_task::feeds::query_maker::QueryMaker;
use crate::by_task::stats::points::Points;
use crate::by_task::tasks::new_analyzer_task::NewAnalyzerTask;
use crate::by_task::tasks::read_task::ReadTask;
use crate::by_task::tasks::search_task::SearchTask;
use crate::by_task::utils::analyzer_factory::AnalyzerFactory;
use crate::by_task::utils::config::Config;
use crate::classes;
use crate::jdk;

/// `DEFAULT_DIRECTORY`.
const DEFAULT_DIRECTORY: &str = "ByteBuffersDirectory";

/// An object a task stores with [`PerfRunData::set_perf_object`]; one that
/// implements `Closeable` is closed by [`PerfRunData::close`].
pub trait PerfObject: Send + Sync + Any {
    /// `obj instanceof Closeable`.
    fn is_closeable(&self) -> bool {
        false
    }

    /// `Closeable.close()`.
    ///
    /// # Errors
    ///
    /// The `IOException` of closing.
    fn close(&self) -> Result<()> {
        Ok(())
    }

    /// The object as `Any`.
    fn as_any(&self) -> &dyn Any;
}

/// The mutable fields of `PerfRunData`.
#[derive(Default)]
struct State {
    directory: Option<Arc<dyn Directory>>,
    analyzer: Option<Arc<dyn Analyzer>>,
    locale: Option<Locale>,
    taxonomy_dir: Option<Arc<dyn Directory>>,
    taxonomy_writer: Option<Arc<dyn TaxonomyWriter>>,
    taxonomy_reader: Option<Arc<dyn TaxonomyReader>>,
    /// we use separate (identical) instances for each "read" task type, so
    /// each can iterate the quries separately.
    read_task_query_maker: HashMap<TypeId, Arc<dyn QueryMaker>>,
    index_reader: Option<Arc<dyn DirectoryReader>>,
    index_searcher: Option<Arc<IndexSearcher>>,
    index_writer: Option<Arc<IndexWriter>>,
    start_time_millis: i64,
    perf_objects: HashMap<String, Arc<dyn PerfObject>>,
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.PerfRunData`
/// (`public class PerfRunData implements Closeable`): data maintained by a
/// performance test run.
///
/// Data includes:
///
/// * Configuration.
/// * Directory, Writer, Reader.
/// * Taxonomy Directory, Writer, Reader.
/// * DocMaker, FacetSource and a few instances of QueryMaker.
/// * Named AnalysisFactories.
/// * Analyzer.
/// * Statistics data which updated during the run.
///
/// Config properties:
///
/// * **work.dir**=<path to root of docs and index dirs| Default: work>
/// * **analyzer**=<class name for analyzer| Default: StandardAnalyzer>
/// * **doc.maker**=<class name for doc-maker| Default: DocMaker>
/// * **facet.source**=<class name for facet-source| Default:
///   RandomFacetSource>
/// * **query.maker**=<class name for query-maker| Default:
///   SimpleQueryMaker>
/// * **log.queries**=<whether queries should be printed| Default: false>
/// * **directory**=<type of directory to use for the index| Default:
///   ByteBuffersDirectory>
/// * **taxonomy.directory**=<type of directory for taxonomy index| Default:
///   ByteBuffersDirectory>
///
/// The run data is shared by every task and thread (`Arc<PerfRunData>`);
/// Java's `synchronized` members lock its state.
pub struct PerfRunData {
    points: Points,
    analyzer_factories: Mutex<HashMap<String, Arc<AnalyzerFactory>>>,
    doc_maker: Arc<dyn DocMakerClass>,
    content_source: Arc<dyn ContentSource>,
    facet_source: Arc<dyn FacetSource>,
    qmkr_class: String,
    config: Arc<Config>,
    state: Mutex<State>,
}

impl PerfRunData {
    /// `PerfRunData(Config config)`.
    ///
    /// # Errors
    ///
    /// The exception of creating the analyzer, the sources, the doc maker
    /// or the directories.
    pub fn new(config: Arc<Config>) -> Result<Arc<PerfRunData>> {
        // analyzer (default is standard analyzer)
        let analyzer = NewAnalyzerTask::create_analyzer(
            &config
                .get_str(
                    "analyzer",
                    Some("org.apache.lucene.analysis.standard.StandardAnalyzer"),
                )?
                .unwrap_or_default(),
        )?;
        // content source
        let source_class = config
            .get_str(
                "content.source",
                Some("org.apache.lucene.benchmark.byTask.feeds.SingleDocSource"),
            )?
            .unwrap_or_default();
        let content_source = classes::content_source_for_name(&source_class)?;
        content_source.set_config(Arc::clone(&config))?;
        // doc maker
        let doc_maker = classes::doc_maker_for_name(
            &config
                .get_str(
                    "doc.maker",
                    Some("org.apache.lucene.benchmark.byTask.feeds.DocMaker"),
                )?
                .unwrap_or_default(),
        )?;
        doc_maker.set_config(Arc::clone(&config), Arc::clone(&content_source))?;
        // facet source
        let facet_source = classes::facet_source_for_name(
            &config
                .get_str(
                    "facet.source",
                    Some("org.apache.lucene.benchmark.byTask.feeds.RandomFacetSource"),
                )?
                .unwrap_or_default(),
        )?;
        facet_source.set_config(Arc::clone(&config))?;
        // query makers
        let qmkr_class = config
            .get_str(
                "query.maker",
                Some("org.apache.lucene.benchmark.byTask.feeds.SimpleQueryMaker"),
            )?
            .unwrap_or_default();
        classes::check_query_maker_class(&qmkr_class)?;
        let run_data = Arc::new(PerfRunData {
            points: Points::new(&config),
            analyzer_factories: Mutex::new(HashMap::new()),
            doc_maker,
            content_source,
            facet_source,
            qmkr_class,
            config: Arc::clone(&config),
            state: Mutex::new(State {
                analyzer: Some(analyzer),
                ..State::default()
            }),
        });
        // index stuff
        run_data.reinit(false)?;

        if jdk::parse_boolean(config.get_str("log.queries", Some("false"))?.as_deref()) {
            println!("------------> queries:");
            let search_task = SearchTask::new(Arc::clone(&run_data))?;
            println!(
                "{}",
                run_data.get_query_maker(&search_task)?.print_queries()
            );
        }
        Ok(run_data)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        jdk::lock(&self.state)
    }

    /// `close()`.
    ///
    /// # Errors
    ///
    /// The first exception of closing the resources.
    pub fn close(&self) -> Result<()> {
        let (writer, reader, directory, taxo_writer, taxo_reader, taxo_dir, perf_objects) = {
            let s = self.lock();
            (
                s.index_writer.clone(),
                s.index_reader.clone(),
                s.directory.clone(),
                s.taxonomy_writer.clone(),
                s.taxonomy_reader.clone(),
                s.taxonomy_dir.clone(),
                s.perf_objects.values().cloned().collect::<Vec<_>>(),
            )
        };
        let doc_maker = Arc::clone(&self.doc_maker);
        let facet_source = Arc::clone(&self.facet_source);
        let content_source = Arc::clone(&self.content_source);
        IOUtils::close(vec![
            writer.map(|w| Box::new(move || w.close()) as Box<dyn FnOnce() -> Result<()>>),
            reader.map(|r| Box::new(move || r.close()) as Box<dyn FnOnce() -> Result<()>>),
            directory.map(|d| Box::new(move || d.close()) as Box<dyn FnOnce() -> Result<()>>),
            taxo_writer.map(|w| Box::new(move || w.close()) as Box<dyn FnOnce() -> Result<()>>),
            taxo_reader.map(|r| Box::new(move || r.close()) as Box<dyn FnOnce() -> Result<()>>),
            taxo_dir.map(|d| Box::new(move || d.close()) as Box<dyn FnOnce() -> Result<()>>),
            Some(Box::new(move || doc_maker.close()) as Box<dyn FnOnce() -> Result<()>>),
            Some(Box::new(move || facet_source.close()) as Box<dyn FnOnce() -> Result<()>>),
            Some(Box::new(move || content_source.close()) as Box<dyn FnOnce() -> Result<()>>),
        ])?;
        // close all perf objects that are closeable.
        IOUtils::close(
            perf_objects
                .into_iter()
                .filter(|o| o.is_closeable())
                .map(|o| Some(move || o.close())),
        )
    }

    /// `reinit(boolean eraseIndex)`: clean old stuff, reopen.
    ///
    /// # Errors
    ///
    /// The exception of closing the old resources or creating the
    /// directories.
    pub fn reinit(&self, erase_index: bool) -> Result<()> {
        // cleanup index
        let (writer, reader, directory) = {
            let mut s = self.lock();
            (
                s.index_writer.take(),
                s.index_reader.take(),
                s.directory.take(),
            )
        };
        IOUtils::close(vec![
            writer.map(|w| Box::new(move || w.close()) as Box<dyn FnOnce() -> Result<()>>),
            reader.map(|r| Box::new(move || r.close()) as Box<dyn FnOnce() -> Result<()>>),
            directory.map(|d| Box::new(move || d.close()) as Box<dyn FnOnce() -> Result<()>>),
        ])?;

        let (taxo_writer, taxo_reader, taxo_dir) = {
            let mut s = self.lock();
            (
                s.taxonomy_writer.take(),
                s.taxonomy_reader.take(),
                s.taxonomy_dir.take(),
            )
        };
        IOUtils::close(vec![
            taxo_writer.map(|w| Box::new(move || w.close()) as Box<dyn FnOnce() -> Result<()>>),
            taxo_reader.map(|r| Box::new(move || r.close()) as Box<dyn FnOnce() -> Result<()>>),
            taxo_dir.map(|d| Box::new(move || d.close()) as Box<dyn FnOnce() -> Result<()>>),
        ])?;

        // directory (default is ram-dir).
        let directory = self.create_directory(erase_index, "index", "directory")?;
        let taxonomy_dir = self.create_directory(erase_index, "taxo", "taxonomy.directory")?;
        {
            let mut s = self.lock();
            s.directory = Some(directory);
            s.taxonomy_dir = Some(taxonomy_dir);
        }

        // inputs
        self.reset_inputs()?;

        // release unused stuff: `System.runFinalization(); System.gc();`
        // have no counterpart in Rust, which frees memory as it goes.

        // Re-init clock
        self.set_start_time_millis();
        Ok(())
    }

    /// `createDirectory(boolean eraseIndex, String dirName, String
    /// dirParam)`.
    fn create_directory(
        &self,
        erase_index: bool,
        dir_name: &str,
        dir_param: &str,
    ) -> Result<Arc<dyn Directory>> {
        let dir_impl = self
            .config
            .get_str(dir_param, Some(DEFAULT_DIRECTORY))?
            .unwrap_or_default();
        if dir_impl == "FSDirectory" {
            let work_dir = PathBuf::from(
                self.config
                    .get_str("work.dir", Some("work"))?
                    .unwrap_or_default(),
            );
            let index_dir = work_dir.join(dir_name);
            if erase_index && index_dir.exists() {
                IOUtils::rm([&index_dir])?;
            }
            std::fs::create_dir_all(&index_dir)?;
            return Ok(Arc::new(FSDirectory::open(index_dir)?));
        }
        if dir_impl == "RAMDirectory" {
            return Err(rucene::error::LuceneError::from(std::io::Error::other(
                "RAMDirectory has been removed, use ByteBuffersDirectory.",
            )));
        }
        if dir_impl == "ByteBuffersDirectory" {
            return Ok(Arc::new(ByteBuffersDirectory::new()));
        }
        Err(rucene::error::LuceneError::from(std::io::Error::other(
            format!("Directory type not supported: {dir_impl}"),
        )))
    }

    /// `getPerfObject(String key)`: an object that was previously set by
    /// [`set_perf_object`](Self::set_perf_object).
    pub fn get_perf_object(&self, key: &str) -> Option<Arc<dyn PerfObject>> {
        self.lock().perf_objects.get(key).cloned()
    }

    /// `setPerfObject(String key, Object obj)`: sets an object that is
    /// required by tasks, keyed by the given `key`. If the object implements
    /// `Closeable`, it will be closed by [`close`](Self::close).
    pub fn set_perf_object(&self, key: &str, obj: Arc<dyn PerfObject>) {
        self.lock().perf_objects.insert(key.to_string(), obj);
    }

    /// `setStartTimeMillis()`.
    pub fn set_start_time_millis(&self) -> i64 {
        let now = jdk::current_time_millis();
        self.lock().start_time_millis = now;
        now
    }

    /// `getStartTimeMillis()`: start time in milliseconds.
    pub fn get_start_time_millis(&self) -> i64 {
        self.lock().start_time_millis
    }

    /// `getPoints()`.
    pub fn get_points(&self) -> &Points {
        &self.points
    }

    /// `getDirectory()`.
    pub fn get_directory(&self) -> Option<Arc<dyn Directory>> {
        self.lock().directory.clone()
    }

    /// `setDirectory(Directory)`.
    pub fn set_directory(&self, directory: Option<Arc<dyn Directory>>) {
        self.lock().directory = directory;
    }

    /// `getTaxonomyDir()`.
    pub fn get_taxonomy_dir(&self) -> Option<Arc<dyn Directory>> {
        self.lock().taxonomy_dir.clone()
    }

    /// `setTaxonomyReader(TaxonomyReader taxoReader)`: set the taxonomy
    /// reader. Takes ownership of that taxonomy reader, that is, internally
    /// performs `taxoReader.incRef()` (If caller no longer needs that reader
    /// it should `decRef()`/`close()` it after calling this method,
    /// otherwise, the reader will remain open).
    ///
    /// # Errors
    ///
    /// The exception of `decRef`/`incRef`.
    pub fn set_taxonomy_reader(&self, taxo_reader: Option<Arc<dyn TaxonomyReader>>) -> Result<()> {
        let mut s = self.lock();
        let same = match (&taxo_reader, &s.taxonomy_reader) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if same {
            return Ok(());
        }
        if let Some(old) = &s.taxonomy_reader {
            old.dec_ref()?;
        }
        if let Some(new) = &taxo_reader {
            new.inc_ref()?;
        }
        s.taxonomy_reader = taxo_reader;
        Ok(())
    }

    /// `getTaxonomyReader()`: the taxonomy reader. NOTE: this returns a
    /// reference. You must call `TaxonomyReader.decRef()` when you're done.
    ///
    /// # Errors
    ///
    /// The `AlreadyClosedException` of `incRef`.
    pub fn get_taxonomy_reader(&self) -> Result<Option<Arc<dyn TaxonomyReader>>> {
        let s = self.lock();
        if let Some(r) = &s.taxonomy_reader {
            r.inc_ref()?;
        }
        Ok(s.taxonomy_reader.clone())
    }

    /// `setTaxonomyWriter(TaxonomyWriter)`.
    pub fn set_taxonomy_writer(&self, taxo_writer: Option<Arc<dyn TaxonomyWriter>>) {
        self.lock().taxonomy_writer = taxo_writer;
    }

    /// `getTaxonomyWriter()`.
    pub fn get_taxonomy_writer(&self) -> Option<Arc<dyn TaxonomyWriter>> {
        self.lock().taxonomy_writer.clone()
    }

    /// `getIndexReader()`: the index reader. NOTE: this returns a
    /// reference. You must call `IndexReader.decRef()` when you're done.
    ///
    /// # Errors
    ///
    /// The `AlreadyClosedException` of `incRef`.
    pub fn get_index_reader(&self) -> Result<Option<Arc<dyn DirectoryReader>>> {
        let s = self.lock();
        if let Some(r) = &s.index_reader {
            r.inc_ref()?;
        }
        Ok(s.index_reader.clone())
    }

    /// `getIndexSearcher()`: the index searcher. NOTE: this returns a
    /// reference to the underlying `IndexReader`. You must call
    /// `IndexReader.decRef()` when you're done.
    ///
    /// # Errors
    ///
    /// The `AlreadyClosedException` of `incRef`.
    pub fn get_index_searcher(&self) -> Result<Option<Arc<IndexSearcher>>> {
        let s = self.lock();
        if let Some(r) = &s.index_reader {
            r.inc_ref()?;
        }
        Ok(s.index_searcher.clone())
    }

    /// `setIndexReader(DirectoryReader indexReader)`: set the index reader.
    /// Takes ownership of that index reader, that is, internally performs
    /// `indexReader.incRef()` (If caller no longer needs that reader it
    /// should `decRef()`/`close()` it after calling this method, otherwise,
    /// the reader will remain open).
    ///
    /// # Errors
    ///
    /// The exception of `decRef`/`incRef` or of building the searcher.
    pub fn set_index_reader(&self, index_reader: Option<Arc<dyn DirectoryReader>>) -> Result<()> {
        let mut s = self.lock();
        let same = match (&index_reader, &s.index_reader) {
            (Some(a), Some(b)) => std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b)),
            (None, None) => true,
            _ => false,
        };
        if same {
            return Ok(());
        }
        if let Some(old) = &s.index_reader {
            // Release current IR
            old.dec_ref()?;
        }
        s.index_reader = index_reader.clone();
        match index_reader {
            Some(reader) => {
                // Hold reference to new IR
                reader.inc_ref()?;
                let mut searcher = IndexSearcher::new(reader as Arc<dyn IndexReader>)?;
                // TODO Some day we should make the query cache in this module configurable and control
                // clearing the cache
                searcher.set_query_cache(None);
                s.index_searcher = Some(Arc::new(searcher));
            }
            None => s.index_searcher = None,
        }
        Ok(())
    }

    /// `getIndexWriter()`.
    pub fn get_index_writer(&self) -> Option<Arc<IndexWriter>> {
        self.lock().index_writer.clone()
    }

    /// `setIndexWriter(IndexWriter)`.
    pub fn set_index_writer(&self, index_writer: Option<Arc<IndexWriter>>) {
        self.lock().index_writer = index_writer;
    }

    /// `getAnalyzer()`.
    pub fn get_analyzer(&self) -> Option<Arc<dyn Analyzer>> {
        self.lock().analyzer.clone()
    }

    /// `setAnalyzer(Analyzer)`.
    pub fn set_analyzer(&self, analyzer: Option<Arc<dyn Analyzer>>) {
        self.lock().analyzer = analyzer;
    }

    /// `getContentSource()`.
    pub fn get_content_source(&self) -> Arc<dyn ContentSource> {
        Arc::clone(&self.content_source)
    }

    /// `getDocMaker()`.
    pub fn get_doc_maker(&self) -> Arc<dyn DocMakerClass> {
        Arc::clone(&self.doc_maker)
    }

    /// `getFacetSource()`.
    pub fn get_facet_source(&self) -> Arc<dyn FacetSource> {
        Arc::clone(&self.facet_source)
    }

    /// `getLocale()`.
    pub fn get_locale(&self) -> Option<Locale> {
        self.lock().locale.clone()
    }

    /// `setLocale(Locale)`.
    pub fn set_locale(&self, locale: Option<Locale>) {
        self.lock().locale = locale;
    }

    /// `getConfig()`.
    pub fn get_config(&self) -> &Arc<Config> {
        &self.config
    }

    /// `resetInputs()`.
    ///
    /// # Errors
    ///
    /// The exception of a source's or a query maker's `resetInputs`.
    pub fn reset_inputs(&self) -> Result<()> {
        self.content_source.reset_inputs()?;
        self.doc_maker.reset_inputs()?;
        self.facet_source.reset_inputs()?;
        let makers: Vec<Arc<dyn QueryMaker>> = self
            .lock()
            .read_task_query_maker
            .values()
            .cloned()
            .collect();
        for query_maker in makers {
            query_maker.reset_inputs().map_err(|e| {
                if e.is_io_exception() {
                    e
                } else {
                    jdk::runtime_exception_from(e)
                }
            })?;
        }
        Ok(())
    }

    /// `getQueryMaker(ReadTask readTask)`: the query maker by read task type
    /// (class).
    ///
    /// # Errors
    ///
    /// A `RuntimeException` wrapping the exception of creating or
    /// configuring the maker.
    pub fn get_query_maker(&self, read_task: &dyn ReadTask) -> Result<Arc<dyn QueryMaker>> {
        self.query_maker_for_class(read_task.as_any().type_id())
    }

    /// [`get_query_maker`](Self::get_query_maker) for the read task class
    /// whose `TypeId` is `read_task_class`, as a read task's constructor
    /// calls it before the task exists.
    ///
    /// # Errors
    ///
    /// As [`get_query_maker`](Self::get_query_maker).
    pub fn query_maker_for_class(&self, read_task_class: TypeId) -> Result<Arc<dyn QueryMaker>> {
        // mapping the query maker by task class allows extending/adding new search/read tasks
        // without needing to modify this class.
        let mut s = self.lock();
        if let Some(qm) = s.read_task_query_maker.get(&read_task_class) {
            return Ok(Arc::clone(qm));
        }
        let qm =
            classes::query_maker_for_name(&self.qmkr_class).map_err(jdk::runtime_exception_from)?;
        qm.set_config(Arc::clone(&self.config))
            .map_err(jdk::runtime_exception_from)?;
        s.read_task_query_maker
            .insert(read_task_class, Arc::clone(&qm));
        Ok(qm)
    }

    /// `getAnalyzerFactories()`.
    pub fn get_analyzer_factories(&self) -> MutexGuard<'_, HashMap<String, Arc<AnalyzerFactory>>> {
        jdk::lock(&self.analyzer_factories)
    }
}
