//! Port of `com.carrotsearch.randomizedtesting.FilterExpressionParser`
//! (randomizedtesting 2.8.1, `FilterExpressionParser.java`): the test group
//! filter expressions of `tests.filter`.

use crate::jdk::exception::Throwable;

/// `FilterExpressionParser.SyntaxException`.
pub const SYNTAX_EXCEPTION: &str =
    "com.carrotsearch.randomizedtesting.FilterExpressionParser$SyntaxException";

/// What an expression is evaluated against (`IContext`).
pub trait IContext {
    /// `defaultValue()`: the value of `default`.
    fn default_value(&self) -> bool;
    /// `hasGroup(value)`: whether the test carries the group `value`
    /// (`@name`).
    fn has_group(&self, value: &str) -> bool;
}

/// A token's place in the input (`InputRange`).
#[derive(Debug, Clone)]
pub struct InputRange {
    s: Vec<u16>,
    start: usize,
    len: usize,
}

impl InputRange {
    /// `value()`.
    pub fn value(&self) -> String {
        String::from_utf16_lossy(&self.s[self.start..self.start + self.len])
    }
}

impl std::fmt::Display for InputRange {
    /// `toString()`: the input with the token marked `>token<`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}>{}<{}",
            String::from_utf16_lossy(&self.s[..self.start]),
            self.value(),
            String::from_utf16_lossy(&self.s[self.start + self.len..])
        )
    }
}

/// The kind of a node (one Java `Node` subclass each).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Eos,
    Default,
    And,
    Or,
    Not,
    OpeningBracket,
    ClosingBracket,
    TestGroup,
}

/// A parsed expression node (`Node`).
#[derive(Debug, Clone)]
pub struct Node {
    kind: Kind,
    lbp: i32,
    args: Vec<Node>,
    /// `range`.
    pub range: Option<InputRange>,
}

impl Node {
    fn new(kind: Kind) -> Node {
        let lbp = match kind {
            Kind::Eos => -1,
            Kind::And => 30,
            Kind::Or => 20,
            Kind::Not => 40,
            _ => 0,
        };
        Node {
            kind,
            lbp,
            args: Vec::new(),
            range: None,
        }
    }

    /// `toString()`: the node class's simple name without `Node`.
    fn name(&self) -> &'static str {
        match self.kind {
            Kind::Eos => "Eos",
            Kind::Default => "Default",
            Kind::And => "And",
            Kind::Or => "Or",
            Kind::Not => "Not",
            Kind::OpeningBracket => "OpeningBracket",
            Kind::ClosingBracket => "ClosingBracket",
            Kind::TestGroup => "TestGroup",
        }
    }

    /// `toExpression()`: the fully parenthesized expression.
    ///
    /// # Panics
    ///
    /// `UnsupportedOperationException` (`Not an expression node: ...`) for a
    /// node that is not an expression.
    pub fn to_expression(&self) -> String {
        match self.kind {
            Kind::Default => "default".to_string(),
            Kind::And => format!(
                "({} AND {})",
                self.args[0].to_expression(),
                self.args[1].to_expression()
            ),
            Kind::Or => format!(
                "({} OR {})",
                self.args[0].to_expression(),
                self.args[1].to_expression()
            ),
            Kind::Not => format!("(NOT {})", self.args[0].to_expression()),
            Kind::TestGroup => self.range.as_ref().map(|r| r.value()).unwrap_or_default(),
            _ => crate::jdk::exception::throw(
                crate::jdk::exception::UNSUPPORTED_OPERATION_EXCEPTION,
                format!("Not an expression node: {}", self.name()),
            ),
        }
    }

    /// `evaluate(context)`.
    ///
    /// # Panics
    ///
    /// `UnsupportedOperationException` (`Not an evaluation node: ...`) for a
    /// node that is not an expression.
    pub fn evaluate(&self, context: &dyn IContext) -> bool {
        match self.kind {
            Kind::Default => context.default_value(),
            Kind::And => self.args[0].evaluate(context) && self.args[1].evaluate(context),
            Kind::Or => self.args[0].evaluate(context) || self.args[1].evaluate(context),
            Kind::Not => !self.args[0].evaluate(context),
            Kind::TestGroup => {
                context.has_group(&self.range.as_ref().map(|r| r.value()).unwrap_or_default())
            }
            _ => crate::jdk::exception::throw(
                crate::jdk::exception::UNSUPPORTED_OPERATION_EXCEPTION,
                format!("Not an evaluation node: {}", self.name()),
            ),
        }
    }
}

fn syntax_error(node: Option<&Node>, msg: &str) -> ! {
    let message = match node.and_then(|n| n.range.as_ref()) {
        Some(r) => format!("{msg} At: \"{r}\""),
        None => msg.to_string(),
    };
    Throwable::new(SYNTAX_EXCEPTION, message).throw()
}

/// A Pratt parser of filter expressions (`FilterExpressionParser`).
#[derive(Debug, Default)]
pub struct FilterExpressionParser {
    nodes: Vec<Node>,
    next: usize,
    current: Option<Node>,
}

impl FilterExpressionParser {
    /// `new FilterExpressionParser()`.
    pub fn new() -> FilterExpressionParser {
        FilterExpressionParser::default()
    }

    fn advance(&mut self) -> Node {
        if self.next >= self.nodes.len() {
            crate::jdk::exception::throw_without_message("java.util.NoSuchElementException");
        }
        let n = self.nodes[self.next].clone();
        self.next += 1;
        n
    }

    fn has_next(&self) -> bool {
        self.next < self.nodes.len()
    }

    fn current(&self) -> &Node {
        self.current.as_ref().expect("current token")
    }

    /// `nud()`.
    fn nud(&mut self, mut n: Node) -> Node {
        match n.kind {
            Kind::Default | Kind::TestGroup => n,
            Kind::Not => {
                let lbp = n.lbp;
                n.args = vec![self.expression(lbp)];
                n
            }
            Kind::OpeningBracket => {
                let expr = self.expression(0);
                if self.current().kind != Kind::ClosingBracket {
                    let c = self.current().clone();
                    syntax_error(Some(&c), "Expected closing bracket.");
                }
                self.current = Some(self.advance());
                expr
            }
            _ => syntax_error(Some(&n), "Syntax error."),
        }
    }

    /// `led(left)`.
    fn led(&mut self, mut n: Node, left: Node) -> Node {
        match n.kind {
            Kind::And | Kind::Or => {
                if !self.has_next() {
                    syntax_error(
                        Some(&n),
                        &format!("Missing argument for {}.", n.name().to_uppercase()),
                    );
                }
                let lbp = n.lbp;
                let right = self.expression(lbp);
                n.args = vec![left, right];
                n
            }
            Kind::ClosingBracket => syntax_error(Some(&n), "Unbalanced parenthesis."),
            _ => syntax_error(Some(&n), "Not an operator."),
        }
    }

    /// `expression(rbp)`.
    fn expression(&mut self, rbp: i32) -> Node {
        let n = self.current().clone();
        self.current = Some(self.advance());
        let mut left = self.nud(n);
        while rbp < self.current().lbp {
            let n = self.current().clone();
            self.current = Some(self.advance());
            left = self.led(n, left);
        }
        left
    }

    /// `parse(rule)`: the expression of `rule`; an empty rule is
    /// `default`.
    ///
    /// # Panics
    ///
    /// `FilterExpressionParser$SyntaxException` with Java's message for a
    /// malformed rule.
    pub fn parse(&mut self, rule: &str) -> Node {
        self.nodes = tokenize(rule);
        self.next = 0;
        self.current = Some(self.advance());
        if self.current().kind == Kind::Eos {
            Node::new(Kind::Default)
        } else {
            self.expression(-1)
        }
    }
}

/// `\s` of `java.util.regex`: `[ \t\n\x0B\f\r]`.
fn is_regex_space(u: u16) -> bool {
    matches!(u, 0x20 | 0x09 | 0x0A | 0x0B | 0x0C | 0x0D)
}

/// `tokenize(rule)`: the matches of `(?:\s*)(([^\s\(\)]+)|([\(\)]))`, then
/// an end-of-stream node.
fn tokenize(rule: &str) -> Vec<Node> {
    let s: Vec<u16> = rule.encode_utf16().collect();
    let paren = |u: u16| u == u16::from(b'(') || u == u16::from(b')');
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < s.len() {
        while i < s.len() && is_regex_space(s[i]) {
            i += 1;
        }
        if i >= s.len() {
            break;
        }
        let start = i;
        if paren(s[i]) {
            i += 1;
        } else {
            while i < s.len() && !is_regex_space(s[i]) && !paren(s[i]) {
                i += 1;
            }
        }
        let range = InputRange {
            s: s.clone(),
            start,
            len: i - start,
        };
        let value = range.value();
        let kind = if value.eq_ignore_ascii_case("DEFAULT") {
            Kind::Default
        } else if value == "(" {
            Kind::OpeningBracket
        } else if value == ")" {
            Kind::ClosingBracket
        } else if value.eq_ignore_ascii_case("and") {
            Kind::And
        } else if value.eq_ignore_ascii_case("or") {
            Kind::Or
        } else if value.eq_ignore_ascii_case("not") {
            Kind::Not
        } else if value.starts_with('@') {
            Kind::TestGroup
        } else {
            syntax_error(
                None,
                &format!("Unrecognized token '{value}'. At: \"{range}\""),
            );
        };
        let mut t = Node::new(kind);
        t.range = Some(range);
        tokens.push(t);
    }
    let mut eos = Node::new(Kind::Eos);
    eos.range = Some(InputRange {
        s: s.clone(),
        start: s.len(),
        len: 0,
    });
    tokens.push(eos);
    tokens
}
