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

//! Port of `NLS.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.messages`, with the emulation of
//! the `java.lang.reflect` calls it makes.

use std::sync::{LazyLock, Mutex, Once, PoisonError, RwLock};

use rucene::error::Result;
use rucene::util::java_collections::JavaHashMap;

use crate::jdk::locale::Locale;
use crate::jdk::message_format::MessageFormat;
use crate::jdk::object::JavaObject;
use crate::jdk::resource_bundle::{MissingResourceException, ResourceBundle};

/// `java.lang.reflect.Modifier.PUBLIC`.
pub const PUBLIC: i32 = 0x0000_0001;
/// `java.lang.reflect.Modifier.PRIVATE`.
pub const PRIVATE: i32 = 0x0000_0002;
/// `java.lang.reflect.Modifier.PROTECTED`.
pub const PROTECTED: i32 = 0x0000_0004;
/// `java.lang.reflect.Modifier.STATIC`.
pub const STATIC: i32 = 0x0000_0008;
/// `java.lang.reflect.Modifier.FINAL`.
pub const FINAL: i32 = 0x0000_0010;

/// A class that extends `NLS`, as the reflection `NLS` calls sees it: its
/// name, its declared fields, and its static initializer.
///
/// **Divergence.** `NLS` fills the message-key fields of such a class by
/// reflection (`Class.getDeclaredFields()`, `Field.getModifiers()`,
/// `Field.set(null, name)`) from the class's static initializer, which the
/// JVM runs on the first access to a static member. Rust has neither
/// reflection nor class initialization: the class is this descriptor, which
/// lists its declared fields in declaration order (the order HotSpot's
/// `getDeclaredFields()` returns) with their modifiers, and runs its static
/// initializer once, on the first read of one of its [`NlsField`]s.
#[derive(Debug)]
pub struct NlsClass {
    name: &'static str,
    fields: &'static [&'static NlsField],
    static_initializer: fn(),
    initialized: Once,
}

impl NlsClass {
    /// A class: its binary name, its declared fields, and the body of its
    /// `static { ... }` block.
    pub const fn new(
        name: &'static str,
        fields: &'static [&'static NlsField],
        static_initializer: fn(),
    ) -> Self {
        Self {
            name,
            fields,
            static_initializer,
            initialized: Once::new(),
        }
    }

    /// `Class.getName()`.
    pub fn get_name(&self) -> &'static str {
        self.name
    }

    /// `Class.getDeclaredFields()`.
    pub fn get_declared_fields(&self) -> &'static [&'static NlsField] {
        self.fields
    }

    /// Runs the static initializer of the class if it has not run, as the
    /// JVM initializes a class on the first access to a static member.
    pub fn ensure_initialized(&self) {
        self.initialized.call_once(self.static_initializer);
    }
}

/// A static `String` field of a class that extends `NLS`.
///
/// Equivalent to the field and to its `java.lang.reflect.Field`: a read of
/// the field initializes its class first (see [`NlsClass`]).
#[derive(Debug)]
pub struct NlsField {
    class: &'static NlsClass,
    name: &'static str,
    modifiers: i32,
    initial: Option<&'static str>,
    /// The value `Field.set` wrote, if any.
    value: RwLock<Option<String>>,
}

impl NlsField {
    /// A field of `class`: its name, its modifiers, and the value its
    /// declaration initializes it to (`None` for none, Java's `null`).
    pub const fn new(
        class: &'static NlsClass,
        name: &'static str,
        modifiers: i32,
        initial: Option<&'static str>,
    ) -> Self {
        Self {
            class,
            name,
            modifiers,
            initial,
            value: RwLock::new(None),
        }
    }

    /// Reads the static field (its class initialized first); `None` for
    /// Java's `null`.
    pub fn get(&self) -> Option<String> {
        self.class.ensure_initialized();
        let value = self
            .value
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        value.or_else(|| self.initial.map(str::to_string))
    }

    /// `Field.getName()`.
    pub fn get_name(&self) -> &'static str {
        self.name
    }

    /// `Field.getModifiers()`.
    pub fn get_modifiers(&self) -> i32 {
        self.modifiers
    }

    /// `Field.set(null, value)`.
    fn set(&self, value: String) {
        *self.value.write().unwrap_or_else(PoisonError::into_inner) = Some(value);
    }
}

/// `private static final Map<String, Class<? extends NLS>> bundles = new
/// HashMap<>(0)`, iterated in Java's `HashMap` order.
static BUNDLES: LazyLock<Mutex<JavaHashMap<&'static NlsClass>>> =
    LazyLock::new(|| Mutex::new(JavaHashMap::with_initial_capacity(0)));

/// MessageBundles classes extend this class, to implement a bundle.
///
/// For Native Language Support (NLS), system of software
/// internationalization.
///
/// This interface is similar to the NLS class in eclipse.osgi.util.NLS
/// class - initializeMessages() method resets the values of all static
/// strings, should only be called by classes that extend from NLS (see
/// TestMessages.java for reference) - performs validation of all message in
/// a bundle, at class load time - performs per message validation at
/// runtime - see NLSTest.java for usage reference
///
/// MessageBundle classes may subclass this type.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.messages.NLS`.
#[derive(Debug)]
pub struct NLS {
    _do_not_instantiate: (),
}

impl NLS {
    /// Equivalent to `NLS.getLocalizedMessage(String key)`: the message in
    /// the default locale.
    ///
    /// # Errors
    ///
    /// As [`get_localized_message_with_locale`](Self::get_localized_message_with_locale).
    pub fn get_localized_message(key: Option<&str>) -> Result<String> {
        Self::get_localized_message_with_locale(key, &Locale::get_default())
    }

    /// Equivalent to `NLS.getLocalizedMessage(String key, Locale locale)`:
    /// the message of the first registered bundle holding `key`, or `"Message
    /// with key:<key> and locale: <locale> not found."`.
    ///
    /// # Errors
    ///
    /// The `MissingResourceException` of a registered class without any
    /// bundle, and the `NullPointerException` of a `null` key.
    pub fn get_localized_message_with_locale(key: Option<&str>, locale: &Locale) -> Result<String> {
        match Self::get_resource_bundle_object(key, locale)? {
            None => Ok(format!(
                "Message with key:{} and locale: {locale} not found.",
                key.unwrap_or("null")
            )),
            Some(message) => Ok(message),
        }
    }

    /// Equivalent to `NLS.getLocalizedMessage(String key, Locale locale,
    /// Object... args)`: the message, formatted by `new MessageFormat(str,
    /// Locale.ROOT).format(args)` when there are arguments.
    ///
    /// # Errors
    ///
    /// As [`get_localized_message_with_locale`](Self::get_localized_message_with_locale),
    /// the `NullPointerException` of a `null` array, and the exceptions of
    /// `MessageFormat`.
    pub fn get_localized_message_with_locale_and_args(
        key: Option<&str>,
        locale: &Locale,
        args: Option<&[JavaObject]>,
    ) -> Result<String> {
        let mut str = Self::get_localized_message_with_locale(key, locale)?;
        let args = args.ok_or_else(rucene::error::LuceneError::null_pointer)?;
        if !args.is_empty() {
            str = MessageFormat::new(&str, &Locale::root())?.format(Some(args))?;
        }
        Ok(str)
    }

    /// Equivalent to `NLS.getLocalizedMessage(String key, Object... args)`:
    /// the message in the default locale, formatted.
    ///
    /// # Errors
    ///
    /// As [`get_localized_message_with_locale_and_args`](Self::get_localized_message_with_locale_and_args).
    pub fn get_localized_message_with_args(
        key: Option<&str>,
        args: Option<&[JavaObject]>,
    ) -> Result<String> {
        Self::get_localized_message_with_locale_and_args(key, &Locale::get_default(), args)
    }

    /// Initialize a given class with the message bundle Keys Should be
    /// called from a class that extends NLS in a static block at class load
    /// time.
    ///
    /// Equivalent to the protected `NLS.initializeMessages(String bundleName,
    /// Class<? extends NLS> clazz)`: fills the fields of `clazz`, then
    /// registers it under `bundle_name` unless a class is registered there.
    pub fn initialize_messages(bundle_name: &str, clazz: &'static NlsClass) {
        // ignore all errors and exceptions because this function is
        // supposed to be called at class load time (`load` raises none).
        Self::load(clazz);
        let mut bundles = BUNDLES.lock().unwrap_or_else(PoisonError::into_inner);
        if !bundles.contains_key(bundle_name) {
            bundles.insert(bundle_name.to_string(), clazz);
        }
    }

    /// Equivalent to the private `NLS.getResourceBundleObject(String
    /// messageKey, Locale locale)`.
    fn get_resource_bundle_object(
        message_key: Option<&str>,
        locale: &Locale,
    ) -> Result<Option<String>> {
        // slow resource checking need to loop thru all registered resource
        // bundles
        let classes: Vec<&'static NlsClass> = BUNDLES
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .copied()
            .collect();
        for clazz in classes {
            let resource_bundle = ResourceBundle::get_bundle(clazz.get_name(), locale)?;
            match resource_bundle.get_object(message_key) {
                Ok(obj) => return Ok(Some(obj)),
                Err(e) if MissingResourceException::of(&e).is_some() => {
                    // just continue it might be on the next resource bundle
                }
                Err(e) => return Err(e),
            }
        }
        // if resource is not found
        Ok(None)
    }

    /// Equivalent to the private `NLS.load(Class<? extends NLS> clazz)`.
    fn load(clazz: &'static NlsClass) {
        let field_array = clazz.get_declared_fields();
        // build a map of field names to Field objects (never read)
        for field in field_array {
            Self::loadfield_value(field, clazz);
        }
    }

    /// Equivalent to the private `NLS.loadfieldValue(Field field, Class<?
    /// extends NLS> clazz)`: a `public static` non-`final` field is set to
    /// its own name.
    fn loadfield_value(field: &NlsField, clazz: &'static NlsClass) {
        let mod_expected = PUBLIC | STATIC;
        let mod_mask = mod_expected | FINAL;
        if (field.get_modifiers() & mod_mask) != mod_expected {
            return;
        }
        // Set a value for this empty field.
        field.set(field.get_name().to_string());
        Self::validate_message(field.get_name(), clazz);
    }

    /// Equivalent to the private `NLS.validateMessage(String key, Class<?
    /// extends NLS> clazz)`: looks the key up in the default locale and
    /// ignores the outcome.
    fn validate_message(key: &str, clazz: &'static NlsClass) {
        // Test if the message is present in the resource bundle
        if let Ok(resource_bundle) =
            ResourceBundle::get_bundle(clazz.get_name(), &Locale::get_default())
        {
            let _ = resource_bundle.get_object(Some(key));
        }
    }
}
