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

//! The port of `Extensions.java`, of the Java package
//! `org.apache.lucene.queryparser.ext`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use rucene::error::{LuceneError, Result};

use super::parser_extension::ParserExtension;
use crate::classic::query_parser_base;

/// The default extension field delimiter character. This constant is set to
/// `:`.
///
/// Equivalent to `Extensions.DEFAULT_EXTENSION_FIELD_DELIMITER`.
pub const DEFAULT_EXTENSION_FIELD_DELIMITER: char = ':';

/// The `Extensions` class represents an extension mapping to associate
/// `ParserExtension` instances with extension keys. An extension key is a
/// string encoded into a Lucene standard query parser field symbol
/// recognized by `ExtendableQueryParser`. The query parser passes each
/// extension field token to `split_extension_field` to separate the
/// extension key from the field identifier.
///
/// In addition to the key to extension mapping this class also defines the
/// field name overloading scheme. `ExtendableQueryParser` uses the given
/// extension to split the actual field name and extension key by calling
/// `split_extension_field`. To change the order or the key / field name
/// encoding scheme users can subclass `Extensions` to implement their own.
///
/// Equivalent to `org.apache.lucene.queryparser.ext.Extensions`. A parser
/// holds a reference to the mapping, which its creator may keep adding to,
/// as in Java; the map is therefore behind a lock. A `null` key or value is
/// `None` (Java's `HashMap` holds both).
#[derive(Debug)]
pub struct Extensions {
    extensions: Mutex<ExtensionMap>,
    extension_field_delimiter: char,
}

impl Default for Extensions {
    fn default() -> Self {
        Self::new()
    }
}

impl Extensions {
    /// Creates a new `Extensions` instance with the
    /// [`DEFAULT_EXTENSION_FIELD_DELIMITER`] as a delimiter character.
    ///
    /// Equivalent to `new Extensions()`.
    pub fn new() -> Self {
        Self::with_delimiter(DEFAULT_EXTENSION_FIELD_DELIMITER)
    }

    /// Creates a new `Extensions` instance.
    ///
    /// Equivalent to `new Extensions(char)`.
    pub fn with_delimiter(extension_field_delimiter: char) -> Self {
        Self {
            extensions: Mutex::new(HashMap::new()),
            extension_field_delimiter,
        }
    }

    /// Adds a new `ParserExtension` instance associated with the given key.
    ///
    /// Equivalent to `add(String, ParserExtension)`: a `None` extension is
    /// stored as Java stores a `null` value, so
    /// [`get_extension`](Self::get_extension) answers `None` for its key.
    pub fn add(&self, key: Option<&str>, extension: Option<Arc<dyn ParserExtension>>) {
        self.extensions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.map(str::to_string), extension);
    }

    /// Returns the `ParserExtension` instance for the given key or `None`
    /// if no extension can be found for the key.
    ///
    /// Equivalent to the `final getExtension(String)`.
    pub fn get_extension(&self, key: Option<&str>) -> Option<Arc<dyn ParserExtension>> {
        self.extensions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key.map(str::to_string))
            .cloned()
            .flatten()
    }

    /// Returns the extension field delimiter.
    ///
    /// Equivalent to `getExtensionFieldDelimiter()`.
    pub fn get_extension_field_delimiter(&self) -> char {
        self.extension_field_delimiter
    }

    /// Splits a extension field and returns the field / extension part as a
    /// [`Pair`]. This method tries to split on the first occurrence of the
    /// extension field delimiter, if the delimiter is not present in the
    /// string the result will contain a `None` value for the extension key
    /// and the given field string as the field value. If the given extension
    /// field string contains no field identifier the result pair will carry
    /// the given default field as the field value.
    ///
    /// Equivalent to `splitExtensionField(String, String)`.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of a `null` field.
    pub fn split_extension_field(
        &self,
        default_field: Option<&str>,
        field: Option<&str>,
    ) -> Result<Pair<Option<String>, Option<String>>> {
        // `field.indexOf(this.extensionFieldDelimiter)` dereferences a `null`
        // field: the JDK's helpful message (observed on Lucene 10.5.0).
        let field = field.ok_or_else(|| {
            LuceneError::null_pointer_with_message(
                "Cannot invoke \"String.indexOf(int)\" because \"field\" is null",
            )
        })?;
        let Some(index_of) = field.find(self.extension_field_delimiter) else {
            return Ok(Pair::new(Some(field.to_string()), None));
        };
        let index_field = if index_of == 0 {
            default_field.map(str::to_string)
        } else {
            Some(field[..index_of].to_string())
        };
        let extension_key =
            field[index_of + self.extension_field_delimiter.len_utf8()..].to_string();
        Ok(Pair::new(index_field, Some(extension_key)))
    }

    /// Escapes an extension field. The default implementation is equivalent
    /// to `QueryParserBase.escape(String)`.
    ///
    /// Equivalent to `escapeExtensionField(String)`.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` `QueryParserBase.escape` throws for a
    /// `null` string (`s.length()`, `QueryParserBase.java`), with the JDK's
    /// helpful message.
    pub fn escape_extension_field(&self, extfield: Option<&str>) -> Result<String> {
        let extfield = extfield.ok_or_else(|| {
            LuceneError::null_pointer_with_message(
                "Cannot invoke \"String.length()\" because \"s\" is null",
            )
        })?;
        Ok(query_parser_base::escape(extfield))
    }

    /// Builds an extension field string from a given extension key and the
    /// default query field. The default field and the key are delimited with
    /// the extension field delimiter character. This method makes no
    /// assumption about the order of the extension key and the field. By
    /// default the extension key is appended to the end of the returned
    /// string while the field is added to the beginning. Special Query
    /// characters are escaped in the result.
    ///
    /// Note: `Extensions` subclasses must maintain the contract between
    /// `build_extension_field` and `split_extension_field` where the
    /// latter inverts the former.
    ///
    /// Equivalent to `buildExtensionField(String)`.
    pub fn build_extension_field(&self, extension_key: &str) -> String {
        self.build_extension_field_with_field(extension_key, "")
    }

    /// Builds an extension field string from a given extension key and the
    /// extensions field.
    ///
    /// Equivalent to `buildExtensionField(String, String)`.
    pub fn build_extension_field_with_field(&self, extension_key: &str, field: &str) -> String {
        let mut builder = String::from(field);
        builder.push(self.extension_field_delimiter);
        builder.push_str(extension_key);
        // `escapeExtensionField(builder.toString())`: a non-null string,
        // which never throws.
        query_parser_base::escape(&builder)
    }
}

/// Java's `Map<String, ParserExtension>`: a `null` key or value is `None`.
type ExtensionMap = HashMap<Option<String>, Option<Arc<dyn ParserExtension>>>;

/// This class represents a generic pair.
///
/// Equivalent to the record `org.apache.lucene.queryparser.ext.Extensions.Pair`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Pair<Cur, Cud> {
    cur: Cur,
    cud: Cud,
}

impl<Cur, Cud> Pair<Cur, Cud> {
    /// Creates a new Pair.
    ///
    /// Equivalent to the canonical constructor `Pair(Cur, Cud)`.
    pub fn new(cur: Cur, cud: Cud) -> Self {
        Self { cur, cud }
    }

    /// Equivalent to the record accessor `cur()`.
    pub fn cur(&self) -> &Cur {
        &self.cur
    }

    /// Equivalent to the record accessor `cud()`.
    pub fn cud(&self) -> &Cud {
        &self.cud
    }
}
