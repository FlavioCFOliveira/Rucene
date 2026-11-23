/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 * 
 *      http://www.apache.org/licenses/LICENSE-2.0
 * 
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `URI.java`, of the Java package `org.apache.xerces.util` (Xerces-J 2.12.2).

use crate::java::{from_chars, io_exception, to_chars, trim, Exception};

/// MalformedURIExceptions are thrown in the process of building a URI or
/// setting fields on a URI when an operation would result in an invalid
/// URI specification.
///
/// Equivalent to `org.apache.xerces.util.URI.MalformedURIException` (an
/// `IOException`; [`Exception::from`] gives it as one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MalformedURIException {
    message: Option<String>,
}

impl MalformedURIException {
    /// Equivalent to `new MalformedURIException()`.
    pub fn new() -> Self {
        Self { message: None }
    }

    /// Equivalent to `new MalformedURIException(String)`.
    pub fn with_message(p_msg: &str) -> Self {
        Self {
            message: Some(p_msg.to_string()),
        }
    }

    /// `getMessage()`.
    pub fn get_message(&self) -> Option<&str> {
        self.message.as_deref()
    }
}

impl Default for MalformedURIException {
    fn default() -> Self {
        Self::new()
    }
}

impl From<MalformedURIException> for Exception {
    fn from(e: MalformedURIException) -> Self {
        io_exception("org.apache.xerces.util.URI$MalformedURIException", e.message)
    }
}

type UResult<T> = std::result::Result<T, MalformedURIException>;

fn err<T>(msg: &str) -> UResult<T> {
    Err(MalformedURIException::with_message(msg))
}

const RESERVED_CHARACTERS: u8 = 0x01;
const MARK_CHARACTERS: u8 = 0x02;
const SCHEME_CHARACTERS: u8 = 0x04;
const USERINFO_CHARACTERS: u8 = 0x08;
const ASCII_ALPHA_CHARACTERS: u8 = 0x10;
const ASCII_DIGIT_CHARACTERS: u8 = 0x20;
const ASCII_HEX_CHARACTERS: u8 = 0x40;
const PATH_CHARACTERS: u8 = 0x80;
const MASK_ALPHA_NUMERIC: u8 = ASCII_ALPHA_CHARACTERS | ASCII_DIGIT_CHARACTERS;
const MASK_UNRESERVED_MASK: u8 = MASK_ALPHA_NUMERIC | MARK_CHARACTERS;
const MASK_URI_CHARACTER: u8 = MASK_UNRESERVED_MASK | RESERVED_CHARACTERS;
const MASK_SCHEME_CHARACTER: u8 = MASK_ALPHA_NUMERIC | SCHEME_CHARACTERS;
const MASK_USERINFO_CHARACTER: u8 = MASK_UNRESERVED_MASK | USERINFO_CHARACTERS;
const MASK_PATH_CHARACTER: u8 = MASK_UNRESERVED_MASK | PATH_CHARACTERS;

/// `fgLookupTable`: character classes for the first 128 characters.
const LOOKUP_TABLE: [u8; 128] = {
    let mut t = [0u8; 128];
    let mut i = b'0';
    while i <= b'9' {
        t[i as usize] |= ASCII_DIGIT_CHARACTERS | ASCII_HEX_CHARACTERS;
        i += 1;
    }
    let mut i = b'A';
    while i <= b'F' {
        t[i as usize] |= ASCII_ALPHA_CHARACTERS | ASCII_HEX_CHARACTERS;
        t[i as usize + 0x20] |= ASCII_ALPHA_CHARACTERS | ASCII_HEX_CHARACTERS;
        i += 1;
    }
    let mut i = b'G';
    while i <= b'Z' {
        t[i as usize] |= ASCII_ALPHA_CHARACTERS;
        t[i as usize + 0x20] |= ASCII_ALPHA_CHARACTERS;
        i += 1;
    }
    let reserved = b";/?:@&=+$,[]";
    let mut i = 0;
    while i < reserved.len() {
        t[reserved[i] as usize] |= RESERVED_CHARACTERS;
        i += 1;
    }
    let mark = b"-_.!~*'()";
    let mut i = 0;
    while i < mark.len() {
        t[mark[i] as usize] |= MARK_CHARACTERS;
        i += 1;
    }
    let scheme = b"+-.";
    let mut i = 0;
    while i < scheme.len() {
        t[scheme[i] as usize] |= SCHEME_CHARACTERS;
        i += 1;
    }
    let userinfo = b";:&=+$,";
    let mut i = 0;
    while i < userinfo.len() {
        t[userinfo[i] as usize] |= USERINFO_CHARACTERS;
        i += 1;
    }
    let path = b";/:@&=+$,";
    let mut i = 0;
    while i < path.len() {
        t[path[i] as usize] |= PATH_CHARACTERS;
        i += 1;
    }
    t
};

fn lookup(c: u16, limit: u8, mask: u8) -> bool {
    c <= limit as u16 && (LOOKUP_TABLE[c as usize] & mask) != 0
}

/// Determine whether a char is a digit.
fn is_digit(c: u16) -> bool {
    (b'0' as u16..=b'9' as u16).contains(&c)
}

/// Determine whether a character is a hexadecimal character.
fn is_hex(c: u16) -> bool {
    lookup(c, b'f', ASCII_HEX_CHARACTERS)
}

/// Determine whether a char is an alphabetic character: a-z or A-Z.
fn is_alpha(c: u16) -> bool {
    (b'a' as u16..=b'z' as u16).contains(&c) || (b'A' as u16..=b'Z' as u16).contains(&c)
}

/// Determine whether a char is an alphanumeric: 0-9, a-z or A-Z.
fn is_alphanum(c: u16) -> bool {
    lookup(c, b'z', MASK_ALPHA_NUMERIC)
}

/// Determine whether a character is a URI character as per RFC 2396.
fn is_uri_character(c: u16) -> bool {
    lookup(c, b'~', MASK_URI_CHARACTER)
}

/// Determine whether a character is a scheme character as per RFC 2396.
fn is_scheme_character(c: u16) -> bool {
    lookup(c, b'z', MASK_SCHEME_CHARACTER)
}

/// Determine whether a character is a userinfo character as per RFC 2396.
fn is_userinfo_character(c: u16) -> bool {
    lookup(c, b'z', MASK_USERINFO_CHARACTER)
}

/// Determine whether a character is a path character as per RFC 2396.
fn is_path_character(c: u16) -> bool {
    lookup(c, b'~', MASK_PATH_CHARACTER)
}

fn ch(c: u16) -> String {
    from_chars(&[c])
}

fn index_of(s: &[u16], c: char, from: usize) -> i32 {
    s.iter()
        .skip(from)
        .position(|&x| x == c as u16)
        .map_or(-1, |p| (p + from) as i32)
}

fn last_index_of(s: &[u16], c: char, from: i32) -> i32 {
    if from < 0 {
        return -1;
    }
    let start = (from as usize).min(s.len().saturating_sub(1));
    if s.is_empty() {
        return -1;
    }
    (0..=start).rev().find(|&i| s[i] == c as u16).map_or(-1, |i| i as i32)
}

fn index_of_str(s: &str, needle: &str, from: usize) -> i32 {
    let s16 = to_chars(s);
    let n16 = to_chars(needle);
    if n16.is_empty() {
        return from.min(s16.len()) as i32;
    }
    if s16.len() < n16.len() {
        return -1;
    }
    (from..=s16.len() - n16.len())
        .find(|&i| s16[i..i + n16.len()] == n16[..])
        .map_or(-1, |i| i as i32)
}

fn substring(s: &str, a: i32, b: i32) -> String {
    let s16 = to_chars(s);
    from_chars(&s16[a as usize..b as usize])
}

fn len(s: &str) -> i32 {
    s.encode_utf16().count() as i32
}

/// A class to represent a Uniform Resource Identifier (URI). This class is
/// designed to handle the parsing of URIs and provide access to the various
/// components (scheme, host, port, userinfo, path, query string and
/// fragment) that may constitute a URI.
///
/// Equivalent to `org.apache.xerces.util.URI`.
#[derive(Debug, Clone, Default)]
pub struct URI {
    scheme: Option<String>,
    userinfo: Option<String>,
    host: Option<String>,
    port: i32,
    reg_authority: Option<String>,
    path: Option<String>,
    query_string: Option<String>,
    fragment: Option<String>,
}

impl PartialEq for URI {
    /// Equivalent to `equals(Object)` (which ignores the registry based
    /// authority).
    fn eq(&self, t: &Self) -> bool {
        self.scheme == t.scheme
            && self.userinfo == t.userinfo
            && self.host == t.host
            && self.port == t.port
            && self.path == t.path
            && self.query_string == t.query_string
            && self.fragment == t.fragment
    }
}

impl URI {
    /// Construct a new and uninitialized URI.
    ///
    /// Equivalent to `new URI()`.
    pub fn new() -> Self {
        Self {
            port: -1,
            ..Self::default()
        }
    }

    /// Construct a new URI from another URI.
    ///
    /// Equivalent to `new URI(URI)`.
    pub fn from_uri(p_other: &URI) -> Self {
        let mut u = Self::new();
        u.initialize_from(p_other);
        u
    }

    /// Construct a new URI from a URI specification string.
    ///
    /// Equivalent to `new URI(String)`.
    ///
    /// # Errors
    ///
    /// `MalformedURIException` for a malformed specification.
    pub fn parse(p_uri_spec: Option<&str>) -> UResult<Self> {
        Self::with_base(None, p_uri_spec)
    }

    /// Equivalent to `new URI(String, boolean)`.
    ///
    /// # Errors
    ///
    /// `MalformedURIException` for a malformed specification.
    pub fn parse_allow_non_absolute(p_uri_spec: Option<&str>, allow_non_absolute_uri: bool) -> UResult<Self> {
        Self::with_base_allow_non_absolute(None, p_uri_spec, allow_non_absolute_uri)
    }

    /// Construct a new URI from a base URI and a URI specification string.
    ///
    /// Equivalent to `new URI(URI, String)`.
    ///
    /// # Errors
    ///
    /// `MalformedURIException` for a malformed specification.
    pub fn with_base(p_base: Option<&URI>, p_uri_spec: Option<&str>) -> UResult<Self> {
        let mut u = Self::new();
        u.initialize(p_base, p_uri_spec)?;
        Ok(u)
    }

    /// Equivalent to `new URI(URI, String, boolean)`.
    ///
    /// # Errors
    ///
    /// `MalformedURIException` for a malformed specification.
    pub fn with_base_allow_non_absolute(
        p_base: Option<&URI>,
        p_uri_spec: Option<&str>,
        allow_non_absolute_uri: bool,
    ) -> UResult<Self> {
        let mut u = Self::new();
        u.initialize_allow(p_base, p_uri_spec, allow_non_absolute_uri)?;
        Ok(u)
    }

    /// Construct a new URI that does not follow the generic URI syntax.
    ///
    /// Equivalent to `new URI(String, String)`.
    ///
    /// # Errors
    ///
    /// `MalformedURIException` for an empty part.
    pub fn with_scheme_specific_part(p_scheme: Option<&str>, p_scheme_specific_part: Option<&str>) -> UResult<Self> {
        if p_scheme.is_none_or(|s| trim(s).is_empty()) {
            return err("Cannot construct URI with null/empty scheme!");
        }
        if p_scheme_specific_part.is_none_or(|s| trim(s).is_empty()) {
            return err("Cannot construct URI with null/empty scheme-specific part!");
        }
        let mut u = Self::new();
        u.set_scheme(p_scheme)?;
        u.set_path(p_scheme_specific_part)?;
        Ok(u)
    }

    /// Construct a new URI that follows the generic URI syntax from its
    /// component parts.
    ///
    /// Equivalent to `new URI(String, String, String, String, String)`.
    ///
    /// # Errors
    ///
    /// `MalformedURIException` for invalid parts.
    pub fn with_host_parts(
        p_scheme: Option<&str>,
        p_host: Option<&str>,
        p_path: Option<&str>,
        p_query_string: Option<&str>,
        p_fragment: Option<&str>,
    ) -> UResult<Self> {
        Self::with_parts(p_scheme, None, p_host, -1, p_path, p_query_string, p_fragment)
    }

    /// Construct a new URI that follows the generic URI syntax from its
    /// component parts.
    ///
    /// Equivalent to `new URI(String, String, String, int, String, String,
    /// String)`.
    ///
    /// # Errors
    ///
    /// `MalformedURIException` for invalid parts.
    pub fn with_parts(
        p_scheme: Option<&str>,
        p_userinfo: Option<&str>,
        p_host: Option<&str>,
        p_port: i32,
        p_path: Option<&str>,
        p_query_string: Option<&str>,
        p_fragment: Option<&str>,
    ) -> UResult<Self> {
        if p_scheme.is_none_or(|s| trim(s).is_empty()) {
            return err("Scheme is required!");
        }
        if p_host.is_none() {
            if p_userinfo.is_some() {
                return err("Userinfo may not be specified if host is not specified!");
            }
            if p_port != -1 {
                return err("Port may not be specified if host is not specified!");
            }
        }
        if let Some(path) = p_path {
            if path.contains('?') && p_query_string.is_some() {
                return err("Query string cannot be specified in path and query string!");
            }
            if path.contains('#') && p_fragment.is_some() {
                return err("Fragment cannot be specified in both the path and fragment!");
            }
        }
        let mut u = Self::new();
        u.set_scheme(p_scheme)?;
        u.set_host(p_host)?;
        u.set_port(p_port)?;
        u.set_userinfo(p_userinfo)?;
        u.set_path(p_path)?;
        u.set_query_string(p_query_string)?;
        u.set_fragment(p_fragment)?;
        Ok(u)
    }

    /// Initialize all fields of this URI from another URI.
    fn initialize_from(&mut self, p_other: &URI) {
        self.scheme = p_other.get_scheme().map(str::to_string);
        self.userinfo = p_other.get_userinfo().map(str::to_string);
        self.host = p_other.get_host().map(str::to_string);
        self.port = p_other.get_port();
        self.reg_authority = p_other.get_reg_based_authority().map(str::to_string);
        self.path = p_other.get_path().map(str::to_string);
        self.query_string = p_other.get_query_string().map(str::to_string);
        self.fragment = p_other.get_fragment().map(str::to_string);
    }

    /// Initializes this URI from a base URI and a URI specification string.
    fn initialize_allow(&mut self, p_base: Option<&URI>, p_uri_spec: Option<&str>, allow_non_absolute_uri: bool) -> UResult<()> {
        self.initialize_common(p_base, p_uri_spec, Some(allow_non_absolute_uri))
    }

    /// Initializes this URI from a base URI and a URI specification string.
    fn initialize(&mut self, p_base: Option<&URI>, p_uri_spec: Option<&str>) -> UResult<()> {
        self.initialize_common(p_base, p_uri_spec, None)
    }

    /// The common body of the two `initialize(URI, String[, boolean])`
    /// overloads; `allow` is `None` for the two-argument one, which differs
    /// only by ignoring the flag.
    fn initialize_common(&mut self, p_base: Option<&URI>, p_uri_spec: Option<&str>, allow: Option<bool>) -> UResult<()> {
        let allow_non_absolute_uri = allow.unwrap_or(false);
        let uri_spec: Vec<u16> = p_uri_spec.map(to_chars).unwrap_or_default();
        let uri_spec_len = uri_spec.len() as i32;

        if p_base.is_none() && uri_spec_len == 0 {
            if allow_non_absolute_uri {
                self.path = Some(String::new());
                return Ok(());
            }
            return err("Cannot initialize URI with empty parameters.");
        }

        // just make a copy of the base if spec is empty
        if uri_spec_len == 0 {
            self.initialize_from(p_base.expect("INVARIANT: checked above"));
            return Ok(());
        }

        let mut index = 0;

        // Check for scheme, which must be before '/', '?' or '#'.
        let colon_idx = index_of(&uri_spec, ':', 0);
        if colon_idx != -1 {
            let search_from = colon_idx - 1;
            // search backwards starting from character before ':'.
            let slash_idx = last_index_of(&uri_spec, '/', search_from);
            let query_idx = last_index_of(&uri_spec, '?', search_from);
            let fragment_idx = last_index_of(&uri_spec, '#', search_from);

            if colon_idx == 0 || slash_idx != -1 || query_idx != -1 || fragment_idx != -1 {
                // A standalone base is a valid URI according to spec
                if colon_idx == 0 || (p_base.is_none() && fragment_idx != 0 && !allow_non_absolute_uri) {
                    return err("No scheme found in URI.");
                }
            } else {
                self.initialize_scheme(&uri_spec)?;
                index = len(self.scheme.as_deref().unwrap_or("")) + 1;

                // Neither 'scheme:' or 'scheme:#fragment' are valid URIs.
                if colon_idx == uri_spec_len - 1 || uri_spec[(colon_idx + 1) as usize] == b'#' as u16 {
                    return err("Scheme specific part cannot be empty.");
                }
            }
        } else if p_base.is_none() && index_of(&uri_spec, '#', 0) != 0 && !allow_non_absolute_uri {
            return err("No scheme found in URI.");
        }

        // Two slashes means we may have authority, but definitely means
        // we're either matching net_path or abs_path. These two productions
        // are ambiguous in that every net_path (except those containing an
        // IPv6Reference) is an abs_path. RFC 2396 resolves this ambiguity
        // by applying a greedy left most matching rule. Try matching net_path
        // first, and if that fails we don't have authority so then attempt
        // to match abs_path.
        //
        // net_path = "//" authority [ abs_path ]
        // abs_path = "/"  path_segments
        if (index + 1) < uri_spec_len
            && uri_spec[index as usize] == b'/' as u16
            && uri_spec[(index + 1) as usize] == b'/' as u16
        {
            index += 2;
            let start_pos = index;

            // Authority will be everything up to path, query or fragment
            while index < uri_spec_len {
                let test_char = uri_spec[index as usize];
                if test_char == b'/' as u16 || test_char == b'?' as u16 || test_char == b'#' as u16 {
                    break;
                }
                index += 1;
            }

            // Attempt to parse authority. If the section is an empty string
            // this is a valid server based authority, so set the host to
            // this value.
            if index > start_pos {
                // If we didn't find authority we need to back up. Attempt to
                // match against abs_path next.
                if !self.initialize_authority(&uri_spec[start_pos as usize..index as usize]) {
                    index = start_pos - 2;
                }
            } else {
                self.host = Some(String::new());
            }
        }

        self.initialize_path(Some(&uri_spec), index)?;

        // Resolve relative URI to base URI - see RFC 2396 Section 5.2
        // In some cases, it might make more sense to throw an exception
        // (when scheme is specified is the string spec and the base URI
        // is also specified, for example), but we're just following the
        // RFC specifications
        if let Some(base) = p_base {
            self.absolutize(base);
        }
        Ok(())
    }

    /// Absolutize URI with given base URI.
    ///
    /// Equivalent to `absolutize(URI)`.
    pub fn absolutize(&mut self, p_base: &URI) {
        // check to see if this is the current doc - RFC 2396 5.2 #2
        // note that this is slightly different from the RFC spec in that
        // we don't include the check for query string being null
        // - this handles cases where the urispec is just a query
        // string or a fragment (e.g. "?y" or "#s") -
        // see <http://www.ics.uci.edu/~fielding/url/test1.html> which
        // identified this as a bug in the RFC
        let m_path = self.path.clone().unwrap_or_default();
        if m_path.is_empty() && self.scheme.is_none() && self.host.is_none() && self.reg_authority.is_none() {
            self.scheme = p_base.get_scheme().map(str::to_string);
            self.userinfo = p_base.get_userinfo().map(str::to_string);
            self.host = p_base.get_host().map(str::to_string);
            self.port = p_base.get_port();
            self.reg_authority = p_base.get_reg_based_authority().map(str::to_string);
            self.path = p_base.get_path().map(str::to_string);

            if self.query_string.is_none() {
                self.query_string = p_base.get_query_string().map(str::to_string);

                if self.fragment.is_none() {
                    self.fragment = p_base.get_fragment().map(str::to_string);
                }
            }
            return;
        }

        // check for scheme - RFC 2396 5.2 #3
        // if we found a scheme, it means absolute URI, so we're done
        if self.scheme.is_none() {
            self.scheme = p_base.get_scheme().map(str::to_string);
        } else {
            return;
        }

        // check for authority - RFC 2396 5.2 #4
        // if we found a host, then we've got a network path, so we're done
        if self.host.is_none() && self.reg_authority.is_none() {
            self.userinfo = p_base.get_userinfo().map(str::to_string);
            self.host = p_base.get_host().map(str::to_string);
            self.port = p_base.get_port();
            self.reg_authority = p_base.get_reg_based_authority().map(str::to_string);
        } else {
            return;
        }

        // check for absolute path - RFC 2396 5.2 #5
        if !m_path.is_empty() && m_path.starts_with('/') {
            return;
        }

        // if we get to this point, we need to resolve relative path
        // RFC 2396 5.2 #6
        let mut path = String::new();
        let base_path = p_base.get_path();

        // 6a - get all but the last segment of the base URI path
        match base_path {
            Some(base_path) if !base_path.is_empty() => {
                let bp = to_chars(base_path);
                let last_slash = last_index_of(&bp, '/', bp.len() as i32);
                if last_slash != -1 {
                    path = substring(base_path, 0, last_slash + 1);
                }
            }
            _ => {
                if !m_path.is_empty() {
                    path = "/".to_string();
                }
            }
        }

        // 6b - append the relative URI path
        path.push_str(&m_path);

        // 6c - remove all "./" where "." is a complete path segment
        loop {
            let index = index_of_str(&path, "/./", 0);
            if index == -1 {
                break;
            }
            path = substring(&path, 0, index + 1) + &substring(&path, index + 3, len(&path));
        }

        // 6d - remove "." if path ends with "." as a complete path segment
        if path.ends_with("/.") {
            path = substring(&path, 0, len(&path) - 1);
        }

        // 6e - remove all "<segment>/../" where "<segment>" is a complete
        // path segment not equal to ".."
        let mut index = 1;
        loop {
            index = index_of_str(&path, "/../", index as usize);
            if index <= 0 {
                break;
            }
            let temp_string = substring(&path, 0, index_of_str(&path, "/../", 0));
            let ts = to_chars(&temp_string);
            let seg_index = last_index_of(&ts, '/', ts.len() as i32);
            if seg_index != -1 {
                if substring(&temp_string, seg_index, len(&temp_string)) != ".." {
                    path = substring(&path, 0, seg_index + 1) + &substring(&path, index + 4, len(&path));
                    index = seg_index;
                } else {
                    index += 4;
                }
            } else {
                index += 4;
            }
        }

        // 6f - remove ending "<segment>/.." where "<segment>" is a
        // complete path segment
        if path.ends_with("/..") {
            let temp_string = substring(&path, 0, len(&path) - 3);
            let ts = to_chars(&temp_string);
            let seg_index = last_index_of(&ts, '/', ts.len() as i32);
            if seg_index != -1 {
                path = substring(&path, 0, seg_index + 1);
            }
        }
        self.path = Some(path);
    }

    /// Initialize the scheme for this URI from a URI string spec.
    fn initialize_scheme(&mut self, p_uri_spec: &[u16]) -> UResult<()> {
        let uri_spec_len = p_uri_spec.len();
        let mut index = 0;
        while index < uri_spec_len {
            let test_char = p_uri_spec[index];
            if test_char == b':' as u16 || test_char == b'/' as u16 || test_char == b'?' as u16 || test_char == b'#' as u16 {
                break;
            }
            index += 1;
        }
        let scheme = from_chars(&p_uri_spec[..index]);
        if scheme.is_empty() {
            return err("No scheme found in URI.");
        }
        self.set_scheme(Some(&scheme))
    }

    /// Initialize the authority (either server or registry based) for this
    /// URI from a URI string spec.
    fn initialize_authority(&mut self, p_uri_spec: &[u16]) -> bool {
        let mut index = 0usize;
        let mut start = 0usize;
        let end = p_uri_spec.len();
        let mut userinfo = None;

        // userinfo is everything up to @
        if index_of(p_uri_spec, '@', start) != -1 {
            while index < end {
                if p_uri_spec[index] == b'@' as u16 {
                    break;
                }
                index += 1;
            }
            userinfo = Some(from_chars(&p_uri_spec[start..index]));
            index += 1;
        }

        // host is everything up to last ':', or up to
        // and including ']' if followed by ':'.
        start = index;
        let mut has_port = false;
        if index < end {
            if p_uri_spec[start] == b'[' as u16 {
                let bracket_index = index_of(p_uri_spec, ']', start);
                index = if bracket_index != -1 { bracket_index as usize } else { end };
                if index + 1 < end && p_uri_spec[index + 1] == b':' as u16 {
                    index += 1;
                    has_port = true;
                } else {
                    index = end;
                }
            } else {
                let colon_index = last_index_of(p_uri_spec, ':', end as i32);
                index = if colon_index > start as i32 { colon_index as usize } else { end };
                has_port = index != end;
            }
        }
        let host = from_chars(&p_uri_spec[start..index]);
        let mut port = -1;
        if !host.is_empty() && has_port {
            index += 1;
            start = index;
            while index < end {
                index += 1;
            }
            let port_str = from_chars(&p_uri_spec[start..index]);
            if !port_str.is_empty() {
                // REVISIT: Remove this code.
                // Store port value as string instead of integer.
                port = match crate::java::parse_int(&port_str) {
                    Some(p) => {
                        if p == -1 {
                            -2
                        } else {
                            p
                        }
                    }
                    None => -2,
                };
            }
        }

        if Self::is_valid_server_based_authority(&host, port, userinfo.as_deref()) {
            self.host = Some(host);
            self.port = port;
            self.userinfo = userinfo;
            return true;
        }
        // Note: Registry based authority is being removed from a
        // new spec for URI which would obsolete RFC 2396. If the
        // spec is added to XML errata, processing of reg_name
        // needs to be removed. - mrglavas.
        else if Self::is_valid_registry_based_authority(p_uri_spec) {
            self.reg_authority = Some(from_chars(p_uri_spec));
            return true;
        }
        false
    }

    /// Determines whether the components host, port, and user info are
    /// valid as a server authority.
    fn is_valid_server_based_authority(host: &str, port: i32, userinfo: Option<&str>) -> bool {
        // Check if the host is well formed.
        if !Self::is_well_formed_address(Some(host)) {
            return false;
        }

        // Check that port is well formed if it exists.
        // REVISIT: There's no restriction on port value ranges, but
        // perform the same check as in setPort to be consistent. Pass
        // in a string to this method instead of an integer.
        if !(-1..=65535).contains(&port) {
            return false;
        }

        // Check that userinfo is well formed if it exists.
        if let Some(userinfo) = userinfo {
            let u = to_chars(userinfo);
            let end = u.len();
            let mut index = 0;
            while index < end {
                let test_char = u[index];
                if test_char == b'%' as u16 {
                    if index + 2 >= end || !is_hex(u[index + 1]) || !is_hex(u[index + 2]) {
                        return false;
                    }
                    index += 2;
                } else if !is_userinfo_character(test_char) {
                    return false;
                }
                index += 1;
            }
        }
        true
    }

    /// Determines whether the given string is a registry based authority.
    fn is_valid_registry_based_authority(authority: &[u16]) -> bool {
        let end = authority.len();
        let mut index = 0;
        while index < end {
            let test_char = authority[index];
            if test_char == b'%' as u16 {
                if index + 2 >= end || !is_hex(authority[index + 1]) || !is_hex(authority[index + 2]) {
                    return false;
                }
                index += 2;
            }
            // can check against path characters because the set
            // is the same except for '/' which we've already excluded.
            else if !is_path_character(test_char) {
                return false;
            }
            index += 1;
        }
        true
    }

    /// Initialize the path for this URI from a URI string spec.
    fn initialize_path(&mut self, p_uri_spec: Option<&[u16]>, p_n_start_index: i32) -> UResult<()> {
        let Some(p_uri_spec) = p_uri_spec else {
            return err("Cannot initialize path from null string!");
        };

        let mut index = p_n_start_index as usize;
        let mut start = p_n_start_index as usize;
        let end = p_uri_spec.len();
        let mut test_char: u16 = 0;

        // path - everything up to query string or fragment
        if start < end {
            // RFC 2732 only allows '[' and ']' to appear in the opaque part.
            if self.get_scheme().is_none() || p_uri_spec[start] == b'/' as u16 {
                // Scan path.
                // abs_path = "/"  path_segments
                // rel_path = rel_segment [ abs_path ]
                while index < end {
                    test_char = p_uri_spec[index];

                    // check for valid escape sequence
                    if test_char == b'%' as u16 {
                        if index + 2 >= end || !is_hex(p_uri_spec[index + 1]) || !is_hex(p_uri_spec[index + 2]) {
                            return err("Path contains invalid escape sequence!");
                        }
                        index += 2;
                    }
                    // Path segments cannot contain '[' or ']' since pchar
                    // production was not changed by RFC 2732.
                    else if !is_path_character(test_char) {
                        if test_char == b'?' as u16 || test_char == b'#' as u16 {
                            break;
                        }
                        return err(&format!("Path contains invalid character: {}", ch(test_char)));
                    }
                    index += 1;
                }
            } else {
                // Scan opaque part.
                // opaque_part = uric_no_slash *uric
                while index < end {
                    test_char = p_uri_spec[index];

                    if test_char == b'?' as u16 || test_char == b'#' as u16 {
                        break;
                    }

                    // check for valid escape sequence
                    if test_char == b'%' as u16 {
                        if index + 2 >= end || !is_hex(p_uri_spec[index + 1]) || !is_hex(p_uri_spec[index + 2]) {
                            return err("Opaque part contains invalid escape sequence!");
                        }
                        index += 2;
                    }
                    // If the scheme specific part is opaque, it can contain
                    // '[' and ']'. uric_no_slash wasn't changed by RFC 2732,
                    // so we allow them anyway.
                    else if !is_uri_character(test_char) {
                        return err(&format!("Opaque part contains invalid character: {}", ch(test_char)));
                    }
                    index += 1;
                }
            }
        }
        self.path = Some(from_chars(&p_uri_spec[start..index]));

        // query - starts with ? and up to fragment or end
        if test_char == b'?' as u16 {
            index += 1;
            start = index;
            while index < end {
                test_char = p_uri_spec[index];
                if test_char == b'#' as u16 {
                    break;
                }
                if test_char == b'%' as u16 {
                    if index + 2 >= end || !is_hex(p_uri_spec[index + 1]) || !is_hex(p_uri_spec[index + 2]) {
                        return err("Query string contains invalid escape sequence!");
                    }
                    index += 2;
                } else if !is_uri_character(test_char) {
                    return err(&format!("Query string contains invalid character: {}", ch(test_char)));
                }
                index += 1;
            }
            self.query_string = Some(from_chars(&p_uri_spec[start..index]));
        }

        // fragment - starts with #
        if test_char == b'#' as u16 {
            index += 1;
            start = index;
            while index < end {
                test_char = p_uri_spec[index];

                if test_char == b'%' as u16 {
                    if index + 2 >= end || !is_hex(p_uri_spec[index + 1]) || !is_hex(p_uri_spec[index + 2]) {
                        return err("Fragment contains invalid escape sequence!");
                    }
                    index += 2;
                } else if !is_uri_character(test_char) {
                    return err(&format!("Fragment contains invalid character: {}", ch(test_char)));
                }
                index += 1;
            }
            self.fragment = Some(from_chars(&p_uri_spec[start..index]));
        }
        Ok(())
    }

    /// Get the scheme for this URI.
    ///
    /// Equivalent to `getScheme()`.
    pub fn get_scheme(&self) -> Option<&str> {
        self.scheme.as_deref()
    }

    /// Get the scheme-specific part for this URI (everything following the
    /// scheme and the first colon).
    ///
    /// Equivalent to `getSchemeSpecificPart()`.
    pub fn get_scheme_specific_part(&self) -> String {
        let mut schemespec = self.get_authority();
        if let Some(path) = &self.path {
            schemespec.push_str(path);
        }
        if let Some(q) = &self.query_string {
            schemespec.push('?');
            schemespec.push_str(q);
        }
        if let Some(f) = &self.fragment {
            schemespec.push('#');
            schemespec.push_str(f);
        }
        schemespec
    }

    /// Get the userinfo for this URI.
    ///
    /// Equivalent to `getUserinfo()`.
    pub fn get_userinfo(&self) -> Option<&str> {
        self.userinfo.as_deref()
    }

    /// Get the host for this URI.
    ///
    /// Equivalent to `getHost()`.
    pub fn get_host(&self) -> Option<&str> {
        self.host.as_deref()
    }

    /// Get the port for this URI.
    ///
    /// Equivalent to `getPort()`.
    pub fn get_port(&self) -> i32 {
        self.port
    }

    /// Get the registry based authority for this URI.
    ///
    /// Equivalent to `getRegBasedAuthority()`.
    pub fn get_reg_based_authority(&self) -> Option<&str> {
        self.reg_authority.as_deref()
    }

    /// Get the authority for this URI.
    ///
    /// Equivalent to `getAuthority()`.
    pub fn get_authority(&self) -> String {
        let mut authority = String::new();
        if self.host.is_some() || self.reg_authority.is_some() {
            authority.push_str("//");
            // Server based authority.
            if let Some(host) = &self.host {
                if let Some(userinfo) = &self.userinfo {
                    authority.push_str(userinfo);
                    authority.push('@');
                }
                authority.push_str(host);
                if self.port != -1 {
                    authority.push(':');
                    authority.push_str(&self.port.to_string());
                }
            }
            // Registry based authority.
            else if let Some(reg) = &self.reg_authority {
                authority.push_str(reg);
            }
        }
        authority
    }

    /// Get the path for this URI (optionally with the query string and
    /// fragment).
    ///
    /// Equivalent to `getPath(boolean, boolean)`.
    pub fn get_path_with(&self, p_include_query_string: bool, p_include_fragment: bool) -> String {
        let mut path_string = self.path.clone().expect("java.lang.NullPointerException: m_path");
        if p_include_query_string {
            if let Some(q) = &self.query_string {
                path_string.push('?');
                path_string.push_str(q);
            }
        }
        if p_include_fragment {
            if let Some(f) = &self.fragment {
                path_string.push('#');
                path_string.push_str(f);
            }
        }
        path_string
    }

    /// Get the path for this URI.
    ///
    /// Equivalent to `getPath()`.
    pub fn get_path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// Get the query string for this URI.
    ///
    /// Equivalent to `getQueryString()`.
    pub fn get_query_string(&self) -> Option<&str> {
        self.query_string.as_deref()
    }

    /// Get the fragment for this URI.
    ///
    /// Equivalent to `getFragment()`.
    pub fn get_fragment(&self) -> Option<&str> {
        self.fragment.as_deref()
    }

    /// Set the scheme for this URI. The scheme is converted to lowercase
    /// before it is set.
    ///
    /// Equivalent to `setScheme(String)`.
    ///
    /// # Errors
    ///
    /// `MalformedURIException` for a null or non-conformant scheme.
    pub fn set_scheme(&mut self, p_scheme: Option<&str>) -> UResult<()> {
        let Some(p_scheme) = p_scheme else {
            return err("Cannot set scheme from null string!");
        };
        if !Self::is_conformant_scheme_name(Some(p_scheme)) {
            return err("The scheme is not conformant.");
        }
        self.scheme = Some(p_scheme.to_lowercase());
        Ok(())
    }

    /// Set the userinfo for this URI. If a non-null value is passed in and
    /// the host value is null, then an exception is thrown.
    ///
    /// Equivalent to `setUserinfo(String)`.
    ///
    /// # Errors
    ///
    /// `MalformedURIException` for an invalid userinfo.
    pub fn set_userinfo(&mut self, p_userinfo: Option<&str>) -> UResult<()> {
        let Some(p_userinfo) = p_userinfo else {
            self.userinfo = None;
            return Ok(());
        };
        if self.host.is_none() {
            return err("Userinfo cannot be set when host is null!");
        }

        // userinfo can contain alphanumerics, mark characters, escaped
        // and ';',':','&','=','+','$',','
        let u = to_chars(p_userinfo);
        let end = u.len();
        let mut index = 0;
        while index < end {
            let test_char = u[index];
            if test_char == b'%' as u16 {
                if index + 2 >= end || !is_hex(u[index + 1]) || !is_hex(u[index + 2]) {
                    return err("Userinfo contains invalid escape sequence!");
                }
            } else if !is_userinfo_character(test_char) {
                return err(&format!("Userinfo contains invalid character:{}", ch(test_char)));
            }
            index += 1;
        }
        self.userinfo = Some(p_userinfo.to_string());
        Ok(())
    }

    /// Set the host for this URI.
    ///
    /// Equivalent to `setHost(String)`.
    ///
    /// # Errors
    ///
    /// `MalformedURIException` for a host that is not a well formed address.
    pub fn set_host(&mut self, p_host: Option<&str>) -> UResult<()> {
        match p_host {
            None | Some("") => {
                if p_host.is_some() {
                    self.reg_authority = None;
                }
                self.host = p_host.map(str::to_string);
                self.userinfo = None;
                self.port = -1;
                Ok(())
            }
            Some(h) => {
                if !Self::is_well_formed_address(Some(h)) {
                    return err("Host is not a well formed address!");
                }
                self.host = Some(h.to_string());
                self.reg_authority = None;
                Ok(())
            }
        }
    }

    /// Set the port for this URI. -1 is used to indicate that the port is
    /// not specified, otherwise valid port numbers are between 0 and 65535.
    ///
    /// Equivalent to `setPort(int)`.
    ///
    /// # Errors
    ///
    /// `MalformedURIException` for an invalid port.
    pub fn set_port(&mut self, p_port: i32) -> UResult<()> {
        if (0..=65535).contains(&p_port) {
            if self.host.is_none() {
                return err("Port cannot be set when host is null!");
            }
        } else if p_port != -1 {
            return err("Invalid port number!");
        }
        self.port = p_port;
        Ok(())
    }

    /// Sets the registry based authority for this URI.
    ///
    /// Equivalent to `setRegBasedAuthority(String)`.
    ///
    /// # Errors
    ///
    /// `MalformedURIException` for an authority that is not well formed.
    pub fn set_reg_based_authority(&mut self, authority: Option<&str>) -> UResult<()> {
        let Some(authority) = authority else {
            self.reg_authority = None;
            return Ok(());
        };
        // reg_name = 1*( unreserved | escaped | "$" | "," |
        //            ";" | ":" | "@" | "&" | "=" | "+" )
        if authority.is_empty()
            || !Self::is_valid_registry_based_authority(&to_chars(authority))
            || authority.contains('/')
        {
            return err("Registry based authority is not well formed.");
        }
        self.reg_authority = Some(authority.to_string());
        self.host = None;
        self.userinfo = None;
        self.port = -1;
        Ok(())
    }

    /// Set the path for this URI. If the supplied path is null, then the
    /// query string and fragment are set to null as well.
    ///
    /// Equivalent to `setPath(String)`.
    ///
    /// # Errors
    ///
    /// `MalformedURIException` for an invalid path.
    pub fn set_path(&mut self, p_path: Option<&str>) -> UResult<()> {
        match p_path {
            None => {
                self.path = None;
                self.query_string = None;
                self.fragment = None;
                Ok(())
            }
            Some(p) => self.initialize_path(Some(&to_chars(p)), 0),
        }
    }

    /// Append to the end of the path of this URI.
    ///
    /// Equivalent to `appendPath(String)`.
    ///
    /// # Errors
    ///
    /// `MalformedURIException` for a path with invalid characters.
    pub fn append_path(&mut self, p_add_to_path: Option<&str>) -> UResult<()> {
        let Some(p_add_to_path) = p_add_to_path else {
            return Ok(());
        };
        if trim(p_add_to_path).is_empty() {
            return Ok(());
        }
        if !Self::is_uri_string(Some(p_add_to_path)) {
            return err("Path contains invalid character!");
        }
        match &self.path {
            None => {
                self.path = Some(if p_add_to_path.starts_with('/') {
                    p_add_to_path.to_string()
                } else {
                    format!("/{p_add_to_path}")
                });
            }
            Some(p) if trim(p).is_empty() => {
                self.path = Some(if p_add_to_path.starts_with('/') {
                    p_add_to_path.to_string()
                } else {
                    format!("/{p_add_to_path}")
                });
            }
            Some(p) if p.ends_with('/') => {
                let mut p = p.clone();
                if let Some(rest) = p_add_to_path.strip_prefix('/') {
                    p.push_str(rest);
                } else {
                    p.push_str(p_add_to_path);
                }
                self.path = Some(p);
            }
            Some(p) => {
                let mut p = p.clone();
                if p_add_to_path.starts_with('/') {
                    p.push_str(p_add_to_path);
                } else {
                    p.push('/');
                    p.push_str(p_add_to_path);
                }
                self.path = Some(p);
            }
        }
        Ok(())
    }

    /// Set the query string for this URI. A non-null value is valid only if
    /// this is an URI conforming to the generic URI syntax and the path
    /// value is not null.
    ///
    /// Equivalent to `setQueryString(String)`.
    ///
    /// # Errors
    ///
    /// `MalformedURIException` for an invalid query string.
    pub fn set_query_string(&mut self, p_query_string: Option<&str>) -> UResult<()> {
        match p_query_string {
            None => {
                self.query_string = None;
                Ok(())
            }
            Some(q) => {
                if !self.is_generic_uri() {
                    err("Query string can only be set for a generic URI!")
                } else if self.get_path().is_none() {
                    err("Query string cannot be set when path is null!")
                } else if !Self::is_uri_string(Some(q)) {
                    err("Query string contains invalid character!")
                } else {
                    self.query_string = Some(q.to_string());
                    Ok(())
                }
            }
        }
    }

    /// Set the fragment for this URI. A non-null value is valid only if this
    /// is a URI conforming to the generic URI syntax and the path value is
    /// not null.
    ///
    /// Equivalent to `setFragment(String)`.
    ///
    /// # Errors
    ///
    /// `MalformedURIException` for an invalid fragment.
    pub fn set_fragment(&mut self, p_fragment: Option<&str>) -> UResult<()> {
        match p_fragment {
            None => {
                self.fragment = None;
                Ok(())
            }
            Some(f) => {
                if !self.is_generic_uri() {
                    err("Fragment can only be set for a generic URI!")
                } else if self.get_path().is_none() {
                    err("Fragment cannot be set when path is null!")
                } else if !Self::is_uri_string(Some(f)) {
                    err("Fragment contains invalid character!")
                } else {
                    self.fragment = Some(f.to_string());
                    Ok(())
                }
            }
        }
    }

    /// Get the URI as a string specification.
    ///
    /// Equivalent to `toString()`.
    pub fn to_java_string(&self) -> String {
        let mut uri_spec_string = String::new();
        if let Some(scheme) = &self.scheme {
            uri_spec_string.push_str(scheme);
            uri_spec_string.push(':');
        }
        uri_spec_string.push_str(&self.get_scheme_specific_part());
        uri_spec_string
    }

    /// Get the indicator as to whether this URI uses the "generic URI"
    /// syntax.
    ///
    /// Equivalent to `isGenericURI()`.
    pub fn is_generic_uri(&self) -> bool {
        // presence of the host (whether valid or empty) means
        // double-slashes which means generic uri
        self.host.is_some()
    }

    /// Returns whether this URI represents an absolute URI.
    ///
    /// Equivalent to `isAbsoluteURI()`.
    pub fn is_absolute_uri(&self) -> bool {
        // presence of the scheme means absolute uri
        self.scheme.is_some()
    }

    /// Determine whether a scheme conforms to the rules for a scheme name.
    ///
    /// Equivalent to `isConformantSchemeName(String)`.
    pub fn is_conformant_scheme_name(p_scheme: Option<&str>) -> bool {
        let Some(p_scheme) = p_scheme else {
            return false;
        };
        if trim(p_scheme).is_empty() {
            return false;
        }
        let s = to_chars(p_scheme);
        if !is_alpha(s[0]) {
            return false;
        }
        s[1..].iter().all(|&c| is_scheme_character(c))
    }

    /// Determine whether a string is syntactically capable of representing
    /// a valid IPv4 address, IPv6 reference or the domain name of a network
    /// host.
    ///
    /// Equivalent to `isWellFormedAddress(String)`.
    pub fn is_well_formed_address(address: Option<&str>) -> bool {
        let Some(address) = address else {
            return false;
        };
        let a = to_chars(address);
        let addr_length = a.len();
        if addr_length == 0 {
            return false;
        }

        // Check if the host is a valid IPv6reference.
        if address.starts_with('[') {
            return Self::is_well_formed_ipv6_reference(address);
        }

        // Cannot start with a '.', '-', or end with a '-'.
        if address.starts_with('.') || address.starts_with('-') || address.ends_with('-') {
            return false;
        }

        // rightmost domain label starting with digit indicates IP address
        // since top level domain label can only start with an alpha
        // see RFC 2396 Section 3.2.2
        let mut index = last_index_of(&a, '.', a.len() as i32);
        if address.ends_with('.') {
            let head = &a[..index as usize];
            index = last_index_of(head, '.', head.len() as i32);
        }

        if (index + 1) < addr_length as i32 && is_digit(a[(index + 1) as usize]) {
            return Self::is_well_formed_ipv4_address(address);
        } else {
            // hostname      = *( domainlabel "." ) toplabel [ "." ]
            // domainlabel   = alphanum | alphanum *( alphanum | "-" ) alphanum
            // toplabel      = alpha | alpha *( alphanum | "-" ) alphanum

            // RFC 2396 states that hostnames take the form described in
            // RFC 1034 (Section 3) and RFC 1123 (Section 2.1). According
            // to RFC 1034, hostnames are limited to 255 characters.
            if addr_length > 255 {
                return false;
            }

            // domain labels can contain alphanumerics and '-"
            // but must start and end with an alphanumeric
            let mut label_char_count = 0;
            for i in 0..addr_length {
                let test_char = a[i];
                if test_char == b'.' as u16 {
                    if !is_alphanum(a[i - 1]) {
                        return false;
                    }
                    if i + 1 < addr_length && !is_alphanum(a[i + 1]) {
                        return false;
                    }
                    label_char_count = 0;
                } else if !is_alphanum(test_char) && test_char != b'-' as u16 {
                    return false;
                }
                // RFC 1034: Labels must be 63 characters or less.
                else {
                    label_char_count += 1;
                    if label_char_count > 63 {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Determines whether a string is an IPv4 address as defined by RFC
    /// 2373, and under the further constraint that it must be a 32-bit
    /// address.
    ///
    /// Equivalent to `isWellFormedIPv4Address(String)`.
    pub fn is_well_formed_ipv4_address(address: &str) -> bool {
        let a = to_chars(address);
        let addr_length = a.len();
        let mut num_dots = 0;
        let mut num_digits = 0;

        // make sure that 1) we see only digits and dot separators, 2) that
        // any dot separator is preceded and followed by a digit and
        // 3) that we find 3 dots
        //
        // RFC 2732 amended RFC 2396 by replacing the definition
        // of IPv4address with the one defined by RFC 2373. - mrglavas
        //
        // IPv4address = 1*3DIGIT "." 1*3DIGIT "." 1*3DIGIT "." 1*3DIGIT
        //
        // One to three digits must be in each segment.
        for i in 0..addr_length {
            let test_char = a[i];
            if test_char == b'.' as u16 {
                if (i > 0 && !is_digit(a[i - 1])) || (i + 1 < addr_length && !is_digit(a[i + 1])) {
                    return false;
                }
                num_digits = 0;
                num_dots += 1;
                if num_dots > 3 {
                    return false;
                }
            } else if !is_digit(test_char) {
                return false;
            }
            // Check that that there are no more than three digits
            // in this segment.
            else {
                num_digits += 1;
                if num_digits > 3 {
                    return false;
                }
                // Check that this segment is not greater than 255.
                else if num_digits == 3 {
                    let first = a[i - 2];
                    let second = a[i - 1];
                    let (c2, c5) = (b'2' as u16, b'5' as u16);
                    if !(first < c2 || (first == c2 && (second < c5 || (second == c5 && test_char <= c5)))) {
                        return false;
                    }
                }
            }
        }
        num_dots == 3
    }

    /// Determines whether a string is an IPv6 reference as defined by RFC
    /// 2732, where IPv6address is defined in RFC 2373.
    ///
    /// Equivalent to `isWellFormedIPv6Reference(String)`.
    pub fn is_well_formed_ipv6_reference(address: &str) -> bool {
        let a = to_chars(address);
        let addr_length = a.len();
        let mut index = 1usize;
        let end = addr_length.wrapping_sub(1);

        // Check if string is a potential match for IPv6reference.
        if !(addr_length > 2 && a[0] == b'[' as u16 && a[end] == b']' as u16) {
            return false;
        }

        // Counter for the number of 16-bit sections read in the address.
        let mut counter = [0i32; 1];

        // Scan hex sequence before possible '::' or IPv4 address.
        let scanned = Self::scan_hex_sequence(&a, index, end, &mut counter);
        if scanned == -1 {
            return false;
        }
        index = scanned as usize;
        // Address must contain 128-bits of information.
        if index == end {
            return counter[0] == 8;
        }

        if index + 1 < end && a[index] == b':' as u16 {
            if a[index + 1] == b':' as u16 {
                // '::' represents at least one 16-bit group of zeros.
                counter[0] += 1;
                if counter[0] > 8 {
                    return false;
                }
                index += 2;
                // Trailing zeros will fill out the rest of the address.
                if index == end {
                    return true;
                }
            }
            // If the second character wasn't ':', in order to be valid,
            // the remainder of the string must match IPv4Address,
            // and we must have read exactly 6 16-bit groups.
            else {
                return counter[0] == 6 && Self::is_well_formed_ipv4_address(&from_chars(&a[index + 1..end]));
            }
        } else {
            return false;
        }

        // 3. Scan hex sequence after '::'.
        let prev_count = counter[0];
        let scanned = Self::scan_hex_sequence(&a, index, end, &mut counter);

        // We've either reached the end of the string, the address ends in
        // an IPv4 address, or it is invalid. scanHexSequence has already
        // made sure that we have the right number of bits.
        scanned == end as i32
            || (scanned != -1 && {
                let from = if counter[0] > prev_count { scanned as usize + 1 } else { scanned as usize };
                Self::is_well_formed_ipv4_address(&from_chars(&a[from..end]))
            })
    }

    /// Helper method for isWellFormedIPv6Reference which scans the hex
    /// sequences of an IPv6 address.
    fn scan_hex_sequence(address: &[u16], mut index: usize, end: usize, counter: &mut [i32; 1]) -> i32 {
        let mut num_digits = 0;
        let start = index;

        // Trying to match the following productions:
        // hexseq = hex4 *( ":" hex4)
        // hex4   = 1*4HEXDIG
        while index < end {
            let test_char = address[index];
            if test_char == b':' as u16 {
                // IPv6 addresses are 128-bit, so there can be at most eight
                // sections.
                if num_digits > 0 {
                    counter[0] += 1;
                    if counter[0] > 8 {
                        return -1;
                    }
                }
                // This could be '::'.
                if num_digits == 0 || (index + 1 < end && address[index + 1] == b':' as u16) {
                    return index as i32;
                }
                num_digits = 0;
            }
            // This might be invalid or an IPv4address. If it's potentially
            // an IPv4address, backup to just after the last valid character
            // that matches hexseq.
            else if !is_hex(test_char) {
                if test_char == b'.' as u16 && num_digits < 4 && num_digits > 0 && counter[0] <= 6 {
                    let back = index as i32 - num_digits - 1;
                    return if back >= start as i32 { back } else { back + 1 };
                }
                return -1;
            }
            // There can be at most 4 hex digits per group.
            else {
                num_digits += 1;
                if num_digits > 4 {
                    return -1;
                }
            }
            index += 1;
        }
        if num_digits > 0 {
            counter[0] += 1;
            if counter[0] <= 8 {
                return end as i32;
            }
        }
        -1
    }

    /// Determine whether a given string contains only URI characters (also
    /// called "uric" in RFC 2396).
    fn is_uri_string(p_uric: Option<&str>) -> bool {
        let Some(p_uric) = p_uric else {
            return false;
        };
        let u = to_chars(p_uric);
        let end = u.len();
        let mut i = 0;
        while i < end {
            let test_char = u[i];
            if test_char == b'%' as u16 {
                if i + 2 >= end || !is_hex(u[i + 1]) || !is_hex(u[i + 2]) {
                    return false;
                } else {
                    i += 3;
                    continue;
                }
            }
            if is_uri_character(test_char) {
                i += 1;
                continue;
            } else {
                return false;
            }
        }
        true
    }
}

impl std::fmt::Display for URI {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_java_string())
    }
}
