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

//! The members of `org.apache.lucene.tests.store.MockDirectoryWrapper` that
//! `LuceneTestCase`'s static environment reads, ported in full under the
//! sprint's cycle rule; task #343 ports the rest of the class.

/// Port of the enum `MockDirectoryWrapper.Throttling`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Throttling {
    /// `ALWAYS`: always emulate a slow hard disk. Could be very slow!
    Always,
    /// `SOMETIMES`: sometimes (0.5% of the time) emulate a slow hard disk.
    Sometimes,
    /// `NEVER`: never throttle output.
    Never,
}

impl std::fmt::Display for Throttling {
    /// `Enum.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Throttling::Always => "ALWAYS",
            Throttling::Sometimes => "SOMETIMES",
            Throttling::Never => "NEVER",
        })
    }
}
