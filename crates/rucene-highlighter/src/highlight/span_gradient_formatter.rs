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

//! Port of `SpanGradientFormatter.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use std::ops::Deref;

use rucene::error::Result;

use crate::highlight::formatter::Formatter;
use crate::highlight::gradient_formatter::GradientFormatter;
use crate::highlight::token_group::TokenGroup;

// guess how much extra text we'll add to the text we're highlighting to try
// to avoid a StringBuilder resize
const TEMPLATE: &str = "<span style=\"background: #EEEEEE; color: #000000;\">...</span>";
const EXTRA: usize = TEMPLATE.len();

/// Formats text with different color intensity depending on the score of the
/// term using the span tag. GradientFormatter uses a bgcolor argument to the
/// font tag which doesn't work in Mozilla, thus this class.
///
/// Equivalent to `org.apache.lucene.search.highlight.SpanGradientFormatter`,
/// a subclass of `GradientFormatter`: the base is reached through `Deref`.
#[derive(Debug, Clone)]
pub struct SpanGradientFormatter {
    base: GradientFormatter,
}

impl SpanGradientFormatter {
    /// Equivalent to `new SpanGradientFormatter(float, String, String,
    /// String, String)`.
    ///
    /// # Errors
    ///
    /// As [`GradientFormatter::new`].
    pub fn new(
        max_score: f32,
        min_foreground_color: Option<&str>,
        max_foreground_color: Option<&str>,
        min_background_color: Option<&str>,
        max_background_color: Option<&str>,
    ) -> Result<Self> {
        Ok(Self {
            base: GradientFormatter::new(
                max_score,
                min_foreground_color,
                max_foreground_color,
                min_background_color,
                max_background_color,
            )?,
        })
    }
}

impl Deref for SpanGradientFormatter {
    type Target = GradientFormatter;

    fn deref(&self) -> &GradientFormatter {
        &self.base
    }
}

impl Formatter for SpanGradientFormatter {
    fn highlight_term(&self, original_text: &str, token_group: &TokenGroup) -> String {
        let score = token_group.get_total_score();
        if score == 0.0 {
            return original_text.to_string();
        }

        // try to size sb correctly
        let mut sb = String::with_capacity(original_text.len() + EXTRA);

        sb.push_str("<span style=\"");
        if self.highlight_foreground {
            sb.push_str("color: ");
            sb.push_str(&self.get_foreground_color_string(score));
            sb.push_str("; ");
        }
        if self.highlight_background {
            sb.push_str("background: ");
            sb.push_str(&self.get_background_color_string(score));
            sb.push_str("; ");
        }
        sb.push_str("\">");
        sb.push_str(original_text);
        sb.push_str("</span>");
        sb
    }
}