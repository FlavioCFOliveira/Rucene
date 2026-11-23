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

//! `Class.forName(name)` and `getConstructor(...).newInstance(...)` of the
//! benchmark: the classes its properties and algorithm files name, by
//! binary name, one table per expected type.
//!
//! Java finds any class of the classpath by name; Rust has no runtime class
//! loading, so each table lists the classes of the module (and of the
//! modules it depends on) with the constructor the benchmark calls, as the
//! core's `ClassRegistry` does for `ResourceLoader`
//! (`rucene::util::resource_loader`). An application adds its own classes
//! with the `register_*` functions, as it would put them on Java's
//! classpath.
//!
//! A name no table knows fails as `Class.forName` does
//! (`ClassNotFoundException` with the name as message); a name another
//! table knows fails as `Class.asSubclass` does (`ClassCastException`,
//! `"class " + name`).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use rucene::analysis::Analyzer;
use rucene::error::{LuceneError, LuceneErrorKind, Result};

use crate::by_task::feeds::content_source::ContentSource;
use crate::by_task::feeds::doc_maker::{DocMaker, DocMakerClass};
use crate::by_task::feeds::facet_source::FacetSource;
use crate::by_task::feeds::query_maker::QueryMaker;
use crate::by_task::feeds::random_facet_source::RandomFacetSource;
use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::PerfTask;
use crate::by_task::tasks::read_task::AnyCollectorManager;
use crate::by_task::tasks::task_sequence::TaskSequence;

/// The binary name of `java.lang.ClassNotFoundException`.
const CLASS_NOT_FOUND: &str = "java.lang.ClassNotFoundException";

/// `new ClassNotFoundException(message)`.
pub(crate) fn class_not_found(message: impl Into<String>) -> LuceneError {
    LuceneError::unchecked(CLASS_NOT_FOUND, message)
}

/// Whether `error` is a `ClassNotFoundException`.
pub(crate) fn is_class_not_found(error: &LuceneError) -> bool {
    matches!(error.kind(), LuceneErrorKind::Unchecked { class, .. } if *class == CLASS_NOT_FOUND)
}

/// `Class.asSubclass`'s `ClassCastException(this.toString())`.
fn class_cast(name: &str) -> LuceneError {
    LuceneError::unchecked("java.lang.ClassCastException", format!("class {name}"))
}

/// `(TaskSequence) task` for a task known to be a sequence.
pub(crate) fn downcast_sequence(task: &Arc<dyn PerfTask>) -> Arc<TaskSequence> {
    task.as_any()
        .downcast_ref::<TaskSequence>()
        .expect("checked by the caller")
        .as_arc()
}

/// A constructor of a task: `getConstructor(PerfRunData.class)`.
pub type TaskConstructor = Arc<dyn Fn(Arc<PerfRunData>) -> Result<Arc<dyn PerfTask>> + Send + Sync>;
/// A public no-argument constructor of a `ContentSource`.
pub type ContentSourceConstructor = Arc<dyn Fn() -> Arc<dyn ContentSource> + Send + Sync>;
/// A public no-argument constructor of a `DocMaker`.
pub type DocMakerConstructor = Arc<dyn Fn() -> Arc<dyn DocMakerClass> + Send + Sync>;
/// A public no-argument constructor of a `FacetSource`.
pub type FacetSourceConstructor = Arc<dyn Fn() -> Arc<dyn FacetSource> + Send + Sync>;
/// A public no-argument constructor of a `QueryMaker`.
pub type QueryMakerConstructor = Arc<dyn Fn() -> Arc<dyn QueryMaker> + Send + Sync>;
/// A public no-argument constructor of a `CollectorManager`.
pub type CollectorManagerConstructor =
    Arc<dyn Fn() -> Result<Box<dyn AnyCollectorManager>> + Send + Sync>;

/// One table.
struct Table<F> {
    entries: RwLock<HashMap<String, F>>,
}

impl<F: Clone> Table<F> {
    fn new(entries: Vec<(&str, F)>) -> Self {
        Table {
            entries: RwLock::new(
                entries
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
            ),
        }
    }

    fn get(&self, name: &str) -> Option<F> {
        self.entries
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(name)
            .cloned()
    }

    fn contains(&self, name: &str) -> bool {
        self.entries
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(name)
    }

    fn register(&self, name: &str, f: F) {
        self.entries
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(name.to_string(), f);
    }
}

macro_rules! task_entry {
    ($ty:ty, $name:literal) => {
        (
            concat!("org.apache.lucene.benchmark.byTask.tasks.", $name),
            Arc::new(|run_data: Arc<PerfRunData>| -> Result<Arc<dyn PerfTask>> {
                Ok(Arc::new(<$ty>::new(run_data)?))
            }) as TaskConstructor,
        )
    };
}

static TASKS: LazyLock<Table<TaskConstructor>> = LazyLock::new(|| {
    use crate::by_task::tasks::*;
    Table::new(vec![
        task_entry!(add_doc_task::AddDocTask, "AddDocTask"),
        task_entry!(add_faceted_doc_task::AddFacetedDocTask, "AddFacetedDocTask"),
        task_entry!(add_indexes_task::AddIndexesTask, "AddIndexesTask"),
        task_entry!(
            analyzer_factory_task::AnalyzerFactoryTask,
            "AnalyzerFactoryTask"
        ),
        task_entry!(clear_stats_task::ClearStatsTask, "ClearStatsTask"),
        task_entry!(close_index_task::CloseIndexTask, "CloseIndexTask"),
        task_entry!(close_reader_task::CloseReaderTask, "CloseReaderTask"),
        task_entry!(
            close_taxonomy_index_task::CloseTaxonomyIndexTask,
            "CloseTaxonomyIndexTask"
        ),
        task_entry!(
            close_taxonomy_reader_task::CloseTaxonomyReaderTask,
            "CloseTaxonomyReaderTask"
        ),
        task_entry!(commit_index_task::CommitIndexTask, "CommitIndexTask"),
        task_entry!(
            commit_taxonomy_index_task::CommitTaxonomyIndexTask,
            "CommitTaxonomyIndexTask"
        ),
        task_entry!(
            consume_content_source_task::ConsumeContentSourceTask,
            "ConsumeContentSourceTask"
        ),
        task_entry!(create_index_task::CreateIndexTask, "CreateIndexTask"),
        task_entry!(
            create_taxonomy_index_task::CreateTaxonomyIndexTask,
            "CreateTaxonomyIndexTask"
        ),
        task_entry!(flush_index_task::FlushIndexTask, "FlushIndexTask"),
        task_entry!(force_merge_task::ForceMergeTask, "ForceMergeTask"),
        task_entry!(
            near_realtime_reader_task::NearRealtimeReaderTask,
            "NearRealtimeReaderTask"
        ),
        task_entry!(new_analyzer_task::NewAnalyzerTask, "NewAnalyzerTask"),
        task_entry!(
            new_collation_analyzer_task::NewCollationAnalyzerTask,
            "NewCollationAnalyzerTask"
        ),
        task_entry!(new_locale_task::NewLocaleTask, "NewLocaleTask"),
        task_entry!(new_round_task::NewRoundTask, "NewRoundTask"),
        task_entry!(open_index_task::OpenIndexTask, "OpenIndexTask"),
        task_entry!(open_reader_task::OpenReaderTask, "OpenReaderTask"),
        task_entry!(
            open_taxonomy_index_task::OpenTaxonomyIndexTask,
            "OpenTaxonomyIndexTask"
        ),
        task_entry!(
            open_taxonomy_reader_task::OpenTaxonomyReaderTask,
            "OpenTaxonomyReaderTask"
        ),
        task_entry!(print_reader_task::PrintReaderTask, "PrintReaderTask"),
        task_entry!(read_tokens_task::ReadTokensTask, "ReadTokensTask"),
        task_entry!(reopen_reader_task::ReopenReaderTask, "ReopenReaderTask"),
        task_entry!(rep_all_task::RepAllTask, "RepAllTask"),
        task_entry!(
            rep_select_by_pref_task::RepSelectByPrefTask,
            "RepSelectByPrefTask"
        ),
        task_entry!(
            rep_sum_by_name_round_task::RepSumByNameRoundTask,
            "RepSumByNameRoundTask"
        ),
        task_entry!(rep_sum_by_name_task::RepSumByNameTask, "RepSumByNameTask"),
        task_entry!(
            rep_sum_by_pref_round_task::RepSumByPrefRoundTask,
            "RepSumByPrefRoundTask"
        ),
        task_entry!(rep_sum_by_pref_task::RepSumByPrefTask, "RepSumByPrefTask"),
        task_entry!(reset_inputs_task::ResetInputsTask, "ResetInputsTask"),
        task_entry!(
            reset_system_erase_task::ResetSystemEraseTask,
            "ResetSystemEraseTask"
        ),
        task_entry!(
            reset_system_soft_task::ResetSystemSoftTask,
            "ResetSystemSoftTask"
        ),
        task_entry!(rollback_index_task::RollbackIndexTask, "RollbackIndexTask"),
        task_entry!(search_task::SearchTask, "SearchTask"),
        task_entry!(
            search_trav_ret_highlight_task::SearchTravRetHighlightTask,
            "SearchTravRetHighlightTask"
        ),
        task_entry!(search_trav_ret_task::SearchTravRetTask, "SearchTravRetTask"),
        task_entry!(search_trav_task::SearchTravTask, "SearchTravTask"),
        task_entry!(
            search_with_collector_task::SearchWithCollectorTask,
            "SearchWithCollectorTask"
        ),
        task_entry!(
            search_with_sort_task::SearchWithSortTask,
            "SearchWithSortTask"
        ),
        task_entry!(set_prop_task::SetPropTask, "SetPropTask"),
        task_entry!(update_doc_task::UpdateDocTask, "UpdateDocTask"),
        task_entry!(wait_task::WaitTask, "WaitTask"),
        task_entry!(warm_task::WarmTask, "WarmTask"),
        task_entry!(
            write_enwiki_line_doc_task::WriteEnwikiLineDocTask,
            "WriteEnwikiLineDocTask"
        ),
        task_entry!(write_line_doc_task::WriteLineDocTask, "WriteLineDocTask"),
        (
            "org.apache.lucene.benchmark.byTask.tasks.TaskSequence",
            Arc::new(|run_data: Arc<PerfRunData>| -> Result<Arc<dyn PerfTask>> {
                // TaskSequence has no (PerfRunData) constructor:
                // `getConstructor` throws NoSuchMethodException.
                let _ = run_data;
                Err(LuceneError::unchecked(
                    "java.lang.NoSuchMethodException",
                    "org.apache.lucene.benchmark.byTask.tasks.TaskSequence.<init>(org.apache.lucene.benchmark.byTask.PerfRunData)",
                ))
            }) as TaskConstructor,
        ),
    ])
});

macro_rules! content_source_entry {
    ($ty:ty, $name:literal) => {
        (
            concat!("org.apache.lucene.benchmark.byTask.feeds.", $name),
            Arc::new(|| Arc::new(<$ty>::new()) as Arc<dyn ContentSource>)
                as ContentSourceConstructor,
        )
    };
}

static CONTENT_SOURCES: LazyLock<Table<ContentSourceConstructor>> = LazyLock::new(|| {
    use crate::by_task::feeds::*;
    Table::new(vec![
        content_source_entry!(dir_content_source::DirContentSource, "DirContentSource"),
        content_source_entry!(
            enwiki_content_source::EnwikiContentSource,
            "EnwikiContentSource"
        ),
        content_source_entry!(line_doc_source::LineDocSource, "LineDocSource"),
        content_source_entry!(
            long_to_english_content_source::LongToEnglishContentSource,
            "LongToEnglishContentSource"
        ),
        content_source_entry!(
            reuters_content_source::ReutersContentSource,
            "ReutersContentSource"
        ),
        content_source_entry!(single_doc_source::SingleDocSource, "SingleDocSource"),
        content_source_entry!(
            sortable_single_doc_source::SortableSingleDocSource,
            "SortableSingleDocSource"
        ),
        content_source_entry!(trec_content_source::TrecContentSource, "TrecContentSource"),
    ])
});

static DOC_MAKERS: LazyLock<Table<DocMakerConstructor>> = LazyLock::new(|| {
    use crate::by_task::feeds::spatial_doc_maker::SpatialDocMaker;
    Table::new(vec![
        (
            "org.apache.lucene.benchmark.byTask.feeds.DocMaker",
            Arc::new(|| Arc::new(DocMaker::new()) as Arc<dyn DocMakerClass>) as DocMakerConstructor,
        ),
        (
            "org.apache.lucene.benchmark.byTask.feeds.SpatialDocMaker",
            Arc::new(|| Arc::new(SpatialDocMaker::new()) as Arc<dyn DocMakerClass>)
                as DocMakerConstructor,
        ),
    ])
});

static FACET_SOURCES: LazyLock<Table<FacetSourceConstructor>> = LazyLock::new(|| {
    Table::new(vec![(
        "org.apache.lucene.benchmark.byTask.feeds.RandomFacetSource",
        Arc::new(|| Arc::new(RandomFacetSource::new()) as Arc<dyn FacetSource>)
            as FacetSourceConstructor,
    )])
});

macro_rules! query_maker_entry {
    ($ty:ty, $name:literal) => {
        (
            concat!("org.apache.lucene.benchmark.byTask.feeds.", $name),
            Arc::new(|| Arc::new(<$ty>::new()) as Arc<dyn QueryMaker>) as QueryMakerConstructor,
        )
    };
}

static QUERY_MAKERS: LazyLock<Table<QueryMakerConstructor>> = LazyLock::new(|| {
    use crate::by_task::feeds::*;
    Table::new(vec![
        query_maker_entry!(enwiki_query_maker::EnwikiQueryMaker, "EnwikiQueryMaker"),
        query_maker_entry!(
            file_based_query_maker::FileBasedQueryMaker,
            "FileBasedQueryMaker"
        ),
        query_maker_entry!(
            long_to_english_query_maker::LongToEnglishQueryMaker,
            "LongToEnglishQueryMaker"
        ),
        query_maker_entry!(reuters_query_maker::ReutersQueryMaker, "ReutersQueryMaker"),
        query_maker_entry!(simple_query_maker::SimpleQueryMaker, "SimpleQueryMaker"),
        query_maker_entry!(
            simple_sloppy_phrase_query_maker::SimpleSloppyPhraseQueryMaker,
            "SimpleSloppyPhraseQueryMaker"
        ),
        query_maker_entry!(
            spatial_file_query_maker::SpatialFileQueryMaker,
            "SpatialFileQueryMaker"
        ),
    ])
});

static COLLECTOR_MANAGERS: LazyLock<Table<CollectorManagerConstructor>> =
    LazyLock::new(|| Table::new(vec![]));

/// Whether any table knows `name`.
fn known(name: &str) -> bool {
    TASKS.contains(name)
        || CONTENT_SOURCES.contains(name)
        || DOC_MAKERS.contains(name)
        || FACET_SOURCES.contains(name)
        || QUERY_MAKERS.contains(name)
        || COLLECTOR_MANAGERS.contains(name)
        || DELETION_POLICIES.contains(name)
        || MERGE_SCHEDULERS.contains(name)
        || MERGE_POLICIES.contains(name)
        || CODECS.contains(name)
        || LINE_PARSERS.contains(name)
        || TREC_DOC_PARSERS.contains(name)
        || HTML_PARSERS.contains(name)
        || rucene_analysis_common::spi::analyzer_classes()
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .find_class(name)
            .is_ok()
}

/// The error of `Class.forName(name).asSubclass(T.class)` for a name the
/// table of `T` does not hold.
fn missing(name: &str) -> LuceneError {
    if known(name) {
        class_cast(name)
    } else {
        class_not_found(name)
    }
}

/// `Class.forName(name).asSubclass(PerfTask.class).getConstructor(PerfRunData.class).newInstance(runData)`;
/// `None` when no task class has that name (`ClassNotFoundException`).
pub fn task_for_name(name: &str, run_data: &Arc<PerfRunData>) -> Option<Result<Arc<dyn PerfTask>>> {
    if let Some(ctor) = TASKS.get(name) {
        return Some(ctor(Arc::clone(run_data)));
    }
    if known(name) {
        return Some(Err(class_cast(name)));
    }
    None
}

/// `Class.forName(name).asSubclass(ContentSource.class).getConstructor().newInstance()`.
///
/// # Errors
///
/// `ClassNotFoundException` or `ClassCastException`.
pub fn content_source_for_name(name: &str) -> Result<Arc<dyn ContentSource>> {
    CONTENT_SOURCES
        .get(name)
        .map(|c| c())
        .ok_or_else(|| missing(name))
}

/// `Class.forName(name).asSubclass(DocMaker.class).getConstructor().newInstance()`.
///
/// # Errors
///
/// `ClassNotFoundException` or `ClassCastException`.
pub fn doc_maker_for_name(name: &str) -> Result<Arc<dyn DocMakerClass>> {
    DOC_MAKERS
        .get(name)
        .map(|c| c())
        .ok_or_else(|| missing(name))
}

/// `Class.forName(name).asSubclass(FacetSource.class).getConstructor().newInstance()`.
///
/// # Errors
///
/// `ClassNotFoundException` or `ClassCastException`.
pub fn facet_source_for_name(name: &str) -> Result<Arc<dyn FacetSource>> {
    FACET_SOURCES
        .get(name)
        .map(|c| c())
        .ok_or_else(|| missing(name))
}

/// `Class.forName(name).asSubclass(QueryMaker.class)`.
///
/// # Errors
///
/// `ClassNotFoundException` or `ClassCastException`.
pub fn check_query_maker_class(name: &str) -> Result<()> {
    if QUERY_MAKERS.contains(name) {
        Ok(())
    } else {
        Err(missing(name))
    }
}

/// `qmkrClass.getConstructor().newInstance()`.
///
/// # Errors
///
/// `ClassNotFoundException` or `ClassCastException`.
pub fn query_maker_for_name(name: &str) -> Result<Arc<dyn QueryMaker>> {
    QUERY_MAKERS
        .get(name)
        .map(|c| c())
        .ok_or_else(|| missing(name))
}

/// `Class.forName(name).asSubclass(CollectorManager.class).getConstructor().newInstance()`.
///
/// # Errors
///
/// `ClassNotFoundException` or `ClassCastException`.
pub fn collector_manager_for_name(name: &str) -> Result<Box<dyn AnyCollectorManager>> {
    match COLLECTOR_MANAGERS.get(name) {
        Some(c) => c(),
        None => Err(missing(name)),
    }
}

/// `Class.forName(name).asSubclass(Analyzer.class).getConstructor().newInstance()`:
/// the analyzers of `rucene_analysis_common::spi::analyzer_classes()`.
///
/// # Errors
///
/// `ClassNotFoundException` or `ClassCastException`.
pub fn analyzer_for_name(name: &str) -> Result<Arc<dyn Analyzer>> {
    let registry = rucene_analysis_common::spi::analyzer_classes()
        .read()
        .unwrap_or_else(|e| e.into_inner());
    match registry.find_class(name) {
        Ok(factory) => Ok(factory()),
        Err(_) => Err(missing(name)),
    }
}

/// Puts a task class on the "classpath".
pub fn register_task_class(name: &str, constructor: TaskConstructor) {
    TASKS.register(name, constructor);
}

/// Puts a `ContentSource` class on the "classpath".
pub fn register_content_source_class(name: &str, constructor: ContentSourceConstructor) {
    CONTENT_SOURCES.register(name, constructor);
}

/// Puts a `DocMaker` class on the "classpath".
pub fn register_doc_maker_class(name: &str, constructor: DocMakerConstructor) {
    DOC_MAKERS.register(name, constructor);
}

/// Puts a `FacetSource` class on the "classpath".
pub fn register_facet_source_class(name: &str, constructor: FacetSourceConstructor) {
    FACET_SOURCES.register(name, constructor);
}

/// Puts a `QueryMaker` class on the "classpath".
pub fn register_query_maker_class(name: &str, constructor: QueryMakerConstructor) {
    QUERY_MAKERS.register(name, constructor);
}

/// Puts a `CollectorManager` class on the "classpath".
pub fn register_collector_manager_class(name: &str, constructor: CollectorManagerConstructor) {
    COLLECTOR_MANAGERS.register(name, constructor);
}

/// A public no-argument constructor of an `IndexDeletionPolicy`.
pub type DeletionPolicyConstructor = Arc<
    dyn Fn() -> Arc<dyn rucene::index::index_deletion_policy::IndexDeletionPolicy> + Send + Sync,
>;
/// A public no-argument constructor of a `MergeScheduler`.
pub type MergeSchedulerConstructor =
    Arc<dyn Fn() -> Arc<dyn rucene::index::merge_scheduler::MergeScheduler> + Send + Sync>;
/// A public no-argument constructor of a `MergePolicy`.
pub type MergePolicyConstructor =
    Arc<dyn Fn() -> Arc<dyn rucene::index::merge_policy::MergePolicy> + Send + Sync>;
/// A public no-argument constructor of a `Codec`.
pub type CodecConstructor = Arc<dyn Fn() -> Result<Arc<dyn rucene::codecs::Codec>> + Send + Sync>;

static DELETION_POLICIES: LazyLock<Table<DeletionPolicyConstructor>> = LazyLock::new(|| {
    use rucene::index::index_deletion_policy::{
        IndexDeletionPolicy, KeepOnlyLastCommitDeletionPolicy,
    };
    Table::new(vec![(
        "org.apache.lucene.index.KeepOnlyLastCommitDeletionPolicy",
        Arc::new(|| Arc::new(KeepOnlyLastCommitDeletionPolicy) as Arc<dyn IndexDeletionPolicy>)
            as DeletionPolicyConstructor,
    )])
});

static MERGE_SCHEDULERS: LazyLock<Table<MergeSchedulerConstructor>> = LazyLock::new(|| {
    use rucene::index::merge_scheduler::{
        ConcurrentMergeScheduler, MergeScheduler, SerialMergeScheduler,
    };
    Table::new(vec![
        (
            "org.apache.lucene.index.ConcurrentMergeScheduler",
            Arc::new(|| Arc::new(ConcurrentMergeScheduler::new()) as Arc<dyn MergeScheduler>)
                as MergeSchedulerConstructor,
        ),
        (
            "org.apache.lucene.index.SerialMergeScheduler",
            Arc::new(|| Arc::new(SerialMergeScheduler::new()) as Arc<dyn MergeScheduler>)
                as MergeSchedulerConstructor,
        ),
    ])
});

static MERGE_POLICIES: LazyLock<Table<MergePolicyConstructor>> = LazyLock::new(|| {
    use rucene::index::merge_policy::{LogMergePolicy, MergePolicy, TieredMergePolicy};
    Table::new(vec![
        (
            "org.apache.lucene.index.LogByteSizeMergePolicy",
            Arc::new(|| Arc::new(LogMergePolicy::by_byte_size()) as Arc<dyn MergePolicy>)
                as MergePolicyConstructor,
        ),
        (
            "org.apache.lucene.index.LogDocMergePolicy",
            Arc::new(|| Arc::new(LogMergePolicy::by_doc_count()) as Arc<dyn MergePolicy>)
                as MergePolicyConstructor,
        ),
        (
            "org.apache.lucene.index.TieredMergePolicy",
            Arc::new(|| Arc::new(TieredMergePolicy::new()) as Arc<dyn MergePolicy>)
                as MergePolicyConstructor,
        ),
    ])
});

static CODECS: LazyLock<Table<CodecConstructor>> = LazyLock::new(|| {
    Table::new(vec![(
        "org.apache.lucene.codecs.lucene104.Lucene104Codec",
        Arc::new(|| rucene::codecs::lookup("Lucene104")) as CodecConstructor,
    )])
});

/// `Class.forName(name).asSubclass(IndexDeletionPolicy.class).getConstructor().newInstance()`.
///
/// # Errors
///
/// `ClassNotFoundException` or `ClassCastException`.
pub fn deletion_policy_for_name(
    name: &str,
) -> Result<Arc<dyn rucene::index::index_deletion_policy::IndexDeletionPolicy>> {
    DELETION_POLICIES
        .get(name)
        .map(|c| c())
        .ok_or_else(|| missing(name))
}

/// `Class.forName(name).asSubclass(MergeScheduler.class).getConstructor().newInstance()`.
///
/// # Errors
///
/// `ClassNotFoundException` or `ClassCastException`.
pub fn merge_scheduler_for_name(
    name: &str,
) -> Result<Arc<dyn rucene::index::merge_scheduler::MergeScheduler>> {
    MERGE_SCHEDULERS
        .get(name)
        .map(|c| c())
        .ok_or_else(|| missing(name))
}

/// `Class.forName(name).asSubclass(MergePolicy.class).getConstructor().newInstance()`.
///
/// # Errors
///
/// `ClassNotFoundException` or `ClassCastException`.
pub fn merge_policy_for_name(
    name: &str,
) -> Result<Arc<dyn rucene::index::merge_policy::MergePolicy>> {
    MERGE_POLICIES
        .get(name)
        .map(|c| c())
        .ok_or_else(|| missing(name))
}

/// `Class.forName(name).asSubclass(Codec.class).getConstructor().newInstance()`.
///
/// # Errors
///
/// `ClassNotFoundException` or `ClassCastException`.
pub fn codec_for_name(name: &str) -> Result<Arc<dyn rucene::codecs::Codec>> {
    match CODECS.get(name) {
        Some(c) => c(),
        None => Err(missing(name)),
    }
}

/// Puts an `IndexDeletionPolicy` class on the "classpath".
pub fn register_deletion_policy_class(name: &str, constructor: DeletionPolicyConstructor) {
    DELETION_POLICIES.register(name, constructor);
}

/// Puts a `MergeScheduler` class on the "classpath".
pub fn register_merge_scheduler_class(name: &str, constructor: MergeSchedulerConstructor) {
    MERGE_SCHEDULERS.register(name, constructor);
}

/// Puts a `MergePolicy` class on the "classpath".
pub fn register_merge_policy_class(name: &str, constructor: MergePolicyConstructor) {
    MERGE_POLICIES.register(name, constructor);
}

/// Puts a `Codec` class on the "classpath".
pub fn register_codec_class(name: &str, constructor: CodecConstructor) {
    CODECS.register(name, constructor);
}

/// Puts `lucene-analysis-common` on the "classpath" of the analysis SPI
/// loaders, as the benchmark's classpath holds it
/// (`lucene/benchmark/build.gradle`): the core's char filter, token filter
/// and tokenizer loaders are reloaded once with its class loader.
///
/// # Errors
///
/// The exception of the reload.
pub fn ensure_analysis_classpath() -> Result<()> {
    static DONE: std::sync::OnceLock<std::result::Result<(), String>> = std::sync::OnceLock::new();
    let outcome = DONE.get_or_init(|| {
        let loader = rucene_analysis_common::spi::class_loader();
        rucene::analysis::factories::reload_char_filters(loader)
            .and_then(|()| rucene::analysis::factories::reload_token_filters(loader))
            .and_then(|()| rucene::analysis::factories::reload_tokenizers(loader))
            .map_err(|e| e.to_string())
    });
    outcome.clone().map_err(LuceneError::illegal_state)
}

static LINE_PARSERS: LazyLock<
    Table<crate::by_task::feeds::line_doc_source::LineParserConstructor>,
> = LazyLock::new(|| {
    use crate::by_task::feeds::geonames_line_parser::GeonamesLineParser;
    use crate::by_task::feeds::line_doc_source::{
        HeaderLineParser, LineParser, LineParserConstructor, SimpleLineParser,
    };
    Table::new(vec![
        (
            "org.apache.lucene.benchmark.byTask.feeds.LineDocSource$SimpleLineParser",
            Arc::new(|h: Option<Vec<String>>| {
                Ok(Arc::new(SimpleLineParser::new(h)) as Arc<dyn LineParser>)
            }) as LineParserConstructor,
        ),
        (
            "org.apache.lucene.benchmark.byTask.feeds.LineDocSource$HeaderLineParser",
            Arc::new(|h: Option<Vec<String>>| {
                Ok(Arc::new(HeaderLineParser::new(h)?) as Arc<dyn LineParser>)
            }) as LineParserConstructor,
        ),
        (
            "org.apache.lucene.benchmark.byTask.feeds.GeonamesLineParser",
            Arc::new(|h: Option<Vec<String>>| {
                Ok(Arc::new(GeonamesLineParser::new(h)) as Arc<dyn LineParser>)
            }) as LineParserConstructor,
        ),
    ])
});

/// `Class.forName(name).asSubclass(LineParser.class).getConstructor(String[].class).newInstance(header)`.
///
/// # Errors
///
/// `ClassNotFoundException`, `ClassCastException`, or the constructor's
/// exception.
pub fn line_parser_for_name(
    name: &str,
    header: Option<Vec<String>>,
) -> Result<Arc<dyn crate::by_task::feeds::line_doc_source::LineParser>> {
    match LINE_PARSERS.get(name) {
        Some(c) => c(header),
        None => Err(missing(name)),
    }
}

/// Puts a `LineParser` class on the "classpath".
pub fn register_line_parser_class(
    name: &str,
    constructor: crate::by_task::feeds::line_doc_source::LineParserConstructor,
) {
    LINE_PARSERS.register(name, constructor);
}

/// A public no-argument constructor of a `TrecDocParser`.
pub type TrecDocParserConstructor =
    Arc<dyn Fn() -> Arc<dyn crate::by_task::feeds::trec_doc_parser::TrecDocParser> + Send + Sync>;
/// A public no-argument constructor of an `HTMLParser`.
pub type HtmlParserConstructor =
    Arc<dyn Fn() -> Arc<dyn crate::by_task::feeds::html_parser::HTMLParser> + Send + Sync>;

static TREC_DOC_PARSERS: LazyLock<Table<TrecDocParserConstructor>> = LazyLock::new(|| {
    use crate::by_task::feeds::trec_doc_parser::TrecDocParser;
    use crate::by_task::feeds::*;
    macro_rules! entry {
        ($ty:ty, $name:literal) => {
            (
                concat!("org.apache.lucene.benchmark.byTask.feeds.", $name),
                Arc::new(|| Arc::new(<$ty>::default()) as Arc<dyn TrecDocParser>)
                    as TrecDocParserConstructor,
            )
        };
    }
    Table::new(vec![
        entry!(trec_fbis_parser::TrecFBISParser, "TrecFBISParser"),
        entry!(trec_fr94_parser::TrecFR94Parser, "TrecFR94Parser"),
        entry!(trec_ft_parser::TrecFTParser, "TrecFTParser"),
        entry!(trec_gov2_parser::TrecGov2Parser, "TrecGov2Parser"),
        entry!(trec_la_times_parser::TrecLATimesParser, "TrecLATimesParser"),
        entry!(trec_parser_by_path::TrecParserByPath, "TrecParserByPath"),
    ])
});

static HTML_PARSERS: LazyLock<Table<HtmlParserConstructor>> = LazyLock::new(|| {
    use crate::by_task::feeds::demo_html_parser::DemoHTMLParser;
    use crate::by_task::feeds::html_parser::HTMLParser;
    Table::new(vec![(
        "org.apache.lucene.benchmark.byTask.feeds.DemoHTMLParser",
        Arc::new(|| Arc::new(DemoHTMLParser::new()) as Arc<dyn HTMLParser>)
            as HtmlParserConstructor,
    )])
});

/// `Class.forName(name).asSubclass(TrecDocParser.class).getConstructor().newInstance()`.
///
/// # Errors
///
/// `ClassNotFoundException` or `ClassCastException`.
pub fn trec_doc_parser_for_name(
    name: &str,
) -> Result<Arc<dyn crate::by_task::feeds::trec_doc_parser::TrecDocParser>> {
    TREC_DOC_PARSERS
        .get(name)
        .map(|c| c())
        .ok_or_else(|| missing(name))
}

/// `Class.forName(name).asSubclass(HTMLParser.class).getConstructor().newInstance()`.
///
/// # Errors
///
/// `ClassNotFoundException` or `ClassCastException`.
pub fn html_parser_for_name(
    name: &str,
) -> Result<Arc<dyn crate::by_task::feeds::html_parser::HTMLParser>> {
    HTML_PARSERS
        .get(name)
        .map(|c| c())
        .ok_or_else(|| missing(name))
}

/// Puts a `TrecDocParser` class on the "classpath".
pub fn register_trec_doc_parser_class(name: &str, constructor: TrecDocParserConstructor) {
    TREC_DOC_PARSERS.register(name, constructor);
}

/// Puts an `HTMLParser` class on the "classpath".
pub fn register_html_parser_class(name: &str, constructor: HtmlParserConstructor) {
    HTML_PARSERS.register(name, constructor);
}
