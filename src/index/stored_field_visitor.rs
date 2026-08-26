//! Low-level access to the stored field values of a document.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`StoredFieldVisitor`] | `org.apache.lucene.index.StoredFieldVisitor` |
//! | [`StoredFieldVisitorStatus`] | `org.apache.lucene.index.StoredFieldVisitor.Status` |
//!
//! A [`StoredFieldsReader`](crate::codecs::stored_fields::StoredFieldsReader)
//! decodes one document at a time and pushes every stored value it finds into a
//! visitor. The visitor decides, field by field, whether it wants the value at
//! all, so a caller that needs two fields out of fifty pays for two decodes and
//! forty-eight cheap skips.
//!
//! # Java to Rust adaptations
//!
//! * Java models the visitor as an abstract class whose value callbacks are
//!   empty by default and whose `needsField` is abstract. The Rust equivalent
//!   is a trait with default methods for the callbacks and a single required
//!   method, which reproduces the same contract without inheritance.
//! * `binaryField` is overloaded in Java on `byte[]` and `StoredFieldDataInput`.
//!   Rust has no overloading, so the two become
//!   [`StoredFieldVisitor::binary_field`] and
//!   [`StoredFieldVisitor::binary_field_data_input`]; the second still defaults
//!   to reading every byte and delegating to the first, exactly as Java does.
//! * Java hands `binaryField` a freshly allocated `byte[]` the visitor may keep.
//!   Rust passes a borrowed slice: a visitor that needs to own the bytes copies
//!   them, which is the same allocation Java performs eagerly for every visitor,
//!   including the ones that discard the value.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::index::{FieldInfo, StoredFieldDataInput};

/// Expert: a low-level means of accessing the stored field values in an index.
///
/// Equivalent to `org.apache.lucene.index.StoredFieldVisitor`. See
/// [`StoredFields::document_with_visitor`](crate::index::StoredFields::document_with_visitor).
///
/// **NOTE**: an implementation must not try to load or visit other stored
/// documents of the same reader while it is being called: the stored-fields
/// implementation of most codecs is not reentrant and the result would be
/// undefined.
///
/// See [`DocumentStoredFieldVisitor`](crate::document::DocumentStoredFieldVisitor),
/// the visitor that materialises a [`Document`](crate::document::Document) from
/// the stored fields, used by
/// [`StoredFields::document`](crate::index::StoredFields::document).
pub trait StoredFieldVisitor {
    /// Expert: processes a binary field directly from a [`StoredFieldDataInput`].
    ///
    /// Equivalent to
    /// `StoredFieldVisitor.binaryField(FieldInfo, StoredFieldDataInput)`.
    ///
    /// An implementation **must** consume exactly
    /// [`StoredFieldDataInput::length`] bytes from `value`: the reader shares
    /// the decoding cursor of the whole document with the visitor, so reading
    /// fewer or more bytes desynchronises every field that follows. The default
    /// implementation reads all the bytes into a newly allocated buffer and
    /// calls [`Self::binary_field`].
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::CorruptIndex`] when the declared length is
    /// negative, and propagates any error raised while reading.
    fn binary_field_data_input(
        &mut self,
        field_info: &FieldInfo,
        value: &mut StoredFieldDataInput<'_>,
    ) -> Result<()> {
        let length = value.length();
        if length < 0 {
            return Err(LuceneError::CorruptIndex(format!(
                "stored binary field \"{}\" declares a negative length: {length}",
                field_info.name
            )));
        }
        let length = length as usize;
        let mut data = vec![0u8; length];
        value.data_input().read_bytes(&mut data, 0, length)?;
        self.binary_field(field_info, &data)
    }

    /// Processes a binary field.
    ///
    /// Equivalent to `StoredFieldVisitor.binaryField(FieldInfo, byte[])`. The
    /// default implementation discards the value.
    ///
    /// # Errors
    ///
    /// Propagates any error the implementation raises.
    fn binary_field(&mut self, _field_info: &FieldInfo, _value: &[u8]) -> Result<()> {
        Ok(())
    }

    /// Processes a string field.
    ///
    /// Equivalent to `StoredFieldVisitor.stringField(FieldInfo, String)`.
    ///
    /// # Errors
    ///
    /// Propagates any error the implementation raises.
    fn string_field(&mut self, _field_info: &FieldInfo, _value: &str) -> Result<()> {
        Ok(())
    }

    /// Processes an `int` numeric field.
    ///
    /// Equivalent to `StoredFieldVisitor.intField(FieldInfo, int)`.
    ///
    /// # Errors
    ///
    /// Propagates any error the implementation raises.
    fn int_field(&mut self, _field_info: &FieldInfo, _value: i32) -> Result<()> {
        Ok(())
    }

    /// Processes a `long` numeric field.
    ///
    /// Equivalent to `StoredFieldVisitor.longField(FieldInfo, long)`.
    ///
    /// # Errors
    ///
    /// Propagates any error the implementation raises.
    fn long_field(&mut self, _field_info: &FieldInfo, _value: i64) -> Result<()> {
        Ok(())
    }

    /// Processes a `float` numeric field.
    ///
    /// Equivalent to `StoredFieldVisitor.floatField(FieldInfo, float)`.
    ///
    /// # Errors
    ///
    /// Propagates any error the implementation raises.
    fn float_field(&mut self, _field_info: &FieldInfo, _value: f32) -> Result<()> {
        Ok(())
    }

    /// Processes a `double` numeric field.
    ///
    /// Equivalent to `StoredFieldVisitor.doubleField(FieldInfo, double)`.
    ///
    /// # Errors
    ///
    /// Propagates any error the implementation raises.
    fn double_field(&mut self, _field_info: &FieldInfo, _value: f64) -> Result<()> {
        Ok(())
    }

    /// Hook invoked before a field is processed.
    ///
    /// Equivalent to `StoredFieldVisitor.needsField(FieldInfo)`. The returned
    /// [`StoredFieldVisitorStatus`] tells the reader whether the value must be
    /// decoded, skipped, or whether the document should not be read any
    /// further.
    ///
    /// # Errors
    ///
    /// Propagates any error the implementation raises.
    fn needs_field(&mut self, field_info: &FieldInfo) -> Result<StoredFieldVisitorStatus>;
}

/// Decision returned by [`StoredFieldVisitor::needs_field`].
///
/// Equivalent to `org.apache.lucene.index.StoredFieldVisitor.Status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StoredFieldVisitorStatus {
    /// The field should be visited.
    ///
    /// Equivalent to `Status.YES`.
    Yes,
    /// Do not visit this field, but keep processing the remaining fields of
    /// this document.
    ///
    /// Equivalent to `Status.NO`.
    No,
    /// Do not visit this field and stop processing any other field of this
    /// document.
    ///
    /// Equivalent to `Status.STOP`.
    Stop,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ByteArrayDataInput, DataInput};

    /// A visitor that keeps only what it is asked for and records the calls.
    #[derive(Debug, Default)]
    struct RecordingVisitor {
        wanted: Option<&'static str>,
        stop_at: Option<&'static str>,
        seen: Vec<String>,
    }

    impl StoredFieldVisitor for RecordingVisitor {
        fn binary_field(&mut self, field_info: &FieldInfo, value: &[u8]) -> Result<()> {
            self.seen
                .push(format!("binary {} {value:?}", field_info.name));
            Ok(())
        }

        fn string_field(&mut self, field_info: &FieldInfo, value: &str) -> Result<()> {
            self.seen
                .push(format!("string {} {value}", field_info.name));
            Ok(())
        }

        fn needs_field(&mut self, field_info: &FieldInfo) -> Result<StoredFieldVisitorStatus> {
            if self.stop_at == Some(field_info.name.as_str()) {
                return Ok(StoredFieldVisitorStatus::Stop);
            }
            match self.wanted {
                None => Ok(StoredFieldVisitorStatus::Yes),
                Some(name) if name == field_info.name => Ok(StoredFieldVisitorStatus::Yes),
                Some(_) => Ok(StoredFieldVisitorStatus::No),
            }
        }
    }

    /// A visitor that overrides the data-input callback, as
    /// `SortingStoredFieldsConsumer.CopyVisitor` does in Lucene.
    #[derive(Debug, Default)]
    struct StreamingVisitor {
        copied: Vec<u8>,
        delegated: bool,
    }

    impl StoredFieldVisitor for StreamingVisitor {
        fn binary_field_data_input(
            &mut self,
            _field_info: &FieldInfo,
            value: &mut StoredFieldDataInput<'_>,
        ) -> Result<()> {
            let length = value.length() as usize;
            let mut buffer = vec![0u8; length];
            value.data_input().read_bytes(&mut buffer, 0, length)?;
            self.copied = buffer;
            Ok(())
        }

        fn binary_field(&mut self, _field_info: &FieldInfo, _value: &[u8]) -> Result<()> {
            self.delegated = true;
            Ok(())
        }

        fn needs_field(&mut self, _field_info: &FieldInfo) -> Result<StoredFieldVisitorStatus> {
            Ok(StoredFieldVisitorStatus::Yes)
        }
    }

    #[test]
    fn the_status_values_match_the_java_enum_order() {
        // Lucene serialises nothing from this enum, but a reader switches on
        // it, so the three cases must stay distinct and complete.
        assert_ne!(StoredFieldVisitorStatus::Yes, StoredFieldVisitorStatus::No);
        assert_ne!(StoredFieldVisitorStatus::No, StoredFieldVisitorStatus::Stop);
        assert_ne!(
            StoredFieldVisitorStatus::Yes,
            StoredFieldVisitorStatus::Stop
        );
    }

    #[test]
    fn needs_field_can_select_a_subset() {
        let alpha = FieldInfo::new("alpha", 0);
        let beta = FieldInfo::new("beta", 1);
        let mut visitor = RecordingVisitor {
            wanted: Some("beta"),
            ..Default::default()
        };
        assert_eq!(
            visitor.needs_field(&alpha).unwrap(),
            StoredFieldVisitorStatus::No
        );
        assert_eq!(
            visitor.needs_field(&beta).unwrap(),
            StoredFieldVisitorStatus::Yes
        );
    }

    #[test]
    fn needs_field_can_stop_the_document() {
        let alpha = FieldInfo::new("alpha", 0);
        let mut visitor = RecordingVisitor {
            stop_at: Some("alpha"),
            ..Default::default()
        };
        assert_eq!(
            visitor.needs_field(&alpha).unwrap(),
            StoredFieldVisitorStatus::Stop
        );
    }

    #[test]
    fn the_default_data_input_callback_reads_every_byte_and_delegates() {
        let info = FieldInfo::new("payload", 3);
        let mut input = ByteArrayDataInput::new(vec![1, 2, 3, 4, 5]);
        let mut value = StoredFieldDataInput::new(&mut input, 5);
        let mut visitor = RecordingVisitor::default();
        visitor.binary_field_data_input(&info, &mut value).unwrap();
        assert_eq!(visitor.seen, vec!["binary payload [1, 2, 3, 4, 5]"]);
    }

    #[test]
    fn the_default_data_input_callback_leaves_the_cursor_after_the_value() {
        // The reader shares one cursor across the whole document, so the
        // default implementation must consume exactly `length` bytes.
        let info = FieldInfo::new("payload", 3);
        let mut input = ByteArrayDataInput::new(vec![1, 2, 3, 9, 9]);
        {
            let mut value = StoredFieldDataInput::new(&mut input, 3);
            let mut visitor = RecordingVisitor::default();
            visitor.binary_field_data_input(&info, &mut value).unwrap();
        }
        assert_eq!(input.read_byte().unwrap(), 9);
        assert_eq!(input.read_byte().unwrap(), 9);
    }

    #[test]
    fn a_negative_length_is_rejected_instead_of_panicking() {
        let info = FieldInfo::new("payload", 3);
        let mut input = ByteArrayDataInput::new(vec![1, 2, 3]);
        let mut value = StoredFieldDataInput::new(&mut input, -1);
        let mut visitor = RecordingVisitor::default();
        let error = visitor
            .binary_field_data_input(&info, &mut value)
            .expect_err("a negative length is corrupt");
        assert!(matches!(error, LuceneError::CorruptIndex(_)), "{error:?}");
    }

    #[test]
    fn overriding_the_data_input_callback_bypasses_the_byte_slice_callback() {
        let info = FieldInfo::new("payload", 3);
        let mut input = ByteArrayDataInput::new(vec![7, 8]);
        let mut value = StoredFieldDataInput::new(&mut input, 2);
        let mut visitor = StreamingVisitor::default();
        visitor.binary_field_data_input(&info, &mut value).unwrap();
        assert_eq!(visitor.copied, vec![7, 8]);
        assert!(
            !visitor.delegated,
            "an overriding visitor must not fall back to binary_field"
        );
    }

    #[test]
    fn every_value_callback_defaults_to_discarding_the_value() {
        struct OnlyNeedsField;
        impl StoredFieldVisitor for OnlyNeedsField {
            fn needs_field(&mut self, _info: &FieldInfo) -> Result<StoredFieldVisitorStatus> {
                Ok(StoredFieldVisitorStatus::Yes)
            }
        }
        let info = FieldInfo::new("f", 0);
        let mut visitor = OnlyNeedsField;
        visitor.binary_field(&info, b"x").unwrap();
        visitor.string_field(&info, "x").unwrap();
        visitor.int_field(&info, 1).unwrap();
        visitor.long_field(&info, 1).unwrap();
        visitor.float_field(&info, 1.0).unwrap();
        visitor.double_field(&info, 1.0).unwrap();
    }
}
