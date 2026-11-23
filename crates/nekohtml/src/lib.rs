//! This crate ports, from NekoHTML 1.9.17 (`net.sourceforge.nekohtml:nekohtml:1.9.17`,
//! Apache-2.0), the SAX HTML parser `org.cyberneko.html.parsers.SAXParser`
//! that Apache Lucene 10.5.0 `lucene/benchmark` (`byTask.feeds.DemoHTMLParser`)
//! uses, with its transitive closure over NekoHTML's main code, over Xerces-J
//! 2.12.2 (`xerces:xercesImpl:2.12.2`, Apache-2.0) and over the SAX 2.0.2
//! interfaces of `xml-apis` 1.4.01 (`org.xml.sax`, public domain). The rest of
//! both libraries is out of scope (user decision on rmp task #611). Each file
//! keeps its upstream copyright and license notice.
//!
//! The module tree mirrors the Java packages: `org.cyberneko.html.parsers` is
//! the module [`org::cyberneko::html::parsers`]. Each Java source file is one
//! module, named in snake_case (`HTMLScanner.java` is
//! [`org::cyberneko::html::html_scanner`]); a nested Java type is declared in
//! the module of its source file.
//!
//! **Divergence.** Java's objects are shared by reference and called back
//! re-entrantly: the scanner hands itself to the next pipeline stage as the
//! `XMLLocator`, a handler may push input back into the scanner, and a SAX
//! handler is reachable from several parser fields at once. Here every such
//! object is shared as `Rc<T>` (single-threaded, as a Java parser instance is)
//! and keeps the state Java mutates in `Cell`/`RefCell` fields, so its methods
//! take `&self` and every holder sees a change, as in Java. A component never
//! holds a borrow of its own state across a call into another component.
//!
//! **Divergence.** Java's `String` is UTF-16 and may hold an unpaired
//! surrogate; names, identifiers and attribute values are Rust `String`s here,
//! built from the UTF-16 input with `String::from_utf16_lossy`, so an unpaired
//! surrogate in them becomes U+FFFD. Character data (`char[]`, `XMLString`,
//! SAX `characters`) stays UTF-16 (`u16`) and is passed through unchanged.
//!
//! **Divergence.** Java compares the strings of its symbol tables by identity
//! (`==`); every such string is interned, so identity and equality agree, and
//! the port compares by value.
//!
//! **Divergence.** Java's exceptions are values of [`java::Exception`]: the
//! checked `IOException` and `SAXException` (with its subclasses) and the
//! unchecked `XNIException` (with its subclasses) keep their class and data,
//! as Xerces catches and converts them by class; other unchecked exceptions
//! are kept as a [`rucene::error::LuceneError`].

// `Exception` carries the Java throwable (`SAXException`, `XNIException`, ...)
// by value, as every Java method that throws returns it; boxing it would
// change every `JResult` signature of the port.
#![allow(clippy::result_large_err)]

#[macro_use]
mod macros {
    /// Concatenates `&'static str` constants at compile time.
    macro_rules! concatcp {
        ($($s:expr),+ $(,)?) => {{
            const PARTS: &[&str] = &[$($s),+];
            const LEN: usize = {
                let mut len = 0;
                let mut i = 0;
                while i < PARTS.len() {
                    len += PARTS[i].len();
                    i += 1;
                }
                len
            };
            const BYTES: [u8; LEN] = {
                let mut out = [0u8; LEN];
                let mut o = 0;
                let mut i = 0;
                while i < PARTS.len() {
                    let b = PARTS[i].as_bytes();
                    let mut j = 0;
                    while j < b.len() {
                        out[o] = b[j];
                        o += 1;
                        j += 1;
                    }
                    i += 1;
                }
                out
            };
            match std::str::from_utf8(&BYTES) {
                Ok(s) => s,
                Err(_) => panic!("INVARIANT: UTF-8 constants"),
            }
        }};
    }
}

pub mod java;
pub mod org;
