//! `java.util.logging`, as far as randomizedtesting logs: named loggers
//! with `warning`, `info` and `severe` records.
//!
//! **Divergence.** The JDK's default configuration prints a record to
//! `System.err` through a `ConsoleHandler` with `SimpleFormatter`'s two-line
//! format (date, source class and method, then `LEVEL: message`). The port
//! prints `LEVEL: message` (and the thrown throwable) to `System.err`
//! without the date and source line, which the port cannot reproduce; a test
//! may add a handler to observe records instead.

use std::sync::{Arc, LazyLock, RwLock};

use super::exception::Throwable;

/// `java.util.logging.Level`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// `FINE`.
    Fine,
    /// `INFO`.
    Info,
    /// `WARNING`.
    Warning,
    /// `SEVERE`.
    Severe,
}

impl std::fmt::Display for Level {
    /// `Level.getName()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Level::Fine => "FINE",
            Level::Info => "INFO",
            Level::Warning => "WARNING",
            Level::Severe => "SEVERE",
        })
    }
}

/// `java.util.logging.Handler`: receives the logger name, level, message and
/// thrown throwable of every published record.
pub type Handler = Arc<dyn Fn(&str, Level, &str, Option<&Throwable>) + Send + Sync>;

/// The handlers added with [`add_handler`]; when there is none, records go
/// to `System.err`.
static HANDLERS: LazyLock<RwLock<Vec<(u64, Handler)>>> = LazyLock::new(|| RwLock::new(Vec::new()));

/// `Logger.getLogger("").addHandler(handler)` (with the console handler
/// removed while any handler is present). Returns a key for
/// [`remove_handler`].
pub fn add_handler(handler: Handler) -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let key = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    HANDLERS
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .push((key, handler));
    key
}

/// Removes a handler added with [`add_handler`].
pub fn remove_handler(key: u64) {
    HANDLERS
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|(k, _)| *k != key);
}

/// `java.util.logging.Logger`.
#[derive(Debug, Clone)]
pub struct Logger {
    name: String,
}

impl Logger {
    /// `Logger.getLogger(name)`.
    pub fn get_logger(name: impl Into<String>) -> Logger {
        Logger { name: name.into() }
    }

    /// `Logger.getName()`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// `log(level, message, thrown)`.
    pub fn log(&self, level: Level, message: &str, thrown: Option<&Throwable>) {
        if level < Level::Info {
            return;
        }
        let handlers: Vec<Handler> = HANDLERS
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(_, h)| Arc::clone(h))
            .collect();
        if handlers.is_empty() {
            let mut text = format!("{level}: {message}\n");
            if let Some(t) = thrown {
                text.push_str(&super::system::format_stack_trace(t));
                text.push('\n');
            }
            super::system::err_print(&text);
        } else {
            for h in handlers {
                h(&self.name, level, message, thrown);
            }
        }
    }

    /// `info(message)`.
    pub fn info(&self, message: &str) {
        self.log(Level::Info, message, None)
    }

    /// `warning(message)`.
    pub fn warning(&self, message: &str) {
        self.log(Level::Warning, message, None)
    }

    /// `severe(message)`.
    pub fn severe(&self, message: &str) {
        self.log(Level::Severe, message, None)
    }
}
