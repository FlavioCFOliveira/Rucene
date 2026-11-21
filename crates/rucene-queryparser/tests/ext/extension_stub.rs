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

//! Port of `ExtensionStub.java`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::Term;
use rucene::search::term_query::TermQuery;
use rucene::search::Query;
use rucene_queryparser::ext::extension_query::ExtensionQuery;
use rucene_queryparser::ext::parser_extension::ParserExtension;

/// `class ExtensionStub extends ParserExtension`.
#[derive(Debug, Default)]
pub struct ExtensionStub;

impl ParserExtension for ExtensionStub {
    fn parse(&self, components: ExtensionQuery<'_>) -> Result<Option<Arc<dyn Query>>> {
        Ok(Some(Arc::new(TermQuery::new(
            Term::from_text_nullable_field(components.field(), components.raw_query_string()),
        ))))
    }
}
