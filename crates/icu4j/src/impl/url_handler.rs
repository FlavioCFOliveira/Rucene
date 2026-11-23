// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.URLHandler`.

use std::path::PathBuf;
use std::sync::LazyLock;

use rucene::error::{LuceneError, Result};

use crate::jdk::{InputStream, JavaUrl};
use crate::r#impl::class_loader_util::ClassLoaderUtil;
use crate::r#impl::icu_debug::ICUDebug;

/// `DEBUG`.
static DEBUG: LazyLock<bool> = LazyLock::new(|| ICUDebug::enabled_arg("URLHandler"));

/// `handlers`: the handler factories named by `urlhandler.props`, by
/// protocol.
///
/// Java reads `key=class` lines from the resource and resolves each class
/// with `Class.forName(value).getDeclaredMethod("get", URL.class)`; a Rust
/// crate loads no classes by name, so every line meets Java's
/// `ClassNotFoundException` branch (skipped), and the map stays `null`.
static HANDLERS: LazyLock<Option<()>> = LazyLock::new(|| {
    let loader = ClassLoaderUtil::get_class_loader_of(&crate::jdk::JavaClass::new(
        "com.ibm.icu.impl.URLHandler",
    ));
    if let Some(mut is) = loader.get_resource_as_stream(URLHandler::PROPNAME) {
        let mut bytes = Vec::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = is.read(&mut buf, 0, 4096) {
            if n < 0 {
                break;
            }
            bytes.extend_from_slice(&buf[..n as usize]);
        }
        let text = String::from_utf8_lossy(&bytes).into_owned();
        for line in text.lines() {
            let line = line.trim_matches(|c: char| (c as u32) <= 0x20);
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some(ix) = line.find('=') else {
                if *DEBUG {
                    eprintln!("bad urlhandler line: '{line}'");
                }
                break;
            };
            let value = line[ix + 1..].trim_matches(|c: char| (c as u32) <= 0x20);
            // Class.forName(value): ClassNotFoundException.
            if *DEBUG {
                eprintln!("java.lang.ClassNotFoundException: {value}");
            }
        }
        let _ = is.close();
    }
    None
});

/// Visitor of the names a handler enumerates.
///
/// Equivalent to `com.ibm.icu.impl.URLHandler.URLVisitor`.
pub trait URLVisitor {
    /// `visit(String str)`.
    fn visit(&mut self, s: &str);
}

/// The enumeration of the resources below a `file:` or `jar:` URL.
///
/// Equivalent to the abstract class `com.ibm.icu.impl.URLHandler` with its
/// private subclasses `FileURLHandler` and `JarURLHandler`. The `jar:`
/// URLs this crate's class loader returns name the embedded jar
/// ([`crate::jdk::EMBEDDED_JAR`]), whose entries the handler lists.
#[derive(Debug, Clone)]
pub enum URLHandler {
    /// `FileURLHandler`: a file or directory.
    File {
        /// `file`.
        file: PathBuf,
    },
    /// `JarURLHandler`: the entries of a jar below a prefix.
    Jar {
        /// `jarFile.entries()`: `(name, is_directory)`.
        entries: Vec<(String, bool)>,
        /// `prefix`.
        prefix: String,
    },
}

impl URLHandler {
    /// `PROPNAME`.
    pub const PROPNAME: &'static str = "urlhandler.props";

    /// `get(URL url)`: the handler of `url`, `None` when there is none.
    pub fn get(url: Option<&JavaUrl>) -> Option<URLHandler> {
        let url = url?;
        // handlers is always null (see HANDLERS).
        LazyLock::force(&HANDLERS);
        Self::get_default(url)
    }

    /// `getDefault(URL url)` (protected).
    pub fn get_default(url: &JavaUrl) -> Option<URLHandler> {
        let protocol = url.get_protocol();
        let handler = if protocol == "file" {
            Self::file_url_handler(url)
        } else if protocol == "jar" || protocol == "wsjar" {
            Self::jar_url_handler(url)
        } else {
            return None;
        };
        // ignore - just return null
        handler.ok()
    }

    fn file_url_handler(url: &JavaUrl) -> Result<URLHandler> {
        // new File(url.toURI())
        let path = url.get_path();
        let file = PathBuf::from(path.strip_prefix("//").unwrap_or(path));
        if !file.exists() {
            if *DEBUG {
                eprintln!("file does not exist - {url}");
            }
            return Err(crate::jdk::illegal_argument_no_message());
        }
        Ok(URLHandler::File { file })
    }

    fn jar_url_handler(url: &JavaUrl) -> Result<URLHandler> {
        let mut prefix = url.get_path().to_string();
        if let Some(ix) = prefix.rfind("!/") {
            prefix = prefix[ix + 2..].to_string(); // truncate after "!/"
        }
        let mut url = url.clone();
        if url.get_protocol() != "jar" {
            // change the protocol to "jar"
            // Note: is this really OK?
            let url_str = url.to_string();
            if let Some(idx) = url_str.find(':') {
                url = JavaUrl::new(&format!("jar{}", &url_str[idx..]))?;
            }
        }
        match url.jar_entries() {
            Ok(entries) => Ok(URLHandler::Jar { entries, prefix }),
            Err(e) => {
                if *DEBUG {
                    eprintln!("icurb jar error: {e}");
                }
                Err(LuceneError::illegal_argument(format!("jar error: {e}")))
            }
        }
    }

    /// `guide(URLVisitor visitor, boolean recurse)`: names stripped of
    /// their directories.
    pub fn guide(&self, visitor: &mut dyn URLVisitor, recurse: bool) {
        self.guide_strip(visitor, recurse, true);
    }

    /// `guide(URLVisitor visitor, boolean recurse, boolean strip)`.
    pub fn guide_strip(&self, v: &mut dyn URLVisitor, recurse: bool, strip: bool) {
        match self {
            URLHandler::File { file } => {
                if file.is_dir() {
                    Self::process(v, recurse, strip, "/", &Self::list_files(file));
                } else {
                    let name = file
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    v.visit(&name);
                }
            }
            URLHandler::Jar { entries, prefix } => {
                for (name, is_directory) in entries {
                    if !is_directory {
                        // skip just directory paths
                        if let Some(rest) = name.strip_prefix(prefix.as_str()) {
                            let mut name = rest;
                            let ix = name.rfind('/');
                            if ix.is_some_and(|i| i > 0) && !recurse {
                                continue;
                            }
                            if strip {
                                if let Some(i) = ix {
                                    name = &name[i + 1..];
                                }
                            }
                            v.visit(name);
                        }
                    }
                }
            }
        }
    }

    fn list_files(dir: &std::path::Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|r| r.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default()
    }

    fn process(v: &mut dyn URLVisitor, recurse: bool, strip: bool, path: &str, files: &[PathBuf]) {
        for f in files {
            let name = f
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if f.is_dir() {
                if recurse {
                    Self::process(v, recurse, strip, &format!("{path}{name}/"), &Self::list_files(f));
                }
            } else if strip {
                v.visit(&name);
            } else {
                v.visit(&format!("{path}{name}"));
            }
        }
    }
}
