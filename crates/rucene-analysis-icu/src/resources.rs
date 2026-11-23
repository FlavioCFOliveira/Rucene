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

//! The resource files of `lucene-analysis-icu` 10.5.0: the three data files
//! of `lucene/analysis/icu/src/resources` outside `META-INF/services` (see
//! [`spi`](crate::spi)): `org/apache/lucene/analysis/icu/utr30.nrm` (the
//! UTR#30 folding data `ICUFoldingFilter` loads) and
//! `org/apache/lucene/analysis/icu/segmentation/{Default,MyanmarSyllable}.brk`
//! (the compiled break rules `DefaultICUTokenizerConfig` loads).
//!
//! Java finds them on the class path, inside `lucene-analysis-icu.jar`, with
//! `Class.getResourceAsStream(name)` relative to the package of the class
//! (`ICUFoldingFilter.class.getResourceAsStream("utr30.nrm")`). Rust has no
//! class path, so the crate embeds every file, byte for byte, with
//! `include_bytes!` (the copies in `crates/rucene-analysis-icu/resources/`
//! have the SHA-1 of the 10.5.0 originals: `utr30.nrm`
//! `e803f4c6de67fd78cb2d79aa677745764dda8342`, `Default.brk`
//! `afa6f8ffbf50054911ba7beaecf878e9a7464876`, `MyanmarSyllable.brk`
//! `6ad4e9443110cb83412a0bff1ba43031c1604cde`), and this module resolves
//! names as Java's class loader does:
//!
//! | Rucene | Java |
//! | --- | --- |
//! | [`get_resource`] | `ClassLoader.getResource(name)` of the module's loader, an absolute name |
//! | [`get_resource_as_stream`] | `Class.getResourceAsStream(name)`, relative to the class's package |
//!
//! A name no file has answers `None`, Java's `null`.

use std::io;

use rucene::store::java_nio_file::InputStream;

/// One embedded file: its absolute resource name and its bytes.
macro_rules! resource {
    ($name:literal) => {
        (
            $name,
            include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/", $name)) as &[u8],
        )
    };
}

/// Every resource file of the module, by absolute name (no leading `/`), in
/// name order.
static RESOURCES: &[(&str, &[u8])] = &[
    resource!("org/apache/lucene/analysis/icu/segmentation/Default.brk"),
    resource!("org/apache/lucene/analysis/icu/segmentation/MyanmarSyllable.brk"),
    resource!("org/apache/lucene/analysis/icu/utr30.nrm"),
];

/// Returns the bytes of the resource whose absolute name is `name`, or
/// `None` when the module has none.
///
/// Equivalent to `ClassLoader.getResource(name)` (read as
/// `getResourceAsStream`) of the class loader that holds
/// `lucene-analysis-icu.jar`: the name is absolute and has no leading `/`.
pub fn get_resource(name: &str) -> Option<&'static [u8]> {
    RESOURCES
        .binary_search_by(|(resource, _)| (*resource).cmp(name))
        .ok()
        .map(|index| RESOURCES[index].1)
}

/// Opens the resource `name` relative to the class whose binary name is
/// `class_name`, or answers `None` when the module has no such resource.
///
/// Equivalent to `Class.getResourceAsStream(String)` for one of the
/// module's classes (`Class.resolveName`): a name starting with `/` is
/// absolute and loses the `/`; any other name is prefixed with the class's
/// package, its dots turned into `/` (`utr30.nrm` of
/// `org.apache.lucene.analysis.icu.ICUFoldingFilter` is
/// `org/apache/lucene/analysis/icu/utr30.nrm`).
pub fn get_resource_as_stream(class_name: &str, name: &str) -> Option<ResourceInputStream> {
    let absolute = match name.strip_prefix('/') {
        Some(absolute) => absolute.to_string(),
        None => match class_name.rfind('.') {
            Some(dot) => format!("{}/{name}", class_name[..dot].replace('.', "/")),
            None => name.to_string(),
        },
    };
    RESOURCES
        .binary_search_by(|(resource, _)| (*resource).cmp(absolute.as_str()))
        .ok()
        .map(|index| ResourceInputStream {
            name: RESOURCES[index].0,
            data: RESOURCES[index].1,
            pos: 0,
            mark: 0,
        })
}

/// An `InputStream` over an embedded resource, as
/// `Class.getResourceAsStream` opens a jar entry: `available()` is the
/// number of bytes not yet read, as the JDK's jar entry streams answer.
#[derive(Debug)]
pub struct ResourceInputStream {
    name: &'static str,
    data: &'static [u8],
    pos: usize,
    mark: usize,
}

impl InputStream for ResourceInputStream {
    fn read_byte(&mut self) -> io::Result<i32> {
        if self.pos < self.data.len() {
            let b = self.data[self.pos];
            self.pos += 1;
            Ok(i32::from(b))
        } else {
            Ok(-1)
        }
    }

    fn read(&mut self, b: &mut [u8], off: usize, len: usize) -> io::Result<i32> {
        if len == 0 {
            return Ok(0);
        }
        let rem = self.data.len() - self.pos;
        if rem == 0 {
            return Ok(-1);
        }
        let n = len.min(rem);
        b[off..off + n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n as i32)
    }

    fn skip(&mut self, n: i64) -> io::Result<i64> {
        if n <= 0 {
            return Ok(0);
        }
        let k = (n as u64).min((self.data.len() - self.pos) as u64) as usize;
        self.pos += k;
        Ok(k as i64)
    }

    fn available(&mut self) -> io::Result<i32> {
        Ok((self.data.len() - self.pos).min(i32::MAX as usize) as i32)
    }

    fn close(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn mark(&mut self, _readlimit: i32) {
        self.mark = self.pos;
    }

    fn reset(&mut self) -> io::Result<()> {
        self.pos = self.mark;
        Ok(())
    }

    fn mark_supported(&self) -> bool {
        true
    }

    fn stream_to_string(&self) -> String {
        format!("ResourceInputStream[{}]", self.name)
    }
}
