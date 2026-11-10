//! A field added to more than one document, or more than once to one.
//!
//! Java adds the same `Field` object to a `Document` any number of times
//! (`TestLatLonShape.doTestContainsGeometryCollectionIntersects`,
//! `TestXYShape.testContainsIndexedGeometryCollection`). A Rucene `Document`
//! owns its boxed fields, so the shared Java object becomes an [`Arc`] that
//! every addition shares, delegating every method to it.

use std::fmt;
use std::sync::Arc;

use rucene::analysis::{Analyzer, TokenStream};
use rucene::document::{InvertableType, NumericValue, SharedReader, StoredValue};
use rucene::error::Result;
use rucene::index::{IndexableField, IndexableFieldType};
use rucene::util::BytesRef;

/// One Java `Field` object shared by several additions.
pub struct SharedField<F: IndexableField + 'static>(pub Arc<F>);

impl<F: IndexableField + 'static> SharedField<F> {
    /// Shares each field of `fields`.
    pub fn share_all(fields: Vec<F>) -> Vec<Arc<F>> {
        fields.into_iter().map(Arc::new).collect()
    }

    /// One more addition of `field`.
    pub fn boxed(field: &Arc<F>) -> Box<dyn IndexableField> {
        Box::new(SharedField(Arc::clone(field)))
    }
}

impl<F: IndexableField + 'static> fmt::Display for SharedField<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&*self.0, f)
    }
}

impl<F: IndexableField + 'static> IndexableField for SharedField<F> {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        self.0.field_type()
    }

    fn token_stream(
        &self,
        analyzer: Option<&dyn Analyzer>,
        reuse: Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        self.0.token_stream(analyzer, reuse)
    }

    fn binary_value(&self) -> Option<BytesRef> {
        self.0.binary_value()
    }

    fn string_value(&self) -> Option<String> {
        self.0.string_value()
    }

    fn char_sequence_value(&self) -> Option<String> {
        self.0.char_sequence_value()
    }

    fn reader_value(&self) -> Option<SharedReader> {
        self.0.reader_value()
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        self.0.numeric_value()
    }

    fn float_vector_value(&self) -> Option<&[f32]> {
        self.0.float_vector_value()
    }

    fn byte_vector_value(&self) -> Option<&[u8]> {
        self.0.byte_vector_value()
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        self.0.stored_value()
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        self.0.invertable_type()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        self.0.as_any()
    }
}
