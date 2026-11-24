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

//! The types declared in `ReutersContentSource.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use std::any::Any;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};

use crate::by_task::feeds::content_items_source::{ContentItemsSource, ContentItemsSourceBase};
use crate::by_task::feeds::content_source::ContentSource;
use crate::by_task::feeds::dir_content_source::{read_collection_file, DateFormatInfo};
use crate::by_task::feeds::doc_data::DocData;
use crate::by_task::utils::config::Config;
use crate::constants::Constants;
use crate::jdk::{self, ThreadLocal};

#[derive(Default)]
struct State {
    data_dir: Option<PathBuf>,
    input_files: Vec<PathBuf>,
    doc_count_arr: Vec<i32>,
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.ReutersContentSource`
/// (`public class ReutersContentSource extends ContentSource`): a
/// [`ContentSource`] reading from the Reuters collection.
///
/// Config properties:
///
/// * **work.dir** - path to the root of docs and indexes dirs (default
///   **work**).
/// * **docs.dir** - path to the docs dir (default **reuters-out**).
#[derive(Default)]
pub struct ReutersContentSource {
    base: ContentItemsSourceBase,
    date_format: ThreadLocal<DateFormatInfo>,
    state: Mutex<State>,
    doc_count_arr_created: AtomicBool,
}

impl ReutersContentSource {
    /// `new ReutersContentSource()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `getDateFormatInfo()`.
    fn get_date_format_info(&self) -> Result<Arc<Mutex<DateFormatInfo>>> {
        match self.date_format.get() {
            Some(dfi) => Ok(dfi),
            None => Ok(self
                .date_format
                .set(Some(DateFormatInfo::new()?))
                .expect("just set")),
        }
    }

    /// `docCountArrInit()`.
    fn doc_count_arr_init(&self) -> Result<()> {
        let mut s = jdk::lock(&self.state);
        if !self.doc_count_arr_created.load(Ordering::SeqCst) {
            let config = self
                .base
                .get_config()
                .ok_or_else(LuceneError::null_pointer)?;
            s.doc_count_arr = vec![0; config.get_num_threads().max(0) as usize];
            self.doc_count_arr_created.store(true, Ordering::SeqCst);
        }
        Ok(())
    }
}

impl ContentItemsSource for ReutersContentSource {
    fn items_base(&self) -> &ContentItemsSourceBase {
        &self.base
    }

    fn java_simple_name(&self) -> &'static str {
        "ReutersContentSource"
    }

    fn close(&self) -> Result<()> {
        // TODO implement?
        Ok(())
    }

    fn reset_inputs(&self) -> Result<()> {
        let _s = jdk::lock(&self.state);
        self.base.reset_inputs();
        Ok(())
    }

    fn set_config(&self, config: Arc<Config>) -> Result<()> {
        self.base.set_config(Arc::clone(&config))?;
        let work_dir = PathBuf::from(
            config
                .get_str("work.dir", Some("work"))?
                .unwrap_or_default(),
        );
        let d = config
            .get_str("docs.dir", Some("reuters-out"))?
            .unwrap_or_default();
        let mut data_dir = PathBuf::from(&d);
        if !data_dir.is_absolute() {
            data_dir = work_dir.join(&d);
        }
        let mut s = jdk::lock(&self.state);
        s.input_files.clear();
        ContentItemsSourceBase::collect_files(&data_dir, &mut s.input_files)
            .map_err(jdk::runtime_exception_from)?;
        if s.input_files.is_empty() {
            let absolute = std::path::absolute(&data_dir).unwrap_or_else(|_| data_dir.clone());
            return Err(jdk::runtime_exception(format!(
                "No txt files in dataDir: {}",
                absolute.display()
            )));
        }
        s.data_dir = Some(data_dir);
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ContentSource for ReutersContentSource {
    fn get_next_doc_data(&self, doc_data: &mut DocData) -> Result<()> {
        if !self.doc_count_arr_created.load(Ordering::SeqCst) {
            self.doc_count_arr_init()?;
        }
        let thread_name = jdk::current_thread_name();
        let thread_index_size = jdk::utf16_len(&thread_name);
        let parallel_task_thread_size = Constants::PARALLEL_TASK_THREAD_NAME_PREFIX.len();
        let (f, iteration) = {
            let mut s = jdk::lock(&self.state);
            let mut thread_index = 0usize;
            if s.doc_count_arr.len() > 1 {
                // Extract ThreadIndex from unique ThreadName which is set with '"ParallelTaskThread-"+index',
                // in TaskSequence.java's doParallelTasks()
                let begin = parallel_task_thread_size + 1;
                if begin > thread_index_size {
                    return Err(LuceneError::unchecked(
                        "java.lang.StringIndexOutOfBoundsException",
                        format!(
                            "begin {begin}, end {thread_index_size}, length {thread_index_size}"
                        ),
                    ));
                }
                thread_index = jdk::parse_int(&thread_name[begin..thread_index_size])? as usize;
            }
            let len = s.doc_count_arr.len();
            let count = *s.doc_count_arr.get(thread_index).ok_or_else(|| {
                LuceneError::unchecked(
                    "java.lang.ArrayIndexOutOfBoundsException",
                    format!("Index {thread_index} out of bounds for length {len}"),
                )
            })?;
            let stride = thread_index as i64 + i64::from(count) * len as i64;
            let in_file_size = s.input_files.len() as i64;
            // Modulo Operator covers all three possible senarios i.e. 1. If inputFiles.size() < Num Of
            // Threads 2.inputFiles.size() == Num Of Threads 3.inputFiles.size() > Num Of Threads
            let file_index = (stride % in_file_size) as usize;
            let iteration = stride / in_file_size;
            s.doc_count_arr[thread_index] += 1;
            (s.input_files[file_index].clone(), iteration)
        };

        let name = format!("{}_{}", jdk::real_path_string(&f)?, iteration);
        let (date_str, title, body_buf) = read_collection_file(&f)?;
        self.base.add_bytes(std::fs::metadata(&f)?.len() as i64);
        let date_str = date_str.ok_or_else(LuceneError::null_pointer)?;
        let dfi = self.get_date_format_info()?;
        let date = jdk::lock(&dfi).parse_date(jdk::java_trim(&date_str))?;
        doc_data.clear();
        doc_data.set_name(Some(name));
        doc_data.set_body(Some(body_buf));
        doc_data.set_title(title);
        doc_data.set_date_from(date.as_ref());
        Ok(())
    }
}
