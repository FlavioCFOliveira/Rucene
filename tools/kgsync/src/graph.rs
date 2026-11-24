//! Thin client of the `rmp graph` server: one statement per `rmp graph client`
//! invocation, read from standard input (no argv size limit).

use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Hard limit of the server on one statement (bytes); stay well below it.
pub const MAX_STATEMENT: usize = 900_000;

pub struct Graph {
    roadmap: String,
    pub dry_run: bool,
    pub statements: AtomicU64,
    pub write_ms: AtomicU64,
    pub read_ms: AtomicU64,
}

impl Graph {
    pub fn new(roadmap: &str, dry_run: bool) -> Self {
        Graph {
            roadmap: roadmap.to_string(),
            dry_run,
            statements: AtomicU64::new(0),
            write_ms: AtomicU64::new(0),
            read_ms: AtomicU64::new(0),
        }
    }

    fn exec(&self, q: &str) -> Result<Value, String> {
        let mut child = Command::new("rmp")
            .args(["graph", "client", "-r", &self.roadmap])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn rmp: {e}"))?;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(q.as_bytes())
            .map_err(|e| format!("write statement: {e}"))?;
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        self.statements.fetch_add(1, Ordering::Relaxed);
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let head: String = q.chars().take(300).collect();
            return Err(format!(
                "statement failed: {}\n  statement: {head}…",
                err.trim()
            ));
        }
        serde_json::from_slice(&out.stdout).map_err(|e| format!("bad JSON from rmp: {e}"))
    }

    /// Runs a read and returns its rows.
    pub fn rows(&self, q: &str) -> Result<Vec<Vec<Value>>, String> {
        let t = Instant::now();
        let v = self.exec(q)?;
        self.read_ms
            .fetch_add(t.elapsed().as_millis() as u64, Ordering::Relaxed);
        Ok(match v.get("rows") {
            Some(Value::Array(rows)) => rows
                .iter()
                .map(|r| r.as_array().cloned().unwrap_or_default())
                .collect(),
            _ => Vec::new(),
        })
    }

    /// Runs a write (skipped in dry-run mode) and returns its counters.
    pub fn write(&self, q: &str) -> Result<Value, String> {
        if self.dry_run {
            return Ok(Value::Null);
        }
        let t = Instant::now();
        let v = self.exec(q)?;
        self.write_ms
            .fetch_add(t.elapsed().as_millis() as u64, Ordering::Relaxed);
        Ok(v.get("counters").cloned().unwrap_or(Value::Null))
    }

    /// Schema statement: always executed, even in dry-run mode, only when asked.
    pub fn ddl(&self, q: &str) -> Result<Value, String> {
        self.exec(q)
    }
}

/// Encodes a JSON value as a Cypher literal.
pub fn lit(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => {
            out.push('\'');
            for c in s.chars() {
                match c {
                    '\\' => out.push_str("\\\\"),
                    '\'' => out.push_str("\\'"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if (c as u32) < 0x20 => out.push(' '),
                    c => out.push(c),
                }
            }
            out.push('\'');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                lit(x, out);
            }
            out.push(']');
        }
        Value::Object(m) => {
            out.push('{');
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                ident(k, out);
                out.push(':');
                lit(x, out);
            }
            out.push('}');
        }
    }
}

pub fn ident(k: &str, out: &mut String) {
    if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !k.starts_with(|c: char| c.is_ascii_digit())
    {
        out.push_str(k);
    } else {
        out.push('`');
        out.push_str(&k.replace('`', "``"));
        out.push('`');
    }
}

pub fn lit_s(v: &Value) -> String {
    let mut s = String::new();
    lit(v, &mut s);
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn literals_escape_quotes_backslashes_and_control_characters() {
        assert_eq!(lit_s(&json!("a'b\\c\nd\te")), r"'a\'b\\c\nd\te'");
        assert_eq!(lit_s(&json!(null)), "null");
        assert_eq!(lit_s(&json!([1, "x", true])), "[1,'x',true]");
        assert_eq!(lit_s(&json!({"k": 1, "a-b": "v"})), "{k:1,`a-b`:'v'}");
    }
}
