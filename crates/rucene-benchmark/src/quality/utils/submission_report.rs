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

//! The types declared in `SubmissionReport.java`, of the Java package
//! `org.apache.lucene.benchmark.quality.utils`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::top_docs::TopDocs;
use rucene_queryparser::jdk::decimal_format::DecimalFormat;
use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::number_format;

use crate::jdk::PrintWriter;
use crate::quality::quality_query::QualityQuery;
use crate::quality::utils::doc_name_extractor::DocNameExtractor;

/// `padd`.
const PADD: &str = "                                    ";

/// Equivalent to `org.apache.lucene.benchmark.quality.utils.SubmissionReport`:
/// create a log ready for submission. Extend this class and override
/// [`report`](Self::report) to create different reports.
pub struct SubmissionReport {
    nf: DecimalFormat,
    logger: Option<Arc<PrintWriter>>,
    name: String,
}

impl SubmissionReport {
    /// `SubmissionReport(PrintWriter logger, String name)`: if `logger` is
    /// `None`, no submission data is created; `name` is the name of this
    /// run.
    ///
    /// # Errors
    ///
    /// The exception of the number format.
    pub fn new(logger: Option<Arc<PrintWriter>>, name: &str) -> Result<Self> {
        let mut nf = number_format::get_instance(&Locale::root())?;
        nf.set_maximum_fraction_digits(4);
        nf.set_minimum_fraction_digits(4);
        Ok(SubmissionReport {
            nf,
            logger,
            name: name.to_string(),
        })
    }

    /// `report(QualityQuery qq, TopDocs td, String docNameField, IndexSearcher
    /// searcher)`: report a search result for a certain quality query.
    ///
    /// # Errors
    ///
    /// In case of a problem.
    pub fn report(
        &self,
        qq: &QualityQuery,
        td: &TopDocs,
        doc_name_field: &str,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        let Some(logger) = &self.logger else {
            return Ok(());
        };
        let sep = " \t ";
        let xt = DocNameExtractor::new(doc_name_field);
        let stored_fields = searcher.stored_fields()?;
        for (i, sd) in td.score_docs.iter().enumerate() {
            let doc_name = xt.doc_name(&*stored_fields, sd.doc)?;
            logger.println(&format!(
                "{}{sep}Q0{sep}{}{sep}{}{sep}{}{sep}{}",
                qq.get_query_id(),
                Self::format(doc_name.as_deref(), 20),
                Self::format(Some(&i.to_string()), 7),
                self.nf.format_double(f64::from(sd.score))?,
                self.name
            ));
        }
        Ok(())
    }

    /// `flush()`.
    pub fn flush(&self) {
        if let Some(logger) = &self.logger {
            logger.flush();
        }
    }

    /// `format(String s, int minLen)`.
    fn format(s: Option<&str>, min_len: usize) -> String {
        let s = s.unwrap_or("");
        let units: Vec<u16> = format!("{s}{PADD}").encode_utf16().collect();
        let n = min_len.max(crate::jdk::utf16_len(s));
        String::from_utf16_lossy(&units[..n])
    }
}
