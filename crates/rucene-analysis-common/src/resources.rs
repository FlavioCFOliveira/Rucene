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

//! The resource files of `lucene-analysis-common` 10.5.0: the 43 data files
//! of `lucene/analysis/common/src/resources` (stop-word lists, the RSLP
//! stemmer descriptions, the hyphenation DTD), outside `META-INF/services`
//! (see [`spi`](crate::spi)).
//!
//! Java finds them on the class path, inside `lucene-analysis-common.jar`,
//! and reads them three ways: `Class.getResourceAsStream(name)`, relative
//! to the package of the class (`TurkishAnalyzer.class.getResourceAsStream("stopwords.txt")`,
//! wrapped in `IOUtils.requireResourceNonNull`); a `ClasspathResourceLoader`
//! over the module's class loader, by absolute name; and
//! `Class.getResource(name)`. Rust has no class path, so the crate embeds
//! every file, byte for byte, with `include_bytes!` (the copies in
//! `crates/rucene-analysis-common/resources/` have the SHA-1 of the 10.5.0
//! originals), and this module resolves names as Java's class loader does:
//!
//! | Rucene | Java |
//! | --- | --- |
//! | [`get_resource`] | `ClassLoader.getResource(name)` of the module's loader, an absolute name |
//! | [`get_resource_as_stream`] | `Class.getResourceAsStream(name)`, relative to the class's package |
//! | [`class_loader_resources`] | the table `rucene`'s [`ClasspathResourceLoader`](rucene::util::resource_loader::ClasspathResourceLoader) takes for the module's class loader |
//!
//! A name no file has answers `None`, Java's `null`, which
//! `IOUtils.requireResourceNonNull` ([`IOUtils::require_resource_non_null`](rucene::util::IOUtils::require_resource_non_null))
//! turns into its `FileNotFoundException`, and `ClasspathResourceLoader`
//! into its `IOException`.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::{Arc, LazyLock};

/// One embedded file: its absolute resource name and its bytes.
macro_rules! resource {
    ($name:literal) => {
        (
            $name,
            include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/", $name)) as &[u8],
        )
    };
}

/// Every resource file of the module, by absolute name (no leading `/`), in
/// name order.
static RESOURCES: &[(&str, &[u8])] = &[
    resource!("org/apache/lucene/analysis/ar/stopwords.txt"),
    resource!("org/apache/lucene/analysis/bg/stopwords.txt"),
    resource!("org/apache/lucene/analysis/bn/stopwords.txt"),
    resource!("org/apache/lucene/analysis/br/stopwords.txt"),
    resource!("org/apache/lucene/analysis/ca/stopwords.txt"),
    resource!("org/apache/lucene/analysis/cjk/stopwords.txt"),
    resource!("org/apache/lucene/analysis/ckb/stopwords.txt"),
    resource!("org/apache/lucene/analysis/compound/hyphenation/hyphenation.dtd"),
    resource!("org/apache/lucene/analysis/cz/stopwords.txt"),
    resource!("org/apache/lucene/analysis/el/stopwords.txt"),
    resource!("org/apache/lucene/analysis/et/stopwords.txt"),
    resource!("org/apache/lucene/analysis/eu/stopwords.txt"),
    resource!("org/apache/lucene/analysis/fa/stopwords.txt"),
    resource!("org/apache/lucene/analysis/gl/galician.rslp"),
    resource!("org/apache/lucene/analysis/gl/stopwords.txt"),
    resource!("org/apache/lucene/analysis/hi/stopwords.txt"),
    resource!("org/apache/lucene/analysis/hy/stopwords.txt"),
    resource!("org/apache/lucene/analysis/id/stopwords.txt"),
    resource!("org/apache/lucene/analysis/lt/stopwords.txt"),
    resource!("org/apache/lucene/analysis/lv/stopwords.txt"),
    resource!("org/apache/lucene/analysis/ne/stopwords.txt"),
    resource!("org/apache/lucene/analysis/pt/portuguese.rslp"),
    resource!("org/apache/lucene/analysis/ro/stopwords.txt"),
    resource!("org/apache/lucene/analysis/snowball/danish_stop.txt"),
    resource!("org/apache/lucene/analysis/snowball/dutch_stop.txt"),
    resource!("org/apache/lucene/analysis/snowball/english_stop.txt"),
    resource!("org/apache/lucene/analysis/snowball/finnish_stop.txt"),
    resource!("org/apache/lucene/analysis/snowball/french_stop.txt"),
    resource!("org/apache/lucene/analysis/snowball/german_stop.txt"),
    resource!("org/apache/lucene/analysis/snowball/hungarian_stop.txt"),
    resource!("org/apache/lucene/analysis/snowball/indonesian_stop.txt"),
    resource!("org/apache/lucene/analysis/snowball/irish_stop.txt"),
    resource!("org/apache/lucene/analysis/snowball/italian_stop.txt"),
    resource!("org/apache/lucene/analysis/snowball/norwegian_stop.txt"),
    resource!("org/apache/lucene/analysis/snowball/portuguese_stop.txt"),
    resource!("org/apache/lucene/analysis/snowball/russian_stop.txt"),
    resource!("org/apache/lucene/analysis/snowball/spanish_stop.txt"),
    resource!("org/apache/lucene/analysis/snowball/swedish_stop.txt"),
    resource!("org/apache/lucene/analysis/sr/stopwords.txt"),
    resource!("org/apache/lucene/analysis/ta/stopwords.txt"),
    resource!("org/apache/lucene/analysis/te/stopwords.txt"),
    resource!("org/apache/lucene/analysis/th/stopwords.txt"),
    resource!("org/apache/lucene/analysis/tr/stopwords.txt"),
];

/// Returns the bytes of the resource whose absolute name is `name`, or
/// `None` when the module has none.
///
/// Equivalent to `ClassLoader.getResource(name)` (read as
/// `getResourceAsStream`) of the class loader that holds
/// `lucene-analysis-common.jar`: the name is absolute and has no leading
/// `/`, which no resource name starts with, so a name that does is not
/// found.
pub fn get_resource(name: &str) -> Option<&'static [u8]> {
    RESOURCES
        .binary_search_by(|(resource, _)| (*resource).cmp(name))
        .ok()
        .map(|index| RESOURCES[index].1)
}

/// Opens the resource `name` relative to the class whose binary name is
/// `class_name`, or answers `None` when the module has no such resource.
///
/// Equivalent to `Class.getResourceAsStream(String)` for one of the
/// module's classes (`Class.resolveName`): a name starting with `/` is
/// absolute and loses the `/`; any other name is prefixed with the class's
/// package, its dots turned into `/` (`stopwords.txt` of
/// `org.apache.lucene.analysis.tr.TurkishAnalyzer` is
/// `org/apache/lucene/analysis/tr/stopwords.txt`).
pub fn get_resource_as_stream(class_name: &str, name: &str) -> Option<Box<dyn Read + Send>> {
    let absolute = match name.strip_prefix('/') {
        Some(absolute) => absolute.to_string(),
        None => match class_name.rfind('.') {
            Some(dot) => format!("{}/{name}", class_name[..dot].replace('.', "/")),
            None => name.to_string(),
        },
    };
    get_resource(&absolute).map(|bytes| Box::new(Cursor::new(bytes)) as Box<dyn Read + Send>)
}

/// Returns the module's resources keyed by absolute name, the table
/// [`ClasspathResourceLoader::new`](rucene::util::resource_loader::ClasspathResourceLoader::new)
/// takes as the class loader that holds `lucene-analysis-common.jar`
/// (Java's `new ClasspathResourceLoader(CustomAnalyzer.class.getClassLoader())`).
pub fn class_loader_resources() -> HashMap<String, Arc<Vec<u8>>> {
    static TABLE: LazyLock<HashMap<String, Arc<Vec<u8>>>> = LazyLock::new(|| {
        RESOURCES
            .iter()
            .map(|(name, bytes)| ((*name).to_string(), Arc::new(bytes.to_vec())))
            .collect()
    });
    TABLE.clone()
}
