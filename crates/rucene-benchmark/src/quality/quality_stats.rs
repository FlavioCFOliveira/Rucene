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

//! The types declared in `QualityStats.java`, of the Java package
//! `org.apache.lucene.benchmark.quality`.

use rucene::error::{LuceneError, Result};
use rucene_queryparser::jdk::decimal_format::DecimalFormat;
use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::number_format;

use crate::jdk::PrintWriter;

/// Equivalent to `org.apache.lucene.benchmark.quality.QualityStats.RecallPoint`
/// (`public static class RecallPoint`): a certain rank in which a relevant
/// doc was found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecallPoint {
    rank: i32,
    recall: f64,
}

impl RecallPoint {
    /// `getRank()`: the rank: where on the list of returned docs this
    /// relevant doc appeared.
    pub fn get_rank(&self) -> i32 {
        self.rank
    }

    /// `getRecall()`: the recall: how many relevant docs were returned up to
    /// this point, inclusive.
    pub fn get_recall(&self) -> f64 {
        self.recall
    }
}

/// Equivalent to `org.apache.lucene.benchmark.quality.QualityStats`: results
/// of quality benchmark run for a single query or for a set of queries.
#[derive(Clone, Debug)]
pub struct QualityStats {
    max_good_points: f64,
    recall: f64,
    p_at: Vec<f64>,
    p_relevent_sum: f64,
    num_points: f64,
    num_good_points: f64,
    mrr: f64,
    search_time: i64,
    doc_names_extract_time: i64,
    recall_points: Vec<RecallPoint>,
}

/// `padd`.
const PADD: &str = "                                    ";

impl QualityStats {
    /// `MAX_POINTS`: number of points for which precision is computed.
    pub const MAX_POINTS: i32 = 20;

    /// `QualityStats(double maxGoodPoints, long searchTime)`: construct a
    /// QualityStats object with anticipated maximal number of relevant
    /// hits.
    pub fn new(max_good_points: f64, search_time: i64) -> Self {
        QualityStats {
            max_good_points,
            recall: 0.0,
            p_at: vec![0.0; (Self::MAX_POINTS + 1) as usize], // pAt[0] unused.
            p_relevent_sum: 0.0,
            num_points: 0.0,
            num_good_points: 0.0,
            mrr: 0.0,
            search_time,
            doc_names_extract_time: 0,
            recall_points: Vec::new(),
        }
    }

    /// `addResult(int n, boolean isRelevant, long docNameExtractTime)`: add a
    /// (possibly relevant) doc; `n` is the rank of the added doc (its
    /// ordinal position within the query results).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a rank out of order.
    pub fn add_result(&mut self, n: i32, is_relevant: bool, doc_name_extract_time: i64) -> Result<()> {
        if (self.num_points + 1.0 - f64::from(n)).abs() > 1E-6 {
            return Err(LuceneError::illegal_argument(format!(
                "point {n} illegal after {} points!",
                crate::jdk::double_to_string(self.num_points)
            )));
        }
        if is_relevant {
            self.num_good_points += 1.0;
            self.recall_points.push(RecallPoint { rank: n, recall: self.num_good_points });
            if self.recall_points.len() == 1 && n <= 5 {
                // first point, but only within 5 top scores.
                self.mrr = 1.0 / f64::from(n);
            }
        }
        self.num_points = f64::from(n);
        let p = self.num_good_points / self.num_points;
        if is_relevant {
            self.p_relevent_sum += p;
        }
        if (n as usize) < self.p_at.len() {
            self.p_at[n as usize] = p;
        }
        self.recall = if self.max_good_points <= 0.0 { p } else { self.num_good_points / self.max_good_points };
        self.doc_names_extract_time += doc_name_extract_time;
        Ok(())
    }

    /// `getPrecisionAt(int n)`: the precision at rank n: |{relevant hits
    /// within first `n` hits}| / `n`; `n` must be at least 1 and at most
    /// [`MAX_POINTS`](Self::MAX_POINTS).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for `n` out of range.
    pub fn get_precision_at(&self, n: i32) -> Result<f64> {
        if n < 1 || n > Self::MAX_POINTS {
            return Err(LuceneError::illegal_argument(format!(
                "n={n} - but it must be in [1,{}] range!",
                Self::MAX_POINTS
            )));
        }
        if f64::from(n) > self.num_points {
            return Ok((self.num_points * self.p_at[self.num_points as i32 as usize]) / f64::from(n));
        }
        Ok(self.p_at[n as usize])
    }

    /// `getAvp()`: the average precision at recall points.
    pub fn get_avp(&self) -> f64 {
        if self.max_good_points == 0.0 {
            0.0
        } else {
            self.p_relevent_sum / self.max_good_points
        }
    }

    /// `getRecall()`: the recall: |{relevant hits found}| / |{relevant hits
    /// existing}|.
    pub fn get_recall(&self) -> f64 {
        self.recall
    }

    /// `log(String title, int paddLines, PrintWriter logger, String prefix)`:
    /// log information on this QualityStats object.
    ///
    /// # Errors
    ///
    /// The exception of the number format.
    pub fn log(&self, title: Option<&str>, padd_lines: i32, logger: &PrintWriter, prefix: Option<&str>) -> Result<()> {
        for _ in 0..padd_lines {
            logger.println_empty();
        }
        if let Some(title) = title.filter(|t| !crate::jdk::java_trim(t).is_empty()) {
            logger.println(title);
        }
        let prefix = prefix.unwrap_or("");
        let mut nf: DecimalFormat = number_format::get_instance(&Locale::root())?;
        nf.set_maximum_fraction_digits(3);
        nf.set_minimum_fraction_digits(3);
        nf.set_grouping_used(true);
        let m = 19;
        let line = |label: &str, value: f64| -> Result<String> {
            Ok(format!("{prefix}{}{}", Self::format(Some(label), m), Self::frac_format(&nf.format_double(value)?)?))
        };
        logger.println(&line("Search Seconds: ", self.search_time as f64 / 1000.0)?);
        logger.println(&line("DocName Seconds: ", self.doc_names_extract_time as f64 / 1000.0)?);
        logger.println(&line("Num Points: ", self.num_points)?);
        logger.println(&line("Num Good Points: ", self.num_good_points)?);
        logger.println(&line("Max Good Points: ", self.max_good_points)?);
        logger.println(&line("Average Precision: ", self.get_avp())?);
        logger.println(&line("MRR: ", self.get_mrr())?);
        logger.println(&line("Recall: ", self.get_recall())?);
        let mut i = 1;
        while i < self.num_points as i32 && (i as usize) < self.p_at.len() {
            logger.println(&line(&format!("Precision At {i}: "), self.get_precision_at(i)?)?);
            i += 1;
        }
        for _ in 0..padd_lines {
            logger.println_empty();
        }
        Ok(())
    }

    /// `format(String s, int minLen)`.
    fn format(s: Option<&str>, min_len: usize) -> String {
        let s = s.unwrap_or("");
        let units: Vec<u16> = format!("{s}{PADD}").encode_utf16().collect();
        let n = min_len.max(crate::jdk::utf16_len(s));
        String::from_utf16_lossy(&units[..n])
    }

    /// `fracFormat(String frac)`.
    fn frac_format(frac: &str) -> Result<String> {
        let units: Vec<u16> = frac.encode_utf16().collect();
        let Some(k) = units.iter().position(|&c| c == u16::from(b'.')) else {
            return Err(LuceneError::unchecked(
                "java.lang.StringIndexOutOfBoundsException",
                format!("begin 0, end -1, length {}", units.len()),
            ));
        };
        let mut s1: Vec<u16> = PADD.encode_utf16().collect();
        s1.extend_from_slice(&units[..k]);
        let n = k.max(6);
        let s1 = &s1[s1.len() - n..];
        let mut out = String::from_utf16_lossy(s1);
        out.push_str(&String::from_utf16_lossy(&units[k..]));
        Ok(out)
    }

    /// `average(QualityStats[] stats)`: create a QualityStats object that is
    /// the average of the input QualityStats objects.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `getPrecisionAt`.
    pub fn average(stats: &[QualityStats]) -> Result<QualityStats> {
        let mut avg = QualityStats::new(0.0, 0);
        if stats.is_empty() {
            // weired, no stats to average!
            return Ok(avg);
        }
        let mut m = 0; // queries with positive judgements
        // aggregate
        for stat in stats {
            avg.search_time += stat.search_time;
            avg.doc_names_extract_time += stat.doc_names_extract_time;
            if stat.max_good_points > 0.0 {
                m += 1;
                avg.num_good_points += stat.num_good_points;
                avg.num_points += stat.num_points;
                avg.p_relevent_sum += stat.get_avp();
                avg.recall += stat.recall;
                avg.mrr += stat.get_mrr();
                avg.max_good_points += stat.max_good_points;
                for j in 1..avg.p_at.len() {
                    avg.p_at[j] += stat.get_precision_at(j as i32)?;
                }
            }
        }
        // take average: times go by all queries, other measures go by "good" queries only.
        avg.search_time /= stats.len() as i64;
        avg.doc_names_extract_time /= stats.len() as i64;
        let m = f64::from(m);
        avg.num_good_points /= m;
        avg.num_points /= m;
        avg.recall /= m;
        avg.mrr /= m;
        avg.max_good_points /= m;
        for j in 1..avg.p_at.len() {
            avg.p_at[j] /= m;
        }
        avg.p_relevent_sum /= m; // this is actually avgp now
        avg.p_relevent_sum *= avg.max_good_points; // so that getAvgP() would be correct
        Ok(avg)
    }

    /// `getDocNamesExtractTime()`: the time it took to extract doc names for
    /// judging the measured query, in milliseconds.
    pub fn get_doc_names_extract_time(&self) -> i64 {
        self.doc_names_extract_time
    }

    /// `getMaxGoodPoints()`: the maximal number of good points. This is the
    /// number of relevant docs known by the judge for the measured query.
    pub fn get_max_good_points(&self) -> f64 {
        self.max_good_points
    }

    /// `getNumGoodPoints()`: the number of good points (only relevant
    /// points).
    pub fn get_num_good_points(&self) -> f64 {
        self.num_good_points
    }

    /// `getNumPoints()`: the number of points (both relevant and irrelevant
    /// points).
    pub fn get_num_points(&self) -> f64 {
        self.num_points
    }

    /// `getRecallPoints()`.
    pub fn get_recall_points(&self) -> Vec<RecallPoint> {
        self.recall_points.clone()
    }

    /// `getMRR()`: the Mean reciprocal rank over the queries or RR for a
    /// single query.
    ///
    /// Reciprocal rank is defined as `1/r` where `r` is the rank of the
    /// first correct result, or `0` if there are no correct results within
    /// the top 5 results.
    ///
    /// This follows the definition in [Question Answering - CNLP at the
    /// TREC-10 Question Answering Track](http://www.cnlp.org/publications/02cnlptrec10.pdf).
    pub fn get_mrr(&self) -> f64 {
        self.mrr
    }

    /// `getSearchTime()`: the search time in milliseconds for the measured
    /// query.
    pub fn get_search_time(&self) -> i64 {
        self.search_time
    }
}
