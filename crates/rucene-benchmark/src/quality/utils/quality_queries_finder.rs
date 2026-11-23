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

//! The types declared in `QualityQueriesFinder.java`, of the Java package
//! `org.apache.lucene.benchmark.quality.utils`.

use std::path::PathBuf;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_reader::IndexReader;
use rucene::index::multi_fields::MultiTerms;
use rucene::store::{Directory, FSDirectory};
use rucene::util::extra::{PriorityQueue, PriorityQueueComparator};

use crate::jdk::NEW_LINE;

/// Equivalent to `org.apache.lucene.benchmark.quality.utils.QualityQueriesFinder`:
/// suggest Quality queries based on an index contents. Utility class, used
/// for making quality test benchmarks.
pub struct QualityQueriesFinder {
    dir: Arc<dyn Directory>,
}

/// Equivalent to `org.apache.lucene.benchmark.quality.utils.QualityQueriesFinder.TermDf`
/// (`private static class TermDf`).
pub(crate) struct TermDf {
    word: String,
    df: i32,
}

/// Equivalent to `org.apache.lucene.benchmark.quality.utils.QualityQueriesFinder.TermsDfQueue`
/// (`private static class TermsDfQueue extends PriorityQueue<TermDf>`): its
/// `lessThan`, the queue being a core `PriorityQueue` over it.
pub(crate) struct TermsDfQueue;

impl PriorityQueueComparator<TermDf> for TermsDfQueue {
    fn less_than(&self, tf1: &TermDf, tf2: &TermDf) -> bool {
        tf1.df < tf2.df
    }
}

impl QualityQueriesFinder {
    /// `QualityQueriesFinder(Directory dir)`: constructor over a directory
    /// containing the index we search for the quality test.
    fn new(dir: Arc<dyn Directory>) -> Self {
        QualityQueriesFinder { dir }
    }

    /// `main(String[] args)`: args {index-dir}.
    ///
    /// # Errors
    ///
    /// If cannot access the index.
    pub fn main(args: &[String]) -> Result<()> {
        if args.is_empty() {
            eprintln!("Usage: java QualityQueriesFinder <index-dir>");
            std::process::exit(1);
        }
        let qqf = QualityQueriesFinder::new(Arc::new(FSDirectory::open(PathBuf::from(&args[0]))?));
        let q = qqf.best_queries("body", 20)?;
        for (i, query) in q.iter().enumerate() {
            println!("{NEW_LINE}{}", Self::format_query_as_trec_topic(i as i32, Some(query), None, None));
        }
        Ok(())
    }

    /// `bestQueries(String field, int numQueries)`.
    fn best_queries(&self, _field: &str, num_queries: i32) -> Result<Vec<String>> {
        let words = self.best_terms("body", 4 * num_queries)?;
        let n = words.len();
        let m = n / 4;
        let mut res = Vec::with_capacity(m);
        for i in 0..m {
            res.push(format!("{} {}  {} {}", words[i], words[m + i], words[n - 1 - m - i], words[n - 1 - i]));
            // System.out.println("query["+i+"]:  "+res[i]);
        }
        Ok(res)
    }

    /// `formatQueryAsTrecTopic(int qnum, String title, String description,
    /// String narrative)`.
    fn format_query_as_trec_topic(qnum: i32, title: Option<&str>, description: Option<&str>, narrative: Option<&str>) -> String {
        let nl = NEW_LINE;
        format!(
            "<top>{nl}<num> Number: {qnum}{nl}{nl}<title> {}{nl}{nl}<desc> Description:{nl}{}{nl}{nl}<narr> Narrative:{nl}{}{nl}{nl}</top>",
            title.unwrap_or(""),
            description.unwrap_or(""),
            narrative.unwrap_or("")
        )
    }

    /// `bestTerms(String field, int numTerms)`.
    fn best_terms(&self, field: &str, num_terms: i32) -> Result<Vec<String>> {
        let mut pq = PriorityQueue::new(num_terms.max(0) as usize, TermsDfQueue)?;
        let ir: Arc<dyn IndexReader> = directory_reader::open(Arc::clone(&self.dir))?;
        let result = (|| -> Result<()> {
            let threshold = ir.max_doc() / 10; // ignore words too common.
            if let Some(terms) = MultiTerms::get_terms(&ir, field)? {
                let mut terms_enum = terms.iterator()?;
                while terms_enum.next()?.is_some() {
                    let df = terms_enum.doc_freq()?;
                    if df < threshold {
                        let ttxt = terms_enum.term()?.expect("positioned").utf8_to_string()?;
                        pq.insert_with_overflow(TermDf { word: ttxt, df });
                    }
                }
            }
            Ok(())
        })();
        let closed = ir.close();
        result?;
        closed?;
        let mut res = Vec::with_capacity(pq.size());
        let mut i = 0;
        while pq.size() > 0 {
            let tdf = pq.pop().expect("size > 0");
            i += 1;
            println!("{i}.   word:  {}   {}", tdf.df, tdf.word);
            res.push(tdf.word);
        }
        Ok(res)
    }
}
