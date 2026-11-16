//! The generic schema model: how a field of any index is typed, indexed,
//! stored and sorted, and how an input record becomes a `rucene` document.
//!
//! Each index declares its fields once, as a [`EntitySchema`] in
//! [`crate::entities`]. [`Record::to_document`] builds documents from that
//! declaration alone, and [`crate::search`] derives queries and sorts from the
//! same [`FieldKind`]s, so the writer and the searcher cannot disagree.
//!
//! | Kind | Lucene field classes | Stored | Searchable | Sortable |
//! | --- | --- | --- | --- | --- |
//! | [`Exact`](FieldKind::Exact) | `StringField` | yes | exact term, term set | no (no doc values) |
//! | [`Keyword`](FieldKind::Keyword) | `KeywordField` | yes | exact term, term set | `SortedSetSortField` |
//! | [`Flag`](FieldKind::Flag) | `KeywordField`, value normalised to `sim`/`nao` | yes | exact term | `SortedSetSortField` |
//! | [`Text`](FieldKind::Text) | `TextField` (pt-PT analysis) | yes | analysed words, phrase | no |
//! | [`Name`](FieldKind::Name) | `TextField` + `KeywordField` copy `<name>_exact`, not stored | yes | analysed words, phrase; exact on the copy | on the copy |
//! | [`Stored`](FieldKind::Stored) | `StoredField` | yes | no | no |
//! | [`Date`](FieldKind::Date) | `LongField` (epoch ms, not stored) + `StoredField` `YYYY-MM-DD` | yes (text) | exact, range | `SortedNumericSortField` (`LONG`) |
//! | [`Int`](FieldKind::Int) | `IntField` | yes | exact, range | `SortedNumericSortField` (`INT`) |
//! | [`Double`](FieldKind::Double) | `DoubleField` | yes | exact, range | `SortedNumericSortField` (`DOUBLE`) |
//!
//! A field with [`Levels`] also feeds *derived* hierarchy fields
//! ([`FieldSpec::derived`]): not-stored `KeywordField`s holding the code's
//! ancestors (CPV division/group/class/category, NUTS levels 1-3, place
//! country/district/municipality), so `--term cpvDivisao=45` finds every
//! contract in CPV division 45. Derived fields are never read from input.
//!
//! Every document also gets the not-stored `StringField` [`KEY_FIELD`]: the
//! record's key, the term `update`, `delete` and `get` act on.

use rucene::document::NumericValue;
use rucene::document::{
    Document, DoubleField, IntField, KeywordField, LongField, Store, StoredField, StringField,
    TextField,
};

use crate::date::{format_date, parse_date};
use crate::error::{CliError, Result};

/// The not-stored `StringField` holding every document's key.
pub const KEY_FIELD: &str = "_key";

/// The separator of the parts of a composite key (`modificacoes`:
/// `idcontrato/sequencia`).
pub const KEY_SEPARATOR: char = '/';

/// The suffix of the not-stored exact/sort copy of a [`FieldKind::Name`]
/// field.
pub const EXACT_SUFFIX: &str = "_exact";

/// How a field is indexed; each kind maps to fixed Lucene field classes (see
/// the [module documentation](self)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    /// An identifier or foreign key: `StringField`, exact match only, no doc
    /// values (ids are matched, never sorted or aggregated).
    Exact,
    /// A code or closed value list: `KeywordField`, exact match and sort.
    Keyword,
    /// A `Sim`/`Não` (or JSON boolean) flag: `KeywordField` holding `sim` or
    /// `nao`.
    Flag,
    /// Free pt-PT text: `TextField` analysed with
    /// [`PortugueseFoldingAnalyzer`](crate::analysis::PortugueseFoldingAnalyzer).
    Text,
    /// A name or short label: `TextField` plus a not-stored `KeywordField`
    /// copy (`<name>_exact`) for exact match and sorting.
    Name,
    /// Stored only (links): `StoredField`, not searchable.
    Stored,
    /// A calendar day: `LongField` of epoch milliseconds (UTC midnight) plus a
    /// `StoredField` with `YYYY-MM-DD`.
    Date,
    /// A 32-bit integer: `IntField`.
    Int,
    /// A finite floating-point number (money): `DoubleField`.
    Double,
}

impl FieldKind {
    /// Whether values of this kind are text (as opposed to numbers and
    /// dates).
    #[must_use]
    pub fn is_textual(self) -> bool {
        matches!(
            self,
            Self::Exact | Self::Keyword | Self::Flag | Self::Text | Self::Name | Self::Stored
        )
    }

    /// The short name used in schema listings.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Keyword => "keyword",
            Self::Flag => "flag",
            Self::Text => "text",
            Self::Name => "name",
            Self::Stored => "stored",
            Self::Date => "date",
            Self::Int => "int",
            Self::Double => "double",
        }
    }
}

/// A hierarchical code scheme whose ancestors are indexed as derived
/// fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    /// CPV `NNNNNNNN-N`: division, group, class and category are the leading
    /// 2, 3, 4 and 5 digits (Commission Regulation (EC) No 213/2008). A value
    /// of another shape yields no levels.
    Cpv,
    /// NUTS codes (`PT170`): levels 1, 2 and 3 are the leading 3, 4 and 5
    /// characters; a shorter code (`PT`, country level) yields only the
    /// levels it reaches.
    Nuts,
    /// Execution places `Country, District, Municipality`: the
    /// comma-separated path prefixes (`Portugal`, `Portugal, Lisboa`,
    /// `Portugal, Lisboa, Lisboa`).
    Place,
}

impl Scheme {
    /// The ancestor codes of `value`, from the top level down, one per level
    /// the value reaches (at most `count`).
    #[must_use]
    pub fn levels(self, value: &str, count: usize) -> Vec<String> {
        match self {
            Self::Cpv => {
                let bytes = value.as_bytes();
                let shaped = bytes.len() == 10
                    && bytes[..8].iter().all(u8::is_ascii_digit)
                    && bytes[8] == b'-'
                    && bytes[9].is_ascii_digit();
                if !shaped {
                    return Vec::new();
                }
                (2..2 + count).map(|n| value[..n].to_owned()).collect()
            }
            Self::Nuts => {
                if !value.is_ascii() {
                    return Vec::new();
                }
                (3..3 + count)
                    .filter(|&n| n <= value.len())
                    .map(|n| value[..n].to_owned())
                    .collect()
            }
            Self::Place => {
                let parts: Vec<&str> = value
                    .split(',')
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .collect();
                (1..=count.min(parts.len()))
                    .map(|n| parts[..n].join(", "))
                    .collect()
            }
        }
    }
}

/// The derived hierarchy fields a field feeds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Levels {
    /// How ancestors are computed.
    pub scheme: Scheme,
    /// The derived fields, top level first; each must be declared in the
    /// schema with [`FieldSpec::derived`].
    pub fields: &'static [&'static str],
}

/// The declaration of one field of an index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldSpec {
    /// The field name: the JSON member name in input and output and the
    /// Lucene field name.
    pub name: &'static str,
    /// How the field is indexed.
    pub kind: FieldKind,
    /// Whether the field takes several values (a JSON array in input).
    pub multi: bool,
    /// Placeholder values that mean "unknown" in the source and are treated
    /// as absent (no field added). Compared after trimming for text kinds and
    /// numerically for number kinds.
    pub absent: &'static [&'static str],
    /// The derived hierarchy fields this field feeds, if any.
    pub levels: Option<Levels>,
    /// Whether the field is derived (computed from another field's
    /// [`Levels`], never read from input, not stored).
    pub derived: bool,
}

impl FieldSpec {
    /// A single-valued field of `kind`.
    #[must_use]
    pub const fn new(name: &'static str, kind: FieldKind) -> Self {
        Self {
            name,
            kind,
            multi: false,
            absent: &[],
            levels: None,
            derived: false,
        }
    }

    /// A derived hierarchy field (multi-valued, not-stored `KeywordField`).
    #[must_use]
    pub const fn derived(name: &'static str) -> Self {
        Self {
            name,
            kind: FieldKind::Keyword,
            multi: true,
            absent: &[],
            levels: None,
            derived: true,
        }
    }

    /// This field, multi-valued.
    #[must_use]
    pub const fn multi(mut self) -> Self {
        self.multi = true;
        self
    }

    /// This field, with placeholders treated as absent.
    #[must_use]
    pub const fn absent(mut self, absent: &'static [&'static str]) -> Self {
        self.absent = absent;
        self
    }

    /// This field, feeding derived hierarchy fields.
    #[must_use]
    pub const fn levels(mut self, scheme: Scheme, fields: &'static [&'static str]) -> Self {
        self.levels = Some(Levels { scheme, fields });
        self
    }

    /// The field that backs exact queries and sorting on this field: the
    /// `<name>_exact` copy for a [`FieldKind::Name`], the field itself
    /// otherwise.
    #[must_use]
    pub fn exact_field(&self) -> String {
        match self.kind {
            FieldKind::Name => format!("{}{EXACT_SUFFIX}", self.name),
            _ => self.name.to_owned(),
        }
    }
}

/// The declaration of one index: its command and directory name, its
/// fields, its key and its search defaults.
#[derive(Debug, PartialEq, Eq)]
pub struct EntitySchema {
    /// The command name, which is also the fixed name of the index's
    /// subdirectory under the root.
    pub name: &'static str,
    /// A one-line description for the usage text.
    pub summary: &'static str,
    /// Every field, input fields first in documentation order, derived fields
    /// after the field feeding them.
    pub fields: &'static [FieldSpec],
    /// The key fields: one, or several joined by [`KEY_SEPARATOR`].
    pub key: &'static [&'static str],
    /// The fields `--text` and `--phrase` search when no field is named.
    pub text_fields: &'static [&'static str],
    /// The columns `search` prints when `--fields` is not given.
    pub output_fields: &'static [&'static str],
}

impl EntitySchema {
    /// The declaration of field `name`, if declared.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&'static FieldSpec> {
        self.fields.iter().find(|spec| spec.name == name)
    }

    /// The declaration of field `name`.
    ///
    /// # Errors
    ///
    /// Returns [`CliError::Input`] listing the fields of the index when
    /// `name` is not one.
    pub fn require_field(&self, name: &str) -> Result<&'static FieldSpec> {
        self.field(name).ok_or_else(|| {
            let known: Vec<&str> = self.fields.iter().map(|spec| spec.name).collect();
            CliError::Input(format!(
                "{}: unknown field {name:?}; known fields: {}",
                self.name,
                known.join(", ")
            ))
        })
    }

    /// The stored fields (every non-derived field), in declaration order.
    pub fn stored_fields(&self) -> impl Iterator<Item = &'static FieldSpec> + '_ {
        self.fields.iter().filter(|spec| !spec.derived)
    }

    /// A human-readable form of the key: `idcontrato`, or
    /// `idcontrato/sequencia`.
    #[must_use]
    pub fn key_label(&self) -> String {
        self.key.join(&KEY_SEPARATOR.to_string())
    }
}

/// A scalar input value, before it is checked against a field's kind.
#[derive(Clone, Debug, PartialEq)]
pub enum Raw {
    /// JSON `null`.
    Null,
    /// A JSON boolean.
    Bool(bool),
    /// A JSON number, as written.
    Number(String),
    /// A JSON string, or a `--set` value.
    Text(String),
}

/// The input of one field: a scalar or a list (JSON array).
#[derive(Clone, Debug, PartialEq)]
pub enum RawField {
    /// One value.
    One(Raw),
    /// Several values.
    List(Vec<Raw>),
}

/// A typed field value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A value of a textual kind (flags already normalised).
    String(String),
    /// A `Date` value, in epoch milliseconds.
    Date(i64),
    /// An `Int` value.
    Int(i32),
    /// A `Double` value (always finite).
    Double(f64),
}

impl Value {
    /// Parses `raw` as a value of `spec`; `Ok(None)` when it is absent
    /// (`null`, empty or whitespace-only text, or one of the field's
    /// placeholders).
    ///
    /// # Errors
    ///
    /// Returns [`CliError::Input`] naming the field when `raw` does not fit
    /// the kind: a number that does not parse, a date in neither accepted
    /// form, a flag other than `Sim`/`Não`/boolean, a boolean on a number
    /// field.
    pub fn parse(spec: &FieldSpec, raw: &Raw) -> Result<Option<Self>> {
        let invalid =
            |what: &str| CliError::Input(format!("field {:?}: {raw:?} is not {what}", spec.name));
        let text = match raw {
            Raw::Null => return Ok(None),
            Raw::Bool(value) => {
                return match spec.kind {
                    FieldKind::Flag => Ok(Some(Self::String(flag(*value).to_owned()))),
                    kind if kind.is_textual() => Ok(Some(Self::String(value.to_string()))),
                    _ => Err(invalid("a number or a date")),
                };
            }
            Raw::Number(text) | Raw::Text(text) => text.trim(),
        };
        if text.is_empty() {
            return Ok(None);
        }
        let value = Self::parse_text(spec, text).map_err(|error| match error {
            CliError::Input(message) => {
                CliError::Input(format!("field {:?}: {message}", spec.name))
            }
            other => other,
        })?;
        for placeholder in spec.absent {
            if spec.kind.is_textual() {
                if text == *placeholder {
                    return Ok(None);
                }
            } else if Self::parse_text(spec, placeholder).ok().as_ref() == Some(&value) {
                return Ok(None);
            }
        }
        Ok(Some(value))
    }

    /// Parses trimmed, non-empty text as a value of `spec`, without the
    /// placeholder check.
    fn parse_text(spec: &FieldSpec, text: &str) -> Result<Self> {
        match spec.kind {
            FieldKind::Exact
            | FieldKind::Keyword
            | FieldKind::Text
            | FieldKind::Name
            | FieldKind::Stored => Ok(Self::String(text.to_owned())),
            FieldKind::Flag => match crate::analysis::fold(text).to_lowercase().as_str() {
                "sim" | "true" => Ok(Self::String(flag(true).to_owned())),
                "nao" | "false" => Ok(Self::String(flag(false).to_owned())),
                _ => Err(CliError::Input(format!(
                    "{text:?} is not a flag (Sim, Não, true or false)"
                ))),
            },
            FieldKind::Date => parse_date(text).map(Self::Date),
            FieldKind::Int => text
                .parse::<i32>()
                .map(Self::Int)
                .map_err(|_| CliError::Input(format!("{text:?} is not a 32-bit integer"))),
            FieldKind::Double => text
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .map(Self::Double)
                .ok_or_else(|| CliError::Input(format!("{text:?} is not a finite number"))),
        }
    }

    /// Parses a query value given on the command line for `spec`.
    ///
    /// # Errors
    ///
    /// As [`Value::parse`], and [`CliError::Input`] when the value is empty or
    /// a placeholder (placeholders are never indexed, so they match nothing).
    pub fn parse_query(spec: &FieldSpec, raw: &str) -> Result<Self> {
        Self::parse(spec, &Raw::Text(raw.to_owned()))?.ok_or_else(|| {
            CliError::Input(format!(
                "field {:?}: {raw:?} is empty or a placeholder, which is never indexed",
                spec.name
            ))
        })
    }

    /// The text form of the value: dates as `YYYY-MM-DD`, numbers in Rust's
    /// shortest round-trip form.
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            Self::String(value) => value.clone(),
            Self::Date(millis) => format_date(*millis),
            Self::Int(value) => value.to_string(),
            Self::Double(value) => value.to_string(),
        }
    }
}

/// The normalised indexed value of a flag.
#[must_use]
pub fn flag(value: bool) -> &'static str {
    if value {
        "sim"
    } else {
        "nao"
    }
}

/// One input record of an index: the typed values of its fields and its
/// key.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    schema: &'static EntitySchema,
    /// The values of each field, indexed like `schema.fields`.
    values: Vec<Vec<Value>>,
    key: String,
}

impl Record {
    /// Builds a record of `schema` from `(field, input)` pairs. Absent values
    /// are dropped; a list on a multi-valued field keeps its order.
    ///
    /// # Errors
    ///
    /// Returns [`CliError::Input`] when a field is unknown, derived, repeated,
    /// given a list while single-valued, or holds an invalid value, and when a
    /// key field is missing, or (composite key) contains the key separator.
    pub fn from_fields<'a>(
        schema: &'static EntitySchema,
        fields: impl IntoIterator<Item = (&'a str, RawField)>,
    ) -> Result<Self> {
        let mut values: Vec<Vec<Value>> = vec![Vec::new(); schema.fields.len()];
        let mut seen = vec![false; schema.fields.len()];
        for (name, input) in fields {
            let spec = schema.require_field(name)?;
            let position = schema
                .fields
                .iter()
                .position(|candidate| candidate.name == spec.name)
                .expect("INVARIANT: require_field found the field in this schema");
            if spec.derived {
                return Err(CliError::Input(format!(
                    "field {name:?} is derived by the CLI and cannot be given"
                )));
            }
            if std::mem::replace(&mut seen[position], true) {
                return Err(CliError::Input(format!("field {name:?} given twice")));
            }
            let raws = match input {
                RawField::One(raw) => vec![raw],
                RawField::List(raws) if spec.multi => raws,
                RawField::List(_) => {
                    return Err(CliError::Input(format!(
                        "field {name:?} is single-valued but was given a list"
                    )))
                }
            };
            for raw in &raws {
                if let Some(value) = Value::parse(spec, raw)? {
                    values[position].push(value);
                }
            }
        }
        let key = Self::compute_key(schema, &values)?;
        Ok(Self {
            schema,
            values,
            key,
        })
    }

    fn compute_key(schema: &EntitySchema, values: &[Vec<Value>]) -> Result<String> {
        let mut parts = Vec::with_capacity(schema.key.len());
        for name in schema.key {
            let position = schema
                .fields
                .iter()
                .position(|spec| spec.name == *name)
                .expect("INVARIANT: key fields are declared (checked by the schema tests)");
            let part = match values[position].as_slice() {
                [value] => value.display(),
                _ => {
                    return Err(CliError::Input(format!(
                        "the key field {name:?} is required and must not be empty or a placeholder"
                    )))
                }
            };
            if schema.key.len() > 1 && part.contains(KEY_SEPARATOR) {
                return Err(CliError::Input(format!(
                    "key field {name:?} must not contain {KEY_SEPARATOR:?}: {part:?}"
                )));
            }
            parts.push(part);
        }
        Ok(parts.join(&KEY_SEPARATOR.to_string()))
    }

    /// The record's key (the value of [`KEY_FIELD`]).
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The values of field `name` (empty when absent or unknown).
    #[must_use]
    pub fn values(&self, name: &str) -> &[Value] {
        self.schema
            .fields
            .iter()
            .position(|spec| spec.name == name)
            .map_or(&[], |position| self.values[position].as_slice())
    }

    /// Builds the `rucene` document of this record, per its schema.
    ///
    /// # Errors
    ///
    /// Propagates any error a `rucene` field constructor raises.
    pub fn to_document(&self) -> Result<Document> {
        let mut document = Document::new();
        document.add(Box::new(StringField::new(
            KEY_FIELD,
            self.key.clone(),
            Store::NO,
        )?));
        for (spec, values) in self.schema.fields.iter().zip(&self.values) {
            for value in values {
                add_value(&mut document, spec, value)?;
            }
            if let Some(levels) = spec.levels {
                let mut per_level: Vec<Vec<String>> = vec![Vec::new(); levels.fields.len()];
                for value in values {
                    let codes = levels.scheme.levels(&value.display(), levels.fields.len());
                    for (level, code) in codes.into_iter().enumerate() {
                        if !per_level[level].contains(&code) {
                            per_level[level].push(code);
                        }
                    }
                }
                for (field, codes) in levels.fields.iter().zip(per_level) {
                    for code in codes {
                        document.add(Box::new(KeywordField::new(field, code, Store::NO)?));
                    }
                }
            }
        }
        Ok(document)
    }
}

fn add_value(document: &mut Document, spec: &FieldSpec, value: &Value) -> Result<()> {
    let name = spec.name;
    match (spec.kind, value) {
        (FieldKind::Exact, Value::String(text)) => {
            document.add(Box::new(StringField::new(name, text.clone(), Store::YES)?));
        }
        (FieldKind::Keyword | FieldKind::Flag, Value::String(text)) => {
            document.add(Box::new(KeywordField::new(name, text.clone(), Store::YES)?));
        }
        (FieldKind::Text, Value::String(text)) => {
            document.add(Box::new(TextField::new(name, text.clone(), Store::YES)?));
        }
        (FieldKind::Name, Value::String(text)) => {
            document.add(Box::new(TextField::new(name, text.clone(), Store::YES)?));
            document.add(Box::new(KeywordField::new(
                &spec.exact_field(),
                text.clone(),
                Store::NO,
            )?));
        }
        (FieldKind::Stored, Value::String(text)) => {
            document.add(Box::new(StoredField::new_string(name, text.clone())?));
        }
        (FieldKind::Date, Value::Date(millis)) => {
            document.add(Box::new(LongField::new(name, *millis, Store::NO)));
            document.add(Box::new(StoredField::new_string(
                name,
                format_date(*millis),
            )?));
        }
        (FieldKind::Int, Value::Int(number)) => {
            document.add(Box::new(IntField::new(name, *number, Store::YES)));
        }
        (FieldKind::Double, Value::Double(number)) => {
            document.add(Box::new(DoubleField::new(name, *number, Store::YES)));
        }
        (kind, value) => unreachable!("Value::parse gave {value:?} for a {kind:?} field"),
    }
    Ok(())
}

/// A stored value read back from a document.
#[derive(Clone, Debug, PartialEq)]
pub enum StoredValue {
    /// A textual value (dates included, as `YYYY-MM-DD`).
    Text(String),
    /// A number, in its shortest round-trip text form.
    Number(String),
}

impl StoredValue {
    /// The text of the value.
    #[must_use]
    pub fn text(&self) -> &str {
        match self {
            Self::Text(text) | Self::Number(text) => text,
        }
    }
}

/// Reads the stored values of `spec` from a stored document, in indexing
/// order; empty when the document has none.
#[must_use]
pub fn stored_values(document: &Document, spec: &FieldSpec) -> Vec<StoredValue> {
    document
        .get_fields_by_name(spec.name)
        .into_iter()
        .filter_map(|field| match spec.kind {
            FieldKind::Int | FieldKind::Double => field.numeric_value().map(|number| {
                StoredValue::Number(match number {
                    NumericValue::Int(value) => value.to_string(),
                    NumericValue::Long(value) => value.to_string(),
                    NumericValue::Float(value) => value.to_string(),
                    NumericValue::Double(value) => value.to_string(),
                })
            }),
            _ => field.string_value().map(StoredValue::Text),
        })
        .collect()
}

/// Writes the stored fields of `document` as one JSON object in the bulk
/// input format: fields in schema order, multi-valued fields as arrays,
/// numbers as JSON numbers, absent fields omitted.
#[must_use]
pub fn document_to_json(schema: &EntitySchema, document: &Document) -> String {
    let mut out = String::from("{");
    let mut first = true;
    for spec in schema.stored_fields() {
        let values = stored_values(document, spec);
        if values.is_empty() {
            continue;
        }
        if !first {
            out.push_str(", ");
        }
        first = false;
        crate::json::write_string(&mut out, spec.name);
        out.push_str(": ");
        let write = |out: &mut String, value: &StoredValue| match value {
            StoredValue::Number(text) => out.push_str(text),
            StoredValue::Text(text) => crate::json::write_string(out, text),
        };
        if spec.multi {
            out.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write(&mut out, value);
            }
            out.push(']');
        } else {
            write(&mut out, &values[0]);
        }
    }
    out.push('}');
    out
}

/// Renders the schema of an index as a Markdown table: one row per field,
/// with its kind, Lucene field classes, whether it is stored, how it is
/// searchable and sortable, and its placeholders.
///
/// `rucene-base <entity> help` prints it, and the crate README holds the
/// table of every index verbatim (a test checks they match).
#[must_use]
pub fn markdown_table(schema: &EntitySchema) -> String {
    let mut out = String::from(
        "| Field | Kind | Lucene fields | Stored | Searchable | Sortable | Absent when |\n\
         | --- | --- | --- | --- | --- | --- | --- |\n",
    );
    for spec in schema.fields {
        let name = if spec.multi && !spec.derived {
            format!("`{}[]`", spec.name)
        } else {
            format!("`{}`", spec.name)
        };
        let mut kind = spec.kind.label().to_owned();
        if spec.derived {
            kind.push_str(" (derived)");
        } else if schema.key.contains(&spec.name) {
            kind.push_str(" (key)");
        }
        let lucene = if spec.derived {
            let source = schema
                .fields
                .iter()
                .find(|other| {
                    other
                        .levels
                        .is_some_and(|levels| levels.fields.contains(&spec.name))
                })
                .map_or("?", |other| other.name);
            format!("`KeywordField`, from `{source}`")
        } else {
            match spec.kind {
                FieldKind::Exact => "`StringField`".to_owned(),
                FieldKind::Keyword => "`KeywordField`".to_owned(),
                FieldKind::Flag => "`KeywordField` (`sim`/`nao`)".to_owned(),
                FieldKind::Text => "`TextField`".to_owned(),
                FieldKind::Name => {
                    format!("`TextField` + `KeywordField` `{}`", spec.exact_field())
                }
                FieldKind::Stored => "`StoredField`".to_owned(),
                FieldKind::Date => "`LongField` (epoch ms) + `StoredField`".to_owned(),
                FieldKind::Int => "`IntField`".to_owned(),
                FieldKind::Double => "`DoubleField`".to_owned(),
            }
        };
        let stored = if spec.derived { "no" } else { "yes" };
        let (searchable, sortable) = match spec.kind {
            FieldKind::Exact => ("exact, set", "no"),
            FieldKind::Keyword | FieldKind::Flag => ("exact, set", "yes"),
            FieldKind::Text => ("words, phrase", "no"),
            FieldKind::Name => ("words, phrase; exact, set", "yes"),
            FieldKind::Stored => ("no", "no"),
            FieldKind::Date | FieldKind::Int | FieldKind::Double => ("exact, set, range", "yes"),
        };
        let absent: Vec<String> = spec
            .absent
            .iter()
            .map(|placeholder| format!("`{placeholder}`"))
            .collect();
        out.push_str(&format!(
            "| {name} | {kind} | {lucene} | {stored} | {searchable} | {sortable} | {} |\n",
            absent.join(", ")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpv_levels_need_the_full_shape() {
        assert_eq!(
            Scheme::Cpv.levels("45234100-7", 4),
            vec!["45", "452", "4523", "45234"]
        );
        assert!(Scheme::Cpv.levels("45234100", 4).is_empty());
        assert!(Scheme::Cpv.levels("4523410a-7", 4).is_empty());
    }

    #[test]
    fn nuts_levels_stop_at_the_code_length() {
        assert_eq!(
            Scheme::Nuts.levels("PT170", 3),
            vec!["PT1", "PT17", "PT170"]
        );
        assert!(Scheme::Nuts.levels("PT", 3).is_empty());
        assert_eq!(Scheme::Nuts.levels("PT11", 3), vec!["PT1", "PT11"]);
    }

    #[test]
    fn place_levels_are_path_prefixes() {
        assert_eq!(
            Scheme::Place.levels("Portugal, Lisboa, Lisboa", 3),
            vec!["Portugal", "Portugal, Lisboa", "Portugal, Lisboa, Lisboa"]
        );
        assert_eq!(Scheme::Place.levels("Portugal", 3), vec!["Portugal"]);
        assert_eq!(
            Scheme::Place.levels("Espanha ,Galiza", 3),
            vec!["Espanha", "Espanha, Galiza"]
        );
    }

    #[test]
    fn placeholders_are_absent_and_flags_normalise() {
        let price = FieldSpec::new("p", FieldKind::Double).absent(&["0"]);
        assert_eq!(
            Value::parse(&price, &Raw::Number("0.0".into())).unwrap(),
            None
        );
        assert_eq!(
            Value::parse(&price, &Raw::Number("-1".into())).unwrap(),
            Some(Value::Double(-1.0))
        );
        let nif = FieldSpec::new("n", FieldKind::Exact).absent(&["-"]);
        assert_eq!(Value::parse(&nif, &Raw::Text(" - ".into())).unwrap(), None);
        let term = FieldSpec::new("t", FieldKind::Int).absent(&["99999999"]);
        assert_eq!(
            Value::parse(&term, &Raw::Text("99999999".into())).unwrap(),
            None
        );
        assert_eq!(
            Value::parse(&term, &Raw::Text("480".into())).unwrap(),
            Some(Value::Int(480))
        );
        let flag_spec = FieldSpec::new("f", FieldKind::Flag);
        for (raw, expected) in [
            (Raw::Text("Sim".into()), "sim"),
            (Raw::Text("NÃO".into()), "nao"),
            (Raw::Text("nao".into()), "nao"),
            (Raw::Bool(true), "sim"),
            (Raw::Bool(false), "nao"),
        ] {
            assert_eq!(
                Value::parse(&flag_spec, &raw).unwrap(),
                Some(Value::String(expected.into()))
            );
        }
        assert!(Value::parse(&flag_spec, &Raw::Text("talvez".into())).is_err());
        assert!(Value::parse(&term, &Raw::Number("1.5".into())).is_err());
        assert!(Value::parse(&term, &Raw::Bool(true)).is_err());
        assert!(Value::parse(&price, &Raw::Text("NaN".into())).is_err());
    }
}
