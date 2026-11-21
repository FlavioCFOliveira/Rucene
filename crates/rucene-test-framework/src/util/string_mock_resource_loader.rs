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

//! Port of `org.apache.lucene.tests.util.StringMockResourceLoader`.

use std::io::{Cursor, Read};

use rucene::error::Result;
use rucene::util::resource_loader::ResourceLoader;

/// Fake resource loader for tests: works if you want to fake reading a single
/// file.
///
/// Equivalent to `org.apache.lucene.tests.util.StringMockResourceLoader`:
/// every resource opens as the UTF-8 bytes of `text`.
///
/// Java's `findClass(String, Class)` (`Class.forName(cname).asSubclass(..)`)
/// has no counterpart here: the port's `ResourceLoader` declares only
/// `openResource`, and the class half of Lucene's `ResourceLoader` is
/// `rucene::util::resource_loader::ClassRegistry` (the divergence declared in
/// `rucene::util::resource_loader`).
#[derive(Debug, Clone)]
pub struct StringMockResourceLoader {
    pub(crate) text: String,
}

impl StringMockResourceLoader {
    /// `new StringMockResourceLoader(String text)`.
    pub fn new(text: impl Into<String>) -> Self {
        StringMockResourceLoader { text: text.into() }
    }
}

impl ResourceLoader for StringMockResourceLoader {
    /// `new ByteArrayInputStream(text.getBytes(StandardCharsets.UTF_8))`,
    /// whatever the resource's name.
    fn open_resource(&self, _resource: &str) -> Result<Box<dyn Read + Send>> {
        Ok(Box::new(Cursor::new(self.text.as_bytes().to_vec())))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use rucene::util::resource_loader::ResourceLoader;

    use super::StringMockResourceLoader;

    #[test]
    fn every_resource_is_the_text() {
        let loader = StringMockResourceLoader::new("a\nb\u{e9}");
        for name in ["stopwords.txt", ""] {
            let mut bytes = Vec::new();
            loader
                .open_resource(name)
                .unwrap()
                .read_to_end(&mut bytes)
                .unwrap();
            assert_eq!(bytes, "a\nb\u{e9}".as_bytes());
        }
    }
}
