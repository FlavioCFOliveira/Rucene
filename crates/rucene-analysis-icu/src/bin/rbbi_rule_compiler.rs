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

//! Command-line utility to converts RuleBasedBreakIterator (.rbbi) files
//! into binary compiled form (.brk), ported from
//! `org.apache.lucene.analysis.icu.RBBIRuleCompiler`
//! (`lucene/analysis/icu/src/tools/java/org/apache/lucene/analysis/icu/RBBIRuleCompiler.java`).
//!
//! Run as `java org.apache.lucene.analysis.icu.RBBIRuleCompiler <sourcedir>
//! <destdir>` runs (`gradle/generation/icu.gradle`, task `genRbbiInternal`,
//! with `src/data/uax29` and
//! `src/resources/org/apache/lucene/analysis/icu/segmentation`):
//! `rbbi_rule_compiler <sourcedir> <destdir>`. Every `*.rbbi` file of the
//! source directory is compiled with icu4j's `RuleBasedBreakIterator.compileRules`
//! into the `*.brk` file of the same name in the destination directory.
//!
//! The files are read and written through the operating system as Java's
//! `java.nio.file.Files` does: `Files.list` is the directory's `readdir`
//! order ([`std::fs::read_dir`]), `Files.newOutputStream` creates or
//! truncates the file. An exception thrown out of `main` prints as the
//! JVM's uncaught-exception handler prints it, `Exception in thread "main"
//! ` and the exception's stack trace, and exits with status 1.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use icu4j::text::rule_based_break_iterator::RuleBasedBreakIterator;
use rucene::analysis::java_regex::JavaPattern;
use rucene::error::{LuceneError, Result};
use rucene::store::java_nio_file::OutputStream;
use rucene_analysis_common::jdk::io::{BufferedReader, InputStreamReader};

/// `Files.newOutputStream(path)`: an `OutputStream` writing the file
/// directly, as the JDK's channel output stream does (no buffering).
struct FileOutputStream {
    path: PathBuf,
    file: std::fs::File,
}

impl FileOutputStream {
    /// `Files.newOutputStream(path)`: `CREATE`, `TRUNCATE_EXISTING`, `WRITE`.
    fn create(path: &Path) -> std::io::Result<Self> {
        Ok(Self {
            path: path.to_path_buf(),
            file: std::fs::File::create(path)?,
        })
    }
}

impl OutputStream for FileOutputStream {
    fn write_byte(&mut self, b: i32) -> std::io::Result<()> {
        self.file.write_all(&[b as u8])
    }

    fn write(&mut self, b: &[u8], off: usize, len: usize) -> std::io::Result<()> {
        self.file.write_all(&b[off..off + len])
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }

    fn close(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }

    fn stream_to_string(&self) -> String {
        format!("sun.nio.ch.ChannelOutputStream[{}]", self.path.display())
    }
}

/// `Throwable.getMessage()` of `e`, `"null"` for none, as
/// `System.err.println(e.getMessage())` prints it.
fn get_message(e: &LuceneError) -> String {
    e.original_message()
        .map(str::to_string)
        .unwrap_or_else(|| "null".to_string())
}

/// Equivalent to `getRules(Path)`: the file's lines, each followed by
/// `'\n'`, those starting with `#` emptied, decoded as UTF-8 by an
/// `InputStreamReader` (malformed input replaced).
fn get_rules(rule_file: &Path) -> Result<Vec<u16>> {
    let mut rules: Vec<u16> = Vec::new();
    let input = std::fs::File::open(rule_file)?;
    let mut cin = BufferedReader::new(InputStreamReader::new_utf8(input));
    while let Some(line) = cin.read_line()? {
        if !line.starts_with(&[u16::from(b'#')]) {
            rules.extend_from_slice(&line);
        }
        rules.push(u16::from(b'\n'));
    }
    cin.close()?;
    Ok(rules)
}

/// `String.replaceAll(regex, replacement)`.
fn replace_all(s: &str, regex: &str, replacement: &str) -> Result<String> {
    let pattern =
        JavaPattern::compile(regex, 0).map_err(|e| LuceneError::illegal_argument(e.to_string()))?;
    let units: Vec<u16> = s.encode_utf16().collect();
    let replaced = pattern.matcher(&units).replace_all(replacement)?;
    Ok(String::from_utf16_lossy(&replaced))
}

/// The file name of `path` as a string, `Path.getFileName().toString()`.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Equivalent to `compile(Path, Path)`.
fn compile(src_dir: &Path, dest_dir: &Path) -> Result<()> {
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(src_dir)? {
        let path = entry?.path();
        if file_name(&path).ends_with("rbbi") {
            files.push(path);
        }
    }

    if files.is_empty() {
        return Err(LuceneError::from(std::io::Error::other(format!(
            "No input files matching *.rbbi at: {}",
            src_dir.display()
        ))));
    }
    for file in &files {
        let output_file = dest_dir.join(replace_all(&file_name(file), "rbbi$", "brk")?);
        let rules = get_rules(file)?;
        eprint!(
            "Compiling {} to {}: ",
            file_name(file),
            file_name(&output_file)
        );
        /*
         * if there is a syntax error, compileRules() may succeed. the way to
         * check is to try to instantiate from the string. additionally if the
         * rules are invalid, you can get a useful syntax error.
         */
        match RuleBasedBreakIterator::new(&rules) {
            Ok(_) => {}
            Err(e) if e.is_illegal_argument_exception() => {
                /*
                 * do this intentionally, so you don't get a massive stack trace
                 * instead, get a useful syntax error!
                 */
                eprintln!("{}", get_message(&e));
                std::process::exit(1);
            }
            Err(e) => return Err(e),
        }
        {
            let mut os = FileOutputStream::create(&output_file)?;
            let compiled = RuleBasedBreakIterator::compile_rules(&rules, &mut os);
            let closed = os.close();
            compiled?;
            closed?;
        }
        eprintln!("{} bytes.", std::fs::metadata(&output_file)?.len());
    }
    Ok(())
}

/// What the JVM's uncaught-exception handler prints for an exception
/// thrown out of `main`: `Exception in thread "main" ` and the
/// `printStackTrace()` text of `error` (its `toString()`, its suppressed
/// exceptions and its causes; a Rust error carries no JVM frames, so no
/// `\tat` line is printed).
fn uncaught_in_main(error: &LuceneError) -> String {
    let mut out = String::from("Exception in thread \"main\" ");
    out.push_str(&rucene::util::concurrent::throwable_to_string(error));
    out.push('\n');
    for suppressed in error.get_suppressed() {
        out.push_str("\tSuppressed: ");
        out.push_str(&rucene::util::concurrent::throwable_to_string(suppressed));
        out.push('\n');
    }
    let mut cause = std::error::Error::source(error);
    while let Some(current) = cause {
        out.push_str("Caused by: ");
        match current.downcast_ref::<LuceneError>() {
            Some(lucene) => out.push_str(&rucene::util::concurrent::throwable_to_string(lucene)),
            None => out.push_str(&current.to_string()),
        }
        out.push('\n');
        cause = current.source();
    }
    out
}

/// Equivalent to `RBBIRuleCompiler.main(String[])`.
fn main() {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    if args.len() < 2 {
        eprintln!("Usage: RBBIRuleComputer <sourcedir> <destdir>");
        std::process::exit(1);
    }
    if let Err(e) = compile(Path::new(&args[0]), Path::new(&args[1])) {
        let _ = std::io::stdout().flush();
        let mut err = std::io::stderr();
        let _ = err.write_all(uncaught_in_main(&e).as_bytes());
        let _ = err.flush();
        std::process::exit(1);
    }
    std::process::exit(0);
}
