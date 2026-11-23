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

//! Port of `TestLengthGoalBreakIterator.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::error::Result;
use rucene::search::query::Query;
use rucene::util::query_builder::{QueryBuilder, QueryBuilderOps};
use rucene_analysis_common::jdk::break_iterator::BreakIterator;
use rucene_highlighter::uhighlight::custom_separator_break_iterator::CustomSeparatorBreakIterator;
use rucene_highlighter::uhighlight::length_goal_break_iterator::LengthGoalBreakIterator;
use rucene_highlighter::uhighlight::unified_highlighter::UnifiedHighlighter;
use rucene_test_framework::analysis::mock_tokenizer::SIMPLE;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::highlighter_support::core_support::random;
use crate::highlighter_support::{instance_class, ti};

const FIELD: &str = "body";
const ALIGNS: [f32; 3] = [0.0, 0.5, 1.0];

// We do a '.' BreakIterator and test varying the length goal.
//                      0         1
//                      01234567890123456789
const CONTENT: &str = "Aa bb. Cc dd. Ee ff";
const CONTENT2: &str = "Aa bb Cc dd X Ee ff Gg hh.";
const CONTENT3: &str = "Aa bbcc ddxyzee ffgg hh.";

/// `TestLengthGoalBreakIterator`.
pub fn class_def() -> Arc<Class> {
    type T = TestLengthGoalBreakIterator;
    instance_class(
        "org.apache.lucene.search.uhighlight.TestLengthGoalBreakIterator",
        &LuceneTestCase::class(),
        Constructor::no_args(T::new),
        vec![
            ti::<T>(
                "testFragmentAlignmentConstructor",
                T::test_fragment_alignment_constructor,
            ),
            ti::<T>("testTargetLen", T::test_target_len),
            ti::<T>("testMinLen", T::test_min_len),
            ti::<T>("testMinLenPrecision", T::test_min_len_precision),
            ti::<T>(
                "testDefaultSummaryTargetLen",
                T::test_default_summary_target_len,
            ),
        ],
    )
}

/// The `TestLengthGoalBreakIterator` instance.
pub struct TestLengthGoalBreakIterator {
    // We test LengthGoalBreakIterator as it is used by the UnifiedHighlighter instead of directly,
    // because it is not a general purpose BreakIterator. A unit test of it directly wouldn't give
    // as much confidence.
    analyzer: Arc<dyn Analyzer>, // whitespace, punctuation, lowercase
}

fn next_int(bound: i32) -> i32 {
    random().next_int_bound(bound)
}

impl TestLengthGoalBreakIterator {
    fn new() -> Self {
        Self {
            analyzer: Arc::new(MockAnalyzer::with_automaton(
                &*random(),
                Arc::clone(&SIMPLE),
                true,
            )),
        }
    }

    fn test_fragment_alignment_constructor(&mut self) -> Result<()> {
        let base_bi = CustomSeparatorBreakIterator::new(b'.' as u16);
        // test fragmentAlignment validation
        let valid_aligns = [0.0f32, 0.3333, 0.5, 0.99, 1.0];
        for alignment in valid_aligns {
            LengthGoalBreakIterator::create_closest_to_length(base_bi.clone_box(), 50, alignment)?;
        }
        let invalid_aligns = [
            -0.01f32,
            -1.0,
            1.5,
            f32::NAN,
            f32::NEG_INFINITY,
            f32::INFINITY,
        ];
        for alignment in invalid_aligns {
            expect_throws("java.lang.IllegalArgumentException", || {
                LengthGoalBreakIterator::create_closest_to_length(
                    base_bi.clone_box(),
                    50,
                    alignment,
                )
            });
        }
        Ok(())
    }

    fn test_target_len(&mut self) -> Result<()> {
        // "goal" means target length goal to find closest break

        // at first word:
        let mut query = self.query("aa")?;
        Assert::assert_equals_msg(
            Some("almost two sent A"),
            "<b>Aa</b> bb.".to_string(),
            self.highlight_closest_to_len(CONTENT, &query, 7, 0.0)?,
        );
        Assert::assert_equals_msg(
            Some("almost two sent B"),
            "<b>Aa</b> bb.".to_string(),
            self.highlight_closest_to_len(CONTENT, &query, 15, 0.5)?,
        );
        Assert::assert_equals_msg(
            Some("almost two sent C"),
            "<b>Aa</b> bb.".to_string(),
            self.highlight_closest_to_len(CONTENT, &query, 64, 1.0)?,
        );
        Assert::assert_equals_msg(
            Some("barely two sent A"),
            "<b>Aa</b> bb. Cc dd.".to_string(),
            self.highlight_closest_to_len(CONTENT, &query, 8, 0.0)?,
        );
        Assert::assert_equals_msg(
            Some("barely two sent B"),
            "<b>Aa</b> bb. Cc dd.".to_string(),
            self.highlight_closest_to_len(CONTENT, &query, 16, 0.5)?,
        );
        Assert::assert_equals_msg(
            Some("long goal A"),
            "<b>Aa</b> bb. Cc dd. Ee ff".to_string(),
            self.highlight_closest_to_len(CONTENT, &query, 14 + next_int(20), 0.0)?,
        );
        Assert::assert_equals_msg(
            Some("long goal B"),
            "<b>Aa</b> bb. Cc dd. Ee ff".to_string(),
            self.highlight_closest_to_len(CONTENT, &query, 28 + next_int(20), 0.5)?,
        );
        // at some word not at start of passage
        query = self.query("dd")?;
        for align in ALIGNS {
            // alignment is not meaningful if fragsize is shorter than or closer to match-fragment
            // boundaries
            Assert::assert_equals_msg(
                Some(&format!("short goal {}", java_float(align))),
                " Cc <b>dd</b>.".to_string(),
                self.highlight_closest_to_len(CONTENT, &query, next_int(4), align)?,
            );
        }
        // preceding/following inclusion by alignment parameter
        Assert::assert_equals_msg(
            Some("barely two sent A"),
            " Cc <b>dd</b>. Ee ff".to_string(),
            self.highlight_closest_to_len(CONTENT, &query, 11, 0.0)?,
        );
        Assert::assert_equals_msg(
            Some("barely two sent B"),
            " Cc <b>dd</b>. Ee ff".to_string(),
            self.highlight_closest_to_len(CONTENT, &query, 11, 0.5)?,
        );
        Assert::assert_equals_msg(
            Some("barely two sent C"),
            "Aa bb. Cc <b>dd</b>.".to_string(),
            self.highlight_closest_to_len(CONTENT, &query, 11, 1.0)?,
        );
        Assert::assert_equals_msg(
            Some("long goal A"),
            " Cc <b>dd</b>. Ee ff".to_string(),
            self.highlight_closest_to_len(CONTENT, &query, 17 + next_int(20), 0.0)?,
        );
        Assert::assert_equals_msg(
            Some("long goal B"),
            "Aa bb. Cc <b>dd</b>. Ee ff".to_string(),
            self.highlight_closest_to_len(CONTENT, &query, 17 + next_int(20), 0.5)?,
        );
        Assert::assert_equals_msg(
            Some("long goal C"),
            "Aa bb. Cc <b>dd</b>.".to_string(),
            self.highlight_closest_to_len(CONTENT, &query, 17 + next_int(20), 1.0)?,
        );

        query = self.query("ddxyzee")?;
        Assert::assert_equals_msg(
            Some("test fragment search from the middle of the match; almost including"),
            "<b>ddxyzee</b> ".to_string(),
            self.highlight_closest_to_len_sep(CONTENT3, &query, 7, 0.5, 1, ' ')?,
        );
        Assert::assert_equals_msg(
            Some("test fragment search from the middle of the match; barely including"),
            "bbcc <b>ddxyzee</b> ffgg ".to_string(),
            self.highlight_closest_to_len_sep(CONTENT3, &query, 14, 0.5, 1, ' ')?,
        );
        Ok(())
    }

    fn test_min_len(&mut self) -> Result<()> {
        // minLen mode is simpler than targetLen... just test a few cases
        let mut query = self.query("dd")?;
        let cases: [(&str, &str, i32, f32); 11] = [
            ("almost two sent A", " Cc <b>dd</b>.", 0, 0.0),
            ("almost two sent B", " Cc <b>dd</b>.", 1, 0.5),
            ("almost two sent C", " Cc <b>dd</b>.", 5, 1.0),
            ("barely two sent A", " Cc <b>dd</b>. Ee ff", 1, 0.0),
            ("barely two sent B", " Cc <b>dd</b>. Ee ff", 2, 0.5),
            ("barely two sent C", "Aa bb. Cc <b>dd</b>.", 7, 1.0),
            ("barely two sent D/a", " Cc <b>dd</b>.", 2, 0.55),
            ("barely two sent D/b", " Cc <b>dd</b>. Ee ff", 3, 0.55),
            ("barely two sent E/a", " Cc <b>dd</b>. Ee ff", 10, 0.5),
            ("barely two sent E/b", "Aa bb. Cc <b>dd</b>. Ee ff", 10, 0.7),
            ("barely two sent E/c", "Aa bb. Cc <b>dd</b>.", 9, 0.9),
        ];
        for (message, expected, length_goal, align) in cases {
            Assert::assert_equals_msg(
                Some(message),
                expected.to_string(),
                self.highlight_min_len(CONTENT, &query, length_goal, align)?,
            );
        }

        query = self.query("ddxyzee")?;
        Assert::assert_equals_msg(
            Some("test fragment search from the middle of the match; almost including"),
            "<b>ddxyzee</b> ".to_string(),
            self.highlight_min_len_sep(CONTENT3, &query, 7, 0.5, ' ')?,
        );
        Assert::assert_equals_msg(
            Some("test fragment search from the middle of the match; barely including"),
            "bbcc <b>ddxyzee</b> ffgg ".to_string(),
            self.highlight_min_len_sep(CONTENT3, &query, 8, 0.5, ' ')?,
        );
        Ok(())
    }

    fn test_min_len_precision(&mut self) -> Result<()> {
        let query = self.query("x")?;
        Assert::assert_equals_msg(
            Some("test absolute minimal length"),
            "<b>X</b> ".to_string(),
            self.highlight_min_len_sep(CONTENT2, &query, 1, 0.5, ' ')?,
        );
        Assert::assert_equals_msg(
            Some("test slightly above minimal length"),
            "dd <b>X</b> Ee ".to_string(),
            self.highlight_min_len_sep(CONTENT2, &query, 4, 0.5, ' ')?,
        );
        Ok(())
    }

    fn test_default_summary_target_len(&mut self) -> Result<()> {
        let query = self.query("zz")?;
        for align in ALIGNS {
            // alignment is not used for creating default-summary
            Assert::assert_equals(
                "Aa bb.".to_string(),
                self.highlight_closest_to_len(CONTENT, &query, 6 + next_int(4), align)?,
            );
            Assert::assert_equals(
                "Aa bb. Cc dd.".to_string(),
                self.highlight_closest_to_len(CONTENT, &query, 12 + next_int(4), align)?,
            );
            Assert::assert_equals(
                "Aa bb. Cc dd. Ee ff".to_string(),
                self.highlight_closest_to_len(CONTENT, &query, 17 + next_int(20), align)?,
            );
        }
        Assert::assert_equals(
            "Aa bb. Cc dd.".to_string(),
            self.highlight_closest_to_len_max(CONTENT, &query, 6 + next_int(4), 0.0, 2)?,
        );
        Ok(())
    }

    fn query(&self, q_str: &str) -> Result<Arc<dyn Query>> {
        Ok(QueryBuilder::new(Some(Arc::clone(&self.analyzer)))
            .create_boolean_query(Some(FIELD), q_str)?
            .expect("NullPointerException: no query"))
    }

    fn highlight_closest_to_len(
        &self,
        content: &str,
        query: &Arc<dyn Query>,
        length_goal: i32,
        frag_align: f32,
    ) -> Result<String> {
        self.highlight_closest_to_len_max(content, query, length_goal, frag_align, 1)
    }

    fn highlight_closest_to_len_max(
        &self,
        content: &str,
        query: &Arc<dyn Query>,
        length_goal: i32,
        frag_align: f32,
        max_passages: i32,
    ) -> Result<String> {
        self.highlight_closest_to_len_sep(content, query, length_goal, frag_align, max_passages, '.')
    }

    fn highlight_closest_to_len_sep(
        &self,
        content: &str,
        query: &Arc<dyn Query>,
        length_goal: i32,
        frag_align: f32,
        max_passages: i32,
        separator: char,
    ) -> Result<String> {
        let highlighter = UnifiedHighlighter::builder_without_searcher(Arc::clone(&self.analyzer))
            .with_break_iterator(Arc::new(move || -> Box<dyn BreakIterator> {
                Box::new(
                    LengthGoalBreakIterator::create_closest_to_length(
                        Box::new(CustomSeparatorBreakIterator::new(separator as u16)),
                        length_goal,
                        frag_align,
                    )
                    .unwrap_or_else(|e| {
                        rucene_test_framework::util::java_exceptions::rethrow_java(e)
                    }),
                )
            }))
            .build();
        Ok(highlighter
            .highlight_without_searcher(FIELD, query, content, max_passages)?
            .expect("NullPointerException: null highlight")
            .to_string())
    }

    fn highlight_min_len(
        &self,
        content: &str,
        query: &Arc<dyn Query>,
        length_goal: i32,
        frag_align: f32,
    ) -> Result<String> {
        self.highlight_min_len_sep(content, query, length_goal, frag_align, '.')
    }

    fn highlight_min_len_sep(
        &self,
        content: &str,
        query: &Arc<dyn Query>,
        length_goal: i32,
        frag_align: f32,
        separator: char,
    ) -> Result<String> {
        // differs from above only by "createMinLength"
        let highlighter = UnifiedHighlighter::builder_without_searcher(Arc::clone(&self.analyzer))
            .with_break_iterator(Arc::new(move || -> Box<dyn BreakIterator> {
                Box::new(
                    LengthGoalBreakIterator::create_min_length(
                        Box::new(CustomSeparatorBreakIterator::new(separator as u16)),
                        length_goal,
                        frag_align,
                    )
                    .unwrap_or_else(|e| {
                        rucene_test_framework::util::java_exceptions::rethrow_java(e)
                    }),
                )
            }))
            .build();
        Ok(highlighter
            .highlight_without_searcher(FIELD, query, content, 1)?
            .expect("NullPointerException: null highlight")
            .to_string())
    }
}

/// `Float.toString` of the alignments `0.0`, `0.5` and `1.0`.
fn java_float(f: f32) -> String {
    format!("{f:?}")
}
