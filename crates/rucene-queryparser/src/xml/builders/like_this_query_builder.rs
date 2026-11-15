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

//! Port of `LikeThisQueryBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.xml.builders`.

use std::collections::HashSet;
use std::sync::Arc;

use rucene::analysis::tokenattributes::CharTermAttribute;
use rucene::analysis::Analyzer;
use rucene::error::{LuceneError, Result};
use rucene::search::boost_query::BoostQuery;
use rucene::search::query::Query;
use rucene_queries::mlt::more_like_this_query::MoreLikeThisQuery;

use super::close_after;
use crate::jdk::dom::Element;
use crate::jdk::string::{split_char, trim_str};
use crate::xml::dom_utils::DOMUtils;
use crate::xml::parser_exception::ParserException;
use crate::xml::query_builder::QueryBuilder;

/// `DEFAULT_MAX_QUERY_TERMS`.
const DEFAULT_MAX_QUERY_TERMS: i32 = 20;
/// `DEFAULT_MIN_TERM_FREQUENCY`.
const DEFAULT_MIN_TERM_FREQUENCY: i32 = 1;
/// `DEFAULT_PERCENT_TERMS_TO_MATCH`: default is a 3rd of selected terms must
/// match.
const DEFAULT_PERCENT_TERMS_TO_MATCH: f32 = 30.0;

/// Builder for `MoreLikeThisQuery`
///
/// Equivalent to
/// `org.apache.lucene.queryparser.xml.builders.LikeThisQueryBuilder`; a
/// `None` analyzer (Java's `null`) throws `NullPointerException` where Java
/// first uses it (the stop words, whose attribute is never `null`).
#[derive(Debug)]
pub struct LikeThisQueryBuilder {
    analyzer: Option<Arc<dyn Analyzer>>,
    default_field_names: Option<Vec<String>>,
}

impl LikeThisQueryBuilder {
    /// Equivalent to `new LikeThisQueryBuilder(Analyzer analyzer, String[]
    /// defaultFieldNames)`.
    pub fn new(
        analyzer: Option<Arc<dyn Analyzer>>,
        default_field_names: Option<Vec<String>>,
    ) -> Self {
        Self {
            analyzer,
            default_field_names,
        }
    }

    /// The terms the analyzer produces from `stop_words` in `field`.
    fn stop_words_of(
        &self,
        field: &str,
        stop_words: &str,
        set: &mut HashSet<String>,
    ) -> Result<()> {
        let ts = self
            .analyzer
            .as_ref()
            .ok_or_else(rucene::error::LuceneError::null_pointer)?
            .token_stream_from_str(field, stop_words)?;
        let mut ts = ts.borrow_mut();
        let body = (|| -> Result<()> {
            ts.attribute_source_mut()
                .add_attribute_interface::<dyn CharTermAttribute>()?;
            ts.reset()?;
            while ts.increment_token()? {
                let term: String = {
                    let term_att = ts
                        .attribute_source()
                        .get_attribute_interface::<dyn CharTermAttribute>()
                        .expect("INVARIANT: CharTermAttribute was added above");
                    term_att.buffer()[..term_att.length()].iter().collect()
                };
                set.insert(term);
            }
            ts.end()?;
            Ok(())
        })();
        let closed = ts.close();
        close_after(body, closed)
    }
}

impl QueryBuilder for LikeThisQueryBuilder {
    /// Equivalent to `getQuery(Element e)`: a `MoreLikeThisQuery` of the
    /// element's text over the `fieldNames` attribute (comma-separated) or
    /// the default fields, with the stop words the analyzer produces from
    /// the `stopWords` attribute in each field (an empty set when the
    /// attribute is absent), and the `maxQueryTerms`, `minTermFrequency`,
    /// `percentTermsToMatch`, `minDocFreq` and `boost` attributes.
    ///
    /// # Errors
    ///
    /// `ParserException("IoException parsing stop words list in " + class +
    /// ":" + message)` for an `IOException` of the analysis, the
    /// `NumberFormatException` of an attribute, the `NullPointerException`
    /// of `null` default fields, and the `ArrayIndexOutOfBoundsException` of
    /// an empty field list.
    fn get_query(&self, e: &Element) -> Result<Arc<dyn Query>> {
        let fields_list = e.get_attribute("fieldNames"); // a comma-delimited list of fields
        let mut fields = self.default_field_names.clone();
        if !trim_str(&fields_list).is_empty() {
            // trim the fieldnames
            fields = Some(
                split_char(trim_str(&fields_list), ',')
                    .into_iter()
                    .map(|f| trim_str(&f).to_string())
                    .collect(),
            );
        }
        // Parse any "stopWords" attribute
        // TODO MoreLikeThis needs to ideally have per-field stopWords lists -
        // until then I use all analyzers/fields to generate multi-field
        // compatible stop list
        let stop_words = e.get_attribute("stopWords");
        let mut stop_words_set: Option<HashSet<String>> = None;
        if let Some(fields) = &fields {
            let mut set = HashSet::new();
            for field in fields {
                self.stop_words_of(field, &stop_words, &mut set).map_err(|ioe| {
                    if ioe.is_io_exception() {
                        ParserException::with_message(format!(
                            "IoException parsing stop words list in org.apache.lucene.queryparser.xml.builders.LikeThisQueryBuilder:{}",
                            ioe_message(&ioe)
                        ))
                        .error()
                    } else {
                        ioe
                    }
                })?;
            }
            stop_words_set = Some(set);
        }
        let fields = fields.ok_or_else(LuceneError::null_pointer)?;
        let first = fields.first().cloned().ok_or_else(|| {
            LuceneError::unchecked(
                "java.lang.ArrayIndexOutOfBoundsException",
                "Index 0 out of bounds for length 0",
            )
        })?;
        let analyzer = self
            .analyzer
            .clone()
            .ok_or_else(LuceneError::null_pointer)?;
        let mut mlt = MoreLikeThisQuery::new(DOMUtils::get_text(e), fields, analyzer, first);
        mlt.set_max_query_terms(DOMUtils::get_attribute_int(
            e,
            "maxQueryTerms",
            DEFAULT_MAX_QUERY_TERMS,
        )?);
        mlt.set_min_term_frequency(DOMUtils::get_attribute_int(
            e,
            "minTermFrequency",
            DEFAULT_MIN_TERM_FREQUENCY,
        )?);
        mlt.set_percent_terms_to_match(
            DOMUtils::get_attribute_float(
                e,
                "percentTermsToMatch",
                DEFAULT_PERCENT_TERMS_TO_MATCH,
            )? / 100.0,
        );
        mlt.set_stop_words(stop_words_set.map(Arc::new));
        let min_doc_freq = DOMUtils::get_attribute_int(e, "minDocFreq", -1)?;
        if min_doc_freq >= 0 {
            mlt.set_min_doc_freq(min_doc_freq);
        }
        let mlt = Arc::new(mlt);
        let mut q: Arc<dyn Query> = mlt.clone();
        let boost = DOMUtils::get_attribute_float(e, "boost", 1.0)?;
        if boost != 1.0 {
            q = Arc::new(BoostQuery::new(mlt, boost)?);
        }
        Ok(q)
    }
}

/// `IOException.getLocalizedMessage()`: its message, `null` without one.
fn ioe_message(ioe: &LuceneError) -> String {
    let s = crate::flexible::core::query_node_exception::throwable_to_string(ioe);
    match s.split_once(": ") {
        Some((_, message)) => message.to_string(),
        None => "null".to_string(),
    }
}
