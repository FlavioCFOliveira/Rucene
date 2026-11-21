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

//! The core error `toString` hook (rmp #503): the Lucene 10.5.0 exception
//! classes of the ported modules that override `toString()` are
//! `QueryNodeException` (`QueryNodeException.java:76-79`) and, inheriting
//! it, `QueryNodeParseException` and the flexible `ParseException`. The
//! `Display` of the error each is thrown as must equal the JVM's
//! `toString()` of the same exception, computed by Lucene 10.5.0 on the
//! local JDK 25 in a work directory of its own under `TMPDIR`.

use query_oracle::jvm_fixture;

#[path = "../../rucene-queries/tests/query_oracle/mod.rs"]
mod query_oracle;

use std::process::Command;
use std::sync::Arc;

use rucene_queryparser::flexible::core::messages::query_parser_messages::QueryParserMessages;
use rucene_queryparser::flexible::core::query_node_exception::QueryNodeException;
use rucene_queryparser::flexible::core::query_node_parse_exception::QueryNodeParseException;
use rucene_queryparser::flexible::messages::message_impl::MessageImpl;
use rucene_queryparser::flexible::standard::parser::parse_exception::ParseException;
use rucene_queryparser::flexible::standard::standard_query_parser::StandardQueryParser;
use rucene_queryparser::jdk::object::JavaObject;

use query_oracle::{lucene_jars, WorkDir};

/// Prints `toString()` of one instance of each class, then of the exception
/// a syntax error throws from `StandardQueryParser.parse`.
const SOURCE: &str = r#"
import org.apache.lucene.queryparser.flexible.core.QueryNodeException;
import org.apache.lucene.queryparser.flexible.core.QueryNodeParseException;
import org.apache.lucene.queryparser.flexible.core.messages.QueryParserMessages;
import org.apache.lucene.queryparser.flexible.messages.MessageImpl;
import org.apache.lucene.queryparser.flexible.standard.StandardQueryParser;
import org.apache.lucene.queryparser.flexible.standard.parser.ParseException;

public class ToStringOracle {
  public static void main(String[] args) {
    System.out.println(new QueryNodeException(new MessageImpl(
        QueryParserMessages.INVALID_SYNTAX_CANNOT_PARSE, "foo:", "Encountered EOF")));
    System.out.println(new QueryNodeParseException(new MessageImpl(
        QueryParserMessages.INVALID_SYNTAX, "bar")));
    System.out.println(new ParseException(new MessageImpl(
        QueryParserMessages.INVALID_SYNTAX, "baz")));
    try {
      new StandardQueryParser().parse("a:", "f");
      System.out.println("no exception");
    } catch (Exception e) {
      System.out.println(e.getClass().getName() + "|" + e);
    }
  }
}
"#;

fn java_lines() -> Vec<String> {
    let work = WorkDir::new("query-node-exception-tostring");
    let file = work.path().join("ToStringOracle.java");
    std::fs::write(&file, SOURCE).expect("write the Java source");
    let class_path = std::env::join_paths(lucene_jars()).expect("a class path");
    let output = jvm_fixture::launch(
        Command::new("java")
            .current_dir(work.path())
            .arg(format!("-Djava.io.tmpdir={}", work.path().display()))
            .arg("-Xmx512m")
            .arg("-cp")
            .arg(class_path)
            .arg(&file),
    )
    .output()
    .expect("a JDK 25 is required");
    assert!(
        output.status.success(),
        "ToStringOracle.java failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("UTF-8")
        .lines()
        .map(str::to_string)
        .collect()
}

fn message(
    key: &rucene_queryparser::flexible::messages::nls::NlsField,
    args: &[&str],
) -> Arc<MessageImpl> {
    Arc::new(MessageImpl::with_args(
        key.get(),
        args.iter()
            .map(|a| JavaObject::String(a.to_string()))
            .collect::<Vec<_>>(),
    ))
}

#[test]
fn lucene_exceptions_render_their_own_to_string() {
    let java = java_lines();
    assert_eq!(java.len(), 4, "four lines: {java:?}");

    let qne = QueryNodeException::new(message(
        QueryParserMessages::INVALID_SYNTAX_CANNOT_PARSE,
        &["foo:", "Encountered EOF"],
    ))
    .error();
    assert_eq!(qne.to_string(), java[0], "QueryNodeException.toString()");

    let qnpe = QueryNodeParseException::new(message(QueryParserMessages::INVALID_SYNTAX, &["bar"]))
        .error();
    assert_eq!(
        qnpe.to_string(),
        java[1],
        "QueryNodeParseException.toString()"
    );

    let pe = ParseException::with_message(message(QueryParserMessages::INVALID_SYNTAX, &["baz"]))
        .error();
    assert_eq!(pe.to_string(), java[2], "ParseException.toString()");

    // The `toString()` override differs from `Throwable.toString()`
    // (`class: message`), so the hook is what renders it.
    assert!(!java[0].starts_with("org.apache.lucene"));

    let thrown = StandardQueryParser::new()
        .parse("a:", Some("f"))
        .expect_err("a syntax error");
    let class = rucene_test_framework::util::java_exceptions::java_class_name(&thrown);
    assert_eq!(
        format!("{class}|{thrown}"),
        java[3],
        "a thrown QueryNodeParseException"
    );
}
