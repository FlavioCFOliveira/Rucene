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

//! The `benchmark` executable: `java org.apache.lucene.benchmark.byTask.Benchmark <algorithm file>`.
//!
//! The command-line arguments after the program name are passed to the port of
//! `Benchmark.main(String[] args)`, run as the `java` launcher runs it
//! ([`rucene_benchmark::jdk::run_main`]): an exception thrown out of `main`
//! prints `Exception in thread "main" ` and its stack-trace text on stderr and
//! exits with status 1. `Benchmark.main` reports its own failures and returns;
//! a missing argument or an unreadable algorithm file ends the process with
//! `System.exit(1)`.

use std::process::ExitCode;

use rucene_benchmark::by_task::benchmark::Benchmark;
use rucene_benchmark::jdk::run_main;

fn main() -> ExitCode {
    run_main(|args| {
        Benchmark::main(args);
        Ok(())
    })
}
