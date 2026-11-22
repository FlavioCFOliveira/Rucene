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

//! The forwarding bodies of Java's `FilterLeafReader.FilterTerms`,
//! `FilterTermsEnum` and `FilterPostingsEnum`, for the anonymous and nested
//! subclasses of the highlighter, which override one or two methods.

/// Forwards the `Terms` methods a Java `FilterTerms` inherits to the field
/// `$inner` (an `Arc<dyn Terms>`); the listed methods are left to the
/// implementation.
macro_rules! forward_terms {
    ($inner:ident) => {
        fn size(&self) -> rucene::error::Result<i64> {
            self.$inner.size()
        }

        fn sum_total_term_freq(&self) -> rucene::error::Result<i64> {
            self.$inner.sum_total_term_freq()
        }

        fn sum_doc_freq(&self) -> rucene::error::Result<i64> {
            self.$inner.sum_doc_freq()
        }

        fn doc_count(&self) -> rucene::error::Result<i32> {
            self.$inner.doc_count()
        }

        fn has_freqs(&self) -> bool {
            self.$inner.has_freqs()
        }

        fn has_offsets(&self) -> bool {
            self.$inner.has_offsets()
        }

        fn has_positions(&self) -> bool {
            self.$inner.has_positions()
        }

        fn has_payloads(&self) -> bool {
            self.$inner.has_payloads()
        }

        fn stats(&self) -> rucene::error::Result<Box<dyn rucene::index::terms::StatsObject>> {
            self.$inner.stats()
        }
    };
}
pub(crate) use forward_terms;

/// Forwards the `TermsEnum` methods a Java `FilterTermsEnum` inherits,
/// except `postings`, to the field `$inner` (a `Box<dyn TermsEnum>`).
macro_rules! forward_terms_enum {
    ($inner:ident) => {
        fn attributes(&mut self) -> &mut rucene::util::attribute::AttributeSource {
            self.$inner.attributes()
        }

        fn seek_ceil(
            &mut self,
            text: &rucene::util::BytesRef,
        ) -> rucene::error::Result<rucene::index::SeekStatus> {
            self.$inner.seek_ceil(text)
        }

        fn seek_exact(&mut self, text: &rucene::util::BytesRef) -> rucene::error::Result<bool> {
            self.$inner.seek_exact(text)
        }

        fn seek_ord(&mut self, ord: i64) -> rucene::error::Result<()> {
            self.$inner.seek_ord(ord)
        }

        fn next(&mut self) -> rucene::error::Result<Option<&rucene::util::BytesRef>> {
            self.$inner.next()
        }

        fn term(&self) -> rucene::error::Result<Option<&rucene::util::BytesRef>> {
            self.$inner.term()
        }

        fn ord(&self) -> rucene::error::Result<i64> {
            self.$inner.ord()
        }

        fn doc_freq(&mut self) -> rucene::error::Result<i32> {
            self.$inner.doc_freq()
        }

        fn total_term_freq(&mut self) -> rucene::error::Result<i64> {
            self.$inner.total_term_freq()
        }

        fn impacts(
            &mut self,
            flags: i32,
        ) -> rucene::error::Result<Box<dyn rucene::index::ImpactsEnum>> {
            self.$inner.impacts(flags)
        }

        fn seek_term_state(
            &mut self,
            text: &rucene::util::BytesRef,
            state: &dyn rucene::index::TermState,
        ) -> rucene::error::Result<()> {
            self.$inner.seek_term_state(text, state)
        }

        fn prepare_seek_exact<'a>(
            &'a mut self,
            text: &rucene::util::BytesRef,
        ) -> rucene::error::Result<Option<Box<dyn rucene::util::IOBooleanSupplier + 'a>>> {
            self.$inner.prepare_seek_exact(text)
        }

        fn term_state(&mut self) -> rucene::error::Result<Box<dyn rucene::index::TermState>> {
            self.$inner.term_state()
        }
    };
}
pub(crate) use forward_terms_enum;