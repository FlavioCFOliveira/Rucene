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

//! The `query_driver` executable: `java org.apache.lucene.benchmark.quality.trec.QueryDriver <topicsFile> <qrelsFile> <submissionFile> <indexDir> [querySpec]`.
//!
//! The command-line arguments after the program name are passed to the port of
//! `QueryDriver.main(String[] args)`, run as the `java` launcher runs it
//! ([`rucene_benchmark::jdk::run_main`]): an exception thrown out of `main`
//! prints `Exception in thread "main" ` and its stack-trace text on stderr and
//! exits with status 1. A wrong argument count prints the usage and ends the
//! process with `System.exit(1)`.

use std::process::ExitCode;

use rucene_benchmark::jdk::run_main;
use rucene_benchmark::quality::trec::query_driver::QueryDriver;

fn main() -> ExitCode {
    run_main(QueryDriver::main)
}
