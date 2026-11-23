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

//! Port of `FacetsConfig.java`, of the Java package
//! `org.apache.lucene.facet`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rucene::document::{
    BinaryDocValuesField, Document, SortedNumericDocValuesField, SortedSetDocValuesField, Store,
    StringField,
};
use rucene::error::{LuceneError, Result};
use rucene::index::IndexableField;
use rucene::util::java_collections::JavaHashMap;
use rucene::util::BytesRef;

use crate::facet_field::FacetField;
use crate::sortedset::sorted_set_doc_values_facet_field::SortedSetDocValuesFacetField;
use crate::taxonomy::association_facet_field::{AssociationFacetField, AssociationKind};
use crate::taxonomy::facet_label::FacetLabel;
use crate::taxonomy::taxonomy_writer::TaxonomyWriter;

/// Which Lucene field holds the drill-downs and ords (as doc values).
///
/// Equivalent to `FacetsConfig.DEFAULT_INDEX_FIELD_NAME`.
pub const DEFAULT_INDEX_FIELD_NAME: &str = "$facets";

/// Delimiter character used to separate the components of a path.
///
/// Equivalent to `FacetsConfig.DELIM_CHAR`.
pub const DELIM_CHAR: char = '\u{001F}';

/// Escapes any occurrence of the path component inside the label.
const ESCAPE_CHAR: char = '\u{001E}';

/// Drill down terms indexing option to control whether dimension and
/// sub-path terms should be indexed.
///
/// Equivalent to `org.apache.lucene.facet.FacetsConfig.DrillDownTermsIndexing`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
pub enum DrillDownTermsIndexing {
    /// Index no drill down terms.
    NONE,
    /// Index only full path drill down terms.
    FULL_PATH_ONLY,
    /// Index drill down terms corresponding to all sub-paths except for
    /// dimension.
    ALL_PATHS_NO_DIM,
    /// Index drill down terms for dimension and full path.
    DIMENSION_AND_FULL_PATH,
    /// Index drill down terms for dimension, all sub-paths and full path.
    ALL,
}

/// The fields of a [`DimConfig`]: the public mutable fields of Java's
/// `FacetsConfig.DimConfig`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DimConfigFields {
    /// True if this dimension is hierarchical.
    pub hierarchical: bool,
    /// True if this dimension is multi-valued.
    pub multi_valued: bool,
    /// True if the count/aggregate for the entire dimension is required,
    /// which is unusual (default is false).
    pub require_dim_count: bool,
    /// Default drill down terms indexing option that index all.
    pub drill_down_terms_indexing: DrillDownTermsIndexing,
    /// Actual field where this dimension's facet labels should be indexed.
    pub index_field_name: String,
}

impl Default for DimConfigFields {
    fn default() -> Self {
        Self {
            hierarchical: false,
            multi_valued: false,
            require_dim_count: false,
            drill_down_terms_indexing: DrillDownTermsIndexing::ALL,
            index_field_name: DEFAULT_INDEX_FIELD_NAME.to_string(),
        }
    }
}

/// Holds the configuration for one dimension.
///
/// Equivalent to `org.apache.lucene.facet.FacetsConfig.DimConfig`. Java's
/// `DimConfig` is a mutable object with public fields that `FacetsConfig`
/// hands out live (`getDimConfig`, `getDimConfigs`, `DEFAULT_DIM_CONFIG`,
/// `FacetsConfig.java:113-231`); this is a shared handle to the same kind
/// of object: clones share the fields, and a write through any clone is
/// seen by every holder. `equals` is Java's identity (no override).
#[derive(Clone, Default)]
pub struct DimConfig {
    inner: Arc<Mutex<DimConfigFields>>,
}

impl std::fmt::Debug for DimConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        lock(&self.inner).fmt(f)
    }
}

impl PartialEq for DimConfig {
    /// `Object.equals`: identity.
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Eq for DimConfig {}

impl DimConfig {
    /// Default constructor.
    ///
    /// Equivalent to `DimConfig()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Read access to the fields.
    pub fn fields(&self) -> MutexGuard<'_, DimConfigFields> {
        lock(&self.inner)
    }

    /// Equivalent to reading the field `hierarchical`.
    pub fn hierarchical(&self) -> bool {
        lock(&self.inner).hierarchical
    }

    /// Equivalent to writing the field `hierarchical`.
    pub fn set_hierarchical(&self, v: bool) {
        lock(&self.inner).hierarchical = v;
    }

    /// Equivalent to reading the field `multiValued`.
    pub fn multi_valued(&self) -> bool {
        lock(&self.inner).multi_valued
    }

    /// Equivalent to writing the field `multiValued`.
    pub fn set_multi_valued(&self, v: bool) {
        lock(&self.inner).multi_valued = v;
    }

    /// Equivalent to reading the field `requireDimCount`.
    pub fn require_dim_count(&self) -> bool {
        lock(&self.inner).require_dim_count
    }

    /// Equivalent to writing the field `requireDimCount`.
    pub fn set_require_dim_count(&self, v: bool) {
        lock(&self.inner).require_dim_count = v;
    }

    /// Equivalent to reading the field `drillDownTermsIndexing`.
    pub fn drill_down_terms_indexing(&self) -> DrillDownTermsIndexing {
        lock(&self.inner).drill_down_terms_indexing
    }

    /// Equivalent to writing the field `drillDownTermsIndexing`.
    pub fn set_drill_down_terms_indexing(&self, v: DrillDownTermsIndexing) {
        lock(&self.inner).drill_down_terms_indexing = v;
    }

    /// Equivalent to reading the field `indexFieldName`.
    pub fn index_field_name(&self) -> String {
        lock(&self.inner).index_field_name.clone()
    }

    /// Equivalent to writing the field `indexFieldName`.
    pub fn set_index_field_name(&self, v: &str) {
        lock(&self.inner).index_field_name = v.to_string();
    }
}

static DEFAULT_DIM_CONFIG: std::sync::LazyLock<DimConfig> = std::sync::LazyLock::new(DimConfig::new);

/// Default per-dimension configuration: the one shared instance, as Java's
/// `public static final DimConfig DEFAULT_DIM_CONFIG`.
///
/// Equivalent to `FacetsConfig.DEFAULT_DIM_CONFIG`.
pub fn default_dim_config() -> DimConfig {
    DEFAULT_DIM_CONFIG.clone()
}

/// The live map `getDimConfigs()` returns (Java's `ConcurrentHashMap`
/// field `fieldTypes`).
pub type DimConfigs = Arc<Mutex<HashMap<String, DimConfig>>>;

/// Records per-dimension configuration. By default a dimension is flat,
/// single valued and does not require count for the dimension; use the
/// setters in this class to change these settings for each dim.
///
/// Equivalent to `org.apache.lucene.facet.FacetsConfig`. The `DimConfig`
/// getters hand out the shared [`DimConfig`] objects and the live map, as
/// Java does. A subclass overriding the protected `getDefaultDimConfig()`
/// is [`with_default_dim_config`](Self::with_default_dim_config).
#[derive(Debug, Default)]
pub struct FacetsConfig {
    field_types: DimConfigs,
    assoc_dim_types: Mutex<HashMap<String, String>>,
    default_dim_config: Option<DimConfig>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FacetsConfig {
    /// Sole constructor.
    ///
    /// Equivalent to `FacetsConfig()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// A configuration whose `getDefaultDimConfig()` is overridden to return
    /// `default_dim_config`.
    pub fn with_default_dim_config(default_dim_config: DimConfig) -> Self {
        Self {
            default_dim_config: Some(default_dim_config),
            ..Self::default()
        }
    }

    /// Get the default configuration for new dimensions.
    ///
    /// Equivalent to `getDefaultDimConfig()`.
    pub fn get_default_dim_config(&self) -> DimConfig {
        match &self.default_dim_config {
            Some(c) => c.clone(),
            None => default_dim_config(),
        }
    }

    /// Get the current configuration for a dimension.
    ///
    /// Equivalent to `getDimConfig(String)`.
    pub fn get_dim_config(&self, dim_name: &str) -> DimConfig {
        let dim_config = lock(&self.field_types).get(dim_name).cloned();
        match dim_config {
            Some(dim_config) => dim_config,
            None => self.get_default_dim_config(),
        }
    }

    /// Returns true if the dimension for the provided name has been
    /// explicitly configured.
    ///
    /// Equivalent to `isDimConfigured(String)`.
    pub fn is_dim_configured(&self, dim_name: &str) -> bool {
        lock(&self.field_types).contains_key(dim_name)
    }

    /// The common body of the `synchronized` setters: the dimension's
    /// config, created and registered when absent. The map lock makes the
    /// get-or-put atomic, which is what the setters' monitor guarantees.
    fn dim_config_for_update(&self, dim_name: &str) -> DimConfig {
        lock(&self.field_types)
            .entry(dim_name.to_string())
            .or_default()
            .clone()
    }

    /// Pass `true` if this dimension is hierarchical (has depth &gt; 1
    /// paths).
    ///
    /// Equivalent to `setHierarchical(String, boolean)`.
    pub fn set_hierarchical(&self, dim_name: &str, v: bool) {
        self.dim_config_for_update(dim_name).set_hierarchical(v);
    }

    /// Pass `true` if this dimension may have more than one value per
    /// document.
    ///
    /// Equivalent to `setMultiValued(String, boolean)`.
    pub fn set_multi_valued(&self, dim_name: &str, value: bool) {
        self.dim_config_for_update(dim_name).set_multi_valued(value);
    }

    /// Pass `true` if at search time you require accurate counts of the
    /// dimension, i.e. how many hits have this dimension.
    ///
    /// Equivalent to `setRequireDimCount(String, boolean)`.
    pub fn set_require_dim_count(&self, dim_name: &str, value: bool) {
        self.dim_config_for_update(dim_name).set_require_dim_count(value);
    }

    /// Specify which index field name should hold the ordinals for this
    /// dimension; this is only used by the taxonomy based facet methods.
    ///
    /// Equivalent to `setIndexFieldName(String, String)`.
    pub fn set_index_field_name(&self, dim_name: &str, index_field_name: &str) {
        self.dim_config_for_update(dim_name).set_index_field_name(index_field_name);
    }

    /// Specify drill down terms option on the field / dimension.
    ///
    /// Equivalent to `setDrillDownTermsIndexing(String, DrillDownTermsIndexing)`.
    pub fn set_drill_down_terms_indexing(
        &self,
        dim_name: &str,
        drill_down_terms_indexing: DrillDownTermsIndexing,
    ) {
        self.dim_config_for_update(dim_name)
            .set_drill_down_terms_indexing(drill_down_terms_indexing);
    }

    /// Returns map of field name to `DimConfig`: the live map, as Java.
    ///
    /// Equivalent to `getDimConfigs()`.
    pub fn get_dim_configs(&self) -> DimConfigs {
        Arc::clone(&self.field_types)
    }

    /// Equivalent to `checkSeen(Set, String)`.
    fn check_seen(seen_dims: &mut HashSet<String>, dim: &str) -> Result<()> {
        if seen_dims.contains(dim) {
            return Err(LuceneError::illegal_argument(format!(
                "dimension \"{dim}\" is not multiValued, but it appears more than once in this document"
            )));
        }
        seen_dims.insert(dim.to_string());
        Ok(())
    }

    /// Translates any added `FacetField`s into normal fields for indexing;
    /// only use this version if you did not add any taxonomy-based fields
    /// (`FacetField` or `AssociationFacetField`).
    ///
    /// Equivalent to `build(Document)`.
    pub fn build(&self, doc: &Document) -> Result<Document> {
        self.build_with_taxonomy(None, doc)
    }

    /// Translates any added `FacetField`s into normal fields for indexing.
    ///
    /// Equivalent to `build(TaxonomyWriter, Document)`. The three maps
    /// iterate in the order of Java's `HashMap`s, which decides the order the
    /// translated fields are added to the result.
    pub fn build_with_taxonomy(
        &self,
        taxo_writer: Option<&dyn TaxonomyWriter>,
        doc: &Document,
    ) -> Result<Document> {
        // Find all FacetFields, collated by the actual field:
        let mut by_field: JavaHashMap<Vec<Arc<dyn IndexableField>>> = JavaHashMap::new();

        // ... and also all SortedSetDocValuesFacetFields:
        let mut dv_by_field: JavaHashMap<Vec<Arc<dyn IndexableField>>> = JavaHashMap::new();

        // ... and also all AssociationFacetFields
        let mut assoc_by_field: JavaHashMap<Vec<Arc<dyn IndexableField>>> = JavaHashMap::new();

        let mut seen_dims: HashSet<String> = HashSet::new();

        for field in doc.get_fields() {
            let any = field.as_any();
            if let Some(facet_field) = any.and_then(|a| a.downcast_ref::<FacetField>()) {
                let dim_config = self.get_dim_config(&facet_field.dim);
                if !dim_config.multi_valued() {
                    Self::check_seen(&mut seen_dims, &facet_field.dim)?;
                }
                let index_field_name = dim_config.index_field_name();
                by_field
                    .compute_if_absent(&index_field_name, Vec::new)
                    .push(Arc::clone(field));
            }

            if let Some(facet_field) =
                any.and_then(|a| a.downcast_ref::<SortedSetDocValuesFacetField>())
            {
                let dim_config = self.get_dim_config(&facet_field.dim);
                if !dim_config.multi_valued() {
                    Self::check_seen(&mut seen_dims, &facet_field.dim)?;
                }
                let index_field_name = dim_config.index_field_name();
                dv_by_field
                    .compute_if_absent(&index_field_name, Vec::new)
                    .push(Arc::clone(field));
            }

            if let Some(facet_field) = any.and_then(|a| a.downcast_ref::<AssociationFacetField>()) {
                let dim_config = self.get_dim_config(&facet_field.dim);
                if !dim_config.multi_valued() {
                    Self::check_seen(&mut seen_dims, &facet_field.dim)?;
                }
                if dim_config.hierarchical() {
                    return Err(LuceneError::illegal_argument(format!(
                        "AssociationFacetField cannot be hierarchical (dim=\"{}\")",
                        facet_field.dim
                    )));
                }
                if dim_config.require_dim_count() {
                    return Err(LuceneError::illegal_argument(format!(
                        "AssociationFacetField cannot requireDimCount (dim=\"{}\")",
                        facet_field.dim
                    )));
                }

                let index_field_name = dim_config.index_field_name();
                assoc_by_field
                    .compute_if_absent(&index_field_name, Vec::new)
                    .push(Arc::clone(field));

                // Best effort: detect mis-matched types in same indexed field:
                let field_type = match facet_field.kind() {
                    AssociationKind::Int => "int",
                    AssociationKind::Float => "float",
                    AssociationKind::Bytes => "bytes",
                };
                // NOTE: not thread safe, but this is just best effort:
                let mut assoc_dim_types = lock(&self.assoc_dim_types);
                match assoc_dim_types.get(&index_field_name) {
                    None => {
                        assoc_dim_types.insert(index_field_name, field_type.to_string());
                    }
                    Some(cur_type) if cur_type != field_type => {
                        return Err(LuceneError::illegal_argument(format!(
                            "mixing incompatible types of AssociationFacetField ({cur_type} and {field_type}) in indexed field \"{index_field_name}\"; use FacetsConfig to change the indexFieldName for each dimension"
                        )));
                    }
                    Some(_) => {}
                }
            }
        }

        let mut result = Document::new();

        self.process_facet_fields(taxo_writer, &by_field, &mut result)?;
        self.process_ssdv_facet_fields(&dv_by_field, &mut result)?;
        self.process_assoc_facet_fields(taxo_writer, &assoc_by_field, &mut result)?;

        for field in doc.get_fields() {
            let any = field.as_any();
            let is_facet = any.is_some_and(|a| {
                a.is::<FacetField>()
                    || a.is::<SortedSetDocValuesFacetField>()
                    || a.is::<AssociationFacetField>()
            });
            if !is_facet {
                result.add_shared(Arc::clone(field));
            }
        }

        Ok(result)
    }

    /// Equivalent to `processFacetFields(TaxonomyWriter, Map, Document)`.
    fn process_facet_fields(
        &self,
        taxo_writer: Option<&dyn TaxonomyWriter>,
        by_field: &JavaHashMap<Vec<Arc<dyn IndexableField>>>,
        doc: &mut Document,
    ) -> Result<()> {
        for (index_field_name, fields) in by_field.iter() {
            let mut ordinals: Vec<i32> = Vec::new();
            for field in fields {
                let facet_field = field
                    .as_any()
                    .and_then(|a| a.downcast_ref::<FacetField>())
                    .expect("INVARIANT: collated as a FacetField");
                let dim_config = self.get_dim_config(&facet_field.dim);
                if facet_field.path.len() > 1 && !dim_config.hierarchical() {
                    return Err(LuceneError::illegal_argument(format!(
                        "dimension \"{}\" is not hierarchical yet has {} components",
                        facet_field.dim,
                        facet_field.path.len()
                    )));
                }

                let facet_label = FacetLabel::with_dim(&facet_field.dim, &facet_field.path)?;

                let taxo_writer = Self::check_taxo_writer(taxo_writer)?;
                let ordinal = taxo_writer.add_category(&facet_label)?;

                ordinals.push(ordinal);

                if dim_config.multi_valued()
                    && (dim_config.hierarchical() || dim_config.require_dim_count())
                {
                    // Add all parents too:
                    let mut parent = taxo_writer.get_parent(ordinal)?;
                    while parent > 0 {
                        ordinals.push(parent);
                        parent = taxo_writer.get_parent(parent)?;
                    }

                    if !dim_config.require_dim_count() {
                        // Remove last (dimension) ord:
                        ordinals.pop();
                    }
                }

                // Drill down:
                self.index_drill_down_terms(doc, index_field_name, &dim_config, &facet_label)?;
            }

            // Dedupe ordinals and index as SortedNumericDocValues:
            ordinals.sort_unstable();
            let mut prev = -1;
            for &ord in &ordinals {
                if ord > prev {
                    doc.add(SortedNumericDocValuesField::new(
                        index_field_name.clone(),
                        ord as i64,
                    ));
                    prev = ord;
                }
            }
        }
        Ok(())
    }

    /// Equivalent to `indexDrillDownTerms(Document, String, DimConfig,
    /// FacetLabel)`.
    fn index_drill_down_terms(
        &self,
        doc: &mut Document,
        index_field_name: &str,
        dim_config: &DimConfig,
        facet_label: &FacetLabel,
    ) -> Result<()> {
        if dim_config.drill_down_terms_indexing() != DrillDownTermsIndexing::NONE {
            // index full-path drill down term
            doc.add(StringField::new(
                index_field_name.to_string(),
                Self::path_to_string_len(&facet_label.components, facet_label.length)?,
                Store::NO,
            )?);

            match dim_config.drill_down_terms_indexing() {
                DrillDownTermsIndexing::NONE | DrillDownTermsIndexing::FULL_PATH_ONLY => {}
                DrillDownTermsIndexing::DIMENSION_AND_FULL_PATH => {
                    doc.add(StringField::new(
                        index_field_name.to_string(),
                        Self::path_to_string_len(&facet_label.components, 1)?,
                        Store::NO,
                    )?);
                }
                DrillDownTermsIndexing::ALL_PATHS_NO_DIM => {
                    for i in 2..facet_label.length {
                        doc.add(StringField::new(
                            index_field_name.to_string(),
                            Self::path_to_string_len(&facet_label.components, i)?,
                            Store::NO,
                        )?);
                    }
                }
                DrillDownTermsIndexing::ALL => {
                    for i in 1..facet_label.length {
                        doc.add(StringField::new(
                            index_field_name.to_string(),
                            Self::path_to_string_len(&facet_label.components, i)?,
                            Store::NO,
                        )?);
                    }
                }
            }
        }
        Ok(())
    }

    /// Equivalent to `processSSDVFacetFields(Map, Document)`.
    fn process_ssdv_facet_fields(
        &self,
        by_field: &JavaHashMap<Vec<Arc<dyn IndexableField>>>,
        doc: &mut Document,
    ) -> Result<()> {
        for (index_field_name, fields) in by_field.iter() {
            for field in fields {
                let facet_field = field
                    .as_any()
                    .and_then(|a| a.downcast_ref::<SortedSetDocValuesFacetField>())
                    .expect("INVARIANT: collated as a SortedSetDocValuesFacetField");
                let facet_label = FacetLabel::with_dim(&facet_field.dim, &facet_field.path)?;
                let dim_config = self.get_dim_config(&facet_field.dim);
                if dim_config.hierarchical() {
                    for i in 0..facet_label.length {
                        let full_path = Self::path_to_string_len(&facet_label.components, i + 1)?;
                        doc.add(SortedSetDocValuesField::new(
                            index_field_name.clone(),
                            BytesRef::from_char_sequence(&full_path),
                        ));
                    }
                } else {
                    if facet_label.length != 2 {
                        return Err(LuceneError::illegal_argument(format!(
                            "dimension \"{}\" is not hierarchical yet has {} components",
                            facet_field.dim,
                            facet_field.path.len()
                        )));
                    }
                    if dim_config.multi_valued() && dim_config.require_dim_count() {
                        doc.add(SortedSetDocValuesField::new(
                            index_field_name.clone(),
                            BytesRef::from_char_sequence(&facet_field.dim),
                        ));
                    }
                    let full_path =
                        Self::path_to_string_len(&facet_label.components, facet_label.length)?;
                    doc.add(SortedSetDocValuesField::new(
                        index_field_name.clone(),
                        BytesRef::from_char_sequence(&full_path),
                    ));
                }

                // Drill down:
                self.index_drill_down_terms(doc, index_field_name, &dim_config, &facet_label)?;
            }
        }
        Ok(())
    }

    /// Equivalent to `processAssocFacetFields(TaxonomyWriter, Map, Document)`.
    fn process_assoc_facet_fields(
        &self,
        taxo_writer: Option<&dyn TaxonomyWriter>,
        by_field: &JavaHashMap<Vec<Arc<dyn IndexableField>>>,
        doc: &mut Document,
    ) -> Result<()> {
        for (index_field_name, fields) in by_field.iter() {
            let mut bytes: Vec<u8> = vec![0; 16];
            let mut upto = 0usize;
            for field in fields {
                let field = field
                    .as_any()
                    .and_then(|a| a.downcast_ref::<AssociationFacetField>())
                    .expect("INVARIANT: collated as an AssociationFacetField");
                // NOTE: we don't add parents for associations
                let taxo_writer = Self::check_taxo_writer(taxo_writer)?;
                let facet_label = FacetLabel::with_dim(&field.dim, &field.path)?;
                let ordinal = taxo_writer.add_category(&facet_label)?;
                if upto + 4 > bytes.len() {
                    bytes.resize(rucene::util::ArrayUtil::oversize(upto + 4, 1), 0);
                }
                bytes[upto..upto + 4].copy_from_slice(&ordinal.to_be_bytes());
                upto += 4;
                let assoc_length = field.assoc.length;
                if upto + assoc_length > bytes.len() {
                    bytes.resize(rucene::util::ArrayUtil::oversize(upto + assoc_length, 1), 0);
                }
                bytes[upto..upto + assoc_length].copy_from_slice(field.assoc.slice());
                upto += assoc_length;

                // Drill down:
                self.index_drill_down_terms(
                    doc,
                    index_field_name,
                    &self.get_dim_config(&field.dim),
                    &facet_label,
                )?;
            }
            doc.add(BinaryDocValuesField::new(
                index_field_name.clone(),
                BytesRef::new(bytes[..upto].to_vec()),
            ));
        }
        Ok(())
    }

    /// Equivalent to `checkTaxoWriter(TaxonomyWriter)`.
    fn check_taxo_writer(taxo_writer: Option<&dyn TaxonomyWriter>) -> Result<&dyn TaxonomyWriter> {
        taxo_writer.ok_or_else(|| {
            LuceneError::illegal_state(
                "a non-null TaxonomyWriter must be provided when indexing FacetField or AssociationFacetField",
            )
        })
    }

    /// Turns a dim + path into an encoded string.
    ///
    /// Equivalent to `pathToString(String, String...)`.
    pub fn path_to_string_dim(dim: &str, path: &[impl AsRef<str>]) -> Result<String> {
        let mut full_path: Vec<String> = Vec::with_capacity(1 + path.len());
        full_path.push(dim.to_string());
        full_path.extend(path.iter().map(|p| p.as_ref().to_string()));
        Self::path_to_string_len(&full_path, full_path.len())
    }

    /// Turns a dim + path into an encoded string.
    ///
    /// Equivalent to `pathToString(String[])`.
    pub fn path_to_string(path: &[impl AsRef<str>]) -> Result<String> {
        Self::path_to_string_len(path, path.len())
    }

    /// Turns the first `length` elements of `path` into an encoded string.
    ///
    /// Equivalent to `pathToString(String[], int)`.
    pub fn path_to_string_len(path: &[impl AsRef<str>], length: usize) -> Result<String> {
        if length == 0 {
            return Ok(String::new());
        }
        let mut sb = String::new();
        for s in &path[..length] {
            let s = s.as_ref();
            if s.is_empty() {
                return Err(LuceneError::illegal_argument(
                    "each path component must have length > 0 (got: \"\")",
                ));
            }
            for ch in s.chars() {
                if ch == DELIM_CHAR || ch == ESCAPE_CHAR {
                    sb.push(ESCAPE_CHAR);
                }
                sb.push(ch);
            }
            sb.push(DELIM_CHAR);
        }

        // Trim off last DELIM_CHAR:
        sb.pop();
        Ok(sb)
    }

    /// Turns an encoded string (from a previous call to `pathToString`) back
    /// into the original `String[]`.
    ///
    /// Equivalent to `stringToPath(String)`.
    pub fn string_to_path(s: &str) -> Vec<String> {
        let mut parts = Vec::new();
        if s.is_empty() {
            return Vec::new();
        }
        let mut buffer = String::with_capacity(s.len());
        let mut last_escape = false;
        for ch in s.chars() {
            if last_escape {
                buffer.push(ch);
                last_escape = false;
            } else if ch == ESCAPE_CHAR {
                last_escape = true;
            } else if ch == DELIM_CHAR {
                parts.push(std::mem::take(&mut buffer));
            } else {
                buffer.push(ch);
            }
        }
        parts.push(buffer);
        debug_assert!(!last_escape);
        parts
    }
}
