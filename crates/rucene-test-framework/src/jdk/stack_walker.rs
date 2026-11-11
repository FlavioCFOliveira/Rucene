//! `java.lang.StackWalker.getInstance().walk(...)`: the frames of the calling
//! thread's stack, as class and method names.
//!
//! **Divergence.** The JVM walks its own frames, which carry the class and
//! method of each call. Rust has no such frames; this emulation captures a
//! [`std::backtrace::Backtrace`] and reads each frame's symbol,
//! `path::to::Type::method` (or `<path::to::Type as Trait>::method`), as the
//! class `path::to::Type` and the method `method`. The answer depends on the
//! symbols the build keeps: a frame the optimizer inlined is not reported, and
//! without debug information no frame is.

/// One frame: the class (the Rust type path) and the method name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackFrame {
    /// `getClassName()`: the Rust path of the type (or module) the function
    /// belongs to.
    pub class_name: String,
    /// `getMethodName()`.
    pub method_name: String,
}

/// Strips the `::h0123456789abcdef` hash suffix of a legacy-mangled symbol.
fn strip_hash(symbol: &str) -> &str {
    match symbol.rfind("::h") {
        Some(pos)
            if symbol.len() - pos == 19
                && symbol[pos + 3..].bytes().all(|b| b.is_ascii_hexdigit()) =>
        {
            &symbol[..pos]
        }
        _ => symbol,
    }
}

/// Splits a demangled symbol into its type path and method name.
fn split(symbol: &str) -> Option<StackFrame> {
    let symbol = strip_hash(symbol.trim());
    let (owner, method) = symbol.rsplit_once("::")?;
    // `<path::Type as path::Trait>::method`: the type is the part before ` as `.
    let owner = owner
        .strip_prefix('<')
        .map(|inner| {
            inner
                .split(" as ")
                .next()
                .unwrap_or(inner)
                .trim_end_matches('>')
        })
        .unwrap_or(owner);
    Some(StackFrame {
        class_name: owner.to_string(),
        method_name: method.to_string(),
    })
}

/// The frames of the current thread, innermost first, starting with the
/// caller of this function.
pub fn frames() -> Vec<StackFrame> {
    let trace = std::backtrace::Backtrace::force_capture().to_string();
    let mut out = Vec::new();
    for line in trace.lines() {
        let line = line.trim_start();
        // `  N: symbol` lines; `at file:line` lines follow them.
        let Some((index, symbol)) = line.split_once(": ") else {
            continue;
        };
        if index.is_empty() || !index.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if let Some(frame) = split(symbol) {
            out.push(frame);
        }
    }
    // Drop the capture machinery and this function itself.
    let me = out
        .iter()
        .position(|f| f.method_name == "frames" && f.class_name.ends_with("stack_walker"));
    match me {
        Some(pos) => out.split_off(pos + 1),
        None => out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_symbols() {
        assert_eq!(
            split("rucene::index::index_writer::IndexWriter::flush::h0123456789abcdef"),
            Some(StackFrame {
                class_name: "rucene::index::index_writer::IndexWriter".to_string(),
                method_name: "flush".to_string(),
            })
        );
        assert_eq!(
            split("<rucene::store::FSDirectory as rucene::store::Directory>::sync"),
            Some(StackFrame {
                class_name: "rucene::store::FSDirectory".to_string(),
                method_name: "sync".to_string(),
            })
        );
    }
}
