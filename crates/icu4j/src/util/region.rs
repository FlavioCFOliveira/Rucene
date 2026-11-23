// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.Region`.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, LazyLock, Mutex};

use rucene::analysis::java_regex::JavaPattern;
use rucene::error::{LuceneError, Result};

use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_resource_bundle;
use crate::util::u_resource_bundle::UResourceBundle;

/// `RegionType`: the types of regions.
///
/// Equivalent to the enum `Region.RegionType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RegionType {
    /// `UNKNOWN`: the unknown region (`ZZ`).
    Unknown,
    /// `TERRITORY`: a territory, such as `US`.
    Territory,
    /// `WORLD`: the world (`001`).
    World,
    /// `CONTINENT`: a continent.
    Continent,
    /// `SUBCONTINENT`: a sub-continent or a numeric region below a
    /// continent.
    Subcontinent,
    /// `GROUPING`: a grouping that is not a geographic region (`EU`).
    Grouping,
    /// `DEPRECATED`: a deprecated region.
    Deprecated,
}

impl RegionType {
    const VALUES: [RegionType; 7] = [
        RegionType::Unknown,
        RegionType::Territory,
        RegionType::World,
        RegionType::Continent,
        RegionType::Subcontinent,
        RegionType::Grouping,
        RegionType::Deprecated,
    ];

    fn ordinal(self) -> usize {
        self as usize
    }
}

/// `UNKNOWN_REGION_ID`.
const UNKNOWN_REGION_ID: &str = "ZZ";
/// `OUTLYING_OCEANIA_REGION_ID`.
const OUTLYING_OCEANIA_REGION_ID: &str = "QO";
/// `WORLD_ID`.
const WORLD_ID: &str = "001";

/// The fields of one region.
#[derive(Debug, Clone)]
struct RegionNode {
    id: String,
    code: i32,
    ty: RegionType,
    containing_region: Option<usize>,
    /// `containedRegions`, a `TreeSet<Region>` in id order.
    contained_regions: Vec<usize>,
    preferred_values: Option<Vec<usize>>,
}

/// The region data: Java's static maps and lists.
#[derive(Debug)]
struct RegionData {
    regions: Vec<RegionNode>,
    region_id_map: HashMap<String, usize>,
    numeric_code_map: HashMap<i32, usize>,
    region_aliases: HashMap<String, usize>,
    available_regions: Vec<Vec<usize>>,
}

/// `regionDataIsLoaded` and the data.
static REGION_DATA: Mutex<Option<Arc<RegionData>>> = Mutex::new(None);

/// A region: a geographic or political region, a continent, a grouping or
/// the world, with its containment.
///
/// Equivalent to `com.ibm.icu.util.Region`: Java's instances form a graph
/// built once by `loadRegionData`; here a region is an index into the
/// loaded data, which holds every region's fields.
#[derive(Clone)]
pub struct Region {
    data: Arc<RegionData>,
    idx: usize,
}

impl fmt::Debug for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Region({})", self.node().id)
    }
}

fn sorted_insert(data_regions: &[RegionNode], set: &mut Vec<usize>, r: usize) {
    let key = &data_regions[r].id;
    match set.binary_search_by(|&x| rucene::util::java_collections::java_compare_to(&data_regions[x].id, key)) {
        Ok(_) => {}
        Err(i) => set.insert(i, r),
    }
}

impl Region {
    fn node(&self) -> &RegionNode {
        &self.data.regions[self.idx]
    }

    fn of(&self, idx: usize) -> Region {
        Region {
            data: self.data.clone(),
            idx,
        }
    }

    /// `loadRegionData()` (private): initializes the region data from the
    /// ICU resource bundles, once.
    fn load_region_data() -> Result<Arc<RegionData>> {
        let mut guard = REGION_DATA.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(d) = guard.as_ref() {
            return Ok(d.clone());
        }
        static THREE_DIGITS: LazyLock<JavaPattern> =
            LazyLock::new(|| JavaPattern::compile("[0-9]{3}", 0).expect("valid pattern"));

        let mut region_aliases: HashMap<String, usize> = HashMap::new();
        let mut region_id_map: HashMap<String, usize> = HashMap::new();
        let mut numeric_code_map: HashMap<i32, usize> = HashMap::new();
        let mut available_regions: Vec<Vec<usize>> = Vec::with_capacity(RegionType::VALUES.len());

        let loader = icu_resource_bundle::icu_data_class_loader();
        let metadata = UResourceBundle::get_bundle_instance_loader(Some(ICUData::ICU_BASE_NAME), "metadata", loader)?;
        let metadata_alias = metadata.get("alias")?;
        let territory_alias = metadata_alias.get("territory")?;

        let supplemental_data =
            UResourceBundle::get_bundle_instance_loader(Some(ICUData::ICU_BASE_NAME), "supplementalData", loader)?;
        let code_mappings = supplemental_data.get("codeMappings")?;
        let id_validity = supplemental_data.get("idValidity")?;
        let region_list = id_validity.get("region")?;
        let region_regular = region_list.get("regular")?;
        let region_macro = region_list.get("macroregion")?;
        let region_unknown = region_list.get("unknown")?;

        let territory_containment = supplemental_data.get("territoryContainment")?;
        let world_containment = territory_containment.get("001")?;
        let grouping_containment = territory_containment.get("grouping")?;

        let to_s = |v: Vec<u16>| String::from_utf16_lossy(&v);
        let continents: Vec<String> = world_containment.get_string_array()?.into_iter().map(to_s).collect();
        let groupings = grouping_containment.get_keys()?;

        let mut region_codes: Vec<String> = Vec::new();
        let mut all_regions: Vec<String> = Vec::new();
        all_regions.extend(region_regular.get_string_array()?.into_iter().map(to_s));
        all_regions.extend(region_macro.get_string_array()?.into_iter().map(to_s));
        all_regions.push(to_s(region_unknown.get_string()?));

        for r in &all_regions {
            let units: Vec<u16> = r.encode_utf16().collect();
            let range_marker_location = crate::jdk::index_of_unit(&units, b'~' as u16, 0);
            if range_marker_location > 0 {
                let rml = range_marker_location as usize;
                let mut region_name: Vec<u16> = units.clone();
                let end_range = region_name[rml + 1];
                region_name.truncate(rml);
                let mut last_char = region_name[rml - 1];
                while last_char <= end_range {
                    region_codes.push(String::from_utf16_lossy(&region_name));
                    last_char = last_char.wrapping_add(1);
                    region_name[rml - 1] = last_char;
                }
            } else {
                region_codes.push(r.clone());
            }
        }

        let mut regions: Vec<RegionNode> = Vec::with_capacity(region_codes.len());

        // First process the region codes and create the primary array of regions.
        for id in &region_codes {
            let mut r = RegionNode {
                id: id.clone(),
                code: -1,
                ty: RegionType::Territory, // Only temporary - figure out the real type later once the aliases are known.
                containing_region: None,
                contained_regions: Vec::new(),
                preferred_values: None,
            };
            let idx = regions.len();
            region_id_map.insert(id.clone(), idx);
            if THREE_DIGITS.matches(id) {
                r.code = id.parse::<i32>().map_err(|_| LuceneError::number_format_for_input_string(id, 10))?;
                numeric_code_map.insert(r.code, idx);
                r.ty = RegionType::Subcontinent;
            } else {
                r.code = -1;
            }
            regions.push(r);
        }

        // Process the territory aliases
        for i in 0..territory_alias.get_size() {
            let res = territory_alias.get_at(i)?;
            let alias_from = res.get_key().unwrap_or_else(|| "null".to_string());
            let alias_to = to_s(res.get("replacement")?.get_string()?);

            if region_id_map.contains_key(&alias_to) && !region_id_map.contains_key(&alias_from) {
                // This is just an alias from some string to a region
                region_aliases.insert(alias_from, region_id_map[&alias_to]);
            } else {
                let r = if let Some(&r) = region_id_map.get(&alias_from) {
                    // This is a deprecated region
                    r
                } else {
                    // Deprecated region code not in the primary codes list - so need to create a deprecated region for it.
                    let idx = regions.len();
                    let mut node = RegionNode {
                        id: alias_from.clone(),
                        code: -1,
                        ty: RegionType::Territory,
                        containing_region: None,
                        contained_regions: Vec::new(),
                        preferred_values: None,
                    };
                    region_id_map.insert(alias_from.clone(), idx);
                    if THREE_DIGITS.matches(&alias_from) {
                        node.code = alias_from
                            .parse::<i32>()
                            .map_err(|_| LuceneError::number_format_for_input_string(&alias_from, 10))?;
                        numeric_code_map.insert(node.code, idx);
                    } else {
                        node.code = -1;
                    }
                    regions.push(node);
                    idx
                };
                regions[r].ty = RegionType::Deprecated;
                let mut preferred = Vec::new();
                for s in alias_to.split(' ') {
                    if let Some(&p) = region_id_map.get(s) {
                        preferred.push(p);
                    }
                }
                regions[r].preferred_values = Some(preferred);
            }
        }

        // Process the code mappings - This will allow us to assign numeric codes to most of the territories.
        for i in 0..code_mappings.get_size() {
            let mapping = code_mappings.get_at(i)?;
            if mapping.get_type() == UResourceBundle::ARRAY {
                let code_mapping_strings: Vec<String> = mapping.get_string_array()?.into_iter().map(to_s).collect();
                let code_mapping_id = &code_mapping_strings[0];
                let units: Vec<u16> = code_mapping_strings[1].encode_utf16().collect();
                let code_mapping_number = crate::jdk::parse_int(&units, 10)?;
                let code_mapping_3letter = &code_mapping_strings[2];

                if let Some(&r) = region_id_map.get(code_mapping_id) {
                    regions[r].code = code_mapping_number;
                    numeric_code_map.insert(code_mapping_number, r);
                    region_aliases.insert(code_mapping_3letter.clone(), r);
                }
            }
        }

        // Now fill in the special cases for WORLD, UNKNOWN, CONTINENTS, and GROUPINGS
        if let Some(&r) = region_id_map.get(WORLD_ID) {
            regions[r].ty = RegionType::World;
        }

        if let Some(&r) = region_id_map.get(UNKNOWN_REGION_ID) {
            regions[r].ty = RegionType::Unknown;
        }

        for continent in &continents {
            if let Some(&r) = region_id_map.get(continent) {
                regions[r].ty = RegionType::Continent;
            }
        }

        for grouping in &groupings {
            if let Some(&r) = region_id_map.get(grouping) {
                regions[r].ty = RegionType::Grouping;
            }
        }

        // Special case: The region code "QO" (Outlying Oceania) is a subcontinent code added by CLDR
        // even though it looks like a territory code.  Need to handle it here.
        if let Some(&r) = region_id_map.get(OUTLYING_OCEANIA_REGION_ID) {
            regions[r].ty = RegionType::Subcontinent;
        }

        // Load territory containment info from the supplemental data.
        for i in 0..territory_containment.get_size() {
            let mapping = territory_containment.get_at(i)?;
            let parent = mapping.get_key().unwrap_or_else(|| "null".to_string());
            if parent == "containedGroupings" || parent == "deprecated" || parent == "grouping" {
                continue; // handle new pseudo-parent types added in ICU data per cldrbug 7808; for now just skip.
                // #11232 is to do something useful with these.
                // Also skip "grouping" which has multi-level structure below from CLDR 34.
            }
            let parent_region = region_id_map.get(&parent).copied();
            for j in 0..mapping.get_size() {
                let child = to_s(mapping.get_string_at(j)?);
                let child_region = region_id_map.get(&child).copied();
                if let (Some(p), Some(c)) = (parent_region, child_region) {
                    // Add the child region to the set of regions contained by the parent
                    let mut set = std::mem::take(&mut regions[p].contained_regions);
                    sorted_insert(&regions, &mut set, c);
                    regions[p].contained_regions = set;

                    // Set the parent region to be the containing region of the child.
                    // Regions of type GROUPING can't be set as the parent, since another region
                    // such as a SUBCONTINENT, CONTINENT, or WORLD must always be the parent.
                    if regions[p].ty != RegionType::Grouping {
                        regions[c].containing_region = Some(p);
                    }
                }
            }
        }

        // Fill in the grouping containment resource as well
        for i in 0..grouping_containment.get_size() {
            let mapping = grouping_containment.get_at(i)?;
            let parent = mapping.get_key().unwrap_or_else(|| "null".to_string());
            let parent_region = region_id_map.get(&parent).copied();
            for j in 0..mapping.get_size() {
                let child = to_s(mapping.get_string_at(j)?);
                let child_region = region_id_map.get(&child).copied();
                if let (Some(p), Some(c)) = (parent_region, child_region) {
                    // Add the child region to the set of regions contained by the parent
                    let mut set = std::mem::take(&mut regions[p].contained_regions);
                    sorted_insert(&regions, &mut set, c);
                    regions[p].contained_regions = set;
                    // Do NOT change the parent of the child region, since groupings are
                    // never the primary parent of a region.
                }
            }
        }

        // Create the availableRegions lists
        for _ in 0..RegionType::VALUES.len() {
            available_regions.push(Vec::new());
        }

        for ar in 0..regions.len() {
            let t = regions[ar].ty.ordinal();
            let mut set = std::mem::take(&mut available_regions[t]);
            sorted_insert(&regions, &mut set, ar);
            available_regions[t] = set;
        }

        let data = Arc::new(RegionData {
            regions,
            region_id_map,
            numeric_code_map,
            region_aliases,
            available_regions,
        });
        *guard = Some(data.clone());
        Ok(data)
    }

    fn resolve(data: Arc<RegionData>, idx: usize) -> Region {
        let node = &data.regions[idx];
        if node.ty == RegionType::Deprecated {
            if let Some(p) = &node.preferred_values {
                if p.len() == 1 {
                    let first = p[0];
                    return Region { data, idx: first };
                }
            }
        }
        Region { data, idx }
    }

    /// `getInstance(String id)`: the region of a code (two-letter, numeric
    /// or alias); a deprecated region with one preferred value is replaced
    /// by it.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Unknown region id: ...")`; the errors of
    /// loading the region data.
    pub fn get_instance(id: &str) -> Result<Region> {
        let data = Self::load_region_data()?;
        let r = data
            .region_id_map
            .get(id)
            .or_else(|| data.region_aliases.get(id))
            .copied();
        match r {
            None => Err(LuceneError::illegal_argument(format!("Unknown region id: {id}"))),
            Some(r) => Ok(Self::resolve(data, r)),
        }
    }

    /// `getInstance(int code)`: the region of a numeric code.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Unknown region code: ...")`.
    pub fn get_instance_code(code: i32) -> Result<Region> {
        let data = Self::load_region_data()?;
        let mut r = data.numeric_code_map.get(&code).copied();
        if r.is_none() {
            // Just in case there's an alias that's numeric, try to find it.
            let pad = if code < 10 {
                "00"
            } else if code < 100 {
                "0"
            } else {
                ""
            };
            let id = format!("{pad}{code}");
            r = data.region_aliases.get(&id).copied();
        }
        match r {
            None => Err(LuceneError::illegal_argument(format!("Unknown region code: {code}"))),
            Some(r) => Ok(Self::resolve(data, r)),
        }
    }

    /// `getAvailable(RegionType type)`: the regions of a type, in id order.
    ///
    /// # Errors
    ///
    /// The errors of loading the region data.
    pub fn get_available(ty: RegionType) -> Result<Vec<Region>> {
        let data = Self::load_region_data()?;
        Ok(data.available_regions[ty.ordinal()]
            .iter()
            .map(|&i| Region { data: data.clone(), idx: i })
            .collect())
    }

    /// `getContainingRegion()`.
    pub fn get_containing_region(&self) -> Option<Region> {
        self.node().containing_region.map(|i| self.of(i))
    }

    /// `getContainingRegion(RegionType type)`: the nearest containing region
    /// of the type.
    pub fn get_containing_region_of(&self, ty: RegionType) -> Option<Region> {
        let c = self.get_containing_region()?;
        if c.node().ty == ty {
            Some(c)
        } else {
            c.get_containing_region_of(ty)
        }
    }

    /// `getContainedRegions()`: in id order.
    pub fn get_contained_regions(&self) -> Vec<Region> {
        self.node().contained_regions.iter().map(|&i| self.of(i)).collect()
    }

    /// `getContainedRegions(RegionType type)`: the contained regions of the
    /// type, at any depth, in id order.
    pub fn get_contained_regions_of(&self, ty: RegionType) -> Vec<Region> {
        let mut result: Vec<usize> = Vec::new();
        for r in self.get_contained_regions() {
            if r.get_type() == ty {
                sorted_insert(&self.data.regions, &mut result, r.idx);
            } else {
                for x in r.get_contained_regions_of(ty) {
                    sorted_insert(&self.data.regions, &mut result, x.idx);
                }
            }
        }
        result.into_iter().map(|i| self.of(i)).collect()
    }

    /// `getPreferredValues()`: for a deprecated region, its replacements.
    pub fn get_preferred_values(&self) -> Option<Vec<Region>> {
        if self.node().ty == RegionType::Deprecated {
            self.node()
                .preferred_values
                .as_ref()
                .map(|p| p.iter().map(|&i| self.of(i)).collect())
        } else {
            None
        }
    }

    /// `contains(Region other)`: whether `other` is contained at any depth.
    pub fn contains(&self, other: &Region) -> bool {
        let contained = &self.node().contained_regions;
        if contained.iter().any(|&i| self.data.regions[i].id == other.node().id) {
            true
        } else {
            contained.iter().any(|&i| self.of(i).contains(other))
        }
    }

    /// `getNumericCode()`.
    pub fn get_numeric_code(&self) -> i32 {
        self.node().code
    }

    /// `getType()`.
    pub fn get_type(&self) -> RegionType {
        self.node().ty
    }

    /// `compareTo(Region other)`: by id.
    pub fn compare_to(&self, other: &Region) -> i32 {
        crate::r#impl::locale::ascii_util::string_compare(&self.node().id, &other.node().id)
    }
}

/// `toString()`: the id.
impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.node().id)
    }
}

/// Java's `Region` identity: one instance per id.
impl PartialEq for Region {
    fn eq(&self, other: &Self) -> bool {
        self.node().id == other.node().id
    }
}

impl Eq for Region {}
