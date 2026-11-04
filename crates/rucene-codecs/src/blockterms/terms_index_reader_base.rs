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

//! Port of `org.apache.lucene.codecs.blockterms.TermsIndexReaderBase`.

use std::fmt;

use rucene::error::Result;
use rucene::index::FieldInfo;
use rucene::util::BytesRef;

/// [`BlockTermsReader`](super::BlockTermsReader) interacts with an instance
/// of this class to manage its terms index. The writer must accept indexed
/// terms (many pairs of `BytesRef` text + long file offset), and then this
/// reader must be able to retrieve the nearest index term to a provided term
/// text.
///
/// Port of `org.apache.lucene.codecs.blockterms.TermsIndexReaderBase`
/// (`Closeable`); `toString()` is [`fmt::Display`].
pub trait TermsIndexReaderBase: Send + Sync + fmt::Display {
    /// Port of `getFieldEnum(FieldInfo)`; `None` is Java's `null`.
    fn get_field_enum(&self, field_info: &FieldInfo) -> Option<Box<dyn FieldIndexEnum>>;

    /// Port of `close()`.
    fn close(&mut self) -> Result<()>;

    /// Port of `supportsOrd()`.
    fn supports_ord(&self) -> bool;
}

/// Similar to `TermsEnum`, except, the only "metadata" it reports for a
/// given indexed term is the long file offset into the main terms dictionary
/// file.
///
/// Port of `TermsIndexReaderBase.FieldIndexEnum`.
pub trait FieldIndexEnum {
    /// Port of `seek(BytesRef)`: seeks to "largest" indexed term that's
    /// `<= term`; returns the file pointer (into the main terms index file)
    /// of that term.
    fn seek(&mut self, term: &BytesRef) -> Result<i64>;

    /// Port of `next()`: returns `-1` at the end.
    fn next(&mut self) -> Result<i64>;

    /// Port of `term()`; `None` is Java's `null`.
    fn term(&self) -> Option<BytesRef>;

    /// Port of `seek(long ord)`; only implemented if
    /// [`TermsIndexReaderBase::supports_ord`] returns true.
    fn seek_ord(&mut self, ord: i64) -> Result<i64>;

    /// Port of `ord()`; only implemented if
    /// [`TermsIndexReaderBase::supports_ord`] returns true.
    fn ord(&self) -> Result<i64>;
}
