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

//! The types declared in `Benchmark.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask`.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, LuceneErrorKind, Result};

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::utils::algorithm::Algorithm;
use crate::by_task::utils::config::Config;
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.Benchmark`: run the
/// benchmark algorithm.
///
/// Usage: java Benchmark algorithm-file
///
/// 1. Read algorithm.
/// 2. Run the algorithm.
///
/// Things to be added/fixed in "Benchmarking by tasks":
///
/// 1. TODO - report into Excel and/or graphed view.
/// 2. TODO - perf comparison between Lucene releases over the years.
/// 3. TODO - perf report adequate to include in Lucene nightly build site?
///    (so we can easily track performance changes.)
/// 4. TODO - add overall time control for repeated execution (vs. current
///    by-count only).
/// 5. TODO - query maker that is based on index statistics.
pub struct Benchmark {
    run_data: Arc<PerfRunData>,
    algorithm: Algorithm,
    executed: Mutex<bool>,
}

/// `new Exception(message, cause)`.
fn exception_with_cause(message: &str, cause: LuceneError) -> LuceneError {
    LuceneError::from(LuceneErrorKind::Unchecked {
        class: "java.lang.Exception",
        message: Some(message.to_string()),
        cause: Some(Arc::new(cause)),
    })
}

impl Benchmark {
    /// `Benchmark(Reader algReader)`.
    ///
    /// # Errors
    ///
    /// `Exception("Error: cannot init PerfRunData!", e)` or
    /// `Exception("Error: cannot understand algorithm!", e)`.
    pub fn new(alg_reader: &str) -> Result<Benchmark> {
        // prepare run data
        let run_data = match Config::from_reader(alg_reader).and_then(|c| PerfRunData::new(Arc::new(c))) {
            Ok(r) => r,
            Err(e) => {
                // e.printStackTrace()
                eprintln!("{e}");
                return Err(exception_with_cause("Error: cannot init PerfRunData!", e));
            }
        };

        // parse algorithm
        let algorithm = Algorithm::new(Arc::clone(&run_data))
            .map_err(|e| exception_with_cause("Error: cannot understand algorithm!", e))?;
        Ok(Benchmark { run_data, algorithm, executed: Mutex::new(false) })
    }

    /// `execute()`: execute this benchmark.
    ///
    /// # Errors
    ///
    /// `IllegalStateException("Benchmark was already executed")`, or the
    /// exception of the algorithm.
    pub fn execute(&self) -> Result<()> {
        let mut executed = jdk::lock(&self.executed);
        if *executed {
            return Err(LuceneError::illegal_state("Benchmark was already executed"));
        }
        *executed = true;
        self.run_data.set_start_time_millis();
        self.algorithm.execute()
    }

    /// `main(String[] args)`: run the benchmark algorithm.
    pub fn main(args: &[String]) {
        Self::exec(args);
    }

    /// `exec(String[] args)`: utility: execute benchmark from command line;
    /// a single argument is expected: algorithm-file.
    pub fn exec(args: &[String]) {
        // verify command line args
        if args.is_empty() {
            eprintln!("Usage: java Benchmark <algorithm file>");
            std::process::exit(1);
        }

        // verify input files
        let alg_file = Path::new(&args[0]);
        let absolute = std::path::absolute(alg_file).unwrap_or_else(|_| alg_file.to_path_buf());
        if std::fs::File::open(alg_file).is_err() || alg_file.is_dir() {
            eprintln!("cannot find/read algorithm file: {}", absolute.display());
            std::process::exit(1);
        }

        println!("Running algorithm from: {}", absolute.display());

        let benchmark = match std::fs::read(alg_file)
            .map_err(LuceneError::from)
            .and_then(|bytes| {
                // Files.newBufferedReader(algFile, UTF_8) throws
                // MalformedInputException on bytes that are not UTF-8.
                String::from_utf8(bytes)
                    .map_err(|_| LuceneError::from(std::io::Error::other("Input length = 1")))
            })
            .and_then(|text| Benchmark::new(&text))
        {
            Ok(b) => b,
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        };

        println!("------------> algorithm:");
        println!("{}", benchmark.get_algorithm());

        // execute
        if let Err(e) = benchmark.execute() {
            eprintln!("Error: cannot execute the algorithm! {}", jdk::get_message(&e).unwrap_or_else(|| "null".to_string()));
            eprintln!("{e}");
        }

        println!("####################");
        println!("###  D O N E !!! ###");
        println!("####################");
    }

    /// `getAlgorithm()`.
    pub fn get_algorithm(&self) -> &Algorithm {
        &self.algorithm
    }

    /// `getRunData()`.
    pub fn get_run_data(&self) -> &Arc<PerfRunData> {
        &self.run_data
    }
}
