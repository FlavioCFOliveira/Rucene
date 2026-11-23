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

//! Port of `TokenSources.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use std::sync::Arc;

use rucene::analysis::{Analyzer, SharedTokenStream, TokenStream};
use rucene::document::Document;
use rucene::error::{LuceneError, Result};
use rucene::index::{Fields, IndexReader, Terms};

use crate::highlight::limit_token_offset_filter::LimitTokenOffsetFilter;
use crate::highlight::token_stream_from_term_vector::TokenStreamFromTermVector;
use crate::java::utf16_len;

/// Convenience methods for obtaining a `TokenStream` for use with the
/// `Highlighter` - can obtain from term vectors with offsets and positions or
/// from an Analyzer re-parsing the stored content.
///
/// Equivalent to `org.apache.lucene.search.highlight.TokenSources`. Java's
/// overloads are distinct functions here, named after their arguments.
#[derive(Debug)]
pub struct TokenSources {
    _private: (),
}

fn analyzer_stream(
    analyzer: &dyn Analyzer,
    field: &str,
    text: &str,
) -> Result<Box<dyn TokenStream>> {
    Ok(Box::new(SharedTokenStream::new(
        analyzer.token_stream_from_str(field, text)?,
    )))
}

impl TokenSources {
    /// Get a token stream from either un-inverting a term vector if possible,
    /// or by analyzing the text.
    ///
    /// WARNING: Don't call this if there is more than one value for this
    /// field. If there are, and if there are term vectors, then there is a
    /// single tokenstream with offsets suggesting all the field values were
    /// concatenated.
    ///
    /// Equivalent to the static `getTokenStream(String, Fields, String,
    /// Analyzer, int)`; `tv_fields` is `None` for Java's `null`, and a
    /// negative `max_start_offset` means no limit.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn get_token_stream(
        field: &str,
        tv_fields: Option<&dyn Fields>,
        text: &str,
        analyzer: &dyn Analyzer,
        max_start_offset: i32,
    ) -> Result<Box<dyn TokenStream>> {
        if let Some(token_stream) =
            Self::get_term_vector_token_stream_or_null(field, tv_fields, max_start_offset)?
        {
            return Ok(token_stream);
        }
        let mut token_stream = analyzer_stream(analyzer, field, text)?;
        if max_start_offset >= 0 && max_start_offset < utf16_len(text) - 1 {
            token_stream = Box::new(LimitTokenOffsetFilter::new(token_stream, max_start_offset)?);
        }
        Ok(token_stream)
    }

    /// Get a token stream by un-inverting the term vector. This method
    /// returns `None` if `tv_fields` is `None` or if the field has no term
    /// vector, or if the term vector doesn't have offsets. Positions are
    /// recommended on the term vector but it isn't strictly required.
    ///
    /// Equivalent to the static `getTermVectorTokenStreamOrNull(String,
    /// Fields, int)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn get_term_vector_token_stream_or_null(
        field: &str,
        tv_fields: Option<&dyn Fields>,
        max_start_offset: i32,
    ) -> Result<Option<Box<dyn TokenStream>>> {
        let Some(tv_fields) = tv_fields else {
            return Ok(None);
        };
        let tv_terms = tv_fields.terms(field)?;
        let Some(tv_terms) = tv_terms.filter(|t| t.has_offsets()) else {
            return Ok(None);
        };
        Ok(Some(Box::new(TokenStreamFromTermVector::new(
            tv_terms,
            max_start_offset,
        )?)))
    }

    /// A convenience method that tries to first get a
    /// `TokenStreamFromTermVector` for the specified docId, then, falls back
    /// to using the passed in `Document` to retrieve the TokenStream. This is
    /// useful when you already have the document, but would prefer to use
    /// the vector first.
    ///
    /// Equivalent to the deprecated static `getAnyTokenStream(IndexReader,
    /// int, String, Document, Analyzer)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors, and the `IllegalArgumentException` of
    /// [`get_token_stream_from_document`](Self::get_token_stream_from_document).
    #[deprecated(note = "maintenance reasons LUCENE-6445")]
    pub fn get_any_token_stream_with_document(
        reader: &dyn IndexReader,
        doc_id: i32,
        field: &str,
        document: &Document,
        analyzer: &dyn Analyzer,
    ) -> Result<Box<dyn TokenStream>> {
        let mut ts: Option<Box<dyn TokenStream>> = None;

        if let Some(vectors) = reader.term_vectors()?.get(doc_id)? {
            if let Some(vector) = vectors.terms(field)? {
                #[allow(deprecated)]
                {
                    ts = Some(Self::get_token_stream_from_terms(vector)?);
                }
            }
        }

        // No token info stored so fall back to analyzing raw content
        match ts {
            Some(ts) => Ok(ts),
            #[allow(deprecated)]
            None => Self::get_token_stream_from_document(document, field, analyzer),
        }
    }

    /// A convenience method that tries a number of approaches to getting a
    /// token stream. The cost of finding there are no termVectors in the
    /// index is minimal (1000 invocations still registers 0 ms). So this
    /// "lazy" (flexible?) approach to coding is probably acceptable.
    ///
    /// Equivalent to the deprecated static `getAnyTokenStream(IndexReader,
    /// int, String, Analyzer)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors, and the `IllegalArgumentException` of an
    /// unstored field.
    #[deprecated(note = "maintenance reasons LUCENE-6445")]
    pub fn get_any_token_stream(
        reader: &dyn IndexReader,
        doc_id: i32,
        field: &str,
        analyzer: &dyn Analyzer,
    ) -> Result<Box<dyn TokenStream>> {
        let mut ts: Option<Box<dyn TokenStream>> = None;

        if let Some(vectors) = reader.term_vectors()?.get(doc_id)? {
            if let Some(vector) = vectors.terms(field)? {
                #[allow(deprecated)]
                {
                    ts = Some(Self::get_token_stream_from_terms(vector)?);
                }
            }
        }

        // No token info stored so fall back to analyzing raw content
        match ts {
            Some(ts) => Ok(ts),
            #[allow(deprecated)]
            None => Self::get_token_stream_from_reader(reader, doc_id, field, analyzer),
        }
    }

    /// Equivalent to the deprecated static `getTokenStream(Terms, boolean)`.
    ///
    /// # Errors
    ///
    /// As [`get_token_stream_from_terms`](Self::get_token_stream_from_terms).
    #[deprecated(note = "maintenance reasons LUCENE-6445")]
    pub fn get_token_stream_from_terms_contiguous(
        vector: Arc<dyn Terms>,
        _token_positions_guaranteed_contiguous: bool,
    ) -> Result<Box<dyn TokenStream>> {
        #[allow(deprecated)]
        Self::get_token_stream_from_terms(vector)
    }

    /// Returns a token stream generated from a `Terms`. This can be used to
    /// feed the highlighter with a pre-parsed token stream. The `Terms` must
    /// have offsets available. If there are no positions available, all
    /// tokens will have position increments reflecting adjacent tokens, or
    /// coincident when terms share a start offset. If there are stopwords
    /// filtered from the index, you probably want to ensure term vectors have
    /// positions so that phrase queries won't match across stopwords.
    ///
    /// Equivalent to the deprecated static `getTokenStream(Terms)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` if no offsets are available.
    #[deprecated(note = "maintenance reasons LUCENE-6445")]
    pub fn get_token_stream_from_terms(tpv: Arc<dyn Terms>) -> Result<Box<dyn TokenStream>> {
        if !tpv.has_offsets() {
            return Err(LuceneError::illegal_argument(
                "Highlighting requires offsets from the TokenStream.",
            ));
            // TokenStreamFromTermVector can handle a lack of offsets if there
            // are positions. But highlighters require offsets, so we insist
            // here.
        }

        Ok(Box::new(TokenStreamFromTermVector::new(tpv, -1)?)) // TODO propagate maxStartOffset; see LUCENE-6445
    }

    /// Returns a `TokenStream` with positions and offsets constructed from
    /// field termvectors. If the field has no termvectors or offsets are not
    /// included in the termvector, return `None`.
    ///
    /// Equivalent to the deprecated static `getTokenStreamWithOffsets(IndexReader,
    /// int, String)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    #[deprecated(note = "maintenance reasons LUCENE-6445")]
    pub fn get_token_stream_with_offsets(
        reader: &dyn IndexReader,
        doc_id: i32,
        field: &str,
    ) -> Result<Option<Box<dyn TokenStream>>> {
        let Some(vectors) = reader.term_vectors()?.get(doc_id)? else {
            return Ok(None);
        };

        let Some(vector) = vectors.terms(field)? else {
            return Ok(None);
        };

        if !vector.has_offsets() {
            return Ok(None);
        }

        #[allow(deprecated)]
        Ok(Some(Self::get_token_stream_from_terms(vector)?))
    }

    /// Equivalent to the deprecated static `getTokenStream(IndexReader, int,
    /// String, Analyzer)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors, and the `IllegalArgumentException` of an
    /// unstored field.
    #[deprecated(note = "maintenance reasons LUCENE-6445")]
    pub fn get_token_stream_from_reader(
        reader: &dyn IndexReader,
        doc_id: i32,
        field: &str,
        analyzer: &dyn Analyzer,
    ) -> Result<Box<dyn TokenStream>> {
        let doc = reader.stored_fields()?.document(doc_id)?;
        #[allow(deprecated)]
        Self::get_token_stream_from_document(&doc, field, analyzer)
    }

    /// Equivalent to the deprecated static `getTokenStream(Document, String,
    /// Analyzer)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the field is not stored.
    #[deprecated(note = "maintenance reasons LUCENE-6445")]
    pub fn get_token_stream_from_document(
        doc: &Document,
        field: &str,
        analyzer: &dyn Analyzer,
    ) -> Result<Box<dyn TokenStream>> {
        let Some(contents) = doc.get(field) else {
            return Err(LuceneError::illegal_argument(format!(
                "Field {field} in document is not stored and cannot be analyzed"
            )));
        };
        #[allow(deprecated)]
        Self::get_token_stream_from_contents(field, &contents, analyzer)
    }

    /// Equivalent to the deprecated static `getTokenStream(String, String,
    /// Analyzer)`.
    ///
    /// # Errors
    ///
    /// What the analyzer raises.
    #[deprecated(note = "maintenance reasons LUCENE-6445")]
    pub fn get_token_stream_from_contents(
        field: &str,
        contents: &str,
        analyzer: &dyn Analyzer,
    ) -> Result<Box<dyn TokenStream>> {
        analyzer_stream(analyzer, field, contents)
    }
}
