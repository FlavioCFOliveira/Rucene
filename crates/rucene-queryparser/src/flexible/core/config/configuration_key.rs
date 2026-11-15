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

//! Port of `ConfigurationKey.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.config`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// The source of key identities.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// An instance of this class represents a key that is used to retrieve a
/// value from [`AbstractQueryConfig`](super::abstract_query_config::AbstractQueryConfig).
/// It also holds the value's type, which is defined in the generic argument.
///
/// Equivalent to the final class
/// `org.apache.lucene.queryparser.flexible.core.config.ConfigurationKey<T>`.
/// A key is compared by identity, as Java compares its instances (the class
/// does not override `equals`): each key gets its own identity when it is
/// created, or, for a key held by a `static`
/// ([`new_static`](Self::new_static)), when it is first used.
pub struct ConfigurationKey<T> {
    id: OnceLock<u64>,
    _type: PhantomData<fn() -> T>,
}

impl<T> ConfigurationKey<T> {
    /// Creates a new instance.
    ///
    /// Equivalent to `ConfigurationKey.newInstance()`.
    pub fn new_instance() -> Self {
        let id = OnceLock::new();
        let _ = id.set(NEXT_ID.fetch_add(1, Ordering::Relaxed));
        Self {
            id,
            _type: PhantomData,
        }
    }

    /// The key a `static` holds, as a `public static final` field
    /// initialized by `ConfigurationKey.newInstance()`: its identity is
    /// drawn when it is first used.
    pub const fn new_static() -> Self {
        Self {
            id: OnceLock::new(),
            _type: PhantomData,
        }
    }

    /// The identity of the key.
    pub(crate) fn id(&self) -> u64 {
        *self
            .id
            .get_or_init(|| NEXT_ID.fetch_add(1, Ordering::Relaxed))
    }
}

impl<T> PartialEq for ConfigurationKey<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl<T> Eq for ConfigurationKey<T> {}

impl<T> Hash for ConfigurationKey<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id().hash(state);
    }
}

impl<T> fmt::Debug for ConfigurationKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ConfigurationKey#{}", self.id())
    }
}
