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

//! Port of `SingleFieldTestDb.java`.

use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store, TextField};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene_test_framework::analysis::mock_analyzer::MockAnalyzer;
use rucene_test_framework::store::mock_directory_wrapper::MockDirectoryWrapper;

/// `public class SingleFieldTestDb`.
#[allow(dead_code)]
pub struct SingleFieldTestDb {
    db: Arc<dyn Directory>,
    docs: Vec<String>,
    field_name: String,
}

#[allow(dead_code)]
impl SingleFieldTestDb {
    /// `new SingleFieldTestDb(Random random, String[] documents, String
    /// fName)`: an `IOException` becomes `new Error(ioe)`.
    pub fn new(random: &dyn Random, documents: &[&str], f_name: &str) -> SingleFieldTestDb {
        let result = (|| -> rucene::error::Result<SingleFieldTestDb> {
            let db: Arc<dyn Directory> = Arc::new(MockDirectoryWrapper::new(
                random,
                Arc::new(ByteBuffersDirectory::default()),
            ));
            let docs: Vec<String> = documents.iter().map(|d| d.to_string()).collect();
            let field_name = f_name.to_string();
            let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(random));
            let writer = IndexWriter::new(
                Arc::clone(&db),
                IndexWriterConfig::with_analyzer(Some(analyzer)),
            )?;
            for doc in &docs {
                let mut d = Document::new();
                d.add(TextField::new(&field_name, doc.clone(), Store::NO)?);
                writer.add_document(&d)?;
            }
            writer.close()?;
            Ok(SingleFieldTestDb {
                db,
                docs,
                field_name,
            })
        })();
        match result {
            Ok(db) => db,
            Err(e) => randomizedtesting::jdk::exception::Throwable::from_cause(
                "java.lang.Error",
                rucene_test_framework::util::java_exceptions::java_throwable(&e),
            )
            .throw(),
        }
    }

    /// `getDb()`.
    pub fn get_db(&self) -> Arc<dyn Directory> {
        Arc::clone(&self.db)
    }

    /// `getDocs()`.
    pub fn get_docs(&self) -> &[String] {
        &self.docs
    }

    /// `getFieldname()`.
    pub fn get_fieldname(&self) -> &str {
        &self.field_name
    }
}
