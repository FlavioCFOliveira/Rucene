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

//! The types declared in `WriteEnwikiLineDocTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rucene::document::Document;
use rucene::error::Result;

use crate::by_task::feeds::doc_maker::DocMaker;
use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::by_task::tasks::write_line_doc_task::{LineFileOut, WriteLineDocBase};

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.WriteEnwikiLineDocTask`
/// (`public class WriteEnwikiLineDocTask extends WriteLineDocTask`): a
/// `WriteLineDocTask` which for Wikipedia input, will write category pages to
/// another file, while remaining pages will be written to the original file.
/// The categories file is derived from the original file, by adding a prefix
/// "categories-".
pub struct WriteEnwikiLineDocTask {
    base: PerfTaskBase,
    line: WriteLineDocBase,
    category_line_file_out: LineFileOut,
}

impl WriteEnwikiLineDocTask {
    /// `WriteEnwikiLineDocTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// As `WriteLineDocTask`, and the `IOException` of creating the
    /// categories file.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        let base = PerfTaskBase::new(Arc::clone(&run_data), "WriteEnwikiLineDocTask")?;
        let line = WriteLineDocBase::new(&run_data)?;
        line.write_header(&line.line_file_out());
        let category_line_file_out = LineFileOut::open(&Self::categories_line_file(Path::new(&line.fname)))?;
        line.write_header(&category_line_file_out);
        Ok(WriteEnwikiLineDocTask { base, line, category_line_file_out })
    }

    /// `categoriesLineFile(Path f)`: compose categories line file out of
    /// original line file.
    pub fn categories_line_file(f: &Path) -> PathBuf {
        let absolute = std::path::absolute(f).unwrap_or_else(|_| f.to_path_buf());
        let dir = absolute.parent().map(Path::to_path_buf).unwrap_or_default();
        let categories_name = format!(
            "categories-{}",
            f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        );
        dir.join(categories_name)
    }

    /// `lineFileOut(Document doc)`.
    fn line_file_out(&self, doc: &Document) -> LineFileOut {
        let is_category = doc
            .get_field(DocMaker::TITLE_FIELD)
            .and_then(|f| f.string_value())
            .is_some_and(|title| title.starts_with("Category:"));
        if is_category {
            return self.category_line_file_out.clone();
        }
        self.line.line_file_out()
    }
}

impl PerfTask for WriteEnwikiLineDocTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(WriteEnwikiLineDocTask {
            base: self.base.clone_base(),
            line: self.line.clone_base(),
            category_line_file_out: self.category_line_file_out.clone(),
        }))
    }

    fn get_log_message(&self, recs_count: i32) -> String {
        format!("Wrote {recs_count} line docs")
    }

    fn do_logic(&self) -> Result<i32> {
        self.line.do_logic(|doc| self.line_file_out(doc))
    }

    fn close(&self) -> Result<()> {
        self.category_line_file_out.close();
        self.line.line_file_out().close();
        Ok(())
    }

    fn set_params(&self, params: &str) -> Result<()> {
        perf_task::set_params(self, params)?;
        self.line.set_params(params)
    }

    fn supports_params(&self) -> bool {
        true
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        matches!(simple_class_name, "WriteEnwikiLineDocTask" | "WriteLineDocTask")
    }
}
