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

//! The types declared in `Sample.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.programmatic`.

use std::sync::Arc;

use rucene::error::Result;

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::add_doc_task::AddDocTask;
use crate::by_task::tasks::close_index_task::CloseIndexTask;
use crate::by_task::tasks::create_index_task::CreateIndexTask;
use crate::by_task::tasks::perf_task::PerfTask;
use crate::by_task::tasks::rep_sum_by_name_task::RepSumByNameTask;
use crate::by_task::tasks::task_sequence::TaskSequence;
use crate::by_task::utils::config::Config;
use crate::jdk::properties::Properties;

/// Equivalent to `org.apache.lucene.benchmark.byTask.programmatic.Sample`:
/// sample performance test written programmatically - no algorithm file is
/// needed here.
pub struct Sample;

impl Sample {
    /// `main(String[] args)`.
    ///
    /// # Errors
    ///
    /// The exception of building the run data or running the tasks.
    pub fn main(_args: &[String]) -> Result<()> {
        let p = Self::init_props();
        let conf = Arc::new(Config::from_properties(p));
        let run_data = PerfRunData::new(conf)?;

        // 1. top sequence
        let top = TaskSequence::new(Arc::clone(&run_data), None, None, false)?; // top level, not parallel

        // 2. task to create the index
        let create = CreateIndexTask::new(Arc::clone(&run_data))?;
        top.add_task(Arc::new(create));

        // 3. task seq to add 500 docs (order matters - top to bottom - add seq to top, only then add to
        // seq)
        let seq1 = TaskSequence::new(Arc::clone(&run_data), Some("AddDocs"), Some(&top), false)?;
        seq1.set_repetitions(500)?;
        seq1.set_no_child_report();
        top.add_task(seq1.as_task());

        // 4. task to add the doc
        let add_doc = AddDocTask::new(Arc::clone(&run_data))?;
        // addDoc.setParams("1200"); // doc size limit if supported
        seq1.add_task(Arc::new(add_doc)); // order matters 9see comment above)

        // 5. task to close the index
        let close = CloseIndexTask::new(Arc::clone(&run_data))?;
        top.add_task(Arc::new(close));

        // task to report
        let rep = RepSumByNameTask::new(Arc::clone(&run_data))?;
        top.add_task(Arc::new(rep));

        // print algorithm
        println!("{}", top.task_to_string());

        // execute
        top.do_logic()?;
        Ok(())
    }

    /// `initProps()`: sample programmatic settings. Could also read from
    /// file.
    fn init_props() -> Properties {
        let mut p = Properties::new();
        p.set_property("task.max.depth.log", "3");
        p.set_property("max.buffered", "buf:10:10:100:100:10:10:100:100");
        p.set_property("doc.maker", "org.apache.lucene.benchmark.byTask.feeds.ReutersContentSource");
        p.set_property("log.step", "2000");
        p.set_property("doc.delete.step", "8");
        p.set_property("analyzer", "org.apache.lucene.analysis.standard.StandardAnalyzer");
        p.set_property("doc.term.vector", "false");
        p.set_property("directory", "FSDirectory");
        p.set_property("query.maker", "org.apache.lucene.benchmark.byTask.feeds.ReutersQueryMaker");
        p.set_property("doc.stored", "true");
        p.set_property("docs.dir", "reuters-out");
        p.set_property("compound", "cmpnd:true:true:true:true:false:false:false:false");
        p.set_property("doc.tokenized", "true");
        p.set_property("merge.factor", "mrg:10:100:10:100:10:100:10:100");
        p
    }
}
