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

//! The type declared in `Hyphen.java`, of the Java package
//! `org.apache.lucene.analysis.compound.hyphenation`.

use std::fmt;

/// This class represents a hyphen. A 'full' hyphen is made of 3 parts: the
/// pre-break text, post-break text and no-break. If no line-break is
/// generated at this position, the no-break text is used, otherwise,
/// pre-break and post-break are used. Typically, pre-break is equal to the
/// hyphen character and the others are empty. However, this general scheme
/// allows support for cases in some languages where words change spelling if
/// they're split across lines, like german's 'backen' which hyphenates
/// 'bak-ken'. BTW, this comes from TeX.
///
/// This class has been taken from the Apache FOP project
/// (<http://xmlgraphics.apache.org/fop/>). They have been slightly modified.
///
/// Equivalent to `org.apache.lucene.analysis.compound.hyphenation.Hyphen`;
/// `None` stands for Java's `null`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hyphen {
    /// Equivalent to `Hyphen.preBreak`.
    pub pre_break: Option<String>,
    /// Equivalent to `Hyphen.noBreak`.
    pub no_break: Option<String>,
    /// Equivalent to `Hyphen.postBreak`.
    pub post_break: Option<String>,
}

impl Hyphen {
    /// Equivalent to the package-private `Hyphen(String pre, String no, String post)`.
    pub(crate) fn new(pre: Option<String>, no: Option<String>, post: Option<String>) -> Self {
        Self {
            pre_break: pre,
            no_break: no,
            post_break: post,
        }
    }

    /// Equivalent to the package-private `Hyphen(String pre)`.
    #[allow(dead_code)] // A member of the Java class that Lucene 10.5.0's main code does not use.
    pub(crate) fn with_pre_break(pre: Option<String>) -> Self {
        Self {
            pre_break: pre,
            no_break: None,
            post_break: None,
        }
    }
}

impl fmt::Display for Hyphen {
    /// Equivalent to `Hyphen.toString()`: `-` for a simple hyphen, otherwise
    /// `{pre}{post}{no}`, each `null` printed as `null`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.no_break.is_none()
            && self.post_break.is_none()
            && self.pre_break.as_deref() == Some("-")
        {
            return f.write_str("-");
        }
        let text = |s: &Option<String>| s.clone().unwrap_or_else(|| "null".to_string());
        write!(
            f,
            "{{{}}}{{{}}}{{{}}}",
            text(&self.pre_break),
            text(&self.post_break),
            text(&self.no_break)
        )
    }
}
