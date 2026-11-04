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

//! The members of `org.apache.lucene.tests.util.TestRuleSetupAndRestoreClassEnv`
//! that `TestRuleTemporaryFilesCleanup.initializeFileSystem` calls, ported in
//! full under the sprint's cycle rule; task #348 ports the rest of the class.

use randomizedtesting::jdk::system;
use randomizedtesting::jdk::thread::Thread;
use rucene::util::{InfoStream, PrintStreamInfoStream};

/// Port of the nested class
/// `TestRuleSetupAndRestoreClassEnv.ThreadNameFixingPrintStreamInfoStream`: a
/// `PrintStreamInfoStream` over `System.out` that ignores test points and
/// names the test thread `main`.
pub struct ThreadNameFixingPrintStreamInfoStream {
    base: PrintStreamInfoStream,
}

impl ThreadNameFixingPrintStreamInfoStream {
    /// `new ThreadNameFixingPrintStreamInfoStream(System.out)`.
    pub fn new_system_out() -> ThreadNameFixingPrintStreamInfoStream {
        ThreadNameFixingPrintStreamInfoStream {
            base: PrintStreamInfoStream::system(Box::new(std::io::sink())),
        }
    }
}

impl InfoStream for ThreadNameFixingPrintStreamInfoStream {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.util.TestRuleSetupAndRestoreClassEnv$ThreadNameFixingPrintStreamInfoStream"
    }

    /// `message(component, message)`: "ignore test points!" (`TP`); "The name
    /// of the main thread is way too long when looking at IW verbose output",
    /// so a `TEST-` thread is named `main`.
    fn message(&self, component: &str, message: &str) {
        if component == "TP" {
            return;
        }
        let current = Thread::current().name();
        let name = if current.starts_with("TEST-") {
            "main".to_string()
        } else {
            current
        };
        system::out_println(&format!(
            "{component} {} [{}; {name}]: {message}",
            self.base.message_id(),
            self.base.get_timestamp()
        ));
    }

    /// `PrintStreamInfoStream.isEnabled(component)`: always.
    fn is_enabled(&self, _component: &str) -> bool {
        true
    }

    /// `PrintStreamInfoStream.close()`: `System.out` is not closed.
    fn close(&self) {}
}
