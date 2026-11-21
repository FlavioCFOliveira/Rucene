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

//! Port of `TestFilterSpans.java`.
//!
//! Java walks `FilterSpans.class.getMethods()` by reflection and fails for
//! any method whose declaring class is still `Spans`. Rust has no
//! reflection; the same question is asked of the port's sources, which the
//! test embeds: every method of the trait `Spans` must be defined in
//! `impl Spans for FilterSpans`. `getMethods()` lists public members only,
//! so the trait methods standing for Java's protected `doStartCurrentDoc()`
//! and `doCurrentSpans()` are not asked about.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;

use crate::queries_support::core_support::{class, test};

/// The sources of `Spans` and `FilterSpans`, the port's "classes".
const SPANS_SOURCE: &str = include_str!("../../src/spans/spans.rs");
const FILTER_SPANS_SOURCE: &str = include_str!("../../src/spans/filter_spans.rs");

/// The Rust names of Java's protected members of `Spans`.
const PROTECTED: [&str; 2] = ["do_start_current_doc", "do_current_spans"];

/// The `fn` names declared directly in the item block that opens at the line
/// `header` of `source` and closes at the next line `}`.
fn block_methods(source: &str, header: &str) -> Vec<String> {
    let mut lines = source.lines().skip_while(|l| *l != header);
    assert!(
        lines.next().is_some(),
        "reflection stand-in: `{header}` not found"
    );
    let mut out = Vec::new();
    for l in lines {
        if l == "}" {
            break;
        }
        let t = l.trim_start();
        // only the block's own members (four spaces of indentation)
        if l.len() - t.len() == 4 {
            if let Some(rest) = t.strip_prefix("fn ") {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                out.push(name);
            }
        }
    }
    out
}

/// `TestFilterSpans`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.spans.TestFilterSpans",
        vec![test("testOverrides", test_overrides)],
    )
}

fn test_overrides() -> Result<()> {
    // verify that all methods of Spans are overridden by FilterSpans,
    let spans_methods = block_methods(SPANS_SOURCE, "pub trait Spans: DocIdSetIterator {");
    let filter_methods = block_methods(
        FILTER_SPANS_SOURCE,
        "impl<A: FilterSpansAccept> Spans for FilterSpans<A> {",
    );
    assert!(
        spans_methods.iter().any(|m| m == "next_start_position"),
        "reflection stand-in: the methods of Spans were not found: {spans_methods:?}"
    );
    for m in &spans_methods {
        if PROTECTED.contains(&m.as_str()) {
            continue;
        }
        if !filter_methods.contains(m) {
            Assert::fail(&format!("method {m} not overridden!"));
        }
    }
    Ok(())
}
