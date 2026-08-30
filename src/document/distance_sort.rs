//! Sorting by distance from an origin, ported from
//! `org.apache.lucene.document`.
//!
//! [`LatLonPointSortField`] and [`XYPointSortField`] are the two sort fields
//! `LatLonDocValuesField.newDistanceSort` and `XYDocValuesField.newDistanceSort`
//! build; each hands out the matching comparator, which reads the packed
//! locations out of the field's sorted-numeric doc values.
//!
//! # Divergence from Lucene 10.5.0: the comparator hierarchy
//!
//! Java's comparators extend `FieldComparator<Double>` and implement
//! `LeafFieldComparator`, and the sort fields extend `SortField` overriding
//! `getComparator(int, Pruning)`. Neither `FieldComparator` nor
//! `LeafFieldComparator` nor `Pruning` is part of this crate's search surface
//! yet, so the comparators here are standalone types carrying exactly the same
//! methods and the same state, and the sort fields *hold* a
//! [`SortField`](crate::search::SortField) rather than extending it. Every
//! comparison, every bounding-box short-circuit and every sort key is ported
//! unchanged.

use crate::document::geo_fields::{LatLonDocValuesField, XYDocValuesField};
use crate::error::{LuceneError, Result};
use crate::geo::encoding::{GeoEncodingUtils, GeoUtils, XYEncodingUtils};
use crate::geo::geometry::{Rectangle, XYRectangle};
use crate::index::{DocValues, LeafReader, SortedNumericDocValues};
use crate::search::{DocIdSetIterator, MissingValue, SortField, SortFieldType};
use crate::util::sloppy_math::SloppyMath;

/// The second half of the haversine calculation, converting a sort key back
/// into metres for display.
///
/// Equivalent to the static `LatLonPointDistanceComparator.haversin2(double)`.
pub fn haversin2(partial: f64) -> f64 {
    if partial.is_infinite() {
        return partial;
    }
    SloppyMath::haversin_meters_from_sort_key(partial)
}

/// Compares documents by distance from an origin point.
///
/// Equivalent to `org.apache.lucene.document.LatLonPointDistanceComparator`.
///
/// When the least competitive item on the priority queue changes
/// ([`set_bottom`](Self::set_bottom)), a bounding box representing the
/// competitive distance to the top *N* is recomputed; then
/// [`compare_bottom`](Self::compare_bottom) can reject a hit on the bounding
/// box alone, without computing a distance for every value.
pub struct LatLonPointDistanceComparator {
    field: String,
    latitude: f64,
    longitude: f64,

    values: Vec<f64>,
    bottom: f64,
    top_value: f64,
    current_docs: Box<dyn SortedNumericDocValues>,

    // The current bounding box(es) for the bottom distance on the priority
    // queue, pre-encoded with LatLonPoint's encoding so an uncompetitive hit is
    // excluded without decoding.
    min_lon: i32,
    max_lon: i32,
    min_lat: i32,
    max_lat: i32,
    /// A second longitude range, for the cross-dateline case.
    min_lon2: i32,

    /// How many times [`set_bottom`](Self::set_bottom) has been called, for
    /// adversary protection.
    set_bottom_counter: i32,

    current_values: Vec<i64>,
    values_doc_id: i32,
}

impl std::fmt::Debug for LatLonPointDistanceComparator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LatLonPointDistanceComparator")
            .field("field", &self.field)
            .field("latitude", &self.latitude)
            .field("longitude", &self.longitude)
            .finish_non_exhaustive()
    }
}

impl LatLonPointDistanceComparator {
    /// Creates the comparator for a top-`num_hits` search.
    ///
    /// Equivalent to
    /// `LatLonPointDistanceComparator(String, double, double, int)`. Java
    /// leaves `currentDocs` null until `getLeafComparator` is called; this port
    /// starts on the empty doc values instead, which behaves the same way —
    /// every document is missing — and removes a null field.
    pub fn new(field: impl Into<String>, latitude: f64, longitude: f64, num_hits: usize) -> Self {
        Self {
            field: field.into(),
            latitude,
            longitude,
            values: vec![0.0; num_hits],
            bottom: 0.0,
            top_value: 0.0,
            current_docs: Box::new(DocValues::empty_sorted_numeric()),
            min_lon: i32::MIN,
            max_lon: i32::MAX,
            min_lat: i32::MIN,
            max_lat: i32::MAX,
            min_lon2: i32::MAX,
            set_bottom_counter: 0,
            current_values: vec![0; 4],
            values_doc_id: -1,
        }
    }

    /// Compares the values held in two priority-queue slots.
    ///
    /// Equivalent to `FieldComparator.compare(int, int)`.
    pub fn compare(&self, slot1: usize, slot2: usize) -> std::cmp::Ordering {
        compare_doubles(self.values[slot1], self.values[slot2])
    }

    /// Records the least competitive slot and rebuilds the competitive bounding
    /// box.
    ///
    /// Equivalent to `LeafFieldComparator.setBottom(int)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever building the bounding box raises.
    pub fn set_bottom(&mut self, slot: usize) -> Result<()> {
        self.bottom = self.values[slot];
        // Build the bounding box(es) that exclude non-competitive hits, but
        // start sampling once called far too often, so a worst-case order — a
        // backwards distance order, say — does not build gobs of boxes.
        if self.set_bottom_counter < 1024 || (self.set_bottom_counter & 0x3F) == 0x3F {
            let box_ = Rectangle::from_point_distance(
                self.latitude,
                self.longitude,
                haversin2(self.bottom),
            )?;
            // Pre-encode the box into the integer encoding, so an uncompetitive
            // hit costs no decoding. This has some cost of its own.
            self.min_lat = GeoEncodingUtils::encode_latitude(box_.min_lat())?;
            self.max_lat = GeoEncodingUtils::encode_latitude(box_.max_lat())?;
            if box_.crosses_dateline() {
                // Box one.
                self.min_lon = i32::MIN;
                self.max_lon = GeoEncodingUtils::encode_longitude(box_.max_lon())?;
                // Box two.
                self.min_lon2 = GeoEncodingUtils::encode_longitude(box_.min_lon())?;
            } else {
                self.min_lon = GeoEncodingUtils::encode_longitude(box_.min_lon())?;
                self.max_lon = GeoEncodingUtils::encode_longitude(box_.max_lon())?;
                // Disable box two.
                self.min_lon2 = i32::MAX;
            }
        }
        self.set_bottom_counter += 1;
        Ok(())
    }

    /// Records the value the `searchAfter` document holds.
    ///
    /// Equivalent to `FieldComparator.setTopValue(Double)`.
    pub fn set_top_value(&mut self, value: f64) {
        self.top_value = value;
    }

    /// Equivalent to the private `LatLonPointDistanceComparator.setValues()`.
    fn set_values(&mut self) -> Result<()> {
        if self.values_doc_id != self.current_docs.doc_id() {
            debug_assert!(self.values_doc_id < self.current_docs.doc_id());
            self.values_doc_id = self.current_docs.doc_id();
            let count = self.current_docs.doc_value_count()? as usize;
            if count > self.current_values.len() {
                self.current_values
                    .resize(crate::util::ArrayUtil::oversize(count, 8), 0);
            }
            for i in 0..count {
                self.current_values[i] = self.current_docs.next_value()?;
            }
        }
        Ok(())
    }

    /// Compares the document against the least competitive hit.
    ///
    /// Equivalent to `LeafFieldComparator.compareBottom(int)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the doc values raises.
    pub fn compare_bottom(&mut self, doc: i32) -> Result<i32> {
        if doc > self.current_docs.doc_id() {
            self.current_docs.advance(doc)?;
        }
        if doc < self.current_docs.doc_id() {
            return Ok(ordering_to_int(compare_doubles(self.bottom, f64::INFINITY)));
        }

        self.set_values()?;
        let num_values = self.current_docs.doc_value_count()? as usize;

        let mut cmp = -1;
        for i in 0..num_values {
            let encoded = self.current_values[i];

            // Test the bounding box.
            let latitude_bits = (encoded >> 32) as i32;
            if latitude_bits < self.min_lat || latitude_bits > self.max_lat {
                continue;
            }
            let longitude_bits = (encoded & 0xFFFF_FFFF) as i32;
            if (longitude_bits < self.min_lon || longitude_bits > self.max_lon)
                && longitude_bits < self.min_lon2
            {
                continue;
            }

            // Compute the real distance only inside the competitive bounding box.
            let doc_latitude = GeoEncodingUtils::decode_latitude(latitude_bits);
            let doc_longitude = GeoEncodingUtils::decode_longitude(longitude_bits);
            cmp = cmp.max(ordering_to_int(compare_doubles(
                self.bottom,
                SloppyMath::haversin_sort_key(
                    self.latitude,
                    self.longitude,
                    doc_latitude,
                    doc_longitude,
                ),
            )));
            // Once the document competes in the queue there is no need to go on.
            if cmp > 0 {
                return Ok(cmp);
            }
        }
        Ok(cmp)
    }

    /// Copies the document's sort key into a priority-queue slot.
    ///
    /// Equivalent to `LeafFieldComparator.copy(int, int)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the doc values raises.
    pub fn copy(&mut self, slot: usize, doc: i32) -> Result<()> {
        self.values[slot] = self.sort_key(doc)?;
        Ok(())
    }

    /// Binds the comparator to one leaf.
    ///
    /// Equivalent to `FieldComparator.getLeafComparator(LeafReaderContext)`,
    /// which returns `this` because the class is its own leaf comparator.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the field was indexed with
    /// an incompatible doc-values type, and propagates reader errors.
    pub fn get_leaf_comparator(&mut self, reader: &dyn LeafReader) -> Result<()> {
        let field_infos = reader.get_field_infos();
        if let Some(info) = field_infos.field_info(&self.field) {
            LatLonDocValuesField::check_compatible(info)?;
        }
        self.current_docs = match reader.get_sorted_numeric_doc_values(&self.field)? {
            Some(values) => values,
            // Equivalent to `DocValues.getSortedNumeric`, which substitutes an
            // empty iterator for a field without doc values.
            None => Box::new(DocValues::empty_sorted_numeric()),
        };
        self.values_doc_id = -1;
        Ok(())
    }

    /// Returns the distance in metres held in a priority-queue slot.
    ///
    /// Equivalent to `FieldComparator.value(int)`.
    pub fn value(&self, slot: usize) -> f64 {
        haversin2(self.values[slot])
    }

    /// Compares the document against the `searchAfter` value.
    ///
    /// Equivalent to `LeafFieldComparator.compareTop(int)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the doc values raises.
    pub fn compare_top(&mut self, doc: i32) -> Result<i32> {
        let key = self.sort_key(doc)?;
        Ok(ordering_to_int(compare_doubles(
            self.top_value,
            haversin2(key),
        )))
    }

    /// Returns the smallest haversine sort key over the document's locations.
    ///
    /// Equivalent to `LatLonPointDistanceComparator.sortKey(int)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the doc values raises.
    pub fn sort_key(&mut self, doc: i32) -> Result<f64> {
        if doc > self.current_docs.doc_id() {
            self.current_docs.advance(doc)?;
        }
        let mut min_value = f64::INFINITY;
        if doc == self.current_docs.doc_id() {
            self.set_values()?;
            let num_values = self.current_docs.doc_value_count()? as usize;
            for i in 0..num_values {
                let encoded = self.current_values[i];
                let doc_latitude = GeoEncodingUtils::decode_latitude((encoded >> 32) as i32);
                let doc_longitude =
                    GeoEncodingUtils::decode_longitude((encoded & 0xFFFF_FFFF) as i32);
                min_value = min_value.min(SloppyMath::haversin_sort_key(
                    self.latitude,
                    self.longitude,
                    doc_latitude,
                    doc_longitude,
                ));
            }
        }
        Ok(min_value)
    }
}

/// Compares documents by distance from an origin point, in cartesian space.
///
/// Equivalent to `org.apache.lucene.document.XYPointDistanceComparator`. See
/// [`LatLonPointDistanceComparator`] for the bounding-box strategy, which is
/// the same.
pub struct XYPointDistanceComparator {
    field: String,
    x: f64,
    y: f64,

    /// Distances are kept as their square root, so that two square distances
    /// that differ but whose real distances are equal still compare equal.
    values: Vec<f64>,
    bottom: f64,
    top_value: f64,
    current_docs: Box<dyn SortedNumericDocValues>,

    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,

    set_bottom_counter: i32,

    current_values: Vec<i64>,
    values_doc_id: i32,
}

impl std::fmt::Debug for XYPointDistanceComparator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XYPointDistanceComparator")
            .field("field", &self.field)
            .field("x", &self.x)
            .field("y", &self.y)
            .finish_non_exhaustive()
    }
}

impl XYPointDistanceComparator {
    /// Creates the comparator for a top-`num_hits` search.
    ///
    /// Equivalent to `XYPointDistanceComparator(String, float, float, int)`.
    pub fn new(field: impl Into<String>, x: f32, y: f32, num_hits: usize) -> Self {
        Self {
            field: field.into(),
            x: f64::from(x),
            y: f64::from(y),
            values: vec![0.0; num_hits],
            bottom: 0.0,
            top_value: 0.0,
            current_docs: Box::new(DocValues::empty_sorted_numeric()),
            min_x: i32::MIN,
            max_x: i32::MAX,
            min_y: i32::MIN,
            max_y: i32::MAX,
            set_bottom_counter: 0,
            current_values: vec![0; 4],
            values_doc_id: -1,
        }
    }

    /// Compares the values held in two priority-queue slots.
    ///
    /// Equivalent to `FieldComparator.compare(int, int)`.
    pub fn compare(&self, slot1: usize, slot2: usize) -> std::cmp::Ordering {
        compare_doubles(self.values[slot1], self.values[slot2])
    }

    /// Records the least competitive slot and rebuilds the competitive bounding
    /// box.
    ///
    /// Equivalent to `LeafFieldComparator.setBottom(int)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever building the bounding box raises.
    pub fn set_bottom(&mut self, slot: usize) -> Result<()> {
        self.bottom = self.values[slot];
        if self.bottom < f64::from(f32::MAX)
            && (self.set_bottom_counter < 1024 || (self.set_bottom_counter & 0x3F) == 0x3F)
        {
            let rectangle =
                XYRectangle::from_point_distance(self.x as f32, self.y as f32, self.bottom as f32)?;
            // Pre-encode the box into the integer encoding, so an uncompetitive
            // hit costs no decoding. This has some cost of its own.
            self.min_x = XYEncodingUtils::encode(rectangle.min_x())?;
            self.max_x = XYEncodingUtils::encode(rectangle.max_x())?;
            self.min_y = XYEncodingUtils::encode(rectangle.min_y())?;
            self.max_y = XYEncodingUtils::encode(rectangle.max_y())?;
        }
        self.set_bottom_counter += 1;
        Ok(())
    }

    /// Records the value the `searchAfter` document holds.
    ///
    /// Equivalent to `FieldComparator.setTopValue(Double)`.
    pub fn set_top_value(&mut self, value: f64) {
        self.top_value = value;
    }

    /// Equivalent to the private `XYPointDistanceComparator.setValues()`.
    fn set_values(&mut self) -> Result<()> {
        if self.values_doc_id != self.current_docs.doc_id() {
            debug_assert!(self.values_doc_id < self.current_docs.doc_id());
            self.values_doc_id = self.current_docs.doc_id();
            let count = self.current_docs.doc_value_count()? as usize;
            if count > self.current_values.len() {
                self.current_values
                    .resize(crate::util::ArrayUtil::oversize(count, 8), 0);
            }
            for i in 0..count {
                self.current_values[i] = self.current_docs.next_value()?;
            }
        }
        Ok(())
    }

    /// Compares the document against the least competitive hit.
    ///
    /// Equivalent to `LeafFieldComparator.compareBottom(int)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the doc values raises.
    pub fn compare_bottom(&mut self, doc: i32) -> Result<i32> {
        if doc > self.current_docs.doc_id() {
            self.current_docs.advance(doc)?;
        }
        if doc < self.current_docs.doc_id() {
            return Ok(ordering_to_int(compare_doubles(self.bottom, f64::INFINITY)));
        }

        self.set_values()?;
        let num_values = self.current_docs.doc_value_count()? as usize;

        let mut cmp = -1;
        for i in 0..num_values {
            let encoded = self.current_values[i];

            // Test the bounding box.
            let x_bits = (encoded >> 32) as i32;
            if x_bits < self.min_x || x_bits > self.max_x {
                continue;
            }
            let y_bits = (encoded & 0xFFFF_FFFF) as i32;
            if y_bits < self.min_y || y_bits > self.max_y {
                continue;
            }

            // Compute the real distance only inside the competitive bounding box.
            let doc_x = f64::from(XYEncodingUtils::decode(x_bits));
            let doc_y = f64::from(XYEncodingUtils::decode(y_bits));
            let diff_x = self.x - doc_x;
            let diff_y = self.y - doc_y;
            let distance = (diff_x * diff_x + diff_y * diff_y).sqrt();
            cmp = cmp.max(ordering_to_int(compare_doubles(self.bottom, distance)));
            // Once the document competes in the queue there is no need to go on.
            if cmp > 0 {
                return Ok(cmp);
            }
        }
        Ok(cmp)
    }

    /// Copies the document's sort key into a priority-queue slot.
    ///
    /// Equivalent to `LeafFieldComparator.copy(int, int)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the doc values raises.
    pub fn copy(&mut self, slot: usize, doc: i32) -> Result<()> {
        self.values[slot] = self.sort_key(doc)?;
        Ok(())
    }

    /// Binds the comparator to one leaf.
    ///
    /// Equivalent to `FieldComparator.getLeafComparator(LeafReaderContext)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the field was indexed with
    /// an incompatible doc-values type, and propagates reader errors.
    pub fn get_leaf_comparator(&mut self, reader: &dyn LeafReader) -> Result<()> {
        let field_infos = reader.get_field_infos();
        if let Some(info) = field_infos.field_info(&self.field) {
            XYDocValuesField::check_compatible(info)?;
        }
        self.current_docs = match reader.get_sorted_numeric_doc_values(&self.field)? {
            Some(values) => values,
            None => Box::new(DocValues::empty_sorted_numeric()),
        };
        self.values_doc_id = -1;
        Ok(())
    }

    /// Returns the distance held in a priority-queue slot.
    ///
    /// Equivalent to `FieldComparator.value(int)`.
    pub fn value(&self, slot: usize) -> f64 {
        self.values[slot]
    }

    /// Compares the document against the `searchAfter` value.
    ///
    /// Equivalent to `LeafFieldComparator.compareTop(int)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the doc values raises.
    pub fn compare_top(&mut self, doc: i32) -> Result<i32> {
        let key = self.sort_key(doc)?;
        Ok(ordering_to_int(compare_doubles(self.top_value, key)))
    }

    /// Returns the smallest distance over the document's locations.
    ///
    /// Equivalent to `XYPointDistanceComparator.sortKey(int)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the doc values raises.
    pub fn sort_key(&mut self, doc: i32) -> Result<f64> {
        if doc > self.current_docs.doc_id() {
            self.current_docs.advance(doc)?;
        }
        let mut min_value = f64::INFINITY;
        if doc == self.current_docs.doc_id() {
            self.set_values()?;
            let num_values = self.current_docs.doc_value_count()? as usize;
            for i in 0..num_values {
                let encoded = self.current_values[i];
                let doc_x = f64::from(XYEncodingUtils::decode((encoded >> 32) as i32));
                let doc_y = f64::from(XYEncodingUtils::decode((encoded & 0xFFFF_FFFF) as i32));
                let diff_x = self.x - doc_x;
                let diff_y = self.y - doc_y;
                min_value = min_value.min((diff_x * diff_x + diff_y * diff_y).sqrt());
            }
        }
        Ok(min_value)
    }
}

/// Sorts by distance from an origin location.
///
/// Equivalent to `org.apache.lucene.document.LatLonPointSortField`, which
/// `LatLonDocValuesField.newDistanceSort` builds.
#[derive(Clone, Debug, PartialEq)]
pub struct LatLonPointSortField {
    sort_field: SortField,
    latitude: f64,
    longitude: f64,
}

impl LatLonPointSortField {
    /// Creates the sort field.
    ///
    /// Equivalent to `LatLonPointSortField(String, double, double)`, which
    /// declares `SortField.Type.CUSTOM`, ascending order and a missing value of
    /// positive infinity.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for an invalid latitude or
    /// longitude.
    pub fn new(field: &str, latitude: f64, longitude: f64) -> Result<Self> {
        GeoUtils::check_latitude(latitude)?;
        GeoUtils::check_longitude(longitude)?;
        Ok(Self {
            sort_field: SortField::new_with_missing(
                Some(field.to_string()),
                SortFieldType::Custom,
                false,
                Some(MissingValue::Double(f64::INFINITY)),
            )?,
            latitude,
            longitude,
        })
    }

    /// Returns this sort field as a [`SortField`].
    pub fn sort_field(&self) -> &SortField {
        &self.sort_field
    }

    /// Returns the field being sorted on.
    pub fn get_field(&self) -> Option<&str> {
        self.sort_field.field()
    }

    /// Returns the origin latitude.
    pub fn latitude(&self) -> f64 {
        self.latitude
    }

    /// Returns the origin longitude.
    pub fn longitude(&self) -> f64 {
        self.longitude
    }

    /// Returns the comparator that ranks the top `num_hits`.
    ///
    /// Equivalent to `LatLonPointSortField.getComparator(int, Pruning)`, whose
    /// `Pruning` argument this port omits because that type is not part of the
    /// search surface yet; the Java method ignores it.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the sort field carries no
    /// field name, which its constructor rules out.
    pub fn get_comparator(&self, num_hits: usize) -> Result<LatLonPointDistanceComparator> {
        let field = self.get_field().ok_or_else(|| {
            LuceneError::IllegalState("a distance sort field always names a field".to_string())
        })?;
        Ok(LatLonPointDistanceComparator::new(
            field,
            self.latitude,
            self.longitude,
            num_hits,
        ))
    }

    /// Returns the value used for documents that have no value.
    ///
    /// Equivalent to `LatLonPointSortField.getMissingValue()`.
    pub fn get_missing_value(&self) -> f64 {
        match self.sort_field.missing_value() {
            Some(MissingValue::Double(value)) => value,
            _ => f64::INFINITY,
        }
    }

    /// Replaces the value used for documents that have no value.
    ///
    /// Equivalent to `LatLonPointSortField.setMissingValue(Object)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for anything but positive
    /// infinity, which is what Java rejects.
    pub fn set_missing_value(&mut self, missing_value: f64) -> Result<()> {
        if missing_value != f64::INFINITY {
            return Err(LuceneError::IllegalArgument(format!(
                "Missing value can only be Double.POSITIVE_INFINITY (missing values last), but \
                 got {missing_value}"
            )));
        }
        self.sort_field = SortField::new_with_missing(
            self.sort_field.field().map(str::to_string),
            self.sort_field.field_type(),
            self.sort_field.reverse(),
            Some(MissingValue::Double(missing_value)),
        )?;
        Ok(())
    }
}

impl std::fmt::Display for LatLonPointSortField {
    /// Equivalent to `LatLonPointSortField.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "<distance:\"{}\" latitude={} longitude={}",
            self.get_field().unwrap_or_default(),
            self.latitude,
            self.longitude
        )?;
        if self.get_missing_value() != f64::INFINITY {
            write!(f, " missingValue={}", self.get_missing_value())?;
        }
        f.write_str(">")
    }
}

/// Sorts by distance from an origin location, in cartesian space.
///
/// Equivalent to `org.apache.lucene.document.XYPointSortField`, which
/// `XYDocValuesField.newDistanceSort` builds.
#[derive(Clone, Debug, PartialEq)]
pub struct XYPointSortField {
    sort_field: SortField,
    x: f32,
    y: f32,
}

impl XYPointSortField {
    /// Creates the sort field.
    ///
    /// Equivalent to `XYPointSortField(String, float, float)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`SortField`] construction raises.
    pub fn new(field: &str, x: f32, y: f32) -> Result<Self> {
        Ok(Self {
            sort_field: SortField::new_with_missing(
                Some(field.to_string()),
                SortFieldType::Custom,
                false,
                Some(MissingValue::Double(f64::INFINITY)),
            )?,
            x,
            y,
        })
    }

    /// Returns this sort field as a [`SortField`].
    pub fn sort_field(&self) -> &SortField {
        &self.sort_field
    }

    /// Returns the field being sorted on.
    pub fn get_field(&self) -> Option<&str> {
        self.sort_field.field()
    }

    /// Returns the origin x.
    pub fn x(&self) -> f32 {
        self.x
    }

    /// Returns the origin y.
    pub fn y(&self) -> f32 {
        self.y
    }

    /// Returns the comparator that ranks the top `num_hits`.
    ///
    /// Equivalent to `XYPointSortField.getComparator(int, Pruning)`; see the
    /// note on [`LatLonPointSortField::get_comparator`].
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the sort field carries no
    /// field name, which its constructor rules out.
    pub fn get_comparator(&self, num_hits: usize) -> Result<XYPointDistanceComparator> {
        let field = self.get_field().ok_or_else(|| {
            LuceneError::IllegalState("a distance sort field always names a field".to_string())
        })?;
        Ok(XYPointDistanceComparator::new(
            field, self.x, self.y, num_hits,
        ))
    }

    /// Returns the value used for documents that have no value.
    ///
    /// Equivalent to `XYPointSortField.getMissingValue()`.
    pub fn get_missing_value(&self) -> f64 {
        match self.sort_field.missing_value() {
            Some(MissingValue::Double(value)) => value,
            _ => f64::INFINITY,
        }
    }

    /// Replaces the value used for documents that have no value.
    ///
    /// Equivalent to `XYPointSortField.setMissingValue(Object)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for anything but positive
    /// infinity.
    pub fn set_missing_value(&mut self, missing_value: f64) -> Result<()> {
        if missing_value != f64::INFINITY {
            return Err(LuceneError::IllegalArgument(format!(
                "Missing value can only be Double.POSITIVE_INFINITY (missing values last), but \
                 got {missing_value}"
            )));
        }
        self.sort_field = SortField::new_with_missing(
            self.sort_field.field().map(str::to_string),
            self.sort_field.field_type(),
            self.sort_field.reverse(),
            Some(MissingValue::Double(missing_value)),
        )?;
        Ok(())
    }
}

impl std::fmt::Display for XYPointSortField {
    /// Equivalent to `XYPointSortField.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "<distance:\"{}\" x={} y={}",
            self.get_field().unwrap_or_default(),
            self.x,
            self.y
        )?;
        if self.get_missing_value() != f64::INFINITY {
            write!(f, " missingValue={}", self.get_missing_value())?;
        }
        f.write_str(">")
    }
}

/// Orders two doubles the way `Double.compare(double, double)` does: `NaN` is
/// greater than everything, and `-0.0` sorts before `0.0`.
fn compare_doubles(a: f64, b: f64) -> std::cmp::Ordering {
    a.total_cmp(&b)
}

/// Turns an ordering into the `-1`/`0`/`1` a Java comparator returns.
fn ordering_to_int(ordering: std::cmp::Ordering) -> i32 {
    match ordering {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}
