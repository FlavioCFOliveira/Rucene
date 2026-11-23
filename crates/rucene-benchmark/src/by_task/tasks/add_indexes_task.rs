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

//! The types declared in `AddIndexesTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};
use rucene::index::codec_reader::CodecReader;
use rucene::index::directory_reader;
use rucene::index::filter_directory_reader::SlowCodecReaderWrapper;
use rucene::index::index_reader::IndexReader;
use rucene::store::{Directory, FSDirectory};

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.AddIndexesTask`
/// (`public class AddIndexesTask extends PerfTask`): adds an input index to
/// an existing index, using `IndexWriter.addIndexes(Directory...)` or
/// `IndexWriter.addIndexes(CodecReader...)`. The location of the input index
/// is specified by the parameter [`ADDINDEXES_INPUT_DIR`](Self::ADDINDEXES_INPUT_DIR)
/// and is assumed to be a directory on the file system.
///
/// Takes optional parameter `useAddIndexesDir` which specifies which
/// addIndexes variant to use (defaults to true, to use
/// addIndexes(Directory)).
pub struct AddIndexesTask {
    base: PerfTaskBase,
    use_add_indexes_dir: AtomicBool,
    input_dir: Mutex<Option<Arc<dyn Directory>>>,
}

impl AddIndexesTask {
    /// `ADDINDEXES_INPUT_DIR`.
    pub const ADDINDEXES_INPUT_DIR: &'static str = "addindexes.input.dir";

    /// `AddIndexesTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(AddIndexesTask {
            base: PerfTaskBase::new(run_data, "AddIndexesTask")?,
            use_add_indexes_dir: AtomicBool::new(true),
            input_dir: Mutex::new(None),
        })
    }
}

impl PerfTask for AddIndexesTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(AddIndexesTask {
            base: self.base.clone_base(),
            use_add_indexes_dir: AtomicBool::new(self.use_add_indexes_dir.load(Ordering::SeqCst)),
            input_dir: Mutex::new(jdk::lock(&self.input_dir).clone()),
        }))
    }

    fn setup(&self) -> Result<()> {
        let input_dir_prop = self
            .base
            .get_run_data()
            .get_config()
            .get_str(Self::ADDINDEXES_INPUT_DIR, None)?;
        let Some(input_dir_prop) = input_dir_prop else {
            return Err(LuceneError::illegal_argument(format!(
                "config parameter {} not specified in configuration",
                Self::ADDINDEXES_INPUT_DIR
            )));
        };
        *jdk::lock(&self.input_dir) = Some(Arc::new(FSDirectory::open(std::path::PathBuf::from(
            input_dir_prop,
        ))?));
        Ok(())
    }

    fn do_logic(&self) -> Result<i32> {
        let writer = self
            .base
            .get_run_data()
            .get_index_writer()
            .ok_or_else(LuceneError::null_pointer)?;
        let input_dir = jdk::lock(&self.input_dir)
            .clone()
            .ok_or_else(LuceneError::null_pointer)?;
        if self.use_add_indexes_dir.load(Ordering::SeqCst) {
            writer.add_indexes_dirs(&[input_dir])?;
        } else {
            let r = directory_reader::open(input_dir)?;
            let result = (|| -> Result<()> {
                let leaves = (Arc::clone(&r) as Arc<dyn IndexReader>).leaves()?;
                let mut readers: Vec<Arc<dyn CodecReader>> = Vec::with_capacity(leaves.len());
                for leaf in leaves {
                    readers.push(SlowCodecReaderWrapper::wrap(leaf.leaf_reader())?);
                }
                writer.add_indexes_codec_readers(&readers)?;
                Ok(())
            })();
            let closed = r.close();
            result?;
            closed?;
        }
        Ok(1)
    }

    /// Set the params (useAddIndexesDir only): `useAddIndexesDir=true` for
    /// using `IndexWriter.addIndexes(Directory...)` or `false` for using
    /// `IndexWriter.addIndexes(CodecReader...)`. Defaults to `true`.
    fn set_params(&self, params: &str) -> Result<()> {
        perf_task::set_params(self, params)?;
        self.use_add_indexes_dir
            .store(jdk::parse_boolean(Some(params)), Ordering::SeqCst);
        Ok(())
    }

    fn supports_params(&self) -> bool {
        true
    }

    fn tear_down(&self) -> Result<()> {
        if let Some(input_dir) = jdk::lock(&self.input_dir).as_ref() {
            input_dir.close()?;
        } else {
            return Err(LuceneError::null_pointer());
        }
        perf_task::tear_down(self)
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "AddIndexesTask"
    }
}
