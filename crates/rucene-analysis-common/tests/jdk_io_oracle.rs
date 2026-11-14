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

//! JDK 25 oracle of the JDK emulations rmp #453 added for the synonym and
//! word2vec packages: `jdk::io` (the UTF-8 `InputStreamReader` with
//! `REPLACE` and `REPORT`, `BufferedReader` and `LineNumberReader`),
//! `jdk::base64` (`Base64.getDecoder().decode(byte[])` and
//! `decode(String)`) and `jdk::zip_input_stream` (`ZipInputStream`), against
//! the local JDK through `JdkIoOracle.java`
//! (`tests/fixtures/java-codec-harness/.../codec/`). Results, exceptions
//! (binary class name and message) included, are compared on seeded inputs
//! and edge cases: malformed, truncated, overlong and surrogate UTF-8
//! sequences across the 8192-byte buffers, line terminators (`\n`, `\r`,
//! `\r\n`, split across buffers); valid, padded, unpadded and illegal base64;
//! ZIP streams of stored and deflated entries (with data descriptors), then
//! corrupted and truncated copies of each.

use std::io::{Cursor, Read};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::Command;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::error::{LuceneError, Result};
use rucene_analysis_common::jdk::base64;
use rucene_analysis_common::jdk::io::{
    BufferedReader, CodingErrorAction, InputStreamReader, LineNumberReader,
};
use rucene_analysis_common::jdk::zip_input_stream::ZipInputStream;
use rucene_test_framework::util::java_exceptions::java_throwable;

fn oracle_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         JdkIoOracle.java",
    )
}

fn run_java(args: &[&str]) -> String {
    let output = Command::new("java")
        .arg(oracle_source())
        .args(args)
        .output()
        .expect("a JDK is required to run the JDK I/O oracle");
    assert!(
        output.status.success(),
        "JdkIoOracle.java failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("UTF-8")
}

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("rucene-jdkio-{name}-{}.txt", std::process::id()))
}

/// The JDK's result lines for `lines` in `mode`.
fn java_results(mode: &str, lines: &[String]) -> Vec<String> {
    let path = temp(mode);
    std::fs::write(&path, lines.join("\n")).expect("write the spec file");
    let out = run_java(&[mode, path.to_str().unwrap()]);
    let _ = std::fs::remove_file(&path);
    let out: Vec<String> = out.lines().map(str::to_string).collect();
    assert_eq!(out.len(), lines.len(), "one result per input");
    out
}

fn hex(b: &[u8]) -> String {
    if b.is_empty() {
        return "-".into();
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    if s == "-" {
        return Vec::new();
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn hex_units(u: &[u16]) -> String {
    u.iter().map(|x| format!("{x:04x}")).collect()
}

/// `E<binary class name>:<message>`, as `JdkIoOracle.failure` (Java
/// prints a `null` message as `null`; the test framework's `java_throwable`
/// renders the port's `None` message as `<null>`).
fn failure(e: &LuceneError) -> String {
    let t = java_throwable(e);
    let message = match t.message() {
        None | Some("<null>") => "null",
        Some(m) => m,
    };
    format!("E{}:{message}", t.class_name())
}

/// The failure of a panic (an unchecked Java exception the port throws).
fn panic_failure(panic: Box<dyn std::any::Any + Send>) -> String {
    let t = Throwable::from_panic(panic);
    format!("E{}:{}", t.class_name(), t.message().unwrap_or("null"))
}

/// Compares the Rust results with the JDK's and panics with the first
/// differences.
fn assert_same(what: &str, inputs: &[String], java: &[String], rust: &[String]) {
    let diffs: Vec<usize> = (0..inputs.len()).filter(|&i| java[i] != rust[i]).collect();
    println!(
        "{what}: {} inputs, {} differences",
        inputs.len(),
        diffs.len()
    );
    if !diffs.is_empty() {
        let mut report = String::new();
        for &i in diffs.iter().take(5) {
            report.push_str(&format!(
                "input {}\n  jdk ={}\n  rust={}\n",
                inputs[i].chars().take(200).collect::<String>(),
                java[i].chars().take(400).collect::<String>(),
                rust[i].chars().take(400).collect::<String>()
            ));
        }
        panic!(
            "{what}: {} of {} differ\n{report}",
            diffs.len(),
            inputs.len()
        );
    }
}

// ---------------------------------------------------------------------------
// UTF-8 readers
// ---------------------------------------------------------------------------

fn action(a: &str) -> CodingErrorAction {
    if a == "R" {
        CodingErrorAction::Replace
    } else {
        CodingErrorAction::Report
    }
}

fn rust_utf8(a: &str, bytes: &[u8]) -> String {
    let mut out = String::from("B");
    let mut r = BufferedReader::new(InputStreamReader::with_action(
        Cursor::new(bytes.to_vec()),
        action(a),
    ));
    let lines = catch_unwind(AssertUnwindSafe(|| -> Result<String> {
        let mut s = String::new();
        loop {
            match r.read_line() {
                Ok(Some(line)) => s.push_str(&format!("/{}", hex_units(&line))),
                Ok(None) => break,
                Err(e) => {
                    s.push_str(&format!("/{}", failure(&e)));
                    break;
                }
            }
        }
        Ok(s)
    }));
    match lines {
        Ok(Ok(s)) => out.push_str(&s),
        Ok(Err(e)) => out.push_str(&failure(&e)),
        Err(p) => out.push_str(&format!("/{}", panic_failure(p))),
    }
    out.push_str(" L");
    let mut r = LineNumberReader::new(InputStreamReader::with_action(
        Cursor::new(bytes.to_vec()),
        action(a),
    ));
    loop {
        match r.read_line() {
            Ok(Some(line)) => {
                out.push_str(&format!("/{}={}", r.get_line_number(), hex_units(&line)))
            }
            Ok(None) => {
                out.push_str(&format!("/n={}", r.get_line_number()));
                break;
            }
            Err(e) => {
                out.push_str(&format!("/{}", failure(&e)));
                break;
            }
        }
    }
    out
}

/// Seeded UTF-8 inputs: valid text of every encoded length, mixed with
/// malformed sequences (lone continuation bytes, truncated and overlong
/// forms, encoded surrogates, bytes above `F4`), line terminators, and
/// inputs longer than the 8192-byte buffers.
fn utf8_inputs() -> Vec<Vec<u8>> {
    let mut inputs: Vec<Vec<u8>> = vec![
        vec![],
        b"a".to_vec(),
        b"\n".to_vec(),
        b"\r".to_vec(),
        b"\r\n".to_vec(),
        b"a\rb\r\nc\nd".to_vec(),
        b"no newline at end".to_vec(),
        vec![0xED, 0xA0, 0x80],
        vec![0xED, 0xBF, 0xBF, b'x'],
        vec![0xC0, 0x80],
        vec![0xE0, 0x80, 0x80],
        vec![0xF0, 0x80, 0x80, 0x80],
        vec![0xF4, 0x90, 0x80, 0x80],
        vec![0xF5, b'a'],
        vec![0xFF, 0xFE],
        vec![0xEF, 0xBB, 0xBF, b'b', b'o', b'm'],
        vec![0xE2, 0x82],
        vec![0xE2, 0x82, b'\n', b'x'],
        vec![0xF0, 0x9F, 0x98],
        vec![0x80],
        vec![0xC3],
    ];
    let pieces: &[&[u8]] = &[
        b"a",
        b"Z",
        b" ",
        b"\n",
        b"\r",
        b"\r\n",
        "é".as_bytes(),
        "中".as_bytes(),
        "😀".as_bytes(),
        "ก".as_bytes(),
        &[0x80],
        &[0xC3],
        &[0xE2, 0x82],
        &[0xF0, 0x9F, 0x98],
        &[0xED, 0xA0, 0x80],
        &[0xC0, 0xAF],
        &[0xE0, 0x9F, 0xBF],
        &[0xF8],
        &[0xFF],
        &[0xF4, 0x90],
    ];
    let r = JdkRandom::new(0x453);
    for n in 0..2000 {
        let mut b = Vec::new();
        let len = if n % 50 == 0 {
            3000 + r.next_int_bound(9000)
        } else {
            1 + r.next_int_bound(40)
        };
        for _ in 0..len {
            b.extend_from_slice(pieces[r.next_int_bound(pieces.len() as i32) as usize]);
        }
        inputs.push(b);
    }
    // a malformed sequence and a CR LF at every position around the buffer boundary
    for pad in 8185..8195 {
        let mut b = vec![b'x'; pad];
        b.extend_from_slice(&[0xE2, 0x82, 0xAC, b'\r', b'\n', 0xED, 0xA0, 0x80, b'y']);
        inputs.push(b);
    }
    inputs
}

#[test]
fn utf8_readers_equal_jdk() {
    let mut lines = Vec::new();
    let mut cases = Vec::new();
    for b in utf8_inputs() {
        for a in ["R", "P"] {
            lines.push(format!("{a} {}", hex(&b)));
            cases.push((a, b.clone()));
        }
    }
    let java = java_results("utf8", &lines);
    let rust: Vec<String> = cases.iter().map(|(a, b)| rust_utf8(a, b)).collect();
    assert_same("utf8 readers", &lines, &java, &rust);
}

// ---------------------------------------------------------------------------
// Base64
// ---------------------------------------------------------------------------

fn rust_b64(kind: &str, arg: &str) -> String {
    let r = catch_unwind(AssertUnwindSafe(|| {
        if kind == "B" {
            base64::decode(&unhex(arg))
        } else {
            let arg = if arg == "-" { "" } else { arg };
            let units: Vec<u16> = (0..arg.len())
                .step_by(4)
                .map(|i| u16::from_str_radix(&arg[i..i + 4], 16).unwrap())
                .collect();
            base64::decode_string(&units)
        }
    }));
    match r {
        Ok(Ok(v)) => hex(&v),
        Ok(Err(e)) => failure(&e),
        Err(p) => panic_failure(p),
    }
}

#[test]
fn base64_equals_jdk() {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut srcs: Vec<Vec<u8>> = [
        "",
        "=",
        "==",
        "A",
        "AA",
        "AA=",
        "AA==",
        "AAA",
        "AAA=",
        "AAAA",
        "bHVjZW5l",
        "bHVjZW5",
        "bHVjZW",
        "bHVjZW5l=",
        "bHVj ZW5l",
        "bHVj\nZW5l",
        "bHVjZW5l\r\n",
        "aGk=",
        "aGk",
        "aGl=",
        "aG==",
        "a===",
        "====",
        "AB-_",
        "AB+/",
        "QUJD#",
        "QUJ=D",
        "QU=JD",
        "Zm9vYmFy",
        "Zm9vYmE=",
        "Zm9vYg==",
        "Zm9vYg=",
        "Zm9vYg",
        "Zg==Zg==",
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect();
    let r = JdkRandom::new(0x64);
    for _ in 0..3000 {
        let len = r.next_int_bound(24);
        let mut s: Vec<u8> = (0..len)
            .map(|_| ALPHABET[r.next_int_bound(64) as usize])
            .collect();
        match r.next_int_bound(6) {
            0 => s.push(b'='),
            1 => s.extend_from_slice(b"=="),
            2 => {
                let i = r.next_int_bound(s.len() as i32 + 1) as usize;
                s.insert(
                    i,
                    [b'=', b' ', b'-', b'\n', 0xC3, b'*'][r.next_int_bound(6) as usize],
                );
            }
            _ => {}
        }
        srcs.push(s);
    }
    let mut lines = Vec::new();
    for s in &srcs {
        lines.push(format!("B {}", hex(s)));
        // `decode(String)` of the same text as units, plus a unit above U+00FF
        let mut units: Vec<u16> = s.iter().map(|&b| u16::from(b)).collect();
        if s.len() % 7 == 3 {
            units.push(0x4E2D);
        }
        let units = if units.is_empty() {
            "-".to_string()
        } else {
            hex_units(&units)
        };
        lines.push(format!("S {units}"));
    }
    let java = java_results("b64", &lines);
    let rust: Vec<String> = lines
        .iter()
        .map(|l| {
            let (k, a) = l.split_once(' ').unwrap();
            rust_b64(k, a)
        })
        .collect();
    assert_same("base64", &lines, &java, &rust);
}

// ---------------------------------------------------------------------------
// ZipInputStream
// ---------------------------------------------------------------------------

fn rust_zip(bytes: &[u8]) -> String {
    let mut out = String::from("Z");
    let mut z = ZipInputStream::new(Cursor::new(bytes.to_vec()));
    let r = catch_unwind(AssertUnwindSafe(
        || -> std::result::Result<String, String> {
            let mut s = String::new();
            loop {
                let entry = match z.get_next_entry() {
                    Ok(Some(e)) => e,
                    Ok(None) => break,
                    Err(e) => return Err(format!("{s}/{}", failure(&e))),
                };
                let name: Vec<u16> = entry.get_name().encode_utf16().collect();
                s.push_str(&format!("/{}=", hex_units(&name)));
                let mut data = Vec::new();
                let mut buf = [0u8; 38];
                loop {
                    match z.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => data.extend_from_slice(&buf[..n]),
                        Err(e) => return Err(format!("{s}/{}", failure(&LuceneError::from(e)))),
                    }
                }
                s.push_str(&hex(&data));
                match z.available() {
                    Ok(a) => s.push_str(&format!(";a={a}")),
                    Err(e) => return Err(format!("{s}/{}", failure(&e))),
                }
            }
            match z.available() {
                Ok(a) => s.push_str(&format!("/a={a}")),
                Err(e) => return Err(format!("{s}/{}", failure(&e))),
            }
            Ok(s)
        },
    ));
    match r {
        Ok(Ok(s)) | Ok(Err(s)) => out.push_str(&s),
        Err(p) => out.push_str(&format!("/{}", panic_failure(p))),
    }
    z.close();
    out
}

#[test]
fn zip_input_stream_equals_jdk() {
    let path = temp("zipgen");
    run_java(&["zipgen", "453", "400", path.to_str().unwrap()]);
    let mut lines: Vec<String> = std::fs::read_to_string(&path)
        .expect("the generated ZIP streams")
        .lines()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_file(&path);
    // the module's own resources: Lucene's word2vec test models
    for name in [
        "word2vec-model.zip",
        "word2vec-empty-model.zip",
        "word2vec-corrupted-vector-dimension-model.zip",
    ] {
        let bytes = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/resources/org/apache/lucene/analysis/synonym/word2vec")
                .join(name),
        )
        .expect("a test resource");
        lines.push(hex(&bytes));
    }
    let java = java_results("zip", &lines);
    let rust: Vec<String> = lines.iter().map(|l| rust_zip(&unhex(l))).collect();
    assert_same("ZipInputStream", &lines, &java, &rust);
}
