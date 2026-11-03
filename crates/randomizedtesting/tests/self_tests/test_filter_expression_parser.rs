//! Port of `com.carrotsearch.randomizedtesting.TestFilterExpressionParser`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::system;
use randomizedtesting::{FilterExpressionParser, IContext};

use crate::support::{
    assert_that_bool, assert_that_str, assertj_fail_msg, cls, test, try_catch, unit,
};

const SYNTAX_EXCEPTION: &str =
    "com.carrotsearch.randomizedtesting.FilterExpressionParser$SyntaxException";

/// The anonymous `IContext` over a list of groups.
struct Groups(Vec<String>);

impl IContext for Groups {
    fn default_value(&self) -> bool {
        self.has_group("default")
    }
    fn has_group(&self, value: &str) -> bool {
        self.0.iter().any(|g| g == value)
    }
}

fn check(examples: &[(&[&str], &[&str])], expected: bool) {
    for (groups, rules) in examples {
        let context = Groups(groups.iter().map(|s| s.to_string()).collect());
        for rule in *rules {
            let v = FilterExpressionParser::new().parse(rule).evaluate(&context);
            let b = assert_that_bool(v).as_(format!("ctx=[{}], rule={rule}", groups.join(", ")));
            if expected {
                b.is_true()
            } else {
                b.is_false()
            }
        }
    }
}

/// `TestFilterExpressionParser`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestFilterExpressionParser")
        .constructor(unit())
        .method(test("testParsingSanity", || {
            // input -> expected bracketing.
            let examples = [
                ("", "default"),
                ("default", "default"),
                ("@nightly", "@nightly"),
                ("@nightly or @slow", "(@nightly OR @slow)"),
                ("@nightly and @slow", "(@nightly AND @slow)"),
                ("@nightly and not @slow", "(@nightly AND (NOT @slow))"),
                ("@nightly and (not @slow)", "(@nightly AND (NOT @slow))"),
                ("not @slow", "(NOT @slow)"),
                ("not not @slow", "(NOT (NOT @slow))"),
                ("not @nightly or @slow", "((NOT @nightly) OR @slow)"),
                ("not(@nightly or @slow)", "(NOT (@nightly OR @slow))"),
                ("(@nightly and (not @slow))", "(@nightly AND (NOT @slow))"),
            ];
            for (input, expected) in examples {
                system::out_println(&format!("-- {input}"));
                let expression = FilterExpressionParser::new().parse(input).to_expression();
                system::out_println(&format!("   {expression}"));
                assert_that_str(expression.clone())
                    .as_(input)
                    .is_equal_to(expected);
                assert_that_str(
                    FilterExpressionParser::new()
                        .parse(&expression)
                        .to_expression(),
                )
                .as_(input)
                .is_equal_to(&expression);
            }
        }))
        .method(test("testTrueExpressions", || {
            check(
                &[
                    (&[], &["not @foo", "not default"]),
                    (
                        &["@nightly"],
                        &[
                            "@nightly",
                            "@nightly or @foo",
                            "@foo or @nightly",
                            "not not @nightly",
                        ],
                    ),
                    (
                        &["@nightly", "@slow"],
                        &[
                            "@nightly and @slow",
                            "@nightly and not @foo",
                            "not @nightly or @slow",
                        ],
                    ),
                    (&["default"], &["", "default"]),
                ],
                true,
            );
        }))
        .method(test("testFalseExpressions", || {
            check(
                &[
                    (&[], &["@foo", "default"]),
                    (&["@nightly"], &["not @nightly", "@nightly and @foo"]),
                    (&["@nightly", "@slow"], &["not(@nightly or @slow)"]),
                ],
                false,
            );
        }))
        .method(test("testErrors", || {
            for rule in [
                "nightly",
                "@nightly and ",
                "@nightly or ",
                "@nightly and or @slow",
                "and not @slow",
                "(@nightly))",
                "((@nightly or @slow)",
            ] {
                system::out_println(&format!("-- {rule}"));
                match try_catch(|| FilterExpressionParser::new().parse(rule)) {
                    Ok(_) => assertj_fail_msg(&format!("Should cause an exception: {rule}")),
                    Err(e) if e.is_instance(SYNTAX_EXCEPTION) => {
                        system::out_println(&format!("   {}", e.message().unwrap_or("null")))
                    }
                    Err(e) => e.throw(),
                }
            }
        }))
        .method(test("testInvalidToken", || {
            match try_catch(|| FilterExpressionParser::new().parse("@nightly and foo or @bar")) {
                Ok(_) => {}
                Err(e) if e.is_instance(SYNTAX_EXCEPTION) => {
                    assert_that_str(e.to_string()).contains(">foo<");
                }
                Err(e) => e.throw(),
            }
        }))
        .build()]
}
