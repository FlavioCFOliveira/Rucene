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

//! Port of `CoreParser.java`, of the Java package
//! `org.apache.lucene.queryparser.xml`.

use std::io::Read;
use std::sync::{Arc, LazyLock};

use rucene::analysis::Analyzer;
use rucene::error::Result;
use rucene::search::query::Query;
use rucene_queries::spans::span_query::SpanQuery;

use super::builders::boolean_query_builder::BooleanQueryBuilder;
use super::builders::boosting_term_builder::BoostingTermBuilder;
use super::builders::constant_score_query_builder::ConstantScoreQueryBuilder;
use super::builders::disjunction_max_query_builder::DisjunctionMaxQueryBuilder;
use super::builders::match_all_docs_query_builder::MatchAllDocsQueryBuilder;
use super::builders::point_range_query_builder::PointRangeQueryBuilder;
use super::builders::range_query_builder::RangeQueryBuilder;
use super::builders::span_first_builder::SpanFirstBuilder;
use super::builders::span_near_builder::SpanNearBuilder;
use super::builders::span_not_builder::SpanNotBuilder;
use super::builders::span_or_builder::SpanOrBuilder;
use super::builders::span_or_terms_builder::SpanOrTermsBuilder;
use super::builders::span_position_range_builder::SpanPositionRangeBuilder;
use super::builders::span_query_builder::SpanQueryBuilder;
use super::builders::span_query_builder_factory::SpanQueryBuilderFactory;
use super::builders::span_term_builder::SpanTermBuilder;
use super::builders::term_query_builder::TermQueryBuilder;
use super::builders::terms_query_builder::TermsQueryBuilder;
use super::builders::user_input_query_builder::{SharedQueryParser, UserInputQueryBuilder};
use super::parser_exception::{exception_to_string, ParserException};
use super::query_builder::QueryBuilder;
use super::query_builder_factory::QueryBuilderFactory;
use crate::jdk::dom::{
    Document, DocumentBuilderFactory, Element, EntityResolver, ErrorHandler, InputSource,
    SaxException, FEATURE_SECURE_PROCESSING,
};

/// The entity resolver Java's lambda `DISALLOW_EXTERNAL_ENTITY_RESOLVER` is.
#[derive(Debug, Default, Clone, Copy)]
pub struct DisallowExternalEntityResolver;

impl EntityResolver for DisallowExternalEntityResolver {
    /// Throws `SAXException(String.format(Locale.ENGLISH, "External Entity
    /// resolving unsupported:  publicId=\"%s\" systemId=\"%s\"", publicId,
    /// systemId))`.
    fn resolve_entity(
        &self,
        public_id: Option<&str>,
        system_id: Option<&str>,
    ) -> Result<Option<InputSource>> {
        Err(SaxException::new(format!(
            "External Entity resolving unsupported:  publicId=\"{}\" systemId=\"{}\"",
            public_id.unwrap_or("null"),
            system_id.unwrap_or("null")
        ))
        .error())
    }
}

/// Equivalent to `CoreParser.DISALLOW_EXTERNAL_ENTITY_RESOLVER`: an entity
/// resolver that throws a `SAXException` for every external entity.
pub static DISALLOW_EXTERNAL_ENTITY_RESOLVER: LazyLock<Arc<dyn EntityResolver>> =
    LazyLock::new(|| Arc::new(DisallowExternalEntityResolver));

/// Assembles a QueryBuilder which uses only core Lucene Query objects
///
/// Equivalent to `org.apache.lucene.queryparser.xml.CoreParser`: its state;
/// its overridable and inherited methods are [`CoreParserOps`], which it
/// and its subclasses (which embed it) implement.
pub struct CoreParser {
    /// `protected String defaultField`.
    pub default_field: Option<String>,
    /// `protected Analyzer analyzer`.
    pub analyzer: Option<Arc<dyn Analyzer>>,
    /// `protected QueryParser parser`.
    pub parser: Option<SharedQueryParser>,
    /// `protected QueryBuilderFactory queryFactory`.
    pub query_factory: Arc<QueryBuilderFactory>,
    /// `protected final SpanQueryBuilderFactory spanFactory`.
    pub span_factory: Arc<SpanQueryBuilderFactory>,
}

impl std::fmt::Debug for CoreParser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreParser")
            .field("default_field", &self.default_field)
            .finish()
    }
}

impl CoreParser {
    /// Construct an XML parser that uses a single instance QueryParser for
    /// handling UserQuery tags - all parse operations are synchronised on
    /// this parser
    ///
    /// Equivalent to `new CoreParser(Analyzer analyzer, QueryParser parser)`.
    ///
    /// * `parser` - A QueryParser which will be synchronized on during
    ///   parse calls.
    pub fn with_parser(
        analyzer: Option<Arc<dyn Analyzer>>,
        parser: Option<SharedQueryParser>,
    ) -> Self {
        Self::with_all(None, analyzer, parser)
    }

    /// Constructs an XML parser that creates a QueryParser for each UserQuery
    /// request.
    ///
    /// Equivalent to `new CoreParser(String defaultField, Analyzer
    /// analyzer)`.
    ///
    /// * `default_field` - The default field name used by QueryParsers
    ///   constructed for UserQuery tags
    pub fn new(default_field: Option<String>, analyzer: Option<Arc<dyn Analyzer>>) -> Self {
        Self::with_all(default_field, analyzer, None)
    }

    /// Equivalent to the protected `CoreParser(String defaultField, Analyzer
    /// analyzer, QueryParser parser)`: the core builders registered.
    pub fn with_all(
        default_field: Option<String>,
        analyzer: Option<Arc<dyn Analyzer>>,
        parser: Option<SharedQueryParser>,
    ) -> Self {
        let query_factory = Arc::new(QueryBuilderFactory::new());
        let span_factory = Arc::new(SpanQueryBuilderFactory::new());
        let query_builder_factory: Arc<dyn QueryBuilder> = query_factory.clone();
        let span_builder_factory: Arc<dyn SpanQueryBuilder> = span_factory.clone();

        query_factory.add_builder("TermQuery", Arc::new(TermQueryBuilder::new()));
        query_factory.add_builder(
            "TermsQuery",
            Arc::new(TermsQueryBuilder::new(analyzer.clone())),
        );
        query_factory.add_builder(
            "MatchAllDocsQuery",
            Arc::new(MatchAllDocsQueryBuilder::new()),
        );
        query_factory.add_builder(
            "BooleanQuery",
            Arc::new(BooleanQueryBuilder::new(Arc::clone(&query_builder_factory))),
        );
        query_factory.add_builder("PointRangeQuery", Arc::new(PointRangeQueryBuilder::new()));
        query_factory.add_builder("RangeQuery", Arc::new(RangeQueryBuilder::new()));
        query_factory.add_builder(
            "DisjunctionMaxQuery",
            Arc::new(DisjunctionMaxQueryBuilder::new(Arc::clone(
                &query_builder_factory,
            ))),
        );
        match &parser {
            Some(parser) => {
                query_factory.add_builder(
                    "UserQuery",
                    Arc::new(UserInputQueryBuilder::with_parser(Arc::clone(parser))),
                );
            }
            None => {
                query_factory.add_builder(
                    "UserQuery",
                    Arc::new(UserInputQueryBuilder::new(
                        default_field.clone(),
                        analyzer.clone(),
                    )),
                );
            }
        }
        query_factory.add_builder(
            "ConstantScoreQuery",
            Arc::new(ConstantScoreQueryBuilder::new(Arc::clone(&query_factory))),
        );

        let snb = Arc::new(SpanNearBuilder::new(Arc::clone(&span_builder_factory)));
        span_factory.add_builder("SpanNear", snb.clone());
        query_factory.add_builder("SpanNear", snb);

        let btb = Arc::new(BoostingTermBuilder::new());
        span_factory.add_builder("BoostingTermQuery", btb.clone());
        query_factory.add_builder("BoostingTermQuery", btb);

        let snt = Arc::new(SpanTermBuilder::new());
        span_factory.add_builder("SpanTerm", snt.clone());
        query_factory.add_builder("SpanTerm", snt);

        let sot = Arc::new(SpanOrBuilder::new(Arc::clone(&span_builder_factory)));
        span_factory.add_builder("SpanOr", sot.clone());
        query_factory.add_builder("SpanOr", sot);

        let sots = Arc::new(SpanOrTermsBuilder::new(analyzer.clone()));
        span_factory.add_builder("SpanOrTerms", sots.clone());
        query_factory.add_builder("SpanOrTerms", sots);

        let sft = Arc::new(SpanFirstBuilder::new(Arc::clone(&span_builder_factory)));
        span_factory.add_builder("SpanFirst", sft.clone());
        query_factory.add_builder("SpanFirst", sft);

        let sprt = Arc::new(SpanPositionRangeBuilder::new(Arc::clone(
            &span_builder_factory,
        )));
        span_factory.add_builder("SpanPositionRange", sprt.clone());
        query_factory.add_builder("SpanPositionRange", sprt);

        let snot = Arc::new(SpanNotBuilder::new(span_builder_factory));
        span_factory.add_builder("SpanNot", snot.clone());
        query_factory.add_builder("SpanNot", snot);

        Self {
            default_field,
            analyzer,
            parser,
            query_factory,
            span_factory,
        }
    }
}

/// The overridable and inherited methods of `CoreParser`, which it and its
/// subclasses implement.
///
/// Equivalent to the public and protected methods of
/// `org.apache.lucene.queryparser.xml.CoreParser`.
pub trait CoreParserOps: SpanQueryBuilder {
    /// The `CoreParser` state.
    fn core_parser(&self) -> &CoreParser;

    /// Equivalent to the protected `getEntityResolver()`: by default
    /// [`DISALLOW_EXTERNAL_ENTITY_RESOLVER`].
    fn get_entity_resolver(&self) -> Option<Arc<dyn EntityResolver>> {
        Some(Arc::clone(&DISALLOW_EXTERNAL_ENTITY_RESOLVER))
    }

    /// Equivalent to the protected `getErrorHandler()`: by default `null`.
    fn get_error_handler(&self) -> Option<Arc<dyn ErrorHandler>> {
        None
    }

    /// Parses the given stream as XML file and returns a [`Query`]. By
    /// default this disallows external entities for security reasons.
    ///
    /// Equivalent to `parse(InputStream xmlStream)`.
    ///
    /// # Errors
    ///
    /// A `ParserException` for an unparsable document or query.
    fn parse(&self, xml_stream: &mut dyn Read) -> Result<Arc<dyn Query>> {
        let document = parse_xml(self, xml_stream)?;
        let element = document_element(&document)?;
        self.get_query(&element)
    }

    /// Equivalent to the package-private `parseAsSpanQuery(InputStream
    /// xmlStream)` (for test use).
    ///
    /// # Errors
    ///
    /// As [`parse`](Self::parse).
    fn parse_as_span_query(&self, xml_stream: &mut dyn Read) -> Result<Arc<dyn SpanQuery>> {
        let document = parse_xml(self, xml_stream)?;
        let element = document_element(&document)?;
        self.get_span_query(&element)
    }

    /// Equivalent to `addQueryBuilder(String nodeName, QueryBuilder
    /// builder)`.
    fn add_query_builder(&self, node_name: &str, builder: Arc<dyn QueryBuilder>) {
        self.core_parser()
            .query_factory
            .add_builder(node_name, builder);
    }

    /// Equivalent to `addSpanBuilder(String nodeName, SpanQueryBuilder
    /// builder)`.
    fn add_span_builder(&self, node_name: &str, builder: Arc<dyn SpanQueryBuilder>) {
        self.core_parser()
            .span_factory
            .add_builder(node_name, builder);
    }

    /// Equivalent to `addSpanQueryBuilder(String nodeName, SpanQueryBuilder
    /// builder)`: both factories.
    fn add_span_query_builder(&self, node_name: &str, builder: Arc<dyn SpanQueryBuilder>) {
        self.core_parser()
            .query_factory
            .add_builder(node_name, builder.clone());
        self.core_parser()
            .span_factory
            .add_builder(node_name, builder);
    }
}

/// `getDocumentElement()` of a parsed document.
fn document_element(document: &Document) -> Result<Element> {
    document
        .get_document_element()
        .ok_or_else(rucene::error::LuceneError::null_pointer)
}

/// Equivalent to the private `parseXML(InputStream pXmlFile)`: a factory not
/// validating, with `FEATURE_SECURE_PROCESSING`, the parser's entity
/// resolver and error handler.
///
/// # Errors
///
/// `ParserException("XML Parser configuration error.", se)` when the
/// builder cannot be created, and `ParserException("Error parsing XML
/// stream: " + se, se)` for an exception of the parse (an `Error` passes
/// unchanged).
fn parse_xml<P: CoreParserOps + ?Sized>(this: &P, p_xml_file: &mut dyn Read) -> Result<Document> {
    let mut dbf = DocumentBuilderFactory::new_instance();
    dbf.set_validating(false);
    // ignore an error since all implementations are required to support the
    // FEATURE_SECURE_PROCESSING feature
    let _ = dbf.set_feature(FEATURE_SECURE_PROCESSING, true);
    let mut db = match dbf.new_document_builder() {
        Ok(db) => db,
        Err(se) => {
            return Err(ParserException::with_message_and_cause(
                "XML Parser configuration error.",
                se,
            )
            .error())
        }
    };
    db.set_entity_resolver(this.get_entity_resolver());
    db.set_error_handler(this.get_error_handler());
    db.parse(p_xml_file).map_err(|se| {
        if crate::javacc::is_exception(&se) {
            let message = format!("Error parsing XML stream: {}", exception_to_string(&se));
            ParserException::with_message_and_cause(message, se).error()
        } else {
            se
        }
    })
}

impl QueryBuilder for CoreParser {
    fn get_query(&self, e: &Element) -> Result<Arc<dyn Query>> {
        self.query_factory.get_query(e)
    }
}

impl SpanQueryBuilder for CoreParser {
    fn get_span_query(&self, e: &Element) -> Result<Arc<dyn SpanQuery>> {
        self.span_factory.get_span_query(e)
    }
}

impl CoreParserOps for CoreParser {
    fn core_parser(&self) -> &CoreParser {
        self
    }
}
