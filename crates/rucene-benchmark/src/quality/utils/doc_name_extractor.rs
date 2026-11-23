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

//! The types declared in `DocNameExtractor.java`, of the Java package
//! `org.apache.lucene.benchmark.quality.utils`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::index_reader::StoredFields;
use rucene::index::{FieldInfo, StoredFieldVisitor, StoredFieldVisitorStatus};

/// Equivalent to `org.apache.lucene.benchmark.quality.utils.DocNameExtractor`:
/// utility: extract doc names from an index.
#[derive(Clone, Debug)]
pub struct DocNameExtractor {
    doc_name_field: String,
}

/// The anonymous `StoredFieldVisitor` of `docName`.
struct NameVisitor<'a> {
    doc_name_field: &'a str,
    name: Vec<String>,
}

impl StoredFieldVisitor for NameVisitor<'_> {
    fn string_field(&mut self, _field_info: &Arc<FieldInfo>, value: String) -> Result<()> {
        self.name.push(value);
        Ok(())
    }

    fn needs_field(&mut self, field_info: &FieldInfo) -> Result<StoredFieldVisitorStatus> {
        if !self.name.is_empty() {
            Ok(StoredFieldVisitorStatus::Stop)
        } else if field_info.name == self.doc_name_field {
            Ok(StoredFieldVisitorStatus::Yes)
        } else {
            Ok(StoredFieldVisitorStatus::No)
        }
    }
}

impl DocNameExtractor {
    /// `DocNameExtractor(String docNameField)`: `doc_name_field` is the name
    /// of the stored field containing the doc name.
    pub fn new(doc_name_field: &str) -> Self {
        DocNameExtractor {
            doc_name_field: doc_name_field.to_string(),
        }
    }

    /// `docName(StoredFields storedFields, int docid)`: extract the name of
    /// the input doc from the index.
    ///
    /// # Errors
    ///
    /// If cannot extract the doc name from the index.
    pub fn doc_name(&self, stored_fields: &dyn StoredFields, docid: i32) -> Result<Option<String>> {
        let mut visitor = NameVisitor {
            doc_name_field: &self.doc_name_field,
            name: Vec::new(),
        };
        stored_fields.document_with_visitor(docid, &mut visitor)?;
        Ok(visitor.name.into_iter().next())
    }
}
