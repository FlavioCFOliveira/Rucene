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

//! Downloads/generates lucene/analysis/icu/src/data/utr30/*.txt for the
//! specified icu release tag, ported from
//! `lucene/analysis/icu/src/tools/java/org/apache/lucene/analysis/icu/GenerateUTR30DataFiles.java`.
//!
//! ASSUMPTION: This class will be run with current directory set to
//! lucene/analysis/icu/src/data/utr30/
//!
//! 1. Downloads nfc.txt, nfkc.txt and nfkc_cf.txt from icu-project.org,
//!    overwriting the versions in lucene/analysis/icu/src/data/utr30/.
//! 2. Converts round-trip mappings in nfc.txt (containing '=') that map to
//!    at least one \[:Diacritic:\] character into one-way mappings ('>'
//!    instead of '=').
//!
//! Run as `java org.apache.lucene.analysis.icu.GenerateUTR30DataFiles
//! <releaseTag>` runs (`gradle/generation/icu.gradle`, task
//! `genUtr30DataFilesInternal`, with the working directory
//! `src/data/utr30` and the tag `release-77-1`):
//! `generate_utr30_data_files <releaseTag>`. The tool only rewrites the
//! `*.txt` data files; `utr30.nrm` is then built from them by ICU4C's
//! `gennorm2` and `icupkg -tb`, which the Gradle task runs after this tool
//! and which are not part of the tool (user decision on rmp #641,
//! 2026-11-23).
//!
//! **Divergence from Lucene 10.5.0.** The downloads go through the `ureq`
//! HTTPS client in place of `java.net.URLConnection` (user decision on rmp
//! #641, 2026-11-23). The request is the one `HttpURLConnection` sends for
//! `GenerateUTR30DataFiles.openConnection`: method `GET`, `Cache-Control:
//! no-cache` (`addRequestProperty`), `Accept: */*` and no `Accept-Encoding`
//! (`HttpURLConnection`'s defaults), redirects followed up to 20 times
//! (`http.maxRedirects`), and a status of 400 or above refused as
//! `getInputStream()` refuses it (`FileNotFoundException(url)` for 404 and
//! 410, `IOException("Server returned HTTP response code: <code> for URL:
//! <url>")` otherwise). The differences: the `User-Agent` is `Java/25`
//! where the JDK sends `Java/<java.version>` (the full version string of
//! the running JVM, which a native program has no counterpart of); a
//! redirect to another protocol is followed where `HttpURLConnection`
//! returns the redirect response; and transport failures carry `ureq`'s
//! message (`setUseCaches(false)` needs no counterpart: no response cache
//! is installed).

use std::io::Write as _;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};

use icu4j::lang::u_character::UCharacter;
use icu4j::lang::u_property::UProperty;
use icu4j::text::unicode_set::UnicodeSet;
use icu4j::text::unicode_set_iterator::{UnicodeSetIterator, IS_STRING};
use rucene::analysis::java_regex::{JavaPattern, CASE_INSENSITIVE};
use rucene::error::{LuceneError, Result};
use rucene::store::java_nio_file::JavaIoException;
use rucene_analysis_common::jdk::io::{BufferedReader, InputStreamReader};

const ICU_GIT_TAG_URL: &str = "https://raw.githubusercontent.com/unicode-org/icu";
const ICU_DATA_NORM2_PATH: &str = "icu4c/source/data/unidata/norm2";
const NFC_TXT: &str = "nfc.txt";
const NFKC_TXT: &str = "nfkc.txt";
const NFKC_CF_TXT: &str = "nfkc_cf.txt";

/// `ROUND_TRIP_MAPPING_LINE_PATTERN`.
fn round_trip_mapping_line_pattern() -> JavaPattern {
    compile_pattern(r"^\s*([^=]+?)\s*=\s*(.*)$", 0)
}

/// `VERBATIM_RULE_LINE_PATTERN`.
fn verbatim_rule_line_pattern() -> JavaPattern {
    compile_pattern(r"^#\s*Rule:\s*verbatim\s*$", CASE_INSENSITIVE)
}

/// `RULE_LINE_PATTERN`.
fn rule_line_pattern() -> JavaPattern {
    compile_pattern(r"^#\s*Rule:\s*(.*)>(.*)", CASE_INSENSITIVE)
}

/// `BLANK_OR_COMMENT_LINE_PATTERN`.
fn blank_or_comment_line_pattern() -> JavaPattern {
    compile_pattern(r"^\s*(?:#.*)?$", 0)
}

/// `NUMERIC_VALUE_PATTERN`.
fn numeric_value_pattern() -> JavaPattern {
    compile_pattern(r"Numeric[-\s_]*Value", CASE_INSENSITIVE)
}

/// `Pattern.compile(regex, flags)` of one of the class's constant patterns.
fn compile_pattern(regex: &str, flags: u32) -> JavaPattern {
    JavaPattern::compile(regex, flags).expect("INVARIANT: the class's patterns are well-formed")
}

/// `Pattern.matcher(s).matches()`.
fn matches(pattern: &JavaPattern, s: &[u16]) -> bool {
    pattern.matcher(s).matches()
}

/// `String.trim()` over UTF-16 units.
fn trim(s: &[u16]) -> &[u16] {
    let mut start = 0;
    let mut end = s.len();
    while start < end && s[start] <= 0x20 {
        start += 1;
    }
    while start < end && s[end - 1] <= 0x20 {
        end -= 1;
    }
    &s[start..end]
}

/// UTF-16 units of `s`.
fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// A `java.io.IOException` subclass `class` with `message`.
fn io_exception(class: &'static str, message: String) -> LuceneError {
    LuceneError::from(JavaIoException::new(class, Some(message)).into_io_error())
}

/// Equivalent to `GenerateUTR30DataFiles.main(String[])`: the two steps,
/// any `Throwable` printed with its stack trace and turned into exit
/// status 1. An unchecked exception of the port is a panic, which is caught
/// here as Java's `catch (Throwable t)` catches it.
fn main() {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    // The panic message is printed by the handler below, as
    // `t.printStackTrace(System.err)` prints the exception.
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<()> {
        if args.len() != 1 {
            return Err(LuceneError::illegal_argument(
                "usage: org.apache.lucene.analysis.icu.GenerateUTR30DataFiles <releaseTag>"
                    .to_string(),
            ));
        }
        get_nfkc_data_files_from_icu_project(&args[0])?;
        expand_rules_in_utr30_data_files()?;
        Ok(())
    }));
    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(t)) => {
            print_stack_trace(&t);
            std::process::exit(1);
        }
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_else(|| "java.lang.Error".to_string());
            let mut err = std::io::stderr();
            let _ = writeln!(err, "{message}");
            let _ = err.flush();
            std::process::exit(1);
        }
    }
}

/// `t.printStackTrace(System.err)`: the exception's `toString()`, its
/// suppressed exceptions and its causes (a Rust error carries no JVM
/// frames, so no `\tat` line is printed).
fn print_stack_trace(t: &LuceneError) {
    let mut out = rucene::util::concurrent::throwable_to_string(t);
    out.push('\n');
    for suppressed in t.get_suppressed() {
        out.push_str("\tSuppressed: ");
        out.push_str(&rucene::util::concurrent::throwable_to_string(suppressed));
        out.push('\n');
    }
    let mut cause = std::error::Error::source(t);
    while let Some(current) = cause {
        out.push_str("Caused by: ");
        match current.downcast_ref::<LuceneError>() {
            Some(lucene) => out.push_str(&rucene::util::concurrent::throwable_to_string(lucene)),
            None => out.push_str(&current.to_string()),
        }
        out.push('\n');
        cause = current.source();
    }
    let mut err = std::io::stderr();
    let _ = err.write_all(out.as_bytes());
    let _ = err.flush();
}

/// `Files.list(Paths.get("."))`: the entries of the working directory, as
/// `./<name>` paths, in the directory's order.
fn list_working_directory() -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(".")? {
        paths.push(Path::new(".").join(entry?.file_name()));
    }
    Ok(paths)
}

/// The file name of `path`, `Path.getFileName().toString()`.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Equivalent to `expandRulesInUTR30DataFiles()`.
fn expand_rules_in_utr30_data_files() -> Result<()> {
    let txt = compile_pattern(r".*\.(?s:txt)", 0);
    let predicate = |path: &Path| -> bool {
        let name = file_name(path);
        path.is_file()
            && matches(&txt, &units(&name))
            && name != NFC_TXT
            && name != NFKC_TXT
            && name != NFKC_CF_TXT
    };
    let files: Vec<PathBuf> = list_working_directory()?
        .into_iter()
        .filter(|p| predicate(p))
        .collect();
    for file in &files {
        expand_data_file_rules(file)?;
    }
    Ok(())
}

/// Equivalent to `expandDataFileRules(Path)`.
fn expand_data_file_rules(file: &Path) -> Result<()> {
    let verbatim_rule_line = verbatim_rule_line_pattern();
    let rule_line = rule_line_pattern();
    let blank_or_comment_line = blank_or_comment_line_pattern();

    let mut modified = false;
    let mut builder: Vec<u16> = Vec::new();

    {
        let stream = std::fs::File::open(file)?;
        let mut buffered_reader = BufferedReader::new(InputStreamReader::new_utf8(stream));
        let mut verbatim = false;
        let mut line_num = 0;

        while let Some(line) = buffered_reader.read_line()? {
            line_num += 1;
            if matches(&verbatim_rule_line, &line) {
                verbatim = true;
                builder.extend_from_slice(&line);
                builder.push(u16::from(b'\n'));
            } else {
                let mut rule_matcher = rule_line.matcher(&line);
                if rule_matcher.matches() {
                    verbatim = false;
                    builder.extend_from_slice(&line);
                    builder.push(u16::from(b'\n'));
                    let expanded = (|| -> Result<()> {
                        let group1 = rule_matcher.group(1)?.unwrap_or_default();
                        let group2 = rule_matcher.group(2)?.unwrap_or_default();
                        let left_hand_side = trim(&group1);
                        let right_hand_side = trim(&group2);
                        expand_single_rule(&mut builder, left_hand_side, right_hand_side)
                    })();
                    match expanded {
                        Ok(()) => {}
                        Err(e) if e.is_illegal_argument_exception() => {
                            eprintln!("ERROR in {} line #{line_num}:", file_name(file));
                            print_stack_trace(&e);
                            std::process::exit(1);
                        }
                        Err(e) => return Err(e),
                    }
                    modified = true;
                } else if matches(&blank_or_comment_line, &line) {
                    builder.extend_from_slice(&line);
                    builder.push(u16::from(b'\n'));
                } else if verbatim {
                    builder.extend_from_slice(&line);
                    builder.push(u16::from(b'\n'));
                } else {
                    modified = true;
                }
            }
        }
        buffered_reader.close()?;
    }

    if modified {
        eprintln!("Expanding rules in and overwriting {}", file_name(file));
        // `Files.writeString(file, builder.toString(), UTF_8)`: an unpaired
        // surrogate is a `MalformedInputException` of the encoder.
        let text = String::from_utf16(&builder).map_err(|_| {
            io_exception("java.nio.charset.MalformedInputException", "Input length = 1".to_string())
        })?;
        std::fs::write(file, text.as_bytes())?;
    }
    Ok(())
}

/// Equivalent to `getNFKCDataFilesFromIcuProject(String)`.
fn get_nfkc_data_files_from_icu_project(release_tag: &str) -> Result<()> {
    let icu_tags_uri = Uri::create(&format!("{ICU_GIT_TAG_URL}/"))?;
    let icu_release_tag_uri = icu_tags_uri.resolve(&format!("{release_tag}/"))?;
    let norm2uri = icu_release_tag_uri.resolve(&format!("{ICU_DATA_NORM2_PATH}/"))?;

    eprint!("Downloading {NFKC_TXT} ... ");
    download(&norm2uri.resolve(NFKC_TXT)?, NFKC_TXT)?;
    eprintln!("done.");
    eprint!("Downloading {NFKC_CF_TXT} ... ");
    download(&norm2uri.resolve(NFKC_CF_TXT)?, NFKC_CF_TXT)?;
    eprintln!("done.");

    eprint!("Downloading {NFKC_CF_TXT} and making diacritic rules one-way ... ");
    let round_trip_mapping_line = round_trip_mapping_line_pattern();
    let whitespace = compile_pattern(r"\s+", 0);
    let connection = open_connection(&norm2uri.resolve(NFC_TXT)?)?;
    {
        let input = connection.get_input_stream()?;
        let mut reader = BufferedReader::new(InputStreamReader::new_utf8(input));
        let mut writer = std::fs::File::create(NFC_TXT)?;
        let mut out: Vec<u16> = Vec::new();

        while let Some(mut line) = reader.read_line()? {
            let mut matcher = round_trip_mapping_line.matcher(&line);
            if matcher.matches() {
                let left_hand_side = matcher.group(1)?.unwrap_or_default();
                let group2 = matcher.group(2)?.unwrap_or_default();
                let right_hand_side = trim(&group2).to_vec();
                let mut diacritics: Vec<Vec<u16>> = Vec::new();
                for output_code_point in whitespace.split(&right_hand_side, 0) {
                    let ch = icu4j::jdk::parse_int(&output_code_point, 16)?;
                    if UCharacter::has_binary_property(ch, UProperty::DIACRITIC)
                        // gennorm2 fails if U+0653-U+0656 are included in round-trip mappings
                        || (0x653..=0x656).contains(&ch)
                    {
                        diacritics.push(output_code_point);
                    }
                }
                if !diacritics.is_empty() {
                    let mut replacement_line: Vec<u16> = Vec::new();
                    replacement_line.extend_from_slice(&left_hand_side);
                    replacement_line.push(u16::from(b'>'));
                    replacement_line.extend_from_slice(&right_hand_side);
                    replacement_line.extend(units("  # one-way: diacritic"));
                    if diacritics.len() > 1 {
                        replacement_line.push(u16::from(b's'));
                    }
                    for diacritic in &diacritics {
                        replacement_line.push(u16::from(b' '));
                        replacement_line.extend_from_slice(diacritic);
                    }
                    line = replacement_line;
                }
            }
            out.clear();
            out.extend_from_slice(&line);
            out.push(u16::from(b'\n'));
            // `OutputStreamWriter(..., UTF_8)`: an unpaired surrogate is
            // written as '?', the encoder's replacement.
            let bytes: Vec<u8> = char::decode_utf16(out.iter().copied())
                .map(|r| r.unwrap_or('?'))
                .collect::<String>()
                .into_bytes();
            writer.write_all(&bytes)?;
        }
        writer.flush()?;
        reader.close()?;
    }
    eprintln!("done.");
    Ok(())
}

/// Equivalent to `download(URI, String)`: the resource's bytes copied into
/// the file `output_file` of the working directory.
fn download(uri: &Uri, output_file: &str) -> Result<()> {
    let connection = open_connection(uri)?;
    let mut input_stream = connection.get_input_stream()?;
    let mut output_stream = std::fs::File::create(output_file)?;
    std::io::copy(&mut input_stream, &mut output_stream)?;
    output_stream.flush()?;
    Ok(())
}

/// A connection opened by [`open_connection`]: the response of the request,
/// read through [`get_input_stream`](Self::get_input_stream).
struct UrlConnection {
    url: String,
    status: u16,
    body: Option<ureq::Body>,
}

impl UrlConnection {
    /// `URLConnection.getInputStream()`: the response body, or the
    /// exception `HttpURLConnection` throws for a status of 400 or above.
    fn get_input_stream(mut self) -> Result<Box<dyn std::io::Read + Send>> {
        if self.status >= 400 {
            if self.status == 404 || self.status == 410 {
                return Err(io_exception("java.io.FileNotFoundException", self.url));
            }
            return Err(io_exception(
                "java.io.IOException",
                format!(
                    "Server returned HTTP response code: {} for URL: {}",
                    self.status, self.url
                ),
            ));
        }
        let body = self.body.take().expect("INVARIANT: the body is read once");
        Ok(Box::new(body.into_reader()))
    }
}

/// Equivalent to `openConnection(URL)`: the connection is opened with no
/// caching and `Cache-Control: no-cache`, and connected.
fn open_connection(url: &Uri) -> Result<UrlConnection> {
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(20)
        .user_agent("Java/25")
        .accept("*/*")
        .accept_encoding(ureq::config::AutoHeaderValue::None)
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let url_string = url.to_string();
    match agent
        .get(&url_string)
        .header("Cache-Control", "no-cache")
        .call()
    {
        Ok(response) => {
            let status = response.status().as_u16();
            Ok(UrlConnection {
                url: url_string,
                status,
                body: Some(response.into_body()),
            })
        }
        Err(ureq::Error::HostNotFound) => {
            Err(io_exception("java.net.UnknownHostException", url.host().to_string()))
        }
        Err(ureq::Error::TooManyRedirects) => Err(io_exception(
            "java.net.ProtocolException",
            "Server redirected too many times (20)".to_string(),
        )),
        Err(ureq::Error::Io(e)) => Err(LuceneError::from(e)),
        Err(e) => Err(io_exception("java.io.IOException", e.to_string())),
    }
}

/// The hierarchical URIs this tool builds, as `java.net.URI` parses and
/// resolves them (RFC 2396, `URI.create` and `URI.resolve(String)`):
/// `scheme://authority/path[?query][#fragment]`.
#[derive(Debug, Clone)]
struct Uri {
    scheme: String,
    authority: String,
    path: String,
    query: Option<String>,
    fragment: Option<String>,
}

impl std::fmt::Display for Uri {
    /// `URI.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}://{}{}", self.scheme, self.authority, self.path)?;
        if let Some(query) = &self.query {
            write!(f, "?{query}")?;
        }
        if let Some(fragment) = &self.fragment {
            write!(f, "#{fragment}")?;
        }
        Ok(())
    }
}

/// Whether `c` may appear in a URI component as `java.net.URI`'s parser
/// accepts it: unreserved, reserved and escaped (`%`) characters, and the
/// non-ASCII "other" characters (neither space nor control).
fn is_uri_char(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_alphanumeric() || "-_.!~*'();:@&=+$,%/?".contains(c)
    } else {
        !(c.is_whitespace() || c.is_control())
    }
}

/// `URISyntaxException` as `URI.create` wraps it, `new
/// IllegalArgumentException(x.getMessage(), x)`.
fn illegal_character(component: &str, index: usize, input: &str) -> LuceneError {
    LuceneError::illegal_argument(format!(
        "Illegal character in {component} at index {index}: {input}"
    ))
}

/// Validates the characters of `s` (a path, query or fragment) whose first
/// character is at the UTF-16 index `offset` of `input`, checking each `%`
/// escape.
fn check_chars(s: &str, offset: usize, component: &str, input: &str) -> Result<()> {
    let chars: Vec<char> = s.chars().collect();
    let mut index = offset;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '%' {
            let ok = chars.get(i + 1).is_some_and(char::is_ascii_hexdigit)
                && chars.get(i + 2).is_some_and(char::is_ascii_hexdigit);
            if !ok {
                return Err(LuceneError::illegal_argument(format!(
                    "Malformed escape pair at index {index}: {input}"
                )));
            }
            i += 3;
            index += 3;
            continue;
        }
        if !is_uri_char(c) || (component == "path" && c == '?') {
            return Err(illegal_character(component, index, input));
        }
        index += c.len_utf16();
        i += 1;
    }
    Ok(())
}

/// The path, query and fragment of a relative reference or of the part of
/// an absolute URI after its authority, validated.
fn split_reference(s: &str, offset: usize, input: &str) -> Result<(String, Option<String>, Option<String>)> {
    let (before_fragment, fragment) = match s.find('#') {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let (path, query) = match before_fragment.find('?') {
        Some(i) => (&before_fragment[..i], Some(&before_fragment[i + 1..])),
        None => (before_fragment, None),
    };
    let len16 = |t: &str| t.encode_utf16().count();
    check_chars(path, offset, "path", input)?;
    if let Some(query) = query {
        check_chars(query, offset + len16(path) + 1, "query", input)?;
    }
    if let Some(fragment) = fragment {
        check_chars(fragment, offset + len16(before_fragment) + 1, "fragment", input)?;
    }
    Ok((
        path.to_string(),
        query.map(str::to_string),
        fragment.map(str::to_string),
    ))
}

/// RFC 2396 section 5.2 step 6 as `URI.normalize` applies it: `.`
/// segments removed, `..` segments removed with the segment before them
/// (a leading `..` stays).
fn remove_dot_segments(path: &str) -> String {
    let absolute = path.starts_with('/');
    let trailing_slash = path.ends_with('/');
    let mut segments: Vec<&str> = Vec::new();
    let raw: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let last = raw.len().saturating_sub(1);
    let mut ends_with_dir = trailing_slash;
    for (i, segment) in raw.iter().enumerate() {
        match *segment {
            "." => {
                if i == last {
                    ends_with_dir = true;
                }
            }
            ".." => {
                if segments.last().is_some_and(|s| *s != "..") {
                    segments.pop();
                } else if !absolute {
                    segments.push("..");
                }
                if i == last {
                    ends_with_dir = true;
                }
            }
            s => segments.push(s),
        }
    }
    let mut result = String::new();
    if absolute {
        result.push('/');
    }
    result.push_str(&segments.join("/"));
    if ends_with_dir && !segments.is_empty() {
        result.push('/');
    }
    result
}

impl Uri {
    /// `URI.create(str)`: an absolute hierarchical URI.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` with `URISyntaxException`'s message for a
    /// string that is not one.
    fn create(s: &str) -> Result<Uri> {
        let Some(colon) = s.find("://") else {
            return Err(LuceneError::illegal_argument(format!(
                "Expected scheme-specific part at index 0: {s}"
            )));
        };
        let scheme = &s[..colon];
        let rest = &s[colon + 3..];
        let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let authority = &rest[..authority_end];
        let offset = s[..colon + 3 + authority_end].encode_utf16().count();
        let (path, query, fragment) = split_reference(&rest[authority_end..], offset, s)?;
        Ok(Uri {
            scheme: scheme.to_string(),
            authority: authority.to_string(),
            path,
            query,
            fragment,
        })
    }

    /// The host of the authority (without user information or port).
    fn host(&self) -> &str {
        let host_port = self.authority.rsplit('@').next().unwrap_or(&self.authority);
        host_port.split(':').next().unwrap_or(host_port)
    }

    /// `URI.resolve(String str)`, `resolve(URI.create(str))`: a relative
    /// reference resolved against this URI (RFC 2396 section 5.2).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` with `URISyntaxException`'s message for a
    /// malformed reference.
    fn resolve(&self, s: &str) -> Result<Uri> {
        if s.contains("://") {
            return Uri::create(s);
        }
        let (path, query, fragment) = split_reference(s, 0, s)?;
        if path.is_empty() && query.is_none() {
            // A reference that is a bare fragment: this URI with that fragment.
            if let Some(fragment) = fragment {
                let mut resolved = self.clone();
                resolved.fragment = Some(fragment);
                return Ok(resolved);
            }
        }
        let merged = if path.starts_with('/') {
            path
        } else {
            let base_dir = match self.path.rfind('/') {
                Some(i) => &self.path[..=i],
                None => "/",
            };
            format!("{base_dir}{path}")
        };
        Ok(Uri {
            scheme: self.scheme.clone(),
            authority: self.authority.clone(),
            path: remove_dot_segments(&merged),
            query,
            fragment,
        })
    }
}

/// Equivalent to `expandSingleRule(StringBuilder, String, String)`.
///
/// # Errors
///
/// The `IllegalArgumentException` of a malformed `UnicodeSet` pattern.
fn expand_single_rule(builder: &mut Vec<u16>, left_hand_side: &[u16], right_hand_side: &[u16]) -> Result<()> {
    let set = UnicodeSet::from_pattern_options(left_hand_side, UnicodeSet::IGNORE_SPACE)?;
    let numeric_value = matches(&numeric_value_pattern(), right_hand_side);
    let mut it = UnicodeSetIterator::new(&set);
    while it.next_range() {
        if it.codepoint != IS_STRING {
            if numeric_value {
                for cp in it.codepoint..=it.codepoint_end {
                    builder.extend(units(&format!("{:04X}", cp as u32)));
                    builder.push(u16::from(b'>'));
                    builder.extend(units(&format!(
                        "{:04X}",
                        (0x30 + UCharacter::get_numeric_value(cp)) as u32
                    )));
                    builder.extend(units("   # "));
                    match UCharacter::get_name_cp(cp) {
                        Some(name) => builder.extend_from_slice(&name),
                        None => builder.extend(units("null")),
                    }
                    builder.push(u16::from(b'\n'));
                }
            } else {
                builder.extend(units(&format!("{:04X}", it.codepoint as u32)));
                if it.codepoint_end > it.codepoint {
                    builder.extend(units(".."));
                    builder.extend(units(&format!("{:04X}", it.codepoint_end as u32)));
                }
                builder.push(u16::from(b'>'));
                builder.extend_from_slice(right_hand_side);
                builder.push(u16::from(b'\n'));
            }
        } else {
            let string = it.get_string()?;
            eprintln!(
                "ERROR: String '{}' found in UnicodeSet",
                String::from_utf16_lossy(&string)
            );
            std::process::exit(1);
        }
    }
    Ok(())
}
