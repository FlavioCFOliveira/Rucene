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

//! The types declared in `DocMaker.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use std::any::Any;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use rucene::document::{
    DoublePoint, Field, FieldType, FieldValueSetters, FloatPoint, IntPoint, LongPoint, Store,
    StringField, TextField,
};
use rucene::error::Result;
use rucene::index::{IndexOptions, IndexableField};
use rucene::util::java_random::JavaRandom;
use rucene_queryparser::jdk::calendar::{self, Calendar};
use rucene_queryparser::jdk::date::Date;
use rucene_queryparser::jdk::format::ParsePosition;
use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::simple_date_format::SimpleDateFormat;
use rucene_queryparser::jdk::time_zone::TimeZone;

use crate::by_task::feeds::content_source::ContentSource;
use crate::by_task::feeds::doc_data::DocData;
use crate::by_task::utils::config::Config;
use crate::jdk::{self, ThreadLocal};

pub use rucene::document::Document;

/// `DocMaker.LeftOver`.
#[derive(Debug, Default)]
struct LeftOver {
    /// `docdata`: `None` stands for the thread's `DocState.docData` object
    /// (Java's alias of it); `Some` for a `DocData` of its own.
    docdata: Option<DocData>,
    cnt: i32,
}

/// A field `DocState` keeps: Java's `Field` and the subclasses it stores
/// (`StringField`, `IntPoint`, `LongPoint`, `FloatPoint`, `DoublePoint`).
#[derive(Clone)]
pub enum DocField {
    /// `new Field(name, "", ft)`.
    Field(Arc<Field>),
    /// `new StringField(name, "", Field.Store.YES)`.
    String(Arc<StringField>),
    /// `new IntPoint(name, 0)`.
    Int(Arc<IntPoint>),
    /// `new LongPoint(name, 0L)`.
    Long(Arc<LongPoint>),
    /// `new FloatPoint(name, 0.0F)`.
    Float(Arc<FloatPoint>),
    /// `new DoublePoint(name, 0.0)`.
    Double(Arc<DoublePoint>),
}

impl DocField {
    /// The field as the document holds it.
    pub fn indexable(&self) -> Arc<dyn IndexableField> {
        match self {
            DocField::Field(f) => Arc::clone(f) as Arc<dyn IndexableField>,
            DocField::String(f) => Arc::clone(f) as Arc<dyn IndexableField>,
            DocField::Int(f) => Arc::clone(f) as Arc<dyn IndexableField>,
            DocField::Long(f) => Arc::clone(f) as Arc<dyn IndexableField>,
            DocField::Float(f) => Arc::clone(f) as Arc<dyn IndexableField>,
            DocField::Double(f) => Arc::clone(f) as Arc<dyn IndexableField>,
        }
    }

    fn setters(&self) -> &dyn FieldValueSetters {
        match self {
            DocField::Field(f) => &**f,
            DocField::String(f) => &**f,
            DocField::Int(f) => &**f,
            DocField::Long(f) => &**f,
            DocField::Float(f) => &**f,
            DocField::Double(f) => &**f,
        }
    }

    /// `setStringValue(String)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the field holds another type.
    pub fn set_string_value(&self, value: String) -> Result<()> {
        self.setters().set_string_value(value)
    }

    /// `setLongValue(long)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the field holds another type.
    pub fn set_long_value(&self, value: i64) -> Result<()> {
        self.setters().set_long_value(value)
    }

    /// `setIntValue(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the field holds another type.
    pub fn set_int_value(&self, value: i32) -> Result<()> {
        self.setters().set_int_value(value)
    }

    /// `setBytesValue(byte[])`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the field holds another type.
    pub fn set_bytes_value(&self, value: &[u8]) -> Result<()> {
        self.setters()
            .set_bytes_value(rucene::util::BytesRef::new(value.to_vec()))
    }
}

/// The `Class<? extends Number>` `DocState.getNumericField` takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericType {
    /// `Integer.class`.
    Integer,
    /// `Long.class`.
    Long,
    /// `Float.class`.
    Float,
    /// `Double.class`.
    Double,
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.DocMaker.DocState`:
/// document state, supports reuse of field instances across documents (see
/// `reuseFields` parameter).
pub struct DocState {
    fields: Option<HashMap<String, DocField>>,
    numeric_fields: Option<HashMap<String, DocField>>,
    reuse_fields: bool,
    /// `doc`.
    pub(crate) doc: Option<Document>,
    /// `docData`.
    pub doc_data: DocData,
}

impl DocState {
    /// `DocState(boolean reuseFields, FieldType ft, FieldType bodyFt)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of building a field.
    pub fn new(reuse_fields: bool, ft: &Arc<FieldType>, body_ft: &Arc<FieldType>) -> Result<Self> {
        if reuse_fields {
            let mut fields = HashMap::new();
            let mut numeric_fields = HashMap::new();
            // Initialize the map with the default fields.
            fields.insert(
                DocMaker::BODY_FIELD.to_string(),
                DocField::Field(Arc::new(Field::new(
                    DocMaker::BODY_FIELD,
                    String::new(),
                    Arc::clone(body_ft),
                )?)),
            );
            fields.insert(
                DocMaker::TITLE_FIELD.to_string(),
                DocField::Field(Arc::new(Field::new(
                    DocMaker::TITLE_FIELD,
                    String::new(),
                    Arc::clone(ft),
                )?)),
            );
            fields.insert(
                DocMaker::DATE_FIELD.to_string(),
                DocField::Field(Arc::new(Field::new(
                    DocMaker::DATE_FIELD,
                    String::new(),
                    Arc::clone(ft),
                )?)),
            );
            fields.insert(
                DocMaker::ID_FIELD.to_string(),
                DocField::String(Arc::new(StringField::new(
                    DocMaker::ID_FIELD,
                    String::new(),
                    Store::YES,
                )?)),
            );
            fields.insert(
                DocMaker::NAME_FIELD.to_string(),
                DocField::Field(Arc::new(Field::new(
                    DocMaker::NAME_FIELD,
                    String::new(),
                    Arc::clone(ft),
                )?)),
            );
            numeric_fields.insert(
                DocMaker::DATE_MSEC_FIELD.to_string(),
                DocField::Long(Arc::new(LongPoint::new(DocMaker::DATE_MSEC_FIELD, &[0])?)),
            );
            numeric_fields.insert(
                DocMaker::TIME_SEC_FIELD.to_string(),
                DocField::Int(Arc::new(IntPoint::new(DocMaker::TIME_SEC_FIELD, &[0])?)),
            );
            Ok(DocState {
                fields: Some(fields),
                numeric_fields: Some(numeric_fields),
                reuse_fields,
                doc: Some(Document::new()),
                doc_data: DocData::new(),
            })
        } else {
            Ok(DocState {
                fields: None,
                numeric_fields: None,
                reuse_fields,
                doc: None,
                doc_data: DocData::new(),
            })
        }
    }

    /// `getField(String name, FieldType ft)`: a field corresponding to the
    /// field name. If `reuseFields` was set to true, then it attempts to
    /// reuse a Field instance. If such a field does not exist, it creates a
    /// new one.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of building the field.
    pub fn get_field(&mut self, name: &str, ft: &Arc<FieldType>) -> Result<DocField> {
        if !self.reuse_fields {
            return Ok(DocField::Field(Arc::new(Field::new(
                name.to_string(),
                String::new(),
                Arc::clone(ft),
            )?)));
        }
        let fields = self.fields.as_mut().expect("reuseFields");
        if let Some(f) = fields.get(name) {
            return Ok(f.clone());
        }
        let f = DocField::Field(Arc::new(Field::new(
            name.to_string(),
            String::new(),
            Arc::clone(ft),
        )?));
        fields.insert(name.to_string(), f.clone());
        Ok(f)
    }

    /// `getNumericField(String name, Class<? extends Number> numericType)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of building the field.
    pub fn get_numeric_field(&mut self, name: &str, numeric_type: NumericType) -> Result<DocField> {
        let existing = if self.reuse_fields {
            self.numeric_fields
                .as_ref()
                .and_then(|m| m.get(name).cloned())
        } else {
            None
        };
        if let Some(f) = existing {
            return Ok(f);
        }
        let f = match numeric_type {
            NumericType::Integer => DocField::Int(Arc::new(IntPoint::new(name.to_string(), &[0])?)),
            NumericType::Long => DocField::Long(Arc::new(LongPoint::new(name.to_string(), &[0])?)),
            NumericType::Float => {
                DocField::Float(Arc::new(FloatPoint::new(name.to_string(), &[0.0])?))
            }
            NumericType::Double => {
                DocField::Double(Arc::new(DoublePoint::new(name.to_string(), &[0.0])?))
            }
        };
        if self.reuse_fields {
            self.numeric_fields
                .as_mut()
                .expect("reuseFields")
                .insert(name.to_string(), f.clone());
        }
        Ok(f)
    }
}

/// `DocMaker.DateUtil`.
struct DateUtil {
    parser: SimpleDateFormat,
    cal: Calendar,
    pos: ParsePosition,
}

impl DateUtil {
    fn new() -> Result<Self> {
        let parser = SimpleDateFormat::new("dd-MMM-yyyy HH:mm:ss", &Locale::english())?;
        parser.set_lenient(true);
        Ok(DateUtil {
            parser,
            cal: Calendar::get_instance(&TimeZone::get_time_zone("GMT"), &Locale::root())?,
            pos: ParsePosition::new(0),
        })
    }
}

/// The settings `setConfig` computes.
struct Settings {
    config: Option<Arc<Config>>,
    val_type: Arc<FieldType>,
    body_val_type: Arc<FieldType>,
    source: Option<Arc<dyn ContentSource>>,
    reuse_fields: bool,
    index_properties: bool,
    store_bytes: bool,
    update_doc_id_limit: i32,
    r: Option<JavaRandom>,
    doc_state: Arc<ThreadLocal<DocState>>,
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.DocMaker`
/// (`public class DocMaker implements Closeable`): creates [`Document`]
/// objects. Uses a [`ContentSource`] to generate [`DocData`] objects.
/// Supports the following parameters:
///
/// * **content.source** - specifies the [`ContentSource`] class to use
///   (default **SingleDocSource**).
/// * **doc.stored** - specifies whether fields should be stored (default
///   **false**).
/// * **doc.body.stored** - specifies whether the body field should be stored
///   (default = **doc.stored**).
/// * **doc.tokenized** - specifies whether fields should be tokenized
///   (default **true**).
/// * **doc.body.tokenized** - specifies whether the body field should be
///   tokenized (default = **doc.tokenized**).
/// * **doc.body.offsets** - specifies whether to add offsets into the
///   postings index for the body field. It is useful for highlighting.
///   (default **false**)
/// * **doc.tokenized.norms** - specifies whether norms should be stored in
///   the index or not. (default **false**).
/// * **doc.body.tokenized.norms** - specifies whether norms should be stored
///   in the index for the body field. This can be set to true, while
///   `doc.tokenized.norms` is set to false, to allow norms storing just for
///   the body field. (default **true**).
/// * **doc.term.vector** - specifies whether term vectors should be stored
///   for fields (default **false**).
/// * **doc.term.vector.positions** - specifies whether term vectors should
///   be stored with positions (default **false**).
/// * **doc.term.vector.offsets** - specifies whether term vectors should be
///   stored with offsets (default **false**).
/// * **doc.store.body.bytes** - specifies whether to store the raw bytes of
///   the document's content in the document (default **false**).
/// * **doc.reuse.fields** - specifies whether Field and Document objects
///   should be reused (default **true**).
/// * **doc.index.props** - specifies whether the properties returned by
///   [`DocData::get_props`] will be indexed. (default **false**).
/// * **doc.random.id.limit** - if specified, docs will be assigned random
///   IDs from 0 to this limit. This is useful with UpdateDoc for testing
///   performance of IndexWriter.updateDocument.
///
/// A doc maker is shared by every thread of a run; its per-thread state
/// (`leftovr`, `docState`, `dateParsers`) is thread-local, as in Java.
pub struct DocMaker {
    settings: Mutex<Settings>,
    /// `leftovr`: leftovers are thread local, because it is unsafe to share
    /// residues between threads.
    leftovr: ThreadLocal<LeftOver>,
    date_parsers: ThreadLocal<DateUtil>,
    num_docs_created: AtomicI32,
}

impl Default for DocMaker {
    fn default() -> Self {
        Self::new()
    }
}

impl DocMaker {
    /// `BODY_FIELD`.
    pub const BODY_FIELD: &'static str = "body";
    /// `TITLE_FIELD`.
    pub const TITLE_FIELD: &'static str = "doctitle";
    /// `DATE_FIELD`.
    pub const DATE_FIELD: &'static str = "docdate";
    /// `DATE_MSEC_FIELD`.
    pub const DATE_MSEC_FIELD: &'static str = "docdatenum";
    /// `TIME_SEC_FIELD`.
    pub const TIME_SEC_FIELD: &'static str = "doctimesecnum";
    /// `ID_FIELD`.
    pub const ID_FIELD: &'static str = "docid";
    /// `BYTES_FIELD`.
    pub const BYTES_FIELD: &'static str = "bytes";
    /// `NAME_FIELD`.
    pub const NAME_FIELD: &'static str = "docname";

    /// `new DocMaker()`.
    pub fn new() -> Self {
        DocMaker {
            settings: Mutex::new(Settings {
                config: None,
                val_type: Arc::new(TextField::type_not_stored().clone()),
                body_val_type: Arc::new(TextField::type_not_stored().clone()),
                source: None,
                reuse_fields: false,
                index_properties: false,
                store_bytes: false,
                update_doc_id_limit: 0,
                r: None,
                doc_state: Arc::new(ThreadLocal::new()),
            }),
            leftovr: ThreadLocal::new(),
            date_parsers: ThreadLocal::new(),
            num_docs_created: AtomicI32::new(0),
        }
    }

    fn settings(&self) -> MutexGuard<'_, Settings> {
        jdk::lock(&self.settings)
    }

    /// `source`.
    pub fn source(&self) -> Arc<dyn ContentSource> {
        Arc::clone(
            self.settings()
                .source
                .as_ref()
                .expect("setConfig was called"),
        )
    }

    /// `config`.
    pub fn config(&self) -> Option<Arc<Config>> {
        self.settings().config.clone()
    }

    /// `createDocument(DocData docData, int size, int cnt)`: create a doc;
    /// use only part of the body, modify it to keep the rest (or use all if
    /// size==0); reset the docdata properties so they are not added more
    /// than once.
    fn create_document(
        &self,
        ds: &mut DocState,
        doc_data: &mut DocData,
        mut size: i32,
        cnt: i32,
    ) -> Result<Document> {
        let (val_type, body_val_type, reuse_fields, index_properties, store_bytes) = {
            let s = self.settings();
            (
                Arc::clone(&s.val_type),
                Arc::clone(&s.body_val_type),
                s.reuse_fields,
                s.index_properties,
                s.store_bytes,
            )
        };
        let mut doc = if reuse_fields {
            ds.doc.take().expect("reuseFields")
        } else {
            Document::new()
        };
        doc.clear();

        // Set ID_FIELD
        let ft = FieldType::new_from(&*val_type);
        ft.set_stored(true)?;
        let id_field = ds.get_field(Self::ID_FIELD, &Arc::new(ft))?;
        let id = {
            let mut s = self.settings();
            let limit = s.update_doc_id_limit;
            match s.r.as_mut() {
                Some(r) => r.next_int_bound(limit),
                None => {
                    let id = doc_data.get_id();
                    if id == -1 {
                        self.num_docs_created.fetch_add(1, Ordering::SeqCst)
                    } else {
                        id
                    }
                }
            }
        };
        id_field.set_string_value(id.to_string())?;
        doc.add_shared(id_field.indexable());

        // Set NAME_FIELD
        let name = doc_data.get_name().unwrap_or("").to_string();
        let name = if cnt < 0 {
            name
        } else {
            format!("{name}_{cnt}")
        };
        let name_field = ds.get_field(Self::NAME_FIELD, &val_type)?;
        name_field.set_string_value(name)?;
        doc.add_shared(name_field.indexable());

        // Set DATE_FIELD
        let util = match self.date_parsers.get() {
            Some(util) => util,
            None => self
                .date_parsers
                .set(Some(DateUtil::new()?))
                .expect("just set"),
        };
        let mut util = jdk::lock(&util);
        let mut date: Option<Date> = None;
        let date_string = match doc_data.get_date() {
            Some(date_string) => {
                util.pos.index = 0;
                let DateUtil { parser, pos, .. } = &mut *util;
                date = parser.parse_position(date_string, pos)?;
                date_string.to_string()
            }
            None => String::new(),
        };
        let date_string_field = ds.get_field(Self::DATE_FIELD, &val_type)?;
        date_string_field.set_string_value(date_string)?;
        doc.add_shared(date_string_field.indexable());

        // just set to right now
        let date = date.unwrap_or_else(Date::now);

        let date_field = ds.get_numeric_field(Self::DATE_MSEC_FIELD, NumericType::Long)?;
        date_field.set_long_value(date.get_time())?;
        doc.add_shared(date_field.indexable());

        util.cal.set_time(date);
        let sec = util.cal.get(calendar::HOUR_OF_DAY)? * 3600
            + util.cal.get(calendar::MINUTE)? * 60
            + util.cal.get(calendar::SECOND)?;
        drop(util);

        let time_sec_field = ds.get_numeric_field(Self::TIME_SEC_FIELD, NumericType::Integer)?;
        time_sec_field.set_int_value(sec)?;
        doc.add_shared(time_sec_field.indexable());

        // Set TITLE_FIELD
        // The field holds the Java String; Lucene indexes it as
        // `UnicodeUtil.UTF16toUTF8` of it.
        let title = doc_data.get_title_string().unwrap_or_default();
        let title_field = ds.get_field(Self::TITLE_FIELD, &val_type)?;
        title_field.set_string_value(title)?;
        doc.add_shared(title_field.indexable());

        if let Some(body) = doc_data.get_body().filter(|b| !b.is_empty()) {
            let body: Vec<u16> = body.to_vec();
            let bdy: &[u16];
            if size <= 0 || size as usize >= body.len() {
                bdy = &body; // use all
                doc_data.set_body_utf16(Some(Vec::new())); // nothing left
            } else {
                // attempt not to break words - if whitespace found within next 20 chars...
                let mut n = size - 1;
                while n < size + 20 && (n as usize) < body.len() {
                    if rucene_queryparser::jdk::character::is_whitespace(i32::from(
                        body[n as usize],
                    )) {
                        size = n;
                        break;
                    }
                    n += 1;
                }
                bdy = &body[..size as usize]; // use part
                doc_data.set_body_utf16(Some(body[size as usize..].to_vec())); // some left
            }
            let body_field = ds.get_field(Self::BODY_FIELD, &body_val_type)?;
            body_field.set_string_value(jdk::utf16_to_string(bdy))?;
            doc.add_shared(body_field.indexable());

            if store_bytes {
                let stored = Arc::new(StringField::type_stored().clone());
                let bytes_field = ds.get_field(Self::BYTES_FIELD, &stored)?;
                bytes_field.set_bytes_value(&jdk::string_get_bytes_utf8(bdy))?;
                doc.add_shared(bytes_field.indexable());
            }
        }

        if index_properties {
            if let Some(props) = doc_data.get_props() {
                for (key, value) in props.entries() {
                    let f = ds.get_field(key, &val_type)?;
                    f.set_string_value(value.to_string())?;
                    doc.add_shared(f.indexable());
                }
                doc_data.set_props(None);
            }
        }

        // The document Java returns is the reused object itself: hand out a
        // document holding the same fields, and keep the reused one.
        let mut result = Document::new();
        for f in doc.get_fields() {
            result.add_shared(Arc::clone(f));
        }
        if reuse_fields {
            ds.doc = Some(doc);
        }
        Ok(result)
    }

    /// `resetLeftovers()`.
    fn reset_leftovers(&self) {
        self.leftovr.set(None);
    }

    /// `getDocState()`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of building the reused fields.
    pub fn get_doc_state(&self) -> Result<Arc<Mutex<DocState>>> {
        let (tl, reuse_fields, val_type, body_val_type) = {
            let s = self.settings();
            (
                Arc::clone(&s.doc_state),
                s.reuse_fields,
                Arc::clone(&s.val_type),
                Arc::clone(&s.body_val_type),
            )
        };
        match tl.get() {
            Some(ds) => Ok(ds),
            None => {
                let ds = DocState::new(reuse_fields, &val_type, &body_val_type)?;
                Ok(tl.set(Some(ds)).expect("just set"))
            }
        }
    }

    /// `makeDocument()`: creates a [`Document`] object ready for indexing.
    /// This method uses the [`ContentSource`] to get the next document from
    /// the source, and creates a [`Document`] object from the returned
    /// fields. If `reuseFields` was set to true, it will reuse [`Document`]
    /// and `Field` instances.
    ///
    /// # Errors
    ///
    /// The `NoMoreDataException` or `IOException` of the source.
    pub fn make_document(&self) -> Result<Document> {
        self.reset_leftovers();
        let ds = self.get_doc_state()?;
        let mut ds = jdk::lock(&ds);
        let source = self.source();
        source.get_next_doc_data(&mut ds.doc_data)?;
        let mut doc_data = std::mem::take(&mut ds.doc_data);
        let doc = self.create_document(&mut ds, &mut doc_data, 0, -1);
        ds.doc_data = doc_data;
        doc
    }

    /// `makeDocument(int size)`: same as [`make_document`](Self::make_document),
    /// only this method creates a document of the given size input by
    /// `size`.
    ///
    /// # Errors
    ///
    /// As [`make_document`](Self::make_document).
    pub fn make_document_of_size(&self, size: i32) -> Result<Document> {
        let lvr = self.leftovr.get();
        let lvr_usable = lvr.as_ref().is_some_and(|l| {
            let l = jdk::lock(l);
            match &l.docdata {
                Some(dd) => dd.get_body().is_some_and(|b| !b.is_empty()),
                None => true,
            }
        });
        let ds_cell = self.get_doc_state()?;
        let mut ds = jdk::lock(&ds_cell);
        if lvr.is_none() || !lvr_usable {
            self.reset_leftovers();
        }
        let source = self.source();
        // `dd`: `None` while it is the DocState's own `docData` object.
        let mut dd: Option<DocData>;
        let mut cnt;
        match &lvr {
            None => {
                source.get_next_doc_data(&mut ds.doc_data)?;
                dd = None;
                cnt = 0;
            }
            Some(l) => {
                let mut l = jdk::lock(l);
                dd = l.docdata.take();
                cnt = l.cnt;
                if dd.is_some() {
                    // Java's `lvr.docdata` and `dd` are one object.
                    l.docdata = None;
                }
            }
        }
        loop {
            let body = match &dd {
                Some(d) => d.get_body(),
                None => ds.doc_data.get_body(),
            };
            if body.is_some_and(|b| b.len() >= size as usize) {
                break;
            }
            let dd2_body = body.map(<[u16]>::to_vec);
            let mut next = DocData::new();
            source.get_next_doc_data(&mut next)?;
            cnt = 0;
            // `dd2.getBody() + dd.getBody()`: Java string concatenation, a
            // `null` operand appending "null".
            const NULL: [u16; 4] = [b'n' as u16, b'u' as u16, b'l' as u16, b'l' as u16];
            let mut joined = dd2_body.unwrap_or_else(|| NULL.to_vec());
            joined.extend_from_slice(next.get_body().unwrap_or(&NULL));
            next.set_body_utf16(Some(joined));
            dd = Some(next);
        }
        let doc = match dd.as_mut() {
            Some(d) => self.create_document(&mut ds, d, size, cnt)?,
            None => {
                let mut own = std::mem::take(&mut ds.doc_data);
                let doc = self.create_document(&mut ds, &mut own, size, cnt);
                ds.doc_data = own;
                doc?
            }
        };
        let remaining_empty = match &dd {
            Some(d) => d.get_body().is_none_or(<[u16]>::is_empty),
            None => ds.doc_data.get_body().is_none_or(<[u16]>::is_empty),
        };
        if remaining_empty {
            self.reset_leftovers();
        } else {
            let lvr = match lvr {
                Some(l) => l,
                None => self
                    .leftovr
                    .set(Some(LeftOver::default()))
                    .expect("just set"),
            };
            let mut l = jdk::lock(&lvr);
            l.docdata = dd;
            cnt += 1;
            l.cnt = cnt;
        }
        Ok(doc)
    }

    /// `setConfig(Config config, ContentSource source)`: set the
    /// configuration parameters of this doc maker.
    ///
    /// # Errors
    ///
    /// The exception of reading the configuration.
    pub fn set_config(&self, config: Arc<Config>, source: Arc<dyn ContentSource>) -> Result<()> {
        let stored = config.get_bool("doc.stored", false)?;
        let body_stored = config.get_bool("doc.body.stored", stored)?;
        let tokenized = config.get_bool("doc.tokenized", true)?;
        let body_tokenized = config.get_bool("doc.body.tokenized", tokenized)?;
        let norms = config.get_bool("doc.tokenized.norms", false)?;
        let body_norms = config.get_bool("doc.body.tokenized.norms", true)?;
        let body_offsets = config.get_bool("doc.body.offsets", false)?;
        let term_vec = config.get_bool("doc.term.vector", false)?;
        let term_vec_positions = config.get_bool("doc.term.vector.positions", false)?;
        let term_vec_offsets = config.get_bool("doc.term.vector.offsets", false)?;

        let val_type = FieldType::new_from(TextField::type_not_stored());
        val_type.set_stored(stored)?;
        val_type.set_tokenized(tokenized)?;
        val_type.set_omit_norms(!norms)?;
        val_type.set_store_term_vectors(term_vec)?;
        val_type.set_store_term_vector_positions(term_vec_positions)?;
        val_type.set_store_term_vector_offsets(term_vec_offsets)?;
        val_type.freeze();

        let body_val_type = FieldType::new_from(TextField::type_not_stored());
        body_val_type.set_stored(body_stored)?;
        body_val_type.set_tokenized(body_tokenized)?;
        body_val_type.set_omit_norms(!body_norms)?;
        if body_tokenized && body_offsets {
            body_val_type
                .set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)?;
        }
        body_val_type.set_store_term_vectors(term_vec)?;
        body_val_type.set_store_term_vector_positions(term_vec_positions)?;
        body_val_type.set_store_term_vector_offsets(term_vec_offsets)?;
        body_val_type.freeze();

        let store_bytes = config.get_bool("doc.store.body.bytes", false)?;
        let reuse_fields = config.get_bool("doc.reuse.fields", true)?;
        let index_properties = config.get_bool("doc.index.props", false)?;
        let update_doc_id_limit = config.get_int("doc.random.id.limit", -1)?;

        let mut s = self.settings();
        s.config = Some(config);
        s.source = Some(source);
        s.val_type = Arc::new(val_type);
        s.body_val_type = Arc::new(body_val_type);
        s.store_bytes = store_bytes;
        s.reuse_fields = reuse_fields;
        // In a multi-rounds run, it is important to reset DocState since settings
        // of fields may change between rounds, and this is the only way to reset
        // the cache of all threads.
        s.doc_state = Arc::new(ThreadLocal::new());
        s.index_properties = index_properties;
        s.update_doc_id_limit = update_doc_id_limit;
        if update_doc_id_limit != -1 {
            s.r = Some(JavaRandom::new(179));
        }
        Ok(())
    }

    /// `close()`: closes the [`DocMaker`]. The base implementation closes the
    /// [`ContentSource`].
    ///
    /// # Errors
    ///
    /// The `IOException` of closing the source.
    pub fn close(&self) -> Result<()> {
        self.source().close()
    }
}

/// The virtual methods of `DocMaker`, through which a run calls the doc
/// maker its `doc.maker` property names: [`DocMaker`] itself or a subclass
/// such as [`SpatialDocMaker`](crate::by_task::feeds::spatial_doc_maker::SpatialDocMaker).
pub trait DocMakerClass: Send + Sync + Any {
    /// The `DocMaker` part of this object.
    fn doc_maker(&self) -> &DocMaker;

    /// `makeDocument()`.
    ///
    /// # Errors
    ///
    /// As [`DocMaker::make_document`].
    fn make_document(&self) -> Result<Document> {
        self.doc_maker().make_document()
    }

    /// `makeDocument(int size)`.
    ///
    /// # Errors
    ///
    /// As [`DocMaker::make_document_of_size`].
    fn make_document_of_size(&self, size: i32) -> Result<Document> {
        self.doc_maker().make_document_of_size(size)
    }

    /// `resetInputs()`: reset inputs so that the test run would behave,
    /// input wise, as if it just started.
    ///
    /// # Errors
    ///
    /// The exception of `setConfig` or of the source's `resetInputs`.
    fn reset_inputs(&self) -> Result<()> {
        let source = self.doc_maker().source();
        source.print_statistics("docs");
        let config = self.doc_maker().config().expect("setConfig was called");
        self.set_config(config, Arc::clone(&source))?;
        source.reset_inputs()?;
        self.doc_maker().num_docs_created.store(0, Ordering::SeqCst);
        self.doc_maker().reset_leftovers();
        Ok(())
    }

    /// `setConfig(Config, ContentSource)`.
    ///
    /// # Errors
    ///
    /// As [`DocMaker::set_config`].
    fn set_config(&self, config: Arc<Config>, source: Arc<dyn ContentSource>) -> Result<()> {
        self.doc_maker().set_config(config, source)
    }

    /// `close()`.
    ///
    /// # Errors
    ///
    /// As [`DocMaker::close`].
    fn close(&self) -> Result<()> {
        self.doc_maker().close()
    }

    /// This doc maker as `Any`.
    fn as_any(&self) -> &dyn Any;
}

impl DocMakerClass for DocMaker {
    fn doc_maker(&self) -> &DocMaker {
        self
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
