// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.ICUDebug`.

use std::sync::LazyLock;

/// The class's static state: `params`, `debug` and `help`, set once by the
/// static initializer, which also prints `ICUDebug=<params>` when debugging
/// is on.
struct State {
    params: Option<String>,
    debug: bool,
    help: bool,
}

static STATE: LazyLock<State> = LazyLock::new(|| {
    // System.getProperty("ICUDebug"); a SecurityException leaves it null.
    let params = rucene::util::system_property("ICUDebug");
    let debug = params.is_some();
    let help = debug
        && params
            .as_deref()
            .is_some_and(|p| p.is_empty() || p.contains("help"));
    if debug {
        println!("\nICUDebug={}", params.as_deref().unwrap_or("null"));
    }
    State {
        params,
        debug,
        help,
    }
});

/// Debug switches read from the `ICUDebug` system property.
///
/// Equivalent to `com.ibm.icu.impl.ICUDebug`.
pub struct ICUDebug;

impl ICUDebug {
    /// `enabled()`: whether the `ICUDebug` property is set.
    pub fn enabled() -> bool {
        STATE.debug
    }

    /// `enabled(String arg)`: whether the property contains `arg`.
    pub fn enabled_arg(arg: &str) -> bool {
        let s = &*STATE;
        if s.debug {
            let result = s.params.as_deref().is_some_and(|p| p.contains(arg));
            if s.help {
                println!("\nICUDebug.enabled({arg}) = {result}");
            }
            return result;
        }
        false
    }

    /// `value(String arg)`: the value after `arg=` in the property, `"true"`
    /// when `arg` occurs without a value, `"false"` when it does not occur
    /// or debugging is off.
    pub fn value(arg: &str) -> String {
        let s = &*STATE;
        let mut result = "false".to_string();
        if s.debug {
            let params = s.params.as_deref().unwrap_or("");
            if let Some(found) = params.find(arg) {
                let mut index = found + arg.len();
                if params.len() > index && params.as_bytes()[index] == b'=' {
                    index += 1;
                    let limit = params[index..].find(',').map(|l| l + index);
                    result = params[index..limit.unwrap_or(params.len())].to_string();
                } else {
                    result = "true".to_string();
                }
            }
            if s.help {
                println!("\nICUDebug.value({arg}) = {result}");
            }
        }
        result
    }
}
