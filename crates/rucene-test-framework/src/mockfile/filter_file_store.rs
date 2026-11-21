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

//! Port of `org.apache.lucene.tests.mockfile.FilterFileStore`.

use std::io;
use std::sync::Arc;

use rucene::store::java_nio_file::{throw_unchecked, AttributeValue, FileStore};

/// Port of `org.apache.lucene.tests.mockfile.FilterFileStore`: a `FileStore`
/// that contains another, which it uses as its basic source of data.
pub struct FilterFileStore {
    /// `delegate`: the underlying `FileStore` instance.
    delegate: Arc<dyn FileStore>,
    /// `scheme`: the URI scheme of the filesystem the store belongs to.
    scheme: String,
}

impl FilterFileStore {
    /// `FilterFileStore(delegate, scheme)`.
    pub fn new(delegate: Arc<dyn FileStore>, scheme: &str) -> FilterFileStore {
        FilterFileStore {
            delegate,
            scheme: scheme.to_string(),
        }
    }

    /// `scheme`.
    pub fn scheme(&self) -> &str {
        &self.scheme
    }
}

impl FileStore for FilterFileStore {
    fn name(&self) -> String {
        self.delegate.name()
    }

    fn type_(&self) -> String {
        self.delegate.type_()
    }

    fn is_read_only(&self) -> bool {
        self.delegate.is_read_only()
    }

    fn get_total_space(&self) -> io::Result<i64> {
        self.delegate.get_total_space()
    }

    fn get_usable_space(&self) -> io::Result<i64> {
        self.delegate.get_usable_space()
    }

    fn get_unallocated_space(&self) -> io::Result<i64> {
        self.delegate.get_unallocated_space()
    }

    /// `getBlockSize()`: `FilterFileStore` does not override it, so it is
    /// `FileStore`'s default, which throws `UnsupportedOperationException`
    /// (no message; JDK 25 probe) instead of asking the delegate.
    fn get_block_size(&self) -> io::Result<i64> {
        throw_unchecked("java.lang.UnsupportedOperationException", None)
    }

    fn supports_file_attribute_view(&self, name: &str) -> bool {
        self.delegate.supports_file_attribute_view(name)
    }

    fn get_attribute(&self, attribute: &str) -> io::Result<AttributeValue> {
        self.delegate.get_attribute(attribute)
    }

    /// `toString()`: `delegate.toString()`.
    fn store_to_string(&self) -> String {
        self.delegate.store_to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use randomizedtesting::jdk::exception::Throwable;
    use rucene::store::java_nio_file::default_file_system;

    /// `FilterFileStore` inherits `FileStore.getBlockSize()`, which throws
    /// `UnsupportedOperationException` even when the delegate answers.
    #[test]
    fn block_size_is_unsupported_even_when_the_delegate_answers() {
        let delegate = default_file_system()
            .get_file_stores()
            .into_iter()
            .next()
            .expect("the default filesystem has a store");
        assert!(delegate.get_block_size().is_ok());
        let store = FilterFileStore::new(delegate, "test://");
        let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = store.get_block_size();
        }))
        .unwrap_err();
        let t = Throwable::from_panic(payload);
        assert_eq!(t.class_name(), "java.lang.UnsupportedOperationException");
        assert_eq!(t.message(), None);
    }
}
