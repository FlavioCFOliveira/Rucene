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

//! Port of `org.apache.lucene.codecs.blockterms.TermsIndexWriterBase`.

use rucene::codecs::TermStats;
use rucene::error::Result;
use rucene::index::FieldInfo;
use rucene::util::BytesRef;

/// Terms index API for a single field.
///
/// Port of `TermsIndexWriterBase.FieldWriter`. Java's `FieldWriter` is an
/// inner class that reaches its terms index writer's outputs; an
/// implementation here holds a shared handle on its writer's state instead.
pub trait FieldWriter {
    /// Port of `FieldWriter.checkIndexTerm(BytesRef, TermStats)`: whether
    /// `text` is to be an index term.
    fn check_index_term(&mut self, text: &BytesRef, stats: &TermStats) -> Result<bool>;

    /// Port of `FieldWriter.add(BytesRef, TermStats, long)`: records the
    /// index term `text`, whose block starts at `terms_file_pointer`.
    fn add(&mut self, text: &BytesRef, stats: &TermStats, terms_file_pointer: i64) -> Result<()>;

    /// Port of `FieldWriter.finish(long)`.
    fn finish(&mut self, terms_file_pointer: i64) -> Result<()>;
}

/// Base class for terms index implementations to plug into
/// [`BlockTermsWriter`](super::BlockTermsWriter).
///
/// Port of `org.apache.lucene.codecs.blockterms.TermsIndexWriterBase`
/// (`Closeable`).
pub trait TermsIndexWriterBase {
    /// Port of `TermsIndexWriterBase.addField(FieldInfo, long)`.
    fn add_field(
        &mut self,
        field_info: &FieldInfo,
        terms_file_pointer: i64,
    ) -> Result<Box<dyn FieldWriter>>;

    /// Port of `Closeable.close()`.
    fn close(&mut self) -> Result<()>;
}
