// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.ICUConfig`.

use std::collections::HashMap;
use std::sync::LazyLock;

use crate::jdk::InputStream;
use crate::r#impl::icu_data::ICUData;

/// `CONFIG_PROPS`: `ICUConfig.properties` as the static initializer loads
/// it; a missing resource or a read failure leaves it empty.
static CONFIG_PROPS: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    // ICUData.getStream(CONFIG_PROPS_FILE) never throws (required = false).
    let Some(mut is) = ICUData::get_stream(ICUConfig::CONFIG_PROPS_FILE) else {
        return HashMap::new();
    };
    // Properties.load(InputStream): ISO 8859-1, one unit per byte.
    let mut bytes = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let cap = buf.len();
        match is.read(&mut buf, 0, cap) {
            Ok(n) if n < 0 => break,
            Ok(n) => bytes.extend_from_slice(&buf[..n as usize]),
            // catch (IOException ioe): leave the properties as read.
            Err(_) => return HashMap::new(),
        }
    }
    let _ = is.close();
    let units: Vec<u16> = bytes.iter().map(|&b| b as u16).collect();
    rucene_queryparser::jdk::resource_bundle::load(&units).unwrap_or_default()
});

/// ICU runtime configuration: a system property, or else the value of
/// `com/ibm/icu/ICUConfig.properties`.
///
/// Equivalent to `com.ibm.icu.impl.ICUConfig`.
pub struct ICUConfig;

impl ICUConfig {
    /// `CONFIG_PROPS_FILE`.
    pub const CONFIG_PROPS_FILE: &'static str = "/com/ibm/icu/ICUConfig.properties";

    /// `get(String name)`: [`get_or`](Self::get_or) with no default.
    pub fn get(name: &str) -> Option<String> {
        Self::get_or(name, None)
    }

    /// `get(String name, String def)`: the system property `name` when it is
    /// set and not empty, else the configuration value, else `def`.
    pub fn get_or(name: &str, def: Option<&str>) -> Option<String> {
        let val = rucene::util::system_property(name);
        match val {
            Some(v) if !v.is_empty() => Some(v),
            _ => CONFIG_PROPS
                .get(name)
                .cloned()
                .or_else(|| def.map(str::to_string)),
        }
    }
}
