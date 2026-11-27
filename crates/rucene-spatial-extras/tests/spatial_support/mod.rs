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

//! Helpers shared by the ported spatial-extras test classes: the Java
//! semantics their bodies need (`LinkedHashSet`, `Collections.sort` of
//! strings, `@ParametersFactory` arguments) and the suite plumbing.

#![allow(dead_code, unused_imports)]

use std::sync::Arc;

use randomizedtesting::annotations::ParametersFactory;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{modifier, Method, Value};
use rucene_spatial_extras::spatial_strategy::SpatialStrategy;
use rucene_test_framework::util::LuceneTestCase;

/// `random()`.
pub fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// `java.util.LinkedHashSet<String>`: a set iterating in insertion order.
#[derive(Clone, Debug, Default)]
pub struct LinkedHashSet {
    items: Vec<String>,
}

impl LinkedHashSet {
    /// `new LinkedHashSet<>()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `add(e)`.
    pub fn add(&mut self, e: String) -> bool {
        if self.items.contains(&e) {
            return false;
        }
        self.items.push(e);
        true
    }

    /// `remove(o)`.
    pub fn remove(&mut self, o: &str) -> bool {
        match self.items.iter().position(|i| i == o) {
            Some(p) => {
                self.items.remove(p);
                true
            }
            None => false,
        }
    }

    /// `contains(o)`.
    pub fn contains(&self, o: &str) -> bool {
        self.items.iter().any(|i| i == o)
    }

    /// `removeAll(c)`.
    pub fn remove_all(&mut self, c: &LinkedHashSet) {
        self.items.retain(|i| !c.contains(i));
    }

    /// `iterator().next()` of a non-empty set; `None` when empty.
    pub fn first(&self) -> Option<&String> {
        self.items.first()
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// `Collections.sort(List<String>)`: `String.compareTo`, UTF-16 code unit
/// order.
pub fn java_string_sort(list: &mut [String]) {
    list.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
}

/// A strategy as a `@ParametersFactory` argument (`Object`), with its
/// `toString()`.
pub fn strategy_value(strategy: Arc<dyn SpatialStrategy>) -> Value {
    let name = strategy.to_string();
    Value::Object(Arc::new(strategy), name)
}

/// The strategy argument of a parameterized constructor.
pub fn strategy_arg(value: &Value) -> Arc<dyn SpatialStrategy> {
    match value {
        Value::Object(o, _) => Arc::clone(
            o.downcast_ref::<Arc<dyn SpatialStrategy>>()
                .expect("a strategy argument"),
        ),
        _ => panic!("java.lang.IllegalArgumentException: argument type mismatch"),
    }
}

/// `@ParametersFactory(argumentFormatting = format) public static
/// Iterable<Object[]> parameters()`, whose rows `rows` builds.
pub fn parameters_method(format: &str, rows: fn() -> Vec<Vec<Value>>) -> Method {
    Method::new("parameters")
        .modifiers(modifier::PUBLIC | modifier::STATIC)
        .returns("java.lang.Iterable<java.lang.Object[]>")
        .annotate(ParametersFactory {
            argument_formatting: format.to_string(),
            ..ParametersFactory::default()
        })
        .static_value(move |_| Value::List(rows().into_iter().map(Value::Array).collect()))
}

/// `Calendar` helpers: Java's `Calendar.compareTo`, `Calendar.equals`, and
/// `DateRangePrefixTree.clearFieldsAfter` under a `catch (AssertionError e)`.
pub mod cal {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    use randomizedtesting::jdk::exception::Throwable;
    use randomizedtesting::junit::Assert;
    use rucene_queryparser::jdk::calendar::Calendar;
    use rucene_spatial_extras::prefix::tree::date_range_prefix_tree::DateRangePrefixTree;

    /// `Calendar.getMillisOf(cal)`: the time of a clone, made lenient (the
    /// calendar itself is left untouched).
    pub fn millis_of(cal: &Calendar) -> i64 {
        let mut c = cal.clone();
        c.set_lenient(true);
        c.get_time_in_millis()
            .unwrap_or_else(|e| rucene_test_framework::util::java_exceptions::rethrow_java(e))
    }

    /// `Calendar.compareTo(Calendar anotherCalendar)`.
    pub fn compare_to(a: &Calendar, b: &Calendar) -> i32 {
        match millis_of(a).cmp(&millis_of(b)) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }

    /// `Calendar.equals(Object obj)` between two `GregorianCalendar`s
    /// (`GregorianCalendar.equals` adds the cutover).
    pub fn equals(a: &Calendar, b: &Calendar) -> bool {
        millis_of(a) == millis_of(b)
            && a.is_lenient() == b.is_lenient()
            && a.get_first_day_of_week() == b.get_first_day_of_week()
            && a.get_minimal_days_in_first_week() == b.get_minimal_days_in_first_week()
            && a.get_time_zone() == b.get_time_zone()
            && a.get_gregorian_change() == b.get_gregorian_change()
    }

    /// `assertEquals([message,] Calendar expected, Calendar actual)`.
    pub fn assert_equals(message: Option<&str>, expected: &Calendar, actual: &Calendar) {
        if !equals(expected, actual) {
            let prefix = match message {
                Some(m) if !m.is_empty() => format!("{m} "),
                _ => String::new(),
            };
            Assert::fail(&format!(
                "{prefix}expected:<GregorianCalendar[time={}]> but was:<GregorianCalendar[time={}]>",
                millis_of(expected),
                millis_of(actual)
            ));
        }
    }

    /// `try { tree.clearFieldsAfter(cal, field); } catch (AssertionError e) {
    /// if (!e.getMessage().equals("Calendar underflow")) throw e; … }`:
    /// whether the "Calendar underflow" assertion was caught.
    pub fn clear_fields_after_catching_underflow(
        tree: &DateRangePrefixTree,
        cal: &mut Calendar,
        field: i32,
    ) -> bool {
        match catch_unwind(AssertUnwindSafe(|| tree.clear_fields_after(cal, field))) {
            Ok(()) => false,
            Err(payload) => {
                let t = Throwable::from_panic(payload);
                if t.class_name() == "java.lang.AssertionError"
                    && t.message() == Some("Calendar underflow")
                {
                    true
                } else {
                    t.throw()
                }
            }
        }
    }
}

/// Registers the Java exception classes the spatial modules throw in the
/// runner's class hierarchy (what `catch` and `expectThrows` match).
pub fn register_exception_classes() {
    use randomizedtesting::jdk::exception::register_class;
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        register_class(
            "org.apache.lucene.spatial.query.UnsupportedSpatialOperation",
            "java.lang.UnsupportedOperationException",
        );
        register_class(
            "java.lang.UnsupportedOperationException",
            "java.lang.RuntimeException",
        );
        register_class(
            spatial4j::exception::invalid_shape_exception::InvalidShapeException::CLASS,
            "java.lang.RuntimeException",
        );
        register_class("java.text.ParseException", "java.lang.Exception");
        register_class(
            "java.lang.IllegalStateException",
            "java.lang.RuntimeException",
        );
        register_class("java.time.DateTimeException", "java.lang.RuntimeException");
    });
}

/// `Integer.valueOf(String)` / `Integer.parseInt(String)`: Java's
/// `NumberFormatException` for a malformed number.
pub fn java_parse_int(s: &str) -> rucene::error::Result<i32> {
    randomizedtesting::jdk::lang::parse_int(s).ok_or_else(|| {
        rucene::error::LuceneError::unchecked(
            "java.lang.NumberFormatException",
            format!("For input string: \"{s}\""),
        )
    })
}

/// `Double.parseDouble(String)`: Java's `NumberFormatException` for a
/// malformed number.
pub fn java_parse_double(s: &str) -> rucene::error::Result<f64> {
    rucene::util::java_fmt::parse_double(s).ok_or_else(|| {
        rucene::error::LuceneError::unchecked(
            "java.lang.NumberFormatException",
            format!("For input string: \"{s}\""),
        )
    })
}

/// `Number.intValue()` of a stored field's number (Java's narrowing casts).
pub fn number_int_value(n: rucene::document::NumericValue) -> i32 {
    use rucene::document::NumericValue;
    match n {
        NumericValue::Int(i) => i,
        NumericValue::Long(l) => l as i32,
        NumericValue::Float(f) => f as i32,
        NumericValue::Double(d) => d as i32,
    }
}
