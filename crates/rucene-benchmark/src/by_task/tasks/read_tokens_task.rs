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

//! The types declared in `ReadTokensTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use rucene::analysis::tokenattributes::TermToBytesRefAttribute;
use rucene::document::Document;
use rucene::error::{LuceneError, Result};
use rucene::index::IndexOptions;
use rucene::util::BytesRef;

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.ReadTokensTask`
/// (`public class ReadTokensTask extends PerfTask`): simple task to test
/// performance of tokenizers. It just creates a token stream for each field
/// of the document and read all tokens out of that stream.
pub struct ReadTokensTask {
    base: PerfTaskBase,
    total_token_count: AtomicI32,
    /// volatile data passed between setup(), doLogic(), tearDown().
    doc: Mutex<Option<Document>>,
    /// `stringReader`: a reader Java declares and never uses.
    string_reader: Mutex<ReusableStringReader>,
}

impl ReadTokensTask {
    /// `ReadTokensTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(ReadTokensTask {
            base: PerfTaskBase::new(run_data, "ReadTokensTask")?,
            total_token_count: AtomicI32::new(0),
            doc: Mutex::new(None),
            string_reader: Mutex::new(ReusableStringReader::default()),
        })
    }
}

impl PerfTask for ReadTokensTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        let doc = jdk::lock(&self.doc).as_ref().map(|d| {
            let mut copy = Document::new();
            for f in d.get_fields() {
                copy.add_shared(Arc::clone(f));
            }
            copy
        });
        Ok(Arc::new(ReadTokensTask {
            base: self.base.clone_base(),
            total_token_count: AtomicI32::new(self.total_token_count.load(Ordering::SeqCst)),
            doc: Mutex::new(doc),
            string_reader: Mutex::new(jdk::lock(&self.string_reader).clone()),
        }))
    }

    fn setup(&self) -> Result<()> {
        let doc_maker = self.base.get_run_data().get_doc_maker();
        *jdk::lock(&self.doc) = Some(doc_maker.make_document()?);
        Ok(())
    }

    fn get_log_message(&self, recs_count: i32) -> String {
        format!(
            "read {recs_count} docs; {} tokens",
            self.total_token_count.load(Ordering::SeqCst)
        )
    }

    fn tear_down(&self) -> Result<()> {
        *jdk::lock(&self.doc) = None;
        perf_task::tear_down(self)
    }

    fn do_logic(&self) -> Result<i32> {
        let doc = jdk::lock(&self.doc);
        let fields = doc
            .as_ref()
            .ok_or_else(LuceneError::null_pointer)?
            .get_fields();
        let analyzer = self.base.get_run_data().get_analyzer();
        let mut token_count = 0;
        // PERF: Java's `termAtt.getBytesRef()` returns the attribute's own
        // `BytesRef`; the port's returns an owned copy, so the term is encoded
        // into this one reused buffer instead (`copy_bytes_ref_to`, the same
        // UTF-8 bytes), with no allocation per token.
        let mut term_bytes = BytesRef::default();
        for field in fields {
            if field.field_type().index_options() == IndexOptions::NONE
                || !field.field_type().tokenized()
            {
                continue;
            }

            let mut reuse = None;
            let mut stream = field.token_stream(analyzer.as_deref(), &mut reuse)?;
            // reset the TokenStream to the first token
            stream.reset()?;

            while stream.increment_token()? {
                let term_att = stream
                    .attribute_source()
                    .get_attribute_interface::<dyn TermToBytesRefAttribute>()
                    .ok_or_else(|| LuceneError::illegal_argument("TermToBytesRefAttribute"))?;
                term_att.copy_bytes_ref_to(&mut term_bytes);
                token_count += 1;
            }
            stream.end()?;
            stream.close()?;
        }
        self.total_token_count
            .fetch_add(token_count, Ordering::SeqCst);
        Ok(token_count)
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "ReadTokensTask"
    }
}

/// Equivalent to
/// `org.apache.lucene.benchmark.byTask.tasks.ReadTokensTask.ReusableStringReader`
/// (`private static final class ReusableStringReader extends Reader`):
/// simple StringReader that can be reset to a new string; we use this when
/// tokenizing the string value from a Field.
#[derive(Clone, Debug, Default)]
#[allow(dead_code)] // Java declares the reader and never uses it.
pub(crate) struct ReusableStringReader {
    upto: usize,
    left: usize,
    /// `s`, as UTF-16 code units.
    s: Vec<u16>,
}

#[allow(dead_code)] // Java declares the reader and never uses it.
impl ReusableStringReader {
    /// `read(char[] c)`.
    pub(crate) fn read(&mut self, c: &mut [u16]) -> i32 {
        let len = c.len();
        self.read_range(c, 0, len)
    }

    /// `read(char[] c, int off, int len)`.
    pub(crate) fn read_range(&mut self, c: &mut [u16], off: usize, len: usize) -> i32 {
        if self.left > len {
            c[off..off + len].copy_from_slice(&self.s[self.upto..self.upto + len]);
            self.upto += len;
            self.left -= len;
            len as i32
        } else if self.left == 0 {
            -1
        } else {
            c[off..off + self.left].copy_from_slice(&self.s[self.upto..self.upto + self.left]);
            let r = self.left;
            self.left = 0;
            self.upto = self.s.len();
            r as i32
        }
    }

    /// `close()`.
    pub(crate) fn close(&mut self) {}
}
