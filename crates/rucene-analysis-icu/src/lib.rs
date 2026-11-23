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

//! This crate ports Apache Lucene 10.5.0 `lucene/analysis/icu` to Rust: the
//! analysis integration with ICU (International Components for Unicode),
//! over the workspace's port of ICU4J 77.1 (`icu4j`).
//!
//! Every type of the module is ported here, and carries its port claim
//! (`Equivalent to ...`). The module tree mirrors the Java packages:
//! `org.apache.lucene.analysis.icu` is [`analysis::icu`], with its
//! subpackages `segmentation` and `tokenattributes`.
//!
//! Beside the packages, [`spi`] holds the module's `META-INF/services` files
//! (the factory registry) and [`resources`] its data files (`utr30.nrm`,
//! `segmentation/Default.brk`, `segmentation/MyanmarSyllable.brk`), embedded
//! byte for byte. The two command-line tools of `src/tools`,
//! `RBBIRuleCompiler` and `GenerateUTR30DataFiles`, are the crate's binaries
//! `rbbi_rule_compiler` and `generate_utr30_data_files`.
//!
//! The JPMS descriptor `module-info.java` (`module
//! org.apache.lucene.analysis.icu`) has no Rust counterpart beyond this
//! crate: its `requires` are the crate's dependencies, its `exports` the
//! public modules, and its `provides` the factory registry of [`spi`].

pub mod analysis;
pub mod resources;
pub mod spi;
