/*
 * Copyright 2004-2008 Andy Clark, Marc Guillemot
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `HTMLAugmentations.java`, of the Java package `org.cyberneko.html` (NekoHTML 1.9.17).

use std::cell::RefCell;
use std::rc::Rc;

use crate::java::{JavaHashtable, Object};
use crate::org::apache::xerces::xni::augmentations::Augmentations;
use crate::org::cyberneko::html::html_scanner::LocationItem;

/// This class is here to overcome the XNI changes to the `Augmentations`
/// interface. In early versions of XNI, the augmentations interface
/// contained a `clear()` method to remove all of the items from the
/// augmentations instance. A later version of XNI changed this method to
/// `removeAllItems()`. Therefore, this class extends the augmentations
/// interface and explicitly implements both of these methods.
///
/// Equivalent to `org.cyberneko.html.HTMLAugmentations` (its `Hashtable`
/// is a [`JavaHashtable`], so `keys()` enumerates in Java's order).
#[derive(Default)]
pub struct HTMLAugmentations {
    /// Augmentation items.
    items: RefCell<JavaHashtable<Option<Object>>>,
}

impl std::fmt::Debug for HTMLAugmentations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.items.borrow().keys()).finish()
    }
}

impl HTMLAugmentations {
    /// Equivalent to `new HTMLAugmentations()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Copy constructor.
    ///
    /// Equivalent to the package-private `HTMLAugmentations(Augmentations)`:
    /// a `LocationItem` value is copied, any other value shared.
    pub(crate) fn copy_of(augs: &dyn Augmentations) -> Self {
        let s = Self::new();
        for key in augs.keys() {
            let mut value = augs.get_item(&key);
            if let Some(item) = value
                .as_ref()
                .and_then(|v| v.downcast_ref::<LocationItem>())
            {
                value = Some(Rc::new(LocationItem::copy_of(item)) as Object);
            }
            s.items.borrow_mut().put(&key, value);
        }
        s
    }

    /// Removes all of the elements in the table.
    ///
    /// Equivalent to `clear()`.
    pub fn clear(&self) {
        self.items.borrow_mut().clear();
    }
}

impl Augmentations for HTMLAugmentations {
    fn remove_all_items(&self) {
        self.items.borrow_mut().clear();
    }

    fn put_item(&self, key: &str, item: Option<Object>) -> Option<Object> {
        self.items.borrow_mut().put(key, item).flatten()
    }

    fn get_item(&self, key: &str) -> Option<Object> {
        self.items.borrow().get(key).cloned().flatten()
    }

    fn remove_item(&self, key: &str) -> Option<Object> {
        self.items.borrow_mut().remove(key).flatten()
    }

    fn keys(&self) -> Vec<String> {
        self.items.borrow().keys()
    }
}
