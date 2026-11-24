// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.Properties`.

use crate::r#impl::number::decimal_format_properties::{DecimalFormatProperties, PropertyValue};

/// `serialVersionUID`.
pub const SERIAL_VERSION_UID: i64 = 4095518955889349243;

/// ICU 59 created a new DecimalFormat implementation, and the serialized
/// form of the old one was `com.ibm.icu.impl.number.Properties`; this class
/// is kept so that objects serialized by ICU 59 still deserialize.
///
/// Equivalent to `com.ibm.icu.impl.number.Properties` (`Serializable`); its
/// serialized form is the one of `DecimalFormatProperties`.
#[derive(Debug, Clone, Default)]
pub struct Properties {
    instance: Option<DecimalFormatProperties>,
}

impl Properties {
    /// Equivalent to `getInstance()`.
    pub fn get_instance(&self) -> Option<&DecimalFormatProperties> {
        self.instance.as_ref()
    }

    /// Equivalent to `readObject(ObjectInputStream)`: creates the instance
    /// when absent and reads it (`readObjectImpl`).
    ///
    /// # Errors
    ///
    /// The errors of `DecimalFormatProperties.readObjectImpl`.
    pub fn read_object(
        &mut self,
        entries: Vec<(String, Option<PropertyValue>)>,
    ) -> rucene::error::Result<()> {
        self.instance
            .get_or_insert_with(DecimalFormatProperties::new)
            .read_object_impl(entries)
    }

    /// Equivalent to `writeObject(ObjectOutputStream)`: creates the instance
    /// when absent and writes it (`writeObjectImpl`).
    pub fn write_object(&mut self) -> Vec<(String, Option<PropertyValue>)> {
        self.instance
            .get_or_insert_with(DecimalFormatProperties::new)
            .write_object_impl()
    }
}
