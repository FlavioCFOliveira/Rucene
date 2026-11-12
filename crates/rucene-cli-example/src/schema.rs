//! The index schema: the single source of truth for how each USGS column is
//! indexed, stored and sorted.
//!
//! Every column of the USGS Earthquake Catalog CSV is declared once in
//! [`FIELDS`], with a [`FieldKind`] that fixes the Lucene field classes used
//! for it. [`Record::to_document`] turns a record into a `rucene`
//! [`Document`] from that declaration alone, and the search side
//! ([`crate::search`]) derives its query and sort construction from the same
//! kinds, so the two can never disagree.
//!
//! | Kind | Lucene field classes | Stored | Searchable | Sortable |
//! | --- | --- | --- | --- | --- |
//! | [`Keyword`](FieldKind::Keyword) | `KeywordField` | yes | exact term, term set | `SortedSetSortField` |
//! | [`Text`](FieldKind::Text) | `TextField` (+ `KeywordField` copy `place_exact`, not stored) | yes | analysed terms, phrase; exact on the copy | on the copy |
//! | [`Date`](FieldKind::Date) | `LongField` (epoch ms) | yes | exact, range | `SortedNumericSortField` (`LONG`) |
//! | [`Double`](FieldKind::Double) | `DoubleField` | yes | exact, range | `SortedNumericSortField` (`DOUBLE`) |
//! | [`Int`](FieldKind::Int) | `IntField` | yes | exact, range | `SortedNumericSortField` (`INT`) |
//!
//! When a record has both `latitude` and `longitude`, it also gets the
//! derived field [`LOCATION_FIELD`]: a `LatLonPoint` (box and distance
//! queries) and a `LatLonDocValuesField` (distance sort).

use std::collections::BTreeMap;

use rucene::document::geo_fields::{LatLonDocValuesField, LatLonPoint};
use rucene::document::NumericValue;
use rucene::document::{
    Document, DoubleField, IntField, KeywordField, LongField, Store, TextField,
};

use crate::datetime::{format_iso8601, parse_iso8601};
use crate::error::{CliError, Result};

/// How a column is indexed; each kind maps to fixed Lucene field classes
/// (see the [module documentation](self)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    /// An exact string: `KeywordField`, stored.
    Keyword,
    /// Free text analysed with `StandardAnalyzer`: `TextField`, stored, plus a
    /// `KeywordField` copy named by [`exact_field_of`] for exact match and
    /// sorting.
    Text,
    /// An instant, ISO 8601 in the CSV: `LongField` of epoch milliseconds,
    /// stored.
    Date,
    /// A floating-point number: `DoubleField`, stored.
    Double,
    /// An integer: `IntField`, stored.
    Int,
}

/// The declaration of one USGS column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldSpec {
    /// The column name in the USGS CSV header, which is also the Lucene field
    /// name.
    pub name: &'static str,
    /// How the column is indexed.
    pub kind: FieldKind,
}

const fn spec(name: &'static str, kind: FieldKind) -> FieldSpec {
    FieldSpec { name, kind }
}

/// Every USGS column, in the order of the USGS CSV header.
pub const FIELDS: &[FieldSpec] = &[
    spec("time", FieldKind::Date),
    spec("latitude", FieldKind::Double),
    spec("longitude", FieldKind::Double),
    spec("depth", FieldKind::Double),
    spec("mag", FieldKind::Double),
    spec("magType", FieldKind::Keyword),
    spec("nst", FieldKind::Int),
    spec("gap", FieldKind::Double),
    spec("dmin", FieldKind::Double),
    spec("rms", FieldKind::Double),
    spec("net", FieldKind::Keyword),
    spec("id", FieldKind::Keyword),
    spec("updated", FieldKind::Date),
    spec("place", FieldKind::Text),
    spec("type", FieldKind::Keyword),
    spec("horizontalError", FieldKind::Double),
    spec("depthError", FieldKind::Double),
    spec("magError", FieldKind::Double),
    spec("magNst", FieldKind::Int),
    spec("status", FieldKind::Keyword),
    spec("locationSource", FieldKind::Keyword),
    spec("magSource", FieldKind::Keyword),
];

/// The unique key of a document: the term `update` and `delete` act on.
pub const ID_FIELD: &str = "id";

/// The derived geo field (`LatLonPoint` + `LatLonDocValuesField`) built from
/// `latitude` and `longitude`.
pub const LOCATION_FIELD: &str = "location";

/// The `Text` column; its exact copy is [`PLACE_EXACT_FIELD`].
pub const PLACE_FIELD: &str = "place";

/// The not-stored `KeywordField` copy of `place`, for exact match and sort.
pub const PLACE_EXACT_FIELD: &str = "place_exact";

/// Returns the declaration of column `name`, or `None` when it is not a USGS
/// column.
#[must_use]
pub fn field(name: &str) -> Option<&'static FieldSpec> {
    FIELDS.iter().find(|spec| spec.name == name)
}

/// Returns the declaration of column `name`.
///
/// # Errors
///
/// Returns [`CliError::Input`] listing the known columns when `name` is not
/// one.
pub fn require_field(name: &str) -> Result<&'static FieldSpec> {
    field(name).ok_or_else(|| {
        let known: Vec<&str> = FIELDS.iter().map(|spec| spec.name).collect();
        CliError::Input(format!(
            "unknown field {name:?}; known fields: {}",
            known.join(", ")
        ))
    })
}

/// The name of the exact keyword field that backs term queries and sorting
/// on `spec`: the field itself, except `place_exact` for the text column.
#[must_use]
pub fn exact_field_of(spec: &FieldSpec) -> &'static str {
    match spec.kind {
        FieldKind::Text => PLACE_EXACT_FIELD,
        _ => spec.name,
    }
}

/// A typed column value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A `Keyword` or `Text` value.
    String(String),
    /// A `Date` value, in epoch milliseconds.
    Date(i64),
    /// A `Double` value (always finite).
    Double(f64),
    /// An `Int` value.
    Int(i32),
}

impl Value {
    /// Parses the text form of a value of `spec`'s kind.
    ///
    /// # Errors
    ///
    /// Returns [`CliError::Input`] naming the field when `raw` does not parse
    /// as the field's kind, when a double is not finite, or when `latitude` /
    /// `longitude` fall outside their valid range.
    pub fn parse(spec: &FieldSpec, raw: &str) -> Result<Self> {
        let invalid =
            |what: &str| CliError::Input(format!("field {:?}: {raw:?} is not {what}", spec.name));
        match spec.kind {
            FieldKind::Keyword | FieldKind::Text => Ok(Self::String(raw.to_owned())),
            FieldKind::Date => parse_iso8601(raw)
                .map(Self::Date)
                .map_err(|error| error.context(&format!("field {:?}", spec.name))),
            FieldKind::Int => raw
                .parse::<i32>()
                .map(Self::Int)
                .map_err(|_| invalid("a 32-bit integer")),
            FieldKind::Double => {
                let value = raw
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| invalid("a finite number"))?;
                let limit = match spec.name {
                    "latitude" => Some(90.0),
                    "longitude" => Some(180.0),
                    _ => None,
                };
                if let Some(limit) = limit {
                    if !(-limit..=limit).contains(&value) {
                        return Err(invalid(&format!("within [-{limit}, {limit}]")));
                    }
                }
                Ok(Self::Double(value))
            }
        }
    }

    /// The text form of the value: as the USGS CSV writes it for strings and
    /// dates, Rust's shortest round-trip form for numbers.
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            Self::String(value) => value.clone(),
            Self::Date(millis) => format_iso8601(*millis),
            Self::Double(value) => value.to_string(),
            Self::Int(value) => value.to_string(),
        }
    }
}

/// One earthquake: the non-empty typed values of its columns.
///
/// An empty CSV value is simply absent from the record, so the document gets
/// no field for it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Record {
    values: BTreeMap<&'static str, Value>,
}

impl Record {
    /// Builds a record from `(column, raw value)` pairs; empty values are
    /// skipped.
    ///
    /// # Errors
    ///
    /// Returns [`CliError::Input`] when a column is unknown or repeated, when
    /// a value does not parse, or when the record has no `id`.
    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Result<Self> {
        let mut values = BTreeMap::new();
        let mut seen = Vec::new();
        for (name, raw) in pairs {
            let spec = require_field(name)?;
            if seen.contains(&spec.name) {
                return Err(CliError::Input(format!("field {name:?} given twice")));
            }
            seen.push(spec.name);
            if raw.is_empty() {
                continue;
            }
            values.insert(spec.name, Value::parse(spec, raw)?);
        }
        if !values.contains_key(ID_FIELD) {
            return Err(CliError::Input(format!(
                "the {ID_FIELD:?} field is required and must not be empty"
            )));
        }
        Ok(Self { values })
    }

    /// The value of column `name`, if present.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.values.get(name)
    }

    /// The document id (always present).
    #[must_use]
    pub fn id(&self) -> &str {
        match self.values.get(ID_FIELD) {
            Some(Value::String(id)) => id,
            _ => unreachable!("Record::from_pairs requires a string id"),
        }
    }

    /// Builds the `rucene` document of this record, per [`FIELDS`].
    ///
    /// # Errors
    ///
    /// Propagates any error a `rucene` field constructor raises.
    pub fn to_document(&self) -> Result<Document> {
        let mut document = Document::new();
        for (&name, value) in &self.values {
            match value {
                Value::String(text) => {
                    let spec = require_field(name)?;
                    if spec.kind == FieldKind::Text {
                        document.add(Box::new(TextField::new(name, text.clone(), Store::YES)?));
                        document.add(Box::new(KeywordField::new(
                            exact_field_of(spec),
                            text.clone(),
                            Store::NO,
                        )?));
                    } else {
                        document.add(Box::new(KeywordField::new(name, text.clone(), Store::YES)?));
                    }
                }
                Value::Date(millis) => {
                    document.add(Box::new(LongField::new(name, *millis, Store::YES)));
                }
                Value::Double(number) => {
                    document.add(Box::new(DoubleField::new(name, *number, Store::YES)));
                }
                Value::Int(number) => {
                    document.add(Box::new(IntField::new(name, *number, Store::YES)));
                }
            }
        }
        if let (Some(Value::Double(latitude)), Some(Value::Double(longitude))) =
            (self.get("latitude"), self.get("longitude"))
        {
            document.add(Box::new(LatLonPoint::new(
                LOCATION_FIELD,
                *latitude,
                *longitude,
            )?));
            document.add(Box::new(LatLonDocValuesField::new(
                LOCATION_FIELD,
                *latitude,
                *longitude,
            )?));
        }
        Ok(document)
    }
}

/// Parses USGS CSV text (header line plus data lines) into records.
///
/// The header may hold any subset of the USGS columns, in any order, but must
/// hold `id`.
///
/// # Errors
///
/// Returns [`CliError::Input`], with the CSV line number, when the CSV does
/// not parse, a column is unknown, a record has the wrong number of fields,
/// or a value is invalid.
pub fn parse_csv(text: &str) -> Result<Vec<Record>> {
    let mut rows = crate::csv::parse(text)?.into_iter();
    let header = rows
        .next()
        .ok_or_else(|| CliError::Input("CSV input is empty (no header line)".to_owned()))?;
    for name in &header.fields {
        require_field(name).map_err(|error| error.context("CSV header"))?;
    }
    if !header.fields.iter().any(|name| name == ID_FIELD) {
        return Err(CliError::Input(format!(
            "CSV header has no {ID_FIELD:?} column"
        )));
    }
    rows.map(|row| {
        if row.fields.len() != header.fields.len() {
            return Err(CliError::Input(format!(
                "CSV line {}: {} fields, the header has {}",
                row.line,
                row.fields.len(),
                header.fields.len()
            )));
        }
        Record::from_pairs(
            header
                .fields
                .iter()
                .map(String::as_str)
                .zip(row.fields.iter().map(String::as_str)),
        )
        .map_err(|error| error.context(&format!("CSV line {}", row.line)))
    })
    .collect()
}

/// Reads the stored value of column `spec` from a stored document, in the
/// text form of [`Value::display`]; `None` when the document has no value.
#[must_use]
pub fn stored_value(document: &Document, spec: &FieldSpec) -> Option<String> {
    let field = document.get_field(spec.name)?;
    match spec.kind {
        FieldKind::Keyword | FieldKind::Text => field.string_value(),
        FieldKind::Date | FieldKind::Double | FieldKind::Int => {
            Some(match field.numeric_value()? {
                NumericValue::Long(millis) if spec.kind == FieldKind::Date => {
                    format_iso8601(millis)
                }
                NumericValue::Long(value) => value.to_string(),
                NumericValue::Int(value) => value.to_string(),
                NumericValue::Double(value) => value.to_string(),
                NumericValue::Float(value) => value.to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_column_is_declared_once() {
        for (index, spec) in FIELDS.iter().enumerate() {
            assert_eq!(field(spec.name), Some(spec));
            assert!(!FIELDS[..index].iter().any(|other| other.name == spec.name));
        }
        assert_eq!(FIELDS.len(), 22);
    }

    #[test]
    fn empty_values_are_absent_and_bad_values_are_named() {
        let record = Record::from_pairs([("id", "a"), ("mag", ""), ("nst", "12")]).unwrap();
        assert_eq!(record.get("mag"), None);
        assert_eq!(record.get("nst"), Some(&Value::Int(12)));
        let error = Record::from_pairs([("id", "a"), ("mag", "big")]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "field \"mag\": \"big\" is not a finite number"
        );
        assert!(Record::from_pairs([("mag", "1")]).is_err());
        assert!(Record::from_pairs([("id", "a"), ("latitude", "91")]).is_err());
        assert!(Record::from_pairs([("id", "a"), ("bogus", "1")]).is_err());
    }
}
