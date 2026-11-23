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

//! The types declared in `TrecJudge.java`, of the Java package
//! `org.apache.lucene.benchmark.quality.trec`.

use rucene::error::Result;
use rucene::util::java_collections::JavaHashMap;
use rucene_analysis_common::jdk::io::{BufferedReader, ReadChars};

use crate::jdk::{self, PrintWriter};
use crate::quality::judge::Judge;
use crate::quality::quality_query::QualityQuery;

/// Equivalent to
/// `org.apache.lucene.benchmark.quality.trec.TrecJudge.QRelJudgement`
/// (`private static class QRelJudgement`): single Judgement of a trec
/// quality query.
#[derive(Clone, Debug)]
pub(crate) struct QRelJudgement {
    /// `queryID` (`TrecJudge` compares it through `curr`).
    #[allow(dead_code)]
    query_id: String,
    relevant_docs: JavaHashMap<String>,
}

impl QRelJudgement {
    fn new(query_id: &str) -> Self {
        QRelJudgement { query_id: query_id.to_string(), relevant_docs: JavaHashMap::new() }
    }

    /// `addRelevandDoc(String docName)`.
    pub(crate) fn add_relevand_doc(&mut self, doc_name: &str) {
        self.relevant_docs.insert(doc_name.to_string(), doc_name.to_string());
    }

    /// `isRelevant(String docName)`.
    fn is_relevant(&self, doc_name: &str) -> bool {
        self.relevant_docs.contains_key(doc_name)
    }

    /// `maxRecall()`.
    pub(crate) fn max_recall(&self) -> i32 {
        self.relevant_docs.len() as i32
    }
}

/// Equivalent to `org.apache.lucene.benchmark.quality.trec.TrecJudge`
/// (`public class TrecJudge implements Judge`): judge if given document is
/// relevant to given quality query, based on Trec format for judgements.
pub struct TrecJudge {
    judgements: JavaHashMap<QRelJudgement>,
}

impl TrecJudge {
    /// `TrecJudge(BufferedReader reader)`: constructor from a reader.
    ///
    /// Expected input format:
    ///
    /// ```text
    ///     qnum  0   doc-name     is-relevant
    /// ```
    ///
    /// Two sample lines:
    ///
    /// ```text
    ///     19    0   doc303       1
    ///     19    0   doc7295      0
    /// ```
    ///
    /// # Errors
    ///
    /// If there is a low-level I/O error; `NoSuchElementException` for a
    /// line with fewer than four tokens.
    pub fn new<S: ReadChars>(mut reader: BufferedReader<S>) -> Result<Self> {
        let mut judgements: JavaHashMap<QRelJudgement> = JavaHashMap::new();
        let mut curr: Option<String> = None;
        let zero = "0";
        let result = (|| -> Result<()> {
            while let Some(line) = jdk::read_line(&mut reader)? {
                let line = jdk::java_trim(&line);
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let mut st = jdk::string_tokens(line);
                let query_id = st.next().ok_or_else(jdk::no_such_element)?;
                st.next().ok_or_else(jdk::no_such_element)?;
                let doc_name = st.next().ok_or_else(jdk::no_such_element)?;
                let relevant = zero != st.next().ok_or_else(jdk::no_such_element)?;
                if relevant {
                    // only keep relevant docs
                    if curr.as_deref() != Some(query_id) {
                        if judgements.get(query_id).is_none() {
                            judgements.insert(query_id.to_string(), QRelJudgement::new(query_id));
                        }
                        curr = Some(query_id.to_string());
                    }
                    let key = curr.as_deref().expect("set above");
                    judgements.get_mut(key).expect("present").add_relevand_doc(doc_name);
                }
            }
            Ok(())
        })();
        let closed = reader.close();
        result?;
        closed?;
        Ok(TrecJudge { judgements })
    }
}

impl Judge for TrecJudge {
    fn is_relevant(&self, doc_name: Option<&str>, query: &QualityQuery) -> bool {
        match self.judgements.get(query.get_query_id()) {
            // `relevantDocs.containsKey(null)` is false.
            Some(qrj) => doc_name.is_some_and(|d| qrj.is_relevant(d)),
            None => false,
        }
    }

    fn validate_data(&self, qq: &[QualityQuery], logger: Option<&PrintWriter>) -> bool {
        let mut missing_queries = JavaHashMap::copy_of(&self.judgements);
        let mut missing_judgements: Vec<String> = Vec::new();
        for q in qq {
            let id = q.get_query_id();
            if missing_queries.contains_key(id) {
                missing_queries.remove(id);
            } else {
                missing_judgements.push(id.to_string());
            }
        }
        let mut is_valid = true;
        if !missing_judgements.is_empty() {
            is_valid = false;
            if let Some(logger) = logger {
                logger.println(&format!("WARNING: {} queries have no judgments! - ", missing_judgements.len()));
                for id in &missing_judgements {
                    logger.println(&format!("   {id}"));
                }
            }
        }
        if !missing_queries.is_empty() {
            is_valid = false;
            if let Some(logger) = logger {
                logger.println(&format!("WARNING: {} judgments match no query! - ", missing_queries.len()));
                for id in missing_queries.keys() {
                    logger.println(&format!("   {id}"));
                }
            }
        }
        is_valid
    }

    fn max_recall(&self, query: &QualityQuery) -> i32 {
        match self.judgements.get(query.get_query_id()) {
            Some(qrj) => qrj.max_recall(),
            None => 0,
        }
    }
}
