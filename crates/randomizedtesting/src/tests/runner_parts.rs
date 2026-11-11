//! The runner's helpers against `runner_parts.tsv` (written by
//! `RunnerPartsOracle.java` under randomizedtesting 2.8.1 and JUnit 4.13.1):
//! `GlobFilter`, `FilterExpressionParser.toExpression`, `TraceFormatting`,
//! the argument templates (`String.format`), `RandomizedRunner.methodName`
//! and `Description`'s split, `MultipleFailureException`,
//! `WriterOutputStream`, `TeeOutputStream` and `RamUsageEstimator`.

use std::collections::BTreeMap;
use std::io::Write;

use super::{assert_same_lines, from_units, rows, units_of};
use crate::filter_expression_parser::FilterExpressionParser;
use crate::glob_filter::GlobFilter;
use crate::jdk::exception::{StackTraceElement, Throwable};
use crate::jdk::format::format;
use crate::jdk::reflect::Value;
use crate::junit::Description;
use crate::randomized_runner::{method_name, try_run};
use crate::rules::RamUsageEstimator;
use crate::tee_output_stream::{TeeError, TeeOutputStream};
use crate::trace_formatting::TraceFormatting;
use crate::writer_output_stream::{CharWriter, Charset, WriterOutputStream};

fn res(f: impl FnOnce() -> String) -> String {
    let _q = crate::jdk::exception::QuietPanics::new();
    match try_run(f) {
        Ok(s) => s,
        Err(t) => format!(
            "ERR {}: {}",
            t.class_name(),
            units_of(t.message().unwrap_or("null"))
        ),
    }
}

fn expected(kind: &str) -> BTreeMap<String, String> {
    rows("runner_parts.tsv")
        .into_iter()
        .filter(|r| r[0] == kind)
        .map(|r| {
            let n = r.len();
            (r[1..n - 1].join("\t"), r[n - 1].clone())
        })
        .collect()
}

#[test]
fn glob_filters_match_as_java_regexes() {
    let compile = expected("globCompile");
    let matches = expected("glob");
    let mut actual_compile = BTreeMap::new();
    let mut actual = BTreeMap::new();
    for g in compile.keys() {
        let glob = from_units(g);
        let r = res(|| {
            GlobFilter::new(&glob);
            "OK".to_string()
        });
        actual_compile.insert(g.clone(), r);
    }
    for key in matches.keys() {
        let (g, input) = key.split_once('\t').expect("glob and input");
        let f = GlobFilter::new(&from_units(g));
        actual.insert(key.clone(), f.glob_matches(&from_units(input)).to_string());
    }
    assert_same_lines("globCompile", &compile, &actual_compile);
    assert_same_lines("glob", &matches, &actual);
}

#[test]
fn filter_expressions_parse_as_java() {
    let exp = expected("filter");
    let mut actual = BTreeMap::new();
    for e in exp.keys() {
        let rule = from_units(e);
        actual.insert(
            e.clone(),
            res(|| units_of(&FilterExpressionParser::new().parse(&rule).to_expression())),
        );
    }
    assert_same_lines("filter", &exp, &actual);
}

#[test]
fn trace_formatting_collapses_filtered_frames_as_java() {
    let mut cause = Throwable::new("java.lang.IllegalStateException", "cause");
    cause.set_stack_trace(vec![
        StackTraceElement::new("foo.Cause", "m", Some("Cause.java".into()), 3),
        StackTraceElement::new("org.junit.Runner", "run", None, -1),
    ]);
    let mut t = Throwable::with_cause("java.lang.RuntimeException", Some("top".into()), cause);
    t.set_stack_trace(vec![
        StackTraceElement::new(
            "__randomizedtesting.SeedInfo",
            "seed",
            Some("[DEADBEEF:1]".into()),
            0,
        ),
        StackTraceElement::new("foo.Bar", "test", Some("Bar.java".into()), 10),
        StackTraceElement::new("sun.reflect.X", "invoke", None, -2),
        StackTraceElement::new("org.junit.Y", "run", Some("Y.java".into()), 5),
        StackTraceElement::new(
            "com.carrotsearch.randomizedtesting.Z",
            "go",
            Some("Z.java".into()),
            7,
        ),
        StackTraceElement::new(
            "java.lang.reflect.Method",
            "invoke",
            Some("Method.java".into()),
            1,
        ),
        StackTraceElement::new("foo.Baz", "after", Some("Baz.java".into()), -1),
        StackTraceElement::new("junit.framework.A", "a", Some("A.java".into()), 1),
    ]);
    let exp = expected("trace");
    let mut actual = BTreeMap::new();
    let filtered = TraceFormatting::with_filters(&[
        "org.junit.",
        "junit.framework.",
        "sun.",
        "java.lang.reflect.",
        "com.carrotsearch.randomizedtesting.",
    ]);
    actual.insert(
        "filtered".to_string(),
        units_of(&filtered.format_throwable(&t)),
    );
    actual.insert(
        "plain".to_string(),
        units_of(&TraceFormatting::new().format_throwable(&t)),
    );
    assert_same_lines("trace", &exp, &actual);
}

#[test]
fn argument_templates_format_as_java() {
    let exp = expected("format");
    let cases: Vec<(&str, Vec<Value>)> = vec![
        (
            "p0=%s p1=%s seed=%3$s",
            vec![
                Value::Str("a".into()),
                Value::Int(1),
                Value::Str("[S]".into()),
            ],
        ),
        (
            "%2$s:%1$s",
            vec![
                Value::Str("x".into()),
                Value::Int(10),
                Value::Str("[S]".into()),
            ],
        ),
        (" seed=%1$s", vec![Value::Str("[S]".into())]),
        (
            "%s-%s-%s",
            vec![
                Value::Str("a".into()),
                Value::Str("b".into()),
                Value::Str("c".into()),
            ],
        ),
        (
            "%-5s|%5s|%%|%n|%d",
            vec![
                Value::Str("ab".into()),
                Value::Str("cd".into()),
                Value::Int(7),
            ],
        ),
        (
            "%b %b %B",
            vec![Value::Bool(true), Value::Null, Value::Str("x".into())],
        ),
    ];
    let mut actual = BTreeMap::new();
    for (t, args) in cases {
        actual.insert(units_of(t), units_of(&format(t, &args)));
    }
    assert_same_lines("format", &exp, &actual);
    let missing = rows("runner_parts.tsv")
        .into_iter()
        .find(|r| r[0] == "format-missing")
        .expect("format-missing row");
    let got = res(|| format("%s %s", &[Value::Str("a".into())]));
    assert_eq!(missing[1], got);
}

#[test]
fn description_split_and_method_name_as_junit() {
    let exp: BTreeMap<String, String> = rows("runner_parts.tsv")
        .into_iter()
        .filter(|r| r[0] == "desc")
        .map(|r| (r[1].clone(), r[2..].join("\t")))
        .collect();
    let mut actual = BTreeMap::new();
    for n in exp.keys() {
        let d = Description::create_suite_description(&from_units(n), Vec::new());
        let m = d.method_name();
        let line = format!(
            "{}\t{}\t{}",
            units_of(m.as_deref().unwrap_or("null")),
            units_of(&d.class_name().unwrap_or_else(|| "null".into())),
            if m.is_none() {
                "-".to_string()
            } else {
                units_of(&method_name(&d))
            }
        );
        actual.insert(n.clone(), line);
    }
    assert_same_lines("desc", &exp, &actual);
}

#[test]
fn multiple_failure_exception_as_junit() {
    let exp = expected("mfe");
    let errs = vec![
        Throwable::new("java.lang.RuntimeException", "one"),
        Throwable::with_message("java.lang.IllegalStateException", None),
        Throwable::new("java.lang.Error", "three\nlines"),
    ];
    let mut actual = BTreeMap::new();
    let m = Throwable::multiple_failures_of_class(
        "org.junit.runners.model.MultipleFailureException",
        errs.clone(),
    );
    actual.insert("message".to_string(), units_of(m.message().unwrap_or("")));
    actual.insert(
        "assertEmpty".to_string(),
        Throwable::multiple_failures(errs).class_name().to_string(),
    );
    let solo = res(|| {
        crate::randomized_runner::assert_empty(vec![Throwable::new(
            "java.lang.RuntimeException",
            "solo",
        )]);
        String::new()
    });
    // `res` encodes the message; the oracle row prints it plainly.
    let solo = solo.replace("ERR ", "");
    let (class, msg) = solo.split_once(": ").expect("class and message");
    actual.insert(
        "assertEmpty1".to_string(),
        format!("{class}: {}", from_units(msg)),
    );
    assert_same_lines("mfe", &exp, &actual);
}

struct Recorder {
    units: Vec<u16>,
    writes: usize,
}

impl CharWriter for Recorder {
    fn write_chars(&mut self, cbuf: &[u16]) -> std::io::Result<()> {
        self.units.extend_from_slice(cbuf);
        self.writes += 1;
        Ok(())
    }
    fn flush_chars(&mut self) -> std::io::Result<()> {
        Ok(())
    }
    fn close_chars(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// `List<Integer>.hashCode()`.
fn list_hash(v: &[i32]) -> i32 {
    v.iter()
        .fold(1i32, |h, &e| h.wrapping_mul(31).wrapping_add(e))
}

#[test]
fn writer_output_stream_decodes_as_java() {
    let mut exp = BTreeMap::new();
    for r in rows("runner_parts.tsv")
        .into_iter()
        .filter(|r| r[0] == "wos")
    {
        exp.insert(r[1..6].join("\t"), r[6..].join("\t"));
    }
    assert!(!exp.is_empty());
    let mut actual = BTreeMap::new();
    for key in exp.keys() {
        let f: Vec<&str> = key.split('\t').collect();
        let charset = match f[0] {
            "UTF-8" => Charset::Utf8,
            "ISO-8859-1" => Charset::Iso8859_1,
            _ => Charset::UsAscii,
        };
        let buf_size: usize = f[1].parse().expect("buffer size");
        let imm = f[2] == "true";
        let split: Vec<usize> = f[3]
            .trim_matches(['[', ']'])
            .split(", ")
            .map(|x| x.parse().expect("split"))
            .collect();
        let bytes: Vec<u8> = (0..f[4].len() / 2)
            .map(|i| u8::from_str_radix(&f[4][2 * i..2 * i + 2], 16).expect("hex"))
            .collect();
        let mut os = WriterOutputStream::with_options(
            Recorder {
                units: Vec::new(),
                writes: 0,
            },
            charset,
            buf_size,
            imm,
        );
        let mut off = 0;
        let mut si = 0;
        let mut after = Vec::new();
        while off < bytes.len() {
            let n = split[si % split.len()].min(bytes.len() - off);
            si += 1;
            os.write_all(&bytes[off..off + n]).expect("write");
            off += n;
            after.push(os.writer().units.len() as i32);
        }
        os.close().expect("close");
        let w = os.writer();
        actual.insert(
            key.clone(),
            format!(
                "{}\t{}\t{}",
                units_of(&String::from_utf16_lossy(&w.units)),
                w.writes,
                list_hash(&after)
            ),
        );
    }
    assert_same_lines("wos", &exp, &actual);
}

struct Failing(&'static str);

impl Write for Failing {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other(self.0))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn tee_error(e: std::io::Error) -> (String, usize, Option<String>) {
    let inner = e.into_inner().expect("tee error");
    let t = inner.downcast::<TeeError>().expect("TeeError");
    let s = t.suppressed.as_ref().map(|x| x.to_string());
    (t.error.to_string(), usize::from(t.suppressed.is_some()), s)
}

#[test]
fn tee_output_stream_reports_first_error_as_java() {
    let exp = expected("tee");
    let mut actual = BTreeMap::new();
    let e = TeeOutputStream::new(Failing("first"), Failing("second"))
        .write(&[1])
        .expect_err("both fail");
    let (m, n, s) = tee_error(e);
    actual.insert(
        "both".to_string(),
        format!("{m}\t{n}\t{}", s.unwrap_or_default()),
    );
    let mut good = Vec::new();
    let e = TeeOutputStream::new(&mut good, Failing("second"))
        .write(&[1, 2])
        .expect_err("second fails");
    let (m, n, _) = tee_error(e);
    actual.insert("second".to_string(), format!("{m}\t{n}\t{}", good.len()));
    let e = TeeOutputStream::new(Failing("first"), &mut good)
        .write(&[3])
        .expect_err("first fails");
    let (m, n, _) = tee_error(e);
    actual.insert("first".to_string(), format!("{m}\t{n}\t{}", good.len()));
    let exp: BTreeMap<String, String> = exp
        .into_iter()
        .map(|(k, v)| {
            let mut parts = k.splitn(2, '\t');
            let kind = parts.next().unwrap_or("").to_string();
            let rest = parts.next().unwrap_or("").to_string();
            (kind, format!("{rest}\t{v}"))
        })
        .collect();
    assert_same_lines("tee", &exp, &actual);
}

#[test]
fn ram_usage_estimator_as_java() {
    let mut exp = BTreeMap::new();
    for r in rows("runner_parts.tsv")
        .into_iter()
        .filter(|r| r[0] == "ram")
    {
        exp.insert(r[1..r.len() - 1].join("\t"), r[r.len() - 1].clone());
    }
    let mut actual = BTreeMap::new();
    actual.insert(
        "layout".to_string(),
        format!(
            "{} {} {} {}",
            RamUsageEstimator::NUM_BYTES_OBJECT_REF,
            RamUsageEstimator::NUM_BYTES_OBJECT_HEADER,
            RamUsageEstimator::NUM_BYTES_ARRAY_HEADER,
            RamUsageEstimator::NUM_BYTES_OBJECT_ALIGNMENT
        ),
    );
    for k in exp.keys() {
        if let Some(b) = k.strip_prefix("human\t") {
            let v: i64 = b.parse().expect("bytes");
            actual.insert(k.clone(), RamUsageEstimator::human_readable_units(v));
        } else if let Some(b) = k.strip_prefix("align\t") {
            let v: i64 = b.parse().expect("size");
            actual.insert(
                k.clone(),
                RamUsageEstimator::align_object_size(v).to_string(),
            );
        }
    }
    let sizes: Vec<(&str, Value)> = vec![
        ("byte[0]", Value::primitive_array("byte", 0)),
        ("byte[1]", Value::primitive_array("byte", 1)),
        ("char[3]", Value::primitive_array("char", 3)),
        ("long[3]", Value::primitive_array("long", 3)),
        ("boolean[9]", Value::primitive_array("boolean", 9)),
        (
            "Object[2]",
            Value::Array(vec![Value::primitive_array("int", 5), Value::Int(1000)]),
        ),
        ("Integer", Value::Int(1000)),
        ("Long", Value::Long(1000)),
        ("Double", Value::Double(1.0)),
        ("Boolean", Value::Bool(true)),
    ];
    for (k, v) in sizes {
        actual.insert(
            format!("sizeOf\t{k}"),
            RamUsageEstimator::size_of(&v)
                .expect("measurable")
                .to_string(),
        );
    }
    actual.insert(
        "sizeOf\tString".to_string(),
        res(
            || match RamUsageEstimator::size_of(&Value::Str("x".into())) {
                Ok(_) => "OK".to_string(),
                Err(t) => t.throw(),
            },
        ),
    );
    assert_same_lines("ram", &exp, &actual);
}
