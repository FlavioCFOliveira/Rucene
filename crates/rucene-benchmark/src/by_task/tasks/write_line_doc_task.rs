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

//! The types declared in `WriteLineDocTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::collections::HashSet;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use rucene::document::Document;
use rucene::error::{LuceneError, Result};

use crate::by_task::feeds::doc_maker::{DocMaker, DocMakerClass};
use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::by_task::utils::stream_utils::{OutputStream, StreamUtils};
use crate::jdk;

/// A `PrintWriter(BufferedWriter(OutputStreamWriter(out, UTF_8), BUFFER_SIZE))`
/// shared by a task and its clones: `println` synchronizes internally and,
/// as `PrintWriter` does, never throws (an I/O failure only sets its error
/// state).
#[derive(Clone)]
pub struct LineFileOut {
    out: Arc<Mutex<Option<BufWriter<OutputStream>>>>,
}

impl LineFileOut {
    /// Opens `file` through [`StreamUtils::output_stream`].
    ///
    /// # Errors
    ///
    /// The `IOException` of creating the file.
    pub fn open(file: &Path) -> Result<Self> {
        let out = StreamUtils::output_stream(file)?;
        Ok(LineFileOut { out: Arc::new(Mutex::new(Some(BufWriter::with_capacity(StreamUtils::BUFFER_SIZE, out)))) })
    }

    /// `println(String)`.
    pub fn println(&self, line: &str) {
        if let Some(w) = jdk::lock(&self.out).as_mut() {
            let _ = w.write_all(line.as_bytes());
            let _ = w.write_all(jdk::NEW_LINE.as_bytes());
        }
    }

    /// `close()`: flushes and closes the stream, ignoring failures.
    pub fn close(&self) {
        if let Some(w) = jdk::lock(&self.out).take() {
            if let Ok(out) = w.into_inner() {
                let _ = out.close();
            }
        }
    }
}

/// The state of `WriteLineDocTask`, which `WriteEnwikiLineDocTask`
/// inherits.
pub struct WriteLineDocBase {
    doc_size: AtomicI32,
    /// `fname`.
    pub fname: String,
    line_file_out: LineFileOut,
    doc_maker: Arc<dyn DocMakerClass>,
    fields_to_write: Vec<String>,
    sufficient_fields: Vec<bool>,
    check_sufficient_fields: bool,
}

impl WriteLineDocBase {
    /// The `WriteLineDocTask(PerfRunData runData)` constructor body, without
    /// the header.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a missing `line.file.out` or a
    /// `line.fields` holding the separator; the `IOException` of creating
    /// the file.
    pub fn new(run_data: &PerfRunData) -> Result<Self> {
        let config = run_data.get_config();
        let Some(fname) = config.get_str("line.file.out", None)? else {
            return Err(LuceneError::illegal_argument("line.file.out must be set"));
        };
        let line_file_out = LineFileOut::open(Path::new(&fname))?;
        let doc_maker = run_data.get_doc_maker();

        // init fields
        let f2r = config.get_str("line.fields", None)?;
        let fields_to_write: Vec<String> = match f2r {
            None => WriteLineDocTask::DEFAULT_FIELDS.iter().map(|s| s.to_string()).collect(),
            Some(f2r) => {
                if f2r.contains(WriteLineDocTask::SEP) {
                    return Err(LuceneError::illegal_argument(format!(
                        "line.fields {f2r} should not contain the separator char: {}",
                        WriteLineDocTask::SEP
                    )));
                }
                jdk::split(&f2r, ",")
            }
        };

        // init sufficient fields
        let mut sufficient_fields = vec![false; fields_to_write.len()];
        let suff = config
            .get_str("sufficient.fields", Some(&WriteLineDocTask::default_sufficient_fields()))?
            .unwrap_or_default();
        let check_sufficient_fields = if suff == "," {
            false
        } else {
            let sf: HashSet<String> = jdk::split(&suff, ",").into_iter().collect();
            for (i, f) in fields_to_write.iter().enumerate() {
                if sf.contains(f) {
                    sufficient_fields[i] = true;
                }
            }
            true
        };

        Ok(WriteLineDocBase {
            doc_size: AtomicI32::new(0),
            fname,
            line_file_out,
            doc_maker,
            fields_to_write,
            sufficient_fields,
            check_sufficient_fields,
        })
    }

    /// `super.clone()`: the clone shares the writer and the doc maker.
    pub fn clone_base(&self) -> Self {
        WriteLineDocBase {
            doc_size: AtomicI32::new(self.doc_size.load(Ordering::SeqCst)),
            fname: self.fname.clone(),
            line_file_out: self.line_file_out.clone(),
            doc_maker: Arc::clone(&self.doc_maker),
            fields_to_write: self.fields_to_write.clone(),
            sufficient_fields: self.sufficient_fields.clone(),
            check_sufficient_fields: self.check_sufficient_fields,
        }
    }

    /// `writeHeader(PrintWriter out)`: write header to the lines file -
    /// indicating how to read the file later.
    pub fn write_header(&self, out: &LineFileOut) {
        let mut sb = String::from(WriteLineDocTask::FIELDS_HEADER_INDICATOR);
        for f in &self.fields_to_write {
            sb.push(WriteLineDocTask::SEP);
            sb.push_str(f);
        }
        out.println(&sb);
    }

    /// The line `doLogic()` writes for `doc`, `None` when the document is
    /// not sufficient.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for a field without a string value.
    fn line_of(&self, doc: &Document) -> Result<Option<String>> {
        let mut sb = String::new();
        let mut sufficient = !self.check_sufficient_fields;
        for (i, name) in self.fields_to_write.iter().enumerate() {
            let text = match doc.get_field(name) {
                None => String::new(),
                Some(f) => {
                    let value = f.string_value().ok_or_else(LuceneError::null_pointer)?;
                    // `[\t\r\n]+` replaced by " ", then trimmed.
                    let mut normalized = String::with_capacity(value.len());
                    let mut in_run = false;
                    for c in value.chars() {
                        if c == '\t' || c == '\r' || c == '\n' {
                            if !in_run {
                                normalized.push(' ');
                                in_run = true;
                            }
                        } else {
                            normalized.push(c);
                            in_run = false;
                        }
                    }
                    jdk::java_trim(&normalized).to_string()
                }
            };
            sb.push_str(&text);
            sb.push(WriteLineDocTask::SEP);
            sufficient |= !text.is_empty() && self.sufficient_fields[i];
        }
        if sufficient {
            sb.pop(); // remove redundant last separator
            Ok(Some(sb))
        } else {
            Ok(None)
        }
    }

    /// `doLogic()` with `lineFileOut(doc)` given.
    ///
    /// # Errors
    ///
    /// The exception of the doc maker.
    pub fn do_logic(&self, select: impl FnOnce(&Document) -> LineFileOut) -> Result<i32> {
        let doc_size = self.doc_size.load(Ordering::SeqCst);
        let doc = if doc_size > 0 { self.doc_maker.make_document_of_size(doc_size)? } else { self.doc_maker.make_document()? };
        if let Some(line) = self.line_of(&doc)? {
            // lineFileOut is a PrintWriter, which synchronizes internally in println.
            select(&doc).println(&line);
        }
        Ok(1)
    }

    /// `lineFileOut(Document doc)`: default: original output line file.
    pub fn line_file_out(&self) -> LineFileOut {
        self.line_file_out.clone()
    }

    /// `setParams(String)` after `super.setParams`.
    ///
    /// # Errors
    ///
    /// `NumberFormatException`.
    pub fn set_params(&self, params: &str) -> Result<()> {
        self.doc_size.store(jdk::parse_float(params)? as i32, Ordering::SeqCst);
        Ok(())
    }
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.WriteLineDocTask`
/// (`public class WriteLineDocTask extends PerfTask`): a task which writes
/// documents, one line per document. Each line is in the following format:
/// title <TAB> date <TAB> body. The output of this task can be consumed by
/// `LineDocSource` and is intended to save the IO overhead of opening a file
/// per document to be indexed.
///
/// The format of the output is set according to the output file extension.
/// Compression is recommended when the output file is expected to be large.
/// See info on file extensions in `StreamUtils.Type`.
///
/// Supports the following parameters:
///
/// * **line.file.out** - the name of the file to write the output to. That
///   parameter is mandatory. **NOTE:** the file is re-created.
/// * **line.fields** - which fields should be written in each line.
///   (optional, default: [`DEFAULT_FIELDS`](Self::DEFAULT_FIELDS)).
/// * **sufficient.fields** - list of field names, separated by comma,
///   which, if all of them are missing, the document will be skipped. For
///   example, to require that at least one of f1,f2 is not empty, specify:
///   "f1,f2" in this field. To specify that no field is required, i.e. that
///   even empty docs should be emitted, specify **","**. (optional,
///   default: `DEFAULT_SUFFICIENT_FIELDS`).
///
/// **NOTE:** this class is not thread-safe and if used by multiple threads
/// the output is unspecified (as all will write to the same output file in a
/// non-synchronized way).
pub struct WriteLineDocTask {
    base: PerfTaskBase,
    line: WriteLineDocBase,
}

impl WriteLineDocTask {
    /// `FIELDS_HEADER_INDICATOR`.
    pub const FIELDS_HEADER_INDICATOR: &'static str = "FIELDS_HEADER_INDICATOR###";
    /// `SEP`.
    pub const SEP: char = '\t';
    /// `DEFAULT_FIELDS`: fields to be written by default.
    pub const DEFAULT_FIELDS: [&'static str; 3] = [DocMaker::TITLE_FIELD, DocMaker::DATE_FIELD, DocMaker::BODY_FIELD];

    /// `DEFAULT_SUFFICIENT_FIELDS`: default fields which at least one of them
    /// is required to not skip the doc.
    pub fn default_sufficient_fields() -> String {
        format!("{},{}", DocMaker::TITLE_FIELD, DocMaker::BODY_FIELD)
    }

    /// `WriteLineDocTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// As [`WriteLineDocBase::new`].
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        let base = PerfTaskBase::new(Arc::clone(&run_data), "WriteLineDocTask")?;
        let line = WriteLineDocBase::new(&run_data)?;
        line.write_header(&line.line_file_out);
        Ok(WriteLineDocTask { base, line })
    }
}

impl PerfTask for WriteLineDocTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(WriteLineDocTask { base: self.base.clone_base(), line: self.line.clone_base() }))
    }

    fn get_log_message(&self, recs_count: i32) -> String {
        format!("Wrote {recs_count} line docs")
    }

    fn do_logic(&self) -> Result<i32> {
        self.line.do_logic(|_| self.line.line_file_out())
    }

    fn close(&self) -> Result<()> {
        self.line.line_file_out.close();
        Ok(())
    }

    /// Set the params (docSize only): docSize, or 0 for no limit.
    fn set_params(&self, params: &str) -> Result<()> {
        perf_task::set_params(self, params)?;
        self.line.set_params(params)
    }

    fn supports_params(&self) -> bool {
        true
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "WriteLineDocTask"
    }
}
