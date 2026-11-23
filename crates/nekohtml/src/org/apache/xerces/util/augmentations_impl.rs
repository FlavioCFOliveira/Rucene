/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 * 
 *      http://www.apache.org/licenses/LICENSE-2.0
 * 
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `AugmentationsImpl.java`, of the Java package `org.apache.xerces.util` (Xerces-J 2.12.2).

use std::cell::RefCell;
use std::fmt::Write as _;

use rucene::util::java_collections::JavaHashMap;

use crate::java::Object;
use crate::org::apache::xerces::xni::augmentations::Augmentations;

/// This class provides an implementation for Augmentations interface.
/// Augmentations interface defines a hashtable of additional data that
/// could be passed along the document pipeline.
///
/// Equivalent to `org.apache.xerces.util.AugmentationsImpl`.
#[derive(Default)]
pub struct AugmentationsImpl {
    augmentations_container: RefCell<AugmentationsItemsContainer>,
}

impl std::fmt::Debug for AugmentationsImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_java_string())
    }
}

impl AugmentationsImpl {
    /// Equivalent to `new AugmentationsImpl()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Equivalent to `toString()`.
    pub fn to_java_string(&self) -> String {
        self.augmentations_container.borrow().to_java_string()
    }
}

impl Augmentations for AugmentationsImpl {
    fn put_item(&self, key: &str, item: Option<Object>) -> Option<Object> {
        let mut c = self.augmentations_container.borrow_mut();
        let old_value = c.put_item(key, item);
        if old_value.is_none() && c.is_full() {
            let expanded = c.expand();
            *c = expanded;
        }
        old_value
    }

    fn get_item(&self, key: &str) -> Option<Object> {
        self.augmentations_container.borrow().get_item(key)
    }

    fn remove_item(&self, key: &str) -> Option<Object> {
        self.augmentations_container.borrow_mut().remove_item(key)
    }

    fn keys(&self) -> Vec<String> {
        self.augmentations_container.borrow().keys()
    }

    fn remove_all_items(&self) {
        self.augmentations_container.borrow_mut().clear();
    }
}

/// Equivalent to the abstract nested class
/// `AugmentationsImpl.AugmentationsItemsContainer` and its two subclasses
/// `SmallContainer` and `LargeContainer`.
enum AugmentationsItemsContainer {
    /// `SmallContainer`: up to `SIZE_LIMIT` entries in insertion order.
    Small(Vec<(String, Option<Object>)>),
    /// `LargeContainer`: a `java.util.HashMap`.
    Large(JavaHashMap<Option<Object>>),
}

impl Default for AugmentationsItemsContainer {
    fn default() -> Self {
        AugmentationsItemsContainer::Small(Vec::with_capacity(SIZE_LIMIT))
    }
}

/// `SmallContainer.SIZE_LIMIT`.
const SIZE_LIMIT: usize = 10;

fn object_to_string(o: &Option<Object>) -> String {
    match o {
        None => "null".to_string(),
        Some(o) => match o.downcast_ref::<String>() {
            Some(s) => s.clone(),
            None => format!("{:p}", std::rc::Rc::as_ptr(o)),
        },
    }
}

impl AugmentationsItemsContainer {
    fn put_item(&mut self, key: &str, item: Option<Object>) -> Option<Object> {
        match self {
            AugmentationsItemsContainer::Small(entries) => {
                for entry in entries.iter_mut() {
                    if entry.0 == key {
                        return std::mem::replace(&mut entry.1, item);
                    }
                }
                entries.push((key.to_string(), item));
                None
            }
            AugmentationsItemsContainer::Large(map) => map.insert(key.to_string(), item).flatten(),
        }
    }

    fn get_item(&self, key: &str) -> Option<Object> {
        match self {
            AugmentationsItemsContainer::Small(entries) => {
                entries.iter().find(|e| e.0 == key).and_then(|e| e.1.clone())
            }
            AugmentationsItemsContainer::Large(map) => map.get(key).cloned().flatten(),
        }
    }

    fn remove_item(&mut self, key: &str) -> Option<Object> {
        match self {
            AugmentationsItemsContainer::Small(entries) => {
                let i = entries.iter().position(|e| e.0 == key)?;
                entries.remove(i).1
            }
            AugmentationsItemsContainer::Large(map) => map.remove(key).flatten(),
        }
    }

    fn keys(&self) -> Vec<String> {
        match self {
            AugmentationsItemsContainer::Small(entries) => entries.iter().map(|e| e.0.clone()).collect(),
            AugmentationsItemsContainer::Large(map) => map.keys().cloned().collect(),
        }
    }

    fn clear(&mut self) {
        match self {
            AugmentationsItemsContainer::Small(entries) => entries.clear(),
            AugmentationsItemsContainer::Large(map) => map.clear(),
        }
    }

    fn is_full(&self) -> bool {
        match self {
            AugmentationsItemsContainer::Small(entries) => entries.len() == SIZE_LIMIT,
            AugmentationsItemsContainer::Large(_) => false,
        }
    }

    fn expand(&mut self) -> AugmentationsItemsContainer {
        match self {
            AugmentationsItemsContainer::Small(entries) => {
                let mut map = JavaHashMap::new();
                for (k, v) in entries.drain(..) {
                    map.insert(k, v);
                }
                AugmentationsItemsContainer::Large(map)
            }
            AugmentationsItemsContainer::Large(map) => {
                AugmentationsItemsContainer::Large(std::mem::take(map))
            }
        }
    }

    fn to_java_string(&self) -> String {
        let mut buff = String::new();
        match self {
            AugmentationsItemsContainer::Small(entries) => {
                let _ = write!(buff, "SmallContainer - fNumEntries == {}", entries.len());
                for i in 0..SIZE_LIMIT {
                    let (k, v) = match entries.get(i) {
                        Some((k, v)) => (k.clone(), object_to_string(v)),
                        None => ("null".to_string(), "null".to_string()),
                    };
                    let _ = write!(
                        buff,
                        "\nfAugmentations[{}] == {}; fAugmentations[{}] == {}",
                        i * 2,
                        k,
                        i * 2 + 1,
                        v
                    );
                }
            }
            AugmentationsItemsContainer::Large(map) => {
                buff.push_str("LargeContainer");
                for (k, v) in map.iter() {
                    let _ = write!(buff, "\nkey == {}; value == {}", k, object_to_string(v));
                }
            }
        }
        buff
    }
}
