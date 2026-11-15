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

//! The port of `FieldsQuery.java`, of the Java package
//! `org.apache.lucene.queryparser.surround.query`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::search::Query;

use super::basic_query_factory::BasicQueryFactory;
use super::or_query::OrQuery;
use super::srnd_query::{SrndQuery, SrndQueryState};

/// For expanded queries, not normally visible.
const OR_OPERATOR_NAME: &str = "OR";

/// Forms an OR query of the provided query across multiple fields.
///
/// Equivalent to `org.apache.lucene.queryparser.surround.query.FieldsQuery`.
#[derive(Debug, Clone)]
pub struct FieldsQuery {
    srnd: SrndQueryState,
    /* mostly untested */
    q: Arc<dyn SrndQuery>,
    field_names: Vec<String>,
    field_op: char,
}

impl FieldsQuery {
    /// Equivalent to `new FieldsQuery(SrndQuery, List<String>, char)`.
    pub fn new(q: Arc<dyn SrndQuery>, field_names: Vec<String>, field_op: char) -> Self {
        Self {
            srnd: SrndQueryState::new(),
            q,
            field_names,
            field_op,
        }
    }

    /// Equivalent to `new FieldsQuery(SrndQuery, String, char)`.
    pub fn with_field_name(q: Arc<dyn SrndQuery>, field_name: &str, field_op: char) -> Self {
        Self::new(q, vec![field_name.to_string()], field_op)
    }

    /// Equivalent to `makeLuceneQueryNoBoost(BasicQueryFactory)`.
    ///
    /// # Errors
    ///
    /// The errors of the query construction.
    pub fn make_lucene_query_no_boost(
        &self,
        qf: &Arc<BasicQueryFactory>,
    ) -> Result<Arc<dyn Query>> {
        if self.field_names.len() == 1 {
            /* single field name: no new queries needed */
            Arc::clone(&self.q).make_lucene_query_field_no_boost(Some(&self.field_names[0]), qf)
        } else {
            /* OR query over the fields */
            let mut queries: Vec<Arc<dyn SrndQuery>> = Vec::new();
            for field_name in self.get_field_names() {
                let qc = self.q.clone_srnd();
                queries.push(Arc::new(FieldsQuery::with_field_name(
                    qc,
                    field_name,
                    self.field_op,
                )));
            }
            let oq = Arc::new(OrQuery::new(
                queries,
                true, /* infix OR for field names */
                OR_OPERATOR_NAME,
            )?);
            // System.out.println(getClass().toString() + ", fields expanded: " + oq.toString()); /* needs
            // testing */
            oq.make_lucene_query_field(None, qf)
        }
    }

    /// Equivalent to `getFieldNames()`.
    pub fn get_field_names(&self) -> &[String] {
        &self.field_names
    }

    /// Equivalent to `getFieldOperator()`.
    pub fn get_field_operator(&self) -> char {
        self.field_op
    }

    /// Equivalent to the protected `fieldNamesToString(StringBuilder)`.
    fn field_names_to_string(&self, r: &mut String) {
        for field_name in self.get_field_names() {
            r.push_str(field_name);
            r.push(self.get_field_operator());
        }
    }
}

impl SrndQuery for FieldsQuery {
    fn srnd_state(&self) -> &SrndQueryState {
        &self.srnd
    }

    fn is_fields_sub_query_acceptable(&self) -> bool {
        false
    }

    fn make_lucene_query_field_no_boost(
        self: Arc<Self>,
        _field_name: Option<&str>,
        qf: &Arc<BasicQueryFactory>,
    ) -> Result<Arc<dyn Query>> {
        self.make_lucene_query_no_boost(qf) /* use this.fieldNames instead of fieldName */
    }

    fn srnd_to_string(&self) -> String {
        let mut r = String::new();
        r.push('(');
        self.field_names_to_string(&mut r);
        r.push_str(&self.q.srnd_to_string());
        r.push(')');
        r
    }

    fn clone_srnd(&self) -> Arc<dyn SrndQuery> {
        Arc::new(self.clone())
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queryparser.surround.query.FieldsQuery"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
