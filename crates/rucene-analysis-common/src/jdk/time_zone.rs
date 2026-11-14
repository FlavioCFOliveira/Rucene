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
//! The part of `java.util.TimeZone` `SimpleDateFormat` parsing calls: the
//! IDs `TimeZone.getTimeZone(String)` resolves, and `TimeZone.getDefault()`.
//!
//! **Divergence.** An emulation of JDK 25 on Linux, written from
//! `java.util.TimeZone` and the native `TimeZone_md.c` (OpenJDK `jdk-25-ga`,
//! read as specification), over the zone IDs of the JDK's tzdb
//! (`date_parse_data`). A zone is represented by its ID only: parsing never
//! reads its rules. Two points differ from the JDK, each stated where it
//! applies:
//!
//! * the JVM system property `user.timezone` has no counterpart: the default
//!   zone is the one a JVM started without `-Duser.timezone` computes;
//! * when the platform names a zone the JDK does not know, the JDK falls
//!   back to a `GMT±hh:mm` ID computed from the C library's local time
//!   (`getSystemGMTOffsetID`); the emulation, which has no C library call
//!   for it, falls back to `GMT`.

use std::path::Path;
use std::sync::LazyLock;

use super::locale_data::data;

/// `TimeZone.GMT_ID`.
const GMT_ID: &str = "GMT";

/// The zoneinfo directory of `TimeZone_md.c` on Linux.
const ZONEINFO_DIR: &str = "/usr/share/zoneinfo";

/// The default zoneinfo file of `TimeZone_md.c` on Linux.
const DEFAULT_ZONEINFO_FILE: &str = "/etc/localtime";

/// `TimeZone.getTimeZone(String ID, boolean fallback)` reduced to the ID of
/// the zone it returns: a tzdb ID or alias as given, a custom
/// `GMT±hh:mm` ID normalized, `GMT` for an unknown ID when `fallback`, and
/// `None` otherwise. A three-letter ID of `ZoneId.SHORT_IDS` prints the JDK's
/// deprecation warning on the standard error, as Java does.
pub fn get_time_zone_id(id: &str, fallback: bool) -> Option<String> {
    let data = data();
    if data.short_ids.contains(id) {
        eprintln!(
            "WARNING: Use of the three-letter time zone ID \"{id}\" is deprecated and it will be removed in a future release"
        );
    }
    if let Some(resolved) = data.zone_ids.get(id) {
        return Some(resolved.clone());
    }
    if let Some(custom) = parse_custom_time_zone(id) {
        return Some(custom);
    }
    if fallback {
        Some(GMT_ID.to_string())
    } else {
        None
    }
}

/// `TimeZone.parseCustomTimeZone(String)`, reduced to the normalized ID:
/// `GMT` followed by a sign and `h`, `hh`, `hmm`, `hhmm`, `h:mm`, `hh:mm` or
/// `hh:mm:ss`.
fn parse_custom_time_zone(id: &str) -> Option<String> {
    let id_units: Vec<u16> = id.encode_utf16().collect();
    let length = id_units.len();
    if length < GMT_ID.len() + 2 || !id.starts_with(GMT_ID) {
        return None;
    }
    let mut index = GMT_ID.len();
    let negative = match id_units[index] {
        0x2D => true,  // '-'
        0x2B => false, // '+'
        _ => return None,
    };
    index += 1;
    let mut hours = 0;
    let mut minutes = 0;
    let mut num = 0;
    let mut count_delim = 0;
    let mut len = 0;
    while index < length {
        let c = id_units[index];
        index += 1;
        if c == u16::from(b':') {
            if count_delim > 1 || len == 0 || len > 2 {
                return None;
            }
            if count_delim == 0 {
                hours = num;
            } else if count_delim == 1 {
                minutes = num;
            }
            count_delim += 1;
            num = 0;
            len = 0;
            continue;
        }
        if !(u16::from(b'0')..=u16::from(b'9')).contains(&c) {
            return None;
        }
        num = num * 10 + i32::from(c - u16::from(b'0'));
        len += 1;
    }
    if count_delim == 0 {
        if len <= 2 {
            hours = num;
            minutes = 0;
            num = 0;
        } else if len <= 4 {
            hours = num / 100;
            minutes = num % 100;
            num = 0;
        } else {
            return None;
        }
    } else if count_delim == 1 {
        if len == 2 {
            minutes = num;
            num = 0;
        } else {
            return None;
        }
    } else if len != 2 {
        return None;
    }
    if hours > 23 || minutes > 59 || num > 59 {
        return None;
    }
    let gmt_offset = (hours * 3_600 + minutes * 60 + num) * 1_000;
    if gmt_offset == 0 {
        return Some(if negative { "GMT-00:00" } else { "GMT+00:00" }.to_string());
    }
    Some(to_custom_id(if negative {
        -gmt_offset
    } else {
        gmt_offset
    }))
}

/// `ZoneInfoFile.toCustomID(int)`.
fn to_custom_id(gmt_offset: i32) -> String {
    let mut offset = gmt_offset / 1_000;
    let sign = if offset >= 0 {
        '+'
    } else {
        offset = -offset;
        '-'
    };
    let hh = offset / 3_600;
    let mm = (offset % 3_600) / 60;
    let ss = offset % 60;
    let mut id = format!("GMT{sign}{hh:02}:");
    if mm != 0 {
        id.push_str(&format!("{mm:02}"));
    } else {
        id.push_str("00");
    }
    if ss != 0 {
        id.push_str(&format!(":{ss:02}"));
    }
    id
}

/// `TimeZone.getDefault().getID()`, computed once, as the JVM computes it
/// once (`TimeZone.setDefaultZone`).
pub fn default_id() -> &'static str {
    static DEFAULT: LazyLock<String> = LazyLock::new(|| {
        // `user.timezone` is not set (see the module documentation)
        let zone_id = find_java_tz().unwrap_or_else(|| GMT_ID.to_string());
        match get_time_zone_id(&zone_id, false) {
            Some(id) => id,
            // `getSystemGMTOffsetID()` is not emulated (see the module
            // documentation)
            None => GMT_ID.to_string(),
        }
    });
    &DEFAULT
}

/// `findJavaTZ_md` of `TimeZone_md.c` on Linux: the `TZ` environment
/// variable, else the zone `/etc/localtime` names, without a leading `:` or
/// `posix/`.
fn find_java_tz() -> Option<String> {
    let tz = match std::env::var_os("TZ") {
        Some(tz) if !tz.is_empty() => Some(tz.to_string_lossy().into_owned()),
        _ => platform_time_zone_id(),
    }?;
    let tz = tz.strip_prefix(':').unwrap_or(&tz);
    let tz = tz.strip_prefix("posix/").unwrap_or(tz);
    Some(tz.to_string())
}

/// `getPlatformTimeZoneID()` of `TimeZone_md.c` on Linux.
fn platform_time_zone_id() -> Option<String> {
    let meta = std::fs::symlink_metadata(DEFAULT_ZONEINFO_FILE).ok()?;
    if meta.file_type().is_symlink() {
        let link = std::fs::read_link(DEFAULT_ZONEINFO_FILE).ok()?;
        let link = collapse(&remove_duplicate_slashes(&link.to_string_lossy()));
        if let Some(name) = zone_name(&link) {
            return Some(name.to_string());
        }
    }
    // a regular file: find the zoneinfo file of the same content
    let buf = std::fs::read(DEFAULT_ZONEINFO_FILE).ok()?;
    find_zoneinfo_file(&buf, Path::new(ZONEINFO_DIR))
}

/// `getZoneName(char*)`: the part after the first `zoneinfo/`.
fn zone_name(path: &str) -> Option<&str> {
    const ZIDIR: &str = "zoneinfo/";
    path.find(ZIDIR).map(|pos| &path[pos + ZIDIR.len()..])
}

/// `removeDuplicateSlashes(char*)`.
fn remove_duplicate_slashes(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut previous_slash = false;
    for c in path.chars() {
        if c == '/' && previous_slash {
            continue;
        }
        previous_slash = c == '/';
        out.push(c);
    }
    out
}

/// `collapse(char*)` of `path_util.c`: the `.` names removed, and each `..`
/// removed with the name before it (or alone, at the start of an absolute
/// path).
fn collapse(path: &str) -> String {
    let absolute = path.starts_with('/');
    let names_part = if absolute { &path[1..] } else { path };
    let mut names: Vec<Option<&str>> = names_part.split('/').map(Some).collect();
    if names_part.is_empty() {
        names.clear();
    }
    let dots = names.iter().any(|n| matches!(n, Some(".") | Some("..")));
    if !dots || names.len() < 2 {
        return path.to_string();
    }
    let nc = names.len();
    let mut i = 0;
    while i < nc {
        let mut dots = 0;
        while i < nc {
            match names[i] {
                Some(".") => {
                    dots = 1;
                    break;
                }
                Some("..") => {
                    dots = 2;
                    break;
                }
                _ => i += 1,
            }
        }
        if i >= nc {
            break;
        }
        if dots == 1 {
            names[i] = None;
        } else {
            match (0..i).rev().find(|&j| names[j].is_some()) {
                Some(j) => {
                    names[j] = None;
                    names[i] = None;
                }
                None => {
                    if absolute {
                        names[i] = None;
                    }
                }
            }
        }
        i += 1;
    }
    let joined = names.into_iter().flatten().collect::<Vec<_>>().join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

/// `findZoneinfoFile(char*, size_t, const char*)`: the ID of the first file
/// under `dir` whose content is `buf`, `UTC` and `GMT` first, then in
/// directory order, skipping `.`-names, `ROC`, `posixrules` and `localtime`.
fn find_zoneinfo_file(buf: &[u8], dir: &Path) -> Option<String> {
    if dir == Path::new(ZONEINFO_DIR) {
        for popular in ["UTC", "GMT"] {
            if let Some(tz) = is_file_identical(buf, &dir.join(popular)) {
                return Some(tz);
            }
        }
    }
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "ROC" || name == "posixrules" || name == "localtime" {
            continue;
        }
        if let Some(tz) = is_file_identical(buf, &dir.join(name.as_ref())) {
            return Some(tz);
        }
    }
    None
}

/// `isFileIdentical(char*, size_t, char*)`.
fn is_file_identical(buf: &[u8], path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.is_dir() {
        return find_zoneinfo_file(buf, path);
    }
    if meta.is_file() && meta.len() == buf.len() as u64 {
        let content = std::fs::read(path).ok()?;
        if content == buf {
            let path = path.to_string_lossy();
            return zone_name(&path).map(str::to_string);
        }
    }
    None
}
