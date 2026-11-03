//! `System.getProperty`, `System.setProperty` and `System.clearProperty`.
//!
//! **Divergence.** A JVM's system properties come from `-D` options
//! and the platform; a Rust process has neither. The port keeps a
//! process-wide property table that [`set_property`] and [`clear_property`]
//! write (the test harness writes the `-D` options it is given), and a
//! property that the table does not hold is read from the process
//! environment variable of the same name, so `env tests.seed=DEADBEEF cargo
//! test` sets `tests.seed`. The one platform property randomizedtesting reads,
//! `java.io.tmpdir`, defaults to `std::env::temp_dir()`.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

/// `Some(value)` for a set property, `None` for a cleared one.
static PROPERTIES: LazyLock<RwLock<HashMap<String, Option<String>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// `System.getProperty(key)`.
pub fn get_property(key: &str) -> Option<String> {
    if let Some(entry) = PROPERTIES
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
    {
        return entry.clone();
    }
    if let Some(v) = std::env::var_os(key) {
        return Some(v.to_string_lossy().into_owned());
    }
    if key == "java.io.tmpdir" {
        return Some(std::env::temp_dir().to_string_lossy().into_owned());
    }
    None
}

/// `System.setProperty(key, value)`; returns the previous value.
pub fn set_property(key: &str, value: &str) -> Option<String> {
    let previous = get_property(key);
    PROPERTIES
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.to_string(), Some(value.to_string()));
    previous
}

/// `System.clearProperty(key)`; returns the previous value.
pub fn clear_property(key: &str) -> Option<String> {
    let previous = get_property(key);
    PROPERTIES
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.to_string(), None);
    previous
}
