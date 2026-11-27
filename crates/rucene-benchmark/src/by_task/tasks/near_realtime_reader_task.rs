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

//! The types declared in `NearRealtimeReaderTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rucene::error::Result;
use rucene::index::directory_reader;

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::jdk;

#[derive(Clone)]
struct State {
    pause_msec: i64,
    reopen_count: i32,
    reopen_times: Vec<i32>,
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.NearRealtimeReaderTask`
/// (`public class NearRealtimeReaderTask extends PerfTask`): spawns a BG
/// thread that periodically (defaults to 3.0 seconds, but accepts param in
/// seconds) wakes up and asks IndexWriter for a near real-time reader. Then
/// runs a single query (body: 1) sorted by docdate, and prints time to
/// reopen and time to run the search.
///
/// lucene.experimental: it's also not generally usable, eg you cannot
/// change which query is executed.
pub struct NearRealtimeReaderTask {
    base: PerfTaskBase,
    state: Mutex<State>,
}

impl NearRealtimeReaderTask {
    /// `NearRealtimeReaderTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(NearRealtimeReaderTask {
            base: PerfTaskBase::new(run_data, "NearRealtimeReaderTask")?,
            state: Mutex::new(State {
                pause_msec: 3000,
                reopen_count: 0,
                reopen_times: vec![0; 1],
            }),
        })
    }
}

impl PerfTask for NearRealtimeReaderTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(NearRealtimeReaderTask {
            base: self.base.clone_base(),
            state: Mutex::new(jdk::lock(&self.state).clone()),
        }))
    }

    fn do_logic(&self) -> Result<i32> {
        let run_data = self.base.get_run_data();

        // Get initial reader
        let Some(w) = run_data.get_index_writer() else {
            return Err(jdk::runtime_exception(
                "please open the writer before invoking NearRealtimeReader",
            ));
        };

        if let Some(existing) = run_data.get_index_reader()? {
            // Java's getIndexReader() took a reference it never releases.
            let _ = existing;
            return Err(jdk::runtime_exception(
                "please close the existing reader before invoking NearRealtimeReader",
            ));
        }

        let mut t = jdk::current_time_millis();
        let mut r = directory_reader::open_with_writer(&*w)?;
        run_data.set_index_reader(Some(Arc::clone(&r)))?;
        // Transfer our reference to runData
        r.dec_ref()?;

        // TODO: gather basic metrics for reporting -- eg mean,
        // stddev, min/max reopen latencies

        // Parent sequence sets stopNow
        let pause_msec = {
            let mut s = jdk::lock(&self.state);
            s.reopen_count = 0;
            s.pause_msec
        };
        while !self.base.is_stop_now() {
            let wait_for_msec = pause_msec - (jdk::current_time_millis() - t);
            if wait_for_msec > 0 {
                std::thread::sleep(Duration::from_millis(wait_for_msec as u64));
            }

            t = jdk::current_time_millis();
            let new_reader = directory_reader::open_if_changed(Arc::clone(&r))?;
            if let Some(new_reader) = new_reader {
                let delay = (jdk::current_time_millis() - t) as i32;
                {
                    let mut s = jdk::lock(&self.state);
                    let count = s.reopen_count as usize;
                    if s.reopen_times.len() == count {
                        s.reopen_times = rucene::util::ArrayUtil::grow_int(
                            std::mem::take(&mut s.reopen_times),
                            1 + count,
                        );
                    }
                    s.reopen_times[count] = delay;
                    s.reopen_count += 1;
                }
                // TODO: somehow we need to enable warming, here
                run_data.set_index_reader(Some(Arc::clone(&new_reader)))?;
                // Transfer our reference to runData
                new_reader.dec_ref()?;
                r = new_reader;
            }
        }
        self.base.set_stop_now(false);
        Ok(jdk::lock(&self.state).reopen_count)
    }

    fn set_params(&self, params: &str) -> Result<()> {
        perf_task::set_params(self, params)?;
        jdk::lock(&self.state).pause_msec = (1000.0 * f64::from(jdk::parse_float(params)?)) as i64;
        Ok(())
    }

    fn close(&self) -> Result<()> {
        let s = jdk::lock(&self.state);
        println!("NRT reopen times:");
        let mut line = String::new();
        for time in &s.reopen_times[..s.reopen_count as usize] {
            line.push_str(&format!(" {time}"));
        }
        println!("{line}");
        Ok(())
    }

    fn supports_params(&self) -> bool {
        true
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "NearRealtimeReaderTask"
    }
}
