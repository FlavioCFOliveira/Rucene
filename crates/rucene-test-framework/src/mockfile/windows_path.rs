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

//! Port of `org.apache.lucene.tests.mockfile.WindowsPath`.

use rucene::store::java_nio_file::{throw_unchecked, Path};

/// Port of the package-private `org.apache.lucene.tests.mockfile.WindowsPath`:
/// the path of a [`WindowsFS`](super::WindowsFS), whose `resolve(Path)`
/// refuses the file names Windows reserves.
///
/// **Divergence.** Java's `WindowsPath` is a `FilterPath` subclass that
/// overrides `resolve(Path)`; this port's paths carry their filesystem (see
/// [`FilterPath`](super::FilterPath)), so the override is `WindowsFS`'s
/// [`resolve`](rucene::store::java_nio_file::FileSystemProvider::resolve),
/// which calls [`check_invalid_path`](Self::check_invalid_path) as
/// `WindowsPath.resolve` does.
pub struct WindowsPath;

impl WindowsPath {
    /// `RESERVED_CHARACTERS`.
    pub const RESERVED_CHARACTERS: [char; 8] = ['<', '>', ':', '"', '\\', '|', '?', '*'];

    /// `RESERVED_NAMES`.
    pub const RESERVED_NAMES: [&'static str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];

    /// `checkInvalidPath(other)`.
    ///
    /// # Panics
    ///
    /// `InvalidPathException` ("File name: X is one of the reserved file
    /// names", or "File name: X contains a reserved character: c"), and
    /// `NullPointerException` when `other` has no file name (with the JDK's
    /// helpful message, measured on OpenJDK 25).
    pub fn check_invalid_path(other: &Path) {
        let Some(name) = other.get_file_name() else {
            // The JDK's helpful `NullPointerException` message.
            throw_unchecked(
                "java.lang.NullPointerException",
                Some("Cannot invoke \"java.nio.file.Path.toString()\" because the return value of \"java.nio.file.Path.getFileName()\" is null"),
            )
        };
        let file_name = name.to_string();
        if Self::RESERVED_NAMES.contains(&file_name.as_str()) {
            throw_unchecked(
                "java.nio.file.InvalidPathException",
                Some(&format!(
                    "File name: {file_name} is one of the reserved file names: {file_name}"
                )),
            )
        }
        // `fileName.charAt(i)`: the UTF-16 code units.
        for unit in file_name.encode_utf16() {
            if let Some(c) = char::from_u32(u32::from(unit)) {
                if Self::RESERVED_CHARACTERS.contains(&c) {
                    throw_unchecked(
                        "java.nio.file.InvalidPathException",
                        Some(&format!(
                            "File name: {file_name} contains a reserved character: {c}: {file_name}"
                        )),
                    )
                }
            }
        }
    }
}
