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
//! no-cache` (`addRequestProperty`), `User-Agent: Java/<java.version>` of
//! the reference JDK 25, `Accept: */*` and no `Accept-Encoding`
//! (`HttpURLConnection`'s defaults); redirects followed as
//! `HttpURLConnection.followRedirect()` follows them (same protocol only,
//! a redirect to another protocol returning the redirect response, at most
//! 20 times, `http.maxRedirects`); a status of 400 or above refused as
//! `getInputStream()` refuses it (`FileNotFoundException(url)` for 404 and
//! 410, `IOException("Server returned HTTP response code: <code> for URL:
//! <url>")` otherwise); and transport failures raised as the JDK
//! exceptions `URLConnection` raises for them (`UnknownHostException`,
//! `ConnectException`, `SocketException`, `SSLHandshakeException`, ...; see
//! `transport_exception`).

use std::io::Write as _;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use icu4j::lang::u_character::UCharacter;
use icu4j::lang::u_property::UProperty;
use icu4j::text::unicode_set::UnicodeSet;
use icu4j::text::unicode_set_iterator::{UnicodeSetIterator, IS_STRING};
use rucene::analysis::java_regex::{JavaPattern, CASE_INSENSITIVE};
use rucene::error::{JavaThrowable, LuceneError, LuceneErrorKind, Result};
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
    // The causes of a JDK exception carried with its class (the `java.net`
    // and `javax.net.ssl` exceptions of `open_connection`).
    let mut t = t;
    while let LuceneErrorKind::Throwable(throwable) = t.kind() {
        let Some(cause) = throwable.get_cause() else {
            break;
        };
        out.push_str("Caused by: ");
        out.push_str(&rucene::util::concurrent::throwable_to_string(cause));
        out.push('\n');
        t = cause;
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

/// `System.getProperty("java.version")` of the reference JDK 25, the JVM
/// this port is verified against (`java -XshowSettings:properties
/// -version` prints `java.version = 25.0.5.1`). `HttpURLConnection` sends
/// it as `User-Agent: Java/<java.version>` (`HttpURLConnection.java`, the
/// static initializer: `agent = "Java/" + version`).
const JAVA_VERSION: &str = "25.0.5.1";

/// `HttpURLConnection.defaultmaxRedirects`, the `http.maxRedirects` default.
const MAX_REDIRECTS: u32 = 20;

/// The protocols `java.net.URL` has a stream handler for in JDK 25
/// (`URL.DefaultFactory` and the `sun.net.www.protocol.<protocol>.Handler`
/// classes of `java.base`): any other is `MalformedURLException("unknown
/// protocol: <protocol>")`.
const KNOWN_PROTOCOLS: [&str; 8] = ["file", "ftp", "http", "https", "jar", "jmod", "jrt", "mailto"];

const MALFORMED_URL_EXCEPTION: &str = "java.net.MalformedURLException";
const PROTOCOL_EXCEPTION: &str = "java.net.ProtocolException";
const UNKNOWN_HOST_EXCEPTION: &str = "java.net.UnknownHostException";
const CONNECT_EXCEPTION: &str = "java.net.ConnectException";
const NO_ROUTE_TO_HOST_EXCEPTION: &str = "java.net.NoRouteToHostException";
const BIND_EXCEPTION: &str = "java.net.BindException";
const SOCKET_EXCEPTION: &str = "java.net.SocketException";
const SSL_EXCEPTION: &str = "javax.net.ssl.SSLException";
const SSL_HANDSHAKE_EXCEPTION: &str = "javax.net.ssl.SSLHandshakeException";

/// A connection opened by [`open_connection`]: the response of the request,
/// read through [`get_input_stream`](Self::get_input_stream).
struct UrlConnection {
    /// `HttpURLConnection.url`: the URL of the response, the last redirect
    /// target followed.
    url: JavaUrl,
    status: u16,
    body: Option<ureq::Body>,
}

impl UrlConnection {
    /// `URLConnection.getInputStream()`: the response body, or the
    /// exception `HttpURLConnection` throws for a status of 400 or above.
    fn get_input_stream(mut self) -> Result<Box<dyn std::io::Read + Send>> {
        let url = self.url.to_external_form();
        if self.status >= 400 {
            if self.status == 404 || self.status == 410 {
                return Err(io_exception("java.io.FileNotFoundException", url));
            }
            return Err(io_exception(
                "java.io.IOException",
                format!("Server returned HTTP response code: {} for URL: {url}", self.status),
            ));
        }
        let body = self.body.take().expect("INVARIANT: the body is read once");
        Ok(Box::new(SocketInputStream(body.into_reader())))
    }
}

/// The response stream of a connection: a read failure of the socket is
/// the exception `java.net.Socket`'s input stream raises for it (see
/// [`socket_exception_of`]).
struct SocketInputStream<R>(R);

impl<R: std::io::Read> std::io::Read for SocketInputStream<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf).map_err(|e| match socket_exception_of(&e) {
            Some((class, message)) => JavaIoException::new(class, Some(message)).into_io_error(),
            None => e,
        })
    }
}

/// Equivalent to `openConnection(URL)`: the connection is opened with no
/// caching and `Cache-Control: no-cache`, and connected.
///
/// The request is the one `HttpURLConnection` sends: method `GET`,
/// `User-Agent: Java/<java.version>` ([`JAVA_VERSION`]), `Accept: */*`, no
/// `Accept-Encoding`. Redirects are followed as `HttpURLConnection.
/// followRedirect()` follows them: only the statuses 300 to 307 other than
/// 304 and 306, only with a `Location` header (its last value), only to
/// the same protocol — a `Location` naming another protocol returns the
/// redirect response itself — and a relative `Location` resolved against
/// the current URL as `new URL(url, loc)` resolves it; 305 re-requests the
/// same URL through the proxy the `Location` names. The 20th redirect
/// (`http.maxRedirects`) throws `ProtocolException("Server redirected too
/// many times (20)")` (`getInputStream0`'s `do { ... } while (redirects <
/// maxRedirects)`).
fn open_connection(uri: &Uri) -> Result<UrlConnection> {
    // `URI.toURL()`.
    let mut url = JavaUrl::new(None, &uri.to_string())?;
    let mut proxy: Option<(String, ureq::Proxy)> = None;
    let mut redirects = 0;
    loop {
        let agent = new_agent(proxy.as_ref().map(|(_, p)| p.clone()));
        let connect_host = match &proxy {
            Some((host, _)) => host.clone(),
            None => url.host.clone().unwrap_or_default(),
        };
        let response = agent
            .get(&url.request_url())
            .header("Cache-Control", "no-cache")
            .call()
            .map_err(|e| transport_exception(e, &connect_host, url.protocol == "https"))?;
        let status = response.status().as_u16();
        // `getHeaderField("Location")`: `MessageHeader.findValue` returns the
        // last value, its bytes read as ISO-8859-1.
        let location: Option<String> = response
            .headers()
            .get_all("Location")
            .iter()
            .next_back()
            .map(|v| v.as_bytes().iter().map(|&b| char::from(b)).collect());
        match follow_redirect(&url, status, location.as_deref())? {
            None => {
                return Ok(UrlConnection {
                    url,
                    status,
                    body: Some(response.into_body()),
                });
            }
            Some(Redirect::To(target)) => url = target,
            Some(Redirect::UseProxy { host, port }) => {
                // `HttpClient.newHttpProxy`: a port of -1 is the default port
                // of the connection's protocol ("http" for `HttpClient`,
                // "https" for `HttpsClient`).
                let port = if port < 0 { url.default_port() } else { port };
                let proxy_url = format!("http://{host}:{port}");
                let p = ureq::Proxy::new(&proxy_url)
                    .map_err(|e| io_exception("java.io.IOException", e.to_string()))?;
                proxy = Some((host, p));
            }
        }
        redirects += 1;
        if redirects >= MAX_REDIRECTS {
            return Err(io_exception(
                PROTOCOL_EXCEPTION,
                format!("Server redirected too many times ({redirects})"),
            ));
        }
    }
}

/// The `ureq` agent of one request: no redirect followed by `ureq` (they
/// are followed by [`open_connection`]), statuses returned as responses,
/// and `HttpURLConnection`'s default headers.
fn new_agent(proxy: Option<ureq::Proxy>) -> ureq::Agent {
    let mut builder = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .user_agent(format!("Java/{JAVA_VERSION}"))
        .accept("*/*")
        .accept_encoding(ureq::config::AutoHeaderValue::None);
    if proxy.is_some() {
        builder = builder.proxy(proxy);
    }
    ureq::Agent::with_parts(
        builder.build(),
        ureq::unversioned::transport::DefaultConnector::default(),
        JavaResolver::default(),
    )
}

/// What `HttpURLConnection.followRedirect()` decides for a response.
enum Redirect {
    /// `followRedirect0` for a status other than 305: the next request goes
    /// to this URL.
    To(JavaUrl),
    /// `followRedirect0` for 305 (`HTTP_USE_PROXY`): the same URL is
    /// requested again through the proxy at `host:port` (`-1` for no port).
    UseProxy { host: String, port: i32 },
}

/// Equivalent to `HttpURLConnection.followRedirect()`: `None` when the
/// response is returned as it is.
///
/// # Errors
///
/// `MalformedURLException` when `loc` names an unknown protocol or is a
/// malformed URL even relative to `url` (`newURL(url, loc)`).
fn follow_redirect(url: &JavaUrl, stat: u16, loc: Option<&str>) -> Result<Option<Redirect>> {
    if !(300..=307).contains(&stat) || stat == 306 || stat == 304 {
        return Ok(None);
    }
    let Some(loc) = loc else {
        return Ok(None);
    };
    let loc_url = match JavaUrl::new(None, loc) {
        Ok(loc_url) => {
            if !url.protocol.eq_ignore_ascii_case(&loc_url.protocol) {
                return Ok(None);
            }
            loc_url
        }
        // `MalformedURLException`: "treat loc as a relative URI".
        Err(_) => JavaUrl::new(Some(url), loc)?,
    };
    if stat == 305 {
        return Ok(Some(Redirect::UseProxy {
            host: loc_url.host.clone().unwrap_or_default(),
            port: loc_url.port,
        }));
    }
    Ok(Some(Redirect::To(loc_url)))
}

/// `ureq`'s default resolver, with every resolution failure reported as
/// [`ureq::Error::HostNotFound`]: `InetAddress.getAllByName` throws
/// `UnknownHostException(host)` for any failure of the system resolver.
#[derive(Debug, Default)]
struct JavaResolver(ureq::unversioned::resolver::DefaultResolver);

impl ureq::unversioned::resolver::Resolver for JavaResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> std::result::Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        self.0.resolve(uri, config, timeout).map_err(|e| match e {
            ureq::Error::Io(_) => ureq::Error::HostNotFound,
            other => other,
        })
    }
}

/// The exception `HttpURLConnection` raises for the transport failure
/// `error` of a request to `host` (the proxy host when one is used), as the
/// JDK 25 reference raises it (verified against it):
///
/// * a host the resolver cannot resolve: `UnknownHostException(host)`;
/// * a connect that times out: `ConnectException("Connection timed out")`;
/// * a socket error: [`socket_exception_of`];
/// * the peer closing the connection during the TLS handshake:
///   `SSLHandshakeException("Remote host terminated the handshake")`
///   caused by `EOFException("SSL peer shut down incorrectly")`;
/// * the peer closing the connection before the response:
///   `SocketException("Unexpected end of file from server")`;
/// * a non-TLS answer to a TLS client hello: `SSLException("Unsupported or
///   unrecognized SSL message")`;
/// * a server certificate rejected: [`certificate_exception`].
///
/// Any other failure is an `IOException` with `ureq`'s message. `ureq` has
/// no timeout configured (as `URLConnection` has none by default), so the
/// `SocketTimeoutException` of a read timeout cannot occur.
fn transport_exception(error: ureq::Error, host: &str, https: bool) -> LuceneError {
    match error {
        ureq::Error::HostNotFound => io_exception(UNKNOWN_HOST_EXCEPTION, host.to_string()),
        ureq::Error::Timeout(ureq::Timeout::Connect) => {
            io_exception(CONNECT_EXCEPTION, "Connection timed out".to_string())
        }
        ureq::Error::Io(e) => {
            if let Some((class, message)) = socket_exception_of(&e) {
                return io_exception(class, message);
            }
            match e.kind() {
                std::io::ErrorKind::UnexpectedEof if https && e.get_ref().is_none() => {
                    let cause = jdk_exception(
                        "java.io.EOFException",
                        true,
                        "SSL peer shut down incorrectly".to_string(),
                        None,
                    );
                    jdk_exception(
                        SSL_HANDSHAKE_EXCEPTION,
                        true,
                        "Remote host terminated the handshake".to_string(),
                        Some(cause),
                    )
                }
                std::io::ErrorKind::UnexpectedEof => io_exception(
                    SOCKET_EXCEPTION,
                    "Unexpected end of file from server".to_string(),
                ),
                std::io::ErrorKind::InvalidData => {
                    // The rustls error `ureq` wraps, read through its `Debug`
                    // form (rustls is not a direct dependency of this crate).
                    let tls = e.get_ref().map(|inner| format!("{inner:?}"));
                    match tls.as_deref() {
                        Some("InvalidMessage(InvalidContentType)") => io_exception(
                            SSL_EXCEPTION,
                            "Unsupported or unrecognized SSL message".to_string(),
                        ),
                        Some(tls) if tls.starts_with("InvalidCertificate(") => {
                            certificate_exception(tls, host).unwrap_or_else(|| LuceneError::from(e))
                        }
                        _ => LuceneError::from(e),
                    }
                }
                _ => LuceneError::from(e),
            }
        }
        other => io_exception("java.io.IOException", other.to_string()),
    }
}

/// The `java.net` exception the JDK raises for the socket error `e`, as
/// `sun.nio.ch.Net.handleSocketError` (`Net.c`) classifies the error number
/// with `strerror`'s text as the message, and `NioSocketImpl`'s
/// `ConnectionResetException("Connection reset")` for a reset; `None` for an
/// error that is not a socket error.
fn socket_exception_of(e: &std::io::Error) -> Option<(&'static str, String)> {
    use std::io::ErrorKind;
    if e.kind() == ErrorKind::ConnectionReset {
        return Some((SOCKET_EXCEPTION, "Connection reset".to_string()));
    }
    let errno = e.raw_os_error()?;
    // `strerror(errno)`: the error's text without the " (os error N)" suffix
    // `std` appends.
    let text = e.to_string();
    let message = text
        .strip_suffix(&format!(" (os error {errno})"))
        .unwrap_or(&text)
        .to_string();
    let class = match e.kind() {
        ErrorKind::ConnectionRefused | ErrorKind::TimedOut | ErrorKind::NotConnected => CONNECT_EXCEPTION,
        ErrorKind::HostUnreachable => NO_ROUTE_TO_HOST_EXCEPTION,
        ErrorKind::AddrInUse | ErrorKind::AddrNotAvailable | ErrorKind::PermissionDenied => BIND_EXCEPTION,
        _ => SOCKET_EXCEPTION,
    };
    Some((class, message))
}

/// The `SSLHandshakeException` the JDK's trust manager raises for the
/// certificate failure rustls reports as `tls` (its `Debug` form), with the
/// exception chain JDK 25 prints for it (verified against it); `None` for a
/// failure that has no verified JDK counterpart.
///
/// * an unknown issuer: `(certificate_unknown) PKIX path building failed`;
/// * a certificate not valid for the DNS host `host`: `(certificate_unknown)
///   No subject alternative DNS name matching <host> found.`;
/// * an expired certificate: `(certificate_expired) PKIX path validation
///   failed`, down to `CertificateExpiredException("NotAfter: <date>")`.
///
/// **Divergence.** `CertificateExpiredException`'s message formats the
/// certificate's end date with `Date.toString()`, in the JVM's default time
/// zone (`sun.security.x509.CertificateValidity.valid`); the port formats it
/// in UTC, as the JDK does when its default zone is UTC, because the JDK's
/// time-zone abbreviations (`TimeZone.getDisplayName`) have no counterpart
/// here.
fn certificate_exception(tls: &str, host: &str) -> Option<LuceneError> {
    const BUILD_FAILED: &str = "unable to find valid certification path to requested target";
    const VALIDITY_FAILED: &str = "validity check failed";
    if tls.starts_with("InvalidCertificate(UnknownIssuer") {
        let builder = jdk_exception(
            "sun.security.provider.certpath.SunCertPathBuilderException",
            false,
            BUILD_FAILED.to_string(),
            None,
        );
        let message = format!(
            "PKIX path building failed: sun.security.provider.certpath.SunCertPathBuilderException: {BUILD_FAILED}"
        );
        let validator = jdk_exception(
            "sun.security.validator.ValidatorException",
            false,
            message.clone(),
            Some(builder),
        );
        return Some(jdk_exception(
            SSL_HANDSHAKE_EXCEPTION,
            true,
            format!("(certificate_unknown) {message}"),
            Some(validator),
        ));
    }
    if tls.starts_with("InvalidCertificate(NotValidForName") {
        if host.parse::<std::net::IpAddr>().is_ok() || host.starts_with('[') {
            return None;
        }
        let message = format!("No subject alternative DNS name matching {host} found.");
        let cause = jdk_exception("java.security.cert.CertificateException", false, message.clone(), None);
        return Some(jdk_exception(
            SSL_HANDSHAKE_EXCEPTION,
            true,
            format!("(certificate_unknown) {message}"),
            Some(cause),
        ));
    }
    if tls.starts_with("InvalidCertificate(ExpiredContext") {
        let not_after = tls
            .split("not_after: UnixTime(")
            .nth(1)?
            .split(')')
            .next()?
            .parse::<i64>()
            .ok()?;
        let expired = jdk_exception(
            "java.security.cert.CertificateExpiredException",
            false,
            format!("NotAfter: {}", java_date_to_string_utc(not_after)),
            None,
        );
        let validator_cause = jdk_exception(
            "java.security.cert.CertPathValidatorException",
            false,
            VALIDITY_FAILED.to_string(),
            Some(expired),
        );
        let message =
            format!("PKIX path validation failed: java.security.cert.CertPathValidatorException: {VALIDITY_FAILED}");
        let validator = jdk_exception(
            "sun.security.validator.ValidatorException",
            false,
            message.clone(),
            Some(validator_cause),
        );
        return Some(jdk_exception(
            SSL_HANDSHAKE_EXCEPTION,
            true,
            format!("(certificate_expired) {message}"),
            Some(validator),
        ));
    }
    None
}

/// `java.util.Date.toString()` (`EEE MMM dd HH:mm:ss zzz yyyy`) of the
/// instant `seconds` after the epoch, in UTC.
fn java_date_to_string_utc(seconds: i64) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = seconds.div_euclid(86_400);
    let second_of_day = seconds.rem_euclid(86_400);
    // Days to civil date (proleptic Gregorian), H. Hinnant's algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    // 1970-01-01 was a Thursday.
    let weekday = (days + 4).rem_euclid(7);
    format!(
        "{} {} {:02} {:02}:{:02}:{:02} UTC {}",
        DAYS[weekday as usize],
        MONTHS[(month - 1) as usize],
        day,
        second_of_day / 3_600,
        second_of_day % 3_600 / 60,
        second_of_day % 60,
        year
    )
}

/// A JDK exception carried with its class, message and cause: the
/// `java.net` / `javax.net.ssl` / `java.security.cert` exceptions whose
/// cause chain `printStackTrace` prints.
#[derive(Debug)]
struct JdkException {
    class: &'static str,
    message: String,
    cause: Option<Arc<LuceneError>>,
    io: bool,
}

impl JavaThrowable for JdkException {
    fn java_class(&self) -> &'static str {
        self.class
    }

    fn get_message(&self) -> Option<&str> {
        Some(&self.message)
    }

    fn get_cause(&self) -> Option<&Arc<LuceneError>> {
        self.cause.as_ref()
    }

    fn is_io_exception(&self) -> bool {
        self.io
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// `new <class>(message)` with `initCause(cause)`; `io` tells whether the
/// class extends `java.io.IOException`.
fn jdk_exception(class: &'static str, io: bool, message: String, cause: Option<LuceneError>) -> LuceneError {
    LuceneError::from(LuceneErrorKind::Throwable(Arc::new(JdkException {
        class,
        message,
        cause: cause.map(Arc::new),
        io,
    })))
}

/// A `java.net.URL` with the fields `URL(URL context, String spec)` and the
/// `http`/`https` handlers' `URLStreamHandler.parseURL` set.
#[derive(Debug, Clone)]
struct JavaUrl {
    protocol: String,
    authority: Option<String>,
    host: Option<String>,
    port: i32,
    path: Option<String>,
    query: Option<String>,
    reference: Option<String>,
}

/// `MalformedURLException(message)`.
fn malformed(message: String) -> LuceneError {
    io_exception(MALFORMED_URL_EXCEPTION, message)
}

/// `MalformedURLException(e.getMessage())` with `initCause(e)`, as the
/// `URL` constructor wraps an exception of `parseURL`.
fn malformed_caused_by(class: &'static str, message: String) -> LuceneError {
    let cause = jdk_exception(class, false, message.clone(), None);
    jdk_exception(MALFORMED_URL_EXCEPTION, true, message, Some(cause))
}

/// `Integer.parseInt(CharSequence s, int beginIndex, int endIndex, 10)` over
/// `s[begin..end]`, its `NumberFormatException` wrapped as the `URL`
/// constructor wraps it.
fn parse_port(s: &[u16], begin: usize, end: usize) -> Result<i32> {
    let error = |at: usize| {
        malformed_caused_by(
            "java.lang.NumberFormatException",
            format!(
                "Error at index {} in: \"{}\"",
                at - begin,
                String::from_utf16_lossy(&s[begin..end])
            ),
        )
    };
    let mut i = begin;
    let mut negative = false;
    let mut limit = -i32::MAX;
    let first = s[i];
    if first < u16::from(b'0') {
        if first == u16::from(b'-') {
            negative = true;
            limit = i32::MIN;
        } else if first != u16::from(b'+') {
            return Err(error(i));
        }
        i += 1;
        if i == end {
            return Err(error(i));
        }
    }
    let multmin = limit / 10;
    let mut result: i32 = 0;
    while i < end {
        let digit = icu4j::jdk::character_digit(i32::from(s[i]), 10);
        if digit < 0 || result < multmin {
            return Err(error(i));
        }
        result *= 10;
        if result < limit + digit {
            return Err(error(i));
        }
        i += 1;
        result -= digit;
    }
    Ok(if negative { result } else { -result })
}

/// `URL.isValidProtocol`.
fn is_valid_protocol(protocol: &str) -> bool {
    let mut chars = protocol.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || c == '.' || c == '+' || c == '-')
}

/// The index of the first `c` in `s` at or after `from`.
fn index_of(s: &[u16], c: u8, from: usize) -> Option<usize> {
    s.iter()
        .skip(from)
        .position(|&u| u == u16::from(c))
        .map(|i| i + from)
}

impl JavaUrl {
    /// Equivalent to `new URL(URL context, String spec)` (`URL.java`, the
    /// `URL(URL, String, URLStreamHandler)` constructor) with the `http` and
    /// `https` handlers' `URLStreamHandler.parseURL`. For another known
    /// protocol only the protocol is set: the redirect logic compares it
    /// and never requests such a URL.
    ///
    /// # Errors
    ///
    /// `MalformedURLException` for no protocol, an unknown protocol, or a
    /// malformed port, IPv6 literal or authority.
    fn new(context: Option<&JavaUrl>, spec_str: &str) -> Result<JavaUrl> {
        let spec: Vec<u16> = spec_str.encode_utf16().collect();
        let sub = |a: usize, b: usize| String::from_utf16_lossy(&spec[a..b]);
        let mut limit = spec.len();
        let mut start = 0;
        while limit > 0 && spec[limit - 1] <= 0x20 {
            limit -= 1;
        }
        while start < limit && spec[start] <= 0x20 {
            start += 1;
        }
        if spec.len() >= start + 4 && sub(start, start + 4).eq_ignore_ascii_case("url:") {
            start += 4;
        }
        let a_ref = start < spec.len() && spec[start] == u16::from(b'#');
        let mut new_protocol: Option<String> = None;
        if !a_ref {
            let mut i = start;
            while i < limit && spec[i] != u16::from(b'/') {
                if spec[i] == u16::from(b':') {
                    let s = sub(start, i).to_lowercase();
                    if is_valid_protocol(&s) {
                        new_protocol = Some(s);
                        start = i + 1;
                    }
                    break;
                }
                i += 1;
            }
        }

        let mut url = JavaUrl {
            protocol: String::new(),
            authority: None,
            host: None,
            port: -1,
            path: None,
            query: None,
            reference: None,
        };
        let mut protocol = new_protocol.clone();
        let mut handler_inherited = false;
        let mut is_relative = false;
        if let Some(context) = context {
            if new_protocol
                .as_deref()
                .is_none_or(|p| p.eq_ignore_ascii_case(&context.protocol))
            {
                handler_inherited = true;
                if context.path.as_deref().is_some_and(|p| p.starts_with('/')) {
                    new_protocol = None;
                }
                if new_protocol.is_none() {
                    protocol = Some(context.protocol.clone());
                    url.authority = context.authority.clone();
                    url.host = context.host.clone();
                    url.port = context.port;
                    url.path = context.path.clone();
                    is_relative = true;
                }
            }
        }
        let Some(protocol) = protocol else {
            return Err(malformed(format!("no protocol: {spec_str}")));
        };
        if !handler_inherited && !KNOWN_PROTOCOLS.contains(&protocol.as_str()) {
            return Err(malformed(format!("unknown protocol: {protocol}")));
        }
        url.protocol = protocol;
        if url.protocol != "http" && url.protocol != "https" {
            return Ok(url);
        }

        if let Some(i) = index_of(&spec, b'#', start) {
            if i < limit {
                url.reference = Some(sub(i + 1, limit));
                limit = i;
            }
        }
        if is_relative && start == limit {
            if let Some(context) = context {
                url.query = context.query.clone();
                if url.reference.is_none() {
                    url.reference = context.reference.clone();
                }
            }
        }
        url.parse_url(&spec, start, limit)?;
        Ok(url)
    }

    /// Equivalent to `URLStreamHandler.parseURL(URL, String, int, int)`.
    fn parse_url(&mut self, spec_in: &[u16], mut start: usize, mut limit: usize) -> Result<()> {
        let mut spec: Vec<u16> = spec_in.to_vec();
        let mut is_rel_path = false;
        let mut query_only = false;

        if start < limit {
            let query_start = index_of(&spec, b'?', 0);
            query_only = query_start == Some(start);
            if let Some(qs) = query_start {
                if qs < limit {
                    self.query = Some(String::from_utf16_lossy(&spec[qs + 1..limit]));
                    limit = qs;
                    spec.truncate(qs);
                }
            }
        }

        let slash = u16::from(b'/');
        let is_unc_name = start + 4 <= limit
            && spec[start] == slash
            && spec[start + 1] == slash
            && spec[start + 2] == slash
            && spec[start + 3] == slash;
        if !is_unc_name && start + 2 <= limit && spec[start] == slash && spec[start + 1] == slash {
            start += 2;
            let mut i = match index_of(&spec, b'/', start) {
                Some(i) if i <= limit => i,
                _ => match index_of(&spec, b'?', start) {
                    Some(i) if i <= limit => i,
                    _ => limit,
                },
            };
            if i > spec.len() {
                i = spec.len();
            }
            let authority: Vec<u16> = spec[start..i].to_vec();
            let authority_string = String::from_utf16_lossy(&authority);
            self.authority = Some(authority_string.clone());
            let mut host: Option<Vec<u16>> = Some(authority.clone());
            let at = u16::from(b'@');
            if let Some(ind) = authority.iter().position(|&c| c == at) {
                if Some(ind) != authority.iter().rposition(|&c| c == at) {
                    host = None;
                } else {
                    host = Some(authority[ind + 1..].to_vec());
                }
            }
            match host {
                Some(h) => {
                    if !h.is_empty() && h[0] == u16::from(b'[') {
                        let close = h.iter().position(|&c| c == u16::from(b']'));
                        match close {
                            Some(ind) if ind > 2 => {
                                let literal = String::from_utf16_lossy(&h[1..ind]);
                                if !is_ipv6_literal_address(&literal) {
                                    return Err(malformed_caused_by(
                                        "java.lang.IllegalArgumentException",
                                        format!("Invalid host: {}", String::from_utf16_lossy(&h[..=ind])),
                                    ));
                                }
                                self.port = -1;
                                if h.len() > ind + 1 {
                                    if h[ind + 1] == u16::from(b':') {
                                        if h.len() > ind + 2 {
                                            self.port = parse_port(&h, ind + 2, h.len())?;
                                        }
                                    } else {
                                        return Err(malformed_caused_by(
                                            "java.lang.IllegalArgumentException",
                                            format!("Invalid authority field: {authority_string}"),
                                        ));
                                    }
                                }
                                self.host = Some(String::from_utf16_lossy(&h[..=ind]));
                            }
                            _ => {
                                return Err(malformed_caused_by(
                                    "java.lang.IllegalArgumentException",
                                    format!("Invalid authority field: {authority_string}"),
                                ));
                            }
                        }
                    } else {
                        self.port = -1;
                        let mut host_end = h.len();
                        if let Some(ind) = h.iter().position(|&c| c == u16::from(b':')) {
                            if h.len() > ind + 1 {
                                self.port = parse_port(&h, ind + 1, h.len())?;
                            }
                            host_end = ind;
                        }
                        self.host = Some(String::from_utf16_lossy(&h[..host_end]));
                    }
                }
                None => self.host = Some(String::new()),
            }
            if self.port < -1 {
                return Err(malformed_caused_by(
                    "java.lang.IllegalArgumentException",
                    format!("Invalid port number :{}", self.port),
                ));
            }
            start = i;
            if !authority.is_empty() {
                self.path = Some(String::new());
            }
        }

        if self.host.is_none() {
            self.host = Some(String::new());
        }

        if start < limit {
            let spec_str = String::from_utf16_lossy(&spec[start..limit]);
            if spec[start] == slash {
                self.path = Some(spec_str);
            } else if let Some(path) = self.path.as_deref().filter(|p| !p.is_empty()) {
                is_rel_path = true;
                self.path = Some(match path.rfind('/') {
                    None if self.authority.is_some() => format!("/{spec_str}"),
                    None => spec_str,
                    Some(ind) => format!("{}{spec_str}", &path[..=ind]),
                });
            } else {
                self.path = Some(if self.authority.is_some() {
                    format!("/{spec_str}")
                } else {
                    spec_str
                });
            }
        } else if query_only {
            if let Some(path) = self.path.as_deref() {
                self.path = Some(match path.rfind('/') {
                    None => "/".to_string(),
                    Some(ind) => path[..=ind].to_string(),
                });
            }
        }
        let mut path = self.path.take().unwrap_or_default();

        if is_rel_path {
            // Remove embedded /./
            while let Some(i) = path.find("/./") {
                path = format!("{}{}", &path[..i], &path[i + 2..]);
            }
            // Remove embedded /../ if possible
            let mut i = 0;
            while let Some(found) = path[i..].find("/../").map(|f| f + i) {
                i = found;
                let previous = if i > 0 { path[..i].rfind('/') } else { None };
                match previous {
                    Some(lim) if path[lim..].find("/../") != Some(0) => {
                        path = format!("{}{}", &path[..lim], &path[i + 3..]);
                        i = 0;
                    }
                    _ => i += 3,
                }
            }
            // Remove trailing .. if possible
            while path.ends_with("/..") {
                let i = path.find("/..").expect("INVARIANT: the path ends with /..");
                match if i > 0 { path[..i].rfind('/') } else { None } {
                    Some(lim) => path.truncate(lim + 1),
                    None => break,
                }
            }
            // Remove starting .
            if path.starts_with("./") && path.len() > 2 {
                path = path[2..].to_string();
            }
            // Remove trailing .
            if path.ends_with("/.") {
                path.truncate(path.len() - 1);
            }
        }
        self.path = Some(path);
        Ok(())
    }

    /// `URL.toExternalForm()` (`URLStreamHandler.toExternalForm`).
    fn to_external_form(&self) -> String {
        let mut s = format!("{}:", self.protocol);
        if let Some(authority) = self.authority.as_deref().filter(|a| !a.is_empty()) {
            s.push_str("//");
            s.push_str(authority);
        }
        s.push_str(self.path.as_deref().unwrap_or(""));
        if let Some(query) = &self.query {
            s.push('?');
            s.push_str(query);
        }
        if let Some(reference) = &self.reference {
            s.push('#');
            s.push_str(reference);
        }
        s
    }

    /// The URL `ureq` requests: `HttpClient` connects to the host and port
    /// (the protocol's default for -1) and sends the path and query
    /// (`getURLFile`, `/` for an empty path), never the user information or
    /// the fragment.
    fn request_url(&self) -> String {
        let mut s = format!("{}://{}", self.protocol, self.host.as_deref().unwrap_or(""));
        if self.port != -1 {
            s.push_str(&format!(":{}", self.port));
        }
        match self.path.as_deref() {
            Some(path) if !path.is_empty() => s.push_str(path),
            _ => s.push('/'),
        }
        if let Some(query) = &self.query {
            s.push('?');
            s.push_str(query);
        }
        s
    }

    /// `URLStreamHandler.getDefaultPort()` of the `http` (80) and `https`
    /// (443) handlers.
    fn default_port(&self) -> i32 {
        if self.protocol == "https" { 443 } else { 80 }
    }
}

/// `IPAddressUtil.isIPv6LiteralAddress`: an IPv6 address, optionally with a
/// `%` scope suffix.
fn is_ipv6_literal_address(s: &str) -> bool {
    let address = s.split_once('%').map_or(s, |(a, _)| a);
    address.parse::<std::net::Ipv6Addr>().is_ok()
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
