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

//! The types declared in `CreateIndexTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::fs::File;
use std::io::BufWriter;
use std::sync::Arc;

use rucene::codecs::FilterCodec;
use rucene::error::Result;
use rucene::index::index_commit::IndexCommit;
use rucene::index::index_deletion_policy::{IndexDeletionPolicy, NoDeletionPolicy};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig, OpenMode};
use rucene::index::merge_policy::{LogMergePolicy, MergePolicy, NoMergePolicy};
use rucene::index::merge_scheduler::{ConcurrentMergeScheduler, NoMergeScheduler, AUTO_DETECT_MERGES_AND_THREADS};

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::open_index_task::OpenIndexTask;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::by_task::utils::config::Config;
use crate::classes;
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.CreateIndexTask`
/// (`public class CreateIndexTask extends PerfTask`): create an index.
///
/// Other side effects: index writer object in perfRunData is set.
///
/// Relevant properties: `merge.factor (default 10), max.buffered (default no
/// flush), compound (default true), ram.flush.mb [default 0], merge.policy
/// (default org.apache.lucene.index.LogByteSizeMergePolicy),
/// merge.scheduler (default org.apache.lucene.index.ConcurrentMergeScheduler),
/// concurrent.merge.scheduler.max.thread.count and
/// concurrent.merge.scheduler.max.merge.count (defaults per
/// ConcurrentMergeScheduler), default.codec`.
///
/// This task also supports a "writer.info.stream" property with the
/// following values:
///
/// * SystemOut - sets `IndexWriterConfig.setInfoStream(PrintStream)` to
///   `System.out`.
/// * SystemErr - sets `IndexWriterConfig.setInfoStream(PrintStream)` to
///   `System.err`.
/// * <file_name> - attempts to create a file given that name and sets
///   `IndexWriterConfig.setInfoStream(PrintStream)` to that file. If this
///   denotes an invalid file name, or some error occurs, an exception will
///   be thrown.
pub struct CreateIndexTask {
    base: PerfTaskBase,
}

impl CreateIndexTask {
    /// `CreateIndexTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(CreateIndexTask { base: PerfTaskBase::new(run_data, "CreateIndexTask")? })
    }

    /// `getIndexDeletionPolicy(Config config)`.
    ///
    /// # Errors
    ///
    /// `RuntimeException("unable to instantiate class '...' as
    /// IndexDeletionPolicy", e)`.
    pub fn get_index_deletion_policy(config: &Config) -> Result<Arc<dyn IndexDeletionPolicy>> {
        let deletion_policy_name = config
            .get_str("deletion.policy", Some("org.apache.lucene.index.KeepOnlyLastCommitDeletionPolicy"))?
            .unwrap_or_default();
        if deletion_policy_name == "org.apache.lucene.index.NoDeletionPolicy" {
            Ok(NoDeletionPolicy::instance())
        } else {
            classes::deletion_policy_for_name(&deletion_policy_name).map_err(|e| {
                jdk::runtime_exception_with_cause(
                    format!("unable to instantiate class '{deletion_policy_name}' as IndexDeletionPolicy"),
                    e,
                )
            })
        }
    }

    /// `createWriterConfig(Config config, PerfRunData runData, OpenMode mode,
    /// IndexCommit commit)`.
    ///
    /// # Errors
    ///
    /// The `RuntimeException`s of instantiating the configured classes.
    pub fn create_writer_config(
        config: &Config,
        run_data: &PerfRunData,
        mode: OpenMode,
        commit: Option<Arc<dyn IndexCommit>>,
    ) -> Result<IndexWriterConfig> {
        let iw_conf = IndexWriterConfig::with_analyzer(run_data.get_analyzer());
        iw_conf.set_open_mode(mode)?;
        let index_deletion_policy = Self::get_index_deletion_policy(config)?;
        iw_conf.set_index_deletion_policy(Some(index_deletion_policy))?;
        if commit.is_some() {
            iw_conf.set_index_commit(commit);
        }

        let merge_scheduler = config
            .get_str("merge.scheduler", Some("org.apache.lucene.index.ConcurrentMergeScheduler"))?
            .unwrap_or_default();
        if merge_scheduler == "org.apache.lucene.index.NoMergeScheduler" {
            iw_conf.set_merge_scheduler(Some(NoMergeScheduler::instance()))?;
        } else if merge_scheduler == "org.apache.lucene.index.ConcurrentMergeScheduler" {
            let cms = Arc::new(ConcurrentMergeScheduler::new());
            iw_conf.set_merge_scheduler(Some(Arc::clone(&cms) as _))?;
            let max_thread_count =
                config.get_int("concurrent.merge.scheduler.max.thread.count", AUTO_DETECT_MERGES_AND_THREADS)?;
            let max_merge_count =
                config.get_int("concurrent.merge.scheduler.max.merge.count", AUTO_DETECT_MERGES_AND_THREADS)?;
            cms.set_max_merges_and_threads(max_merge_count, max_thread_count)?;
        } else {
            let scheduler = classes::merge_scheduler_for_name(&merge_scheduler).map_err(|e| {
                jdk::runtime_exception_with_cause(
                    format!("unable to instantiate class '{merge_scheduler}' as merge scheduler"),
                    e,
                )
            })?;
            iw_conf.set_merge_scheduler(Some(scheduler))?;
        }

        let default_codec = config.get_str("default.codec", None)?;
        if let Some(default_codec) = &default_codec {
            let codec = classes::codec_for_name(default_codec).map_err(|e| {
                jdk::runtime_exception_with_cause(format!("Couldn't instantiate Codec: {default_codec}"), e)
            })?;
            iw_conf.set_codec(codec)?;
        }

        let postings_format = config.get_str("codec.postingsFormat", None)?;
        if let (None, Some(postings_format)) = (&default_codec, postings_format) {
            let chosen = (|| -> Result<Arc<dyn rucene::codecs::Codec>> {
                let postings_format_chosen = rucene::codecs::postings::postings_lookup(&postings_format)?;
                let default = rucene::codecs::get_default().ok_or_else(rucene::error::LuceneError::null_pointer)?;
                Ok(Arc::new(
                    FilterCodec::new(default.name().to_string(), Arc::clone(&default))
                        .with_postings_format(SharedPostingsFormat(postings_format_chosen)),
                ))
            })()
            .map_err(|e| {
                jdk::runtime_exception_with_cause(format!("Couldn't instantiate Postings Format: {postings_format}"), e)
            })?;
            iw_conf.set_codec(chosen)?;
        }

        let merge_policy = config
            .get_str("merge.policy", Some("org.apache.lucene.index.LogByteSizeMergePolicy"))?
            .unwrap_or_default();
        let is_compound = config.get_bool("compound", true)?;
        iw_conf.set_use_compound_file(is_compound)?;
        if merge_policy == "org.apache.lucene.index.NoMergePolicy" {
            iw_conf.set_merge_policy(Some(NoMergePolicy::instance() as Arc<dyn MergePolicy>))?;
        } else {
            let policy = classes::merge_policy_for_name(&merge_policy).map_err(|e| {
                jdk::runtime_exception_with_cause(
                    format!("unable to instantiate class '{merge_policy}' as merge policy"),
                    e,
                )
            })?;
            iw_conf.set_merge_policy(Some(policy))?;
            iw_conf.merge_policy().set_no_cfs_ratio(if is_compound { 1.0 } else { 0.0 })?;
            let policy = iw_conf.merge_policy();
            if let Some(log_merge_policy) = policy.as_any().and_then(|a| a.downcast_ref::<LogMergePolicy>()) {
                log_merge_policy.set_merge_factor(config.get_int("merge.factor", OpenIndexTask::DEFAULT_MERGE_PFACTOR)?)?;
            }
        }
        let ram_buffer = config.get_double("ram.flush.mb", OpenIndexTask::DEFAULT_RAM_FLUSH_MB)?;
        let max_buffered = config.get_int("max.buffered", OpenIndexTask::DEFAULT_MAX_BUFFERED)?;
        if max_buffered == LiveIndexWriterConfig::DISABLE_AUTO_FLUSH {
            iw_conf.set_ram_buffer_size_mb(ram_buffer)?;
            iw_conf.set_max_buffered_docs(max_buffered)?;
        } else {
            iw_conf.set_max_buffered_docs(max_buffered)?;
            iw_conf.set_ram_buffer_size_mb(ram_buffer)?;
        }

        Ok(iw_conf)
    }

    /// `configureWriter(Config config, PerfRunData runData, OpenMode mode,
    /// IndexCommit commit)`.
    ///
    /// # Errors
    ///
    /// The exception of configuring or opening the writer.
    pub fn configure_writer(
        config: &Config,
        run_data: &PerfRunData,
        mode: OpenMode,
        commit: Option<Arc<dyn IndexCommit>>,
    ) -> Result<Arc<IndexWriter>> {
        let iwc = Self::create_writer_config(config, run_data, mode, commit)?;
        let info_stream_val = config.get_str("writer.info.stream", None)?;
        if let Some(info_stream_val) = info_stream_val {
            if info_stream_val == "SystemOut" {
                iwc.set_info_stream_print_stream(crate::jdk::system::out())?;
            } else if info_stream_val == "SystemErr" {
                iwc.set_info_stream_print_stream(crate::jdk::system::err())?;
            } else {
                let f = File::create(&info_stream_val)?;
                iwc.set_info_stream_print_stream(Box::new(BufWriter::new(f)))?;
            }
        }
        let directory = run_data.get_directory().ok_or_else(rucene::error::LuceneError::null_pointer)?;
        IndexWriter::new(directory, iwc)
    }
}

/// The `PostingsFormat` `PostingsFormat.forName` returned, installed in the
/// anonymous `FilterCodec` of `createWriterConfig`.
#[derive(Debug)]
struct SharedPostingsFormat(Arc<dyn rucene::codecs::postings::PostingsFormat>);

impl rucene::codecs::postings::PostingsFormat for SharedPostingsFormat {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn fields_consumer<'a>(
        &self,
        state: &rucene::codecs::state::SegmentWriteState<'a>,
    ) -> Result<Box<dyn rucene::codecs::postings::FieldsConsumer + 'a>> {
        self.0.fields_consumer(state)
    }

    fn fields_producer<'a>(
        &self,
        state: &rucene::codecs::state::SegmentReadState<'a>,
    ) -> Result<Box<dyn rucene::codecs::postings::FieldsProducer>> {
        self.0.fields_producer(state)
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        self.0.as_any()
    }

    fn to_java_string(&self) -> String {
        self.0.to_java_string()
    }
}

impl PerfTask for CreateIndexTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(CreateIndexTask { base: self.base.clone_base() }))
    }

    fn do_logic(&self) -> Result<i32> {
        let run_data = self.base.get_run_data();
        let config = Arc::clone(run_data.get_config());
        run_data.set_index_writer(Some(Self::configure_writer(&config, run_data, OpenMode::CREATE, None)?));
        Ok(1)
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "CreateIndexTask"
    }
}
