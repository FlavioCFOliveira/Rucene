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

//! The types declared in `SortableSingleDocSource.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use std::any::Any;
use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};
use rucene::util::java_random::JavaRandom;

use crate::by_task::feeds::content_items_source::{ContentItemsSource, ContentItemsSourceBase};
use crate::by_task::feeds::content_source::ContentSource;
use crate::by_task::feeds::doc_data::DocData;
use crate::by_task::feeds::single_doc_source::SingleDocSource;
use crate::by_task::utils::config::Config;
use crate::jdk;
use crate::jdk::properties::Properties;

/// `COUNTRIES`.
const COUNTRIES: [&str; 211] = [
    "European Union",
    "United States",
    "Japan",
    "Germany",
    "China (PRC)",
    "United Kingdom",
    "France",
    "Italy",
    "Spain",
    "Canada",
    "Brazil",
    "Russia",
    "India",
    "South Korea",
    "Australia",
    "Mexico",
    "Netherlands",
    "Turkey",
    "Sweden",
    "Belgium",
    "Indonesia",
    "Switzerland",
    "Poland",
    "Norway",
    "Republic of China",
    "Saudi Arabia",
    "Austria",
    "Greece",
    "Denmark",
    "Iran",
    "South Africa",
    "Argentina",
    "Ireland",
    "Thailand",
    "Finland",
    "Venezuela",
    "Portugal",
    "Hong Kong",
    "United Arab Emirates",
    "Malaysia",
    "Czech Republic",
    "Colombia",
    "Nigeria",
    "Romania",
    "Chile",
    "Israel",
    "Singapore",
    "Philippines",
    "Pakistan",
    "Ukraine",
    "Hungary",
    "Algeria",
    "New Zealand",
    "Egypt",
    "Kuwait",
    "Peru",
    "Kazakhstan",
    "Slovakia",
    "Morocco",
    "Bangladesh",
    "Vietnam",
    "Qatar",
    "Angola",
    "Libya",
    "Iraq",
    "Croatia",
    "Luxembourg",
    "Sudan",
    "Slovenia",
    "Cuba",
    "Belarus",
    "Ecuador",
    "Serbia",
    "Oman",
    "Bulgaria",
    "Lithuania",
    "Syria",
    "Dominican Republic",
    "Tunisia",
    "Guatemala",
    "Azerbaijan",
    "Sri Lanka",
    "Kenya",
    "Latvia",
    "Turkmenistan",
    "Costa Rica",
    "Lebanon",
    "Uruguay",
    "Uzbekistan",
    "Yemen",
    "Cyprus",
    "Estonia",
    "Trinidad and Tobago",
    "Cameroon",
    "El Salvador",
    "Iceland",
    "Panama",
    "Bahrain",
    "Ivory Coast",
    "Ethiopia",
    "Tanzania",
    "Jordan",
    "Ghana",
    "Bosnia and Herzegovina",
    "Macau",
    "Burma",
    "Bolivia",
    "Brunei",
    "Botswana",
    "Honduras",
    "Gabon",
    "Uganda",
    "Jamaica",
    "Zambia",
    "Senegal",
    "Paraguay",
    "Albania",
    "Equatorial Guinea",
    "Georgia",
    "Democratic Republic of the Congo",
    "Nepal",
    "Afghanistan",
    "Cambodia",
    "Armenia",
    "Republic of the Congo",
    "Mozambique",
    "Republic of Macedonia",
    "Malta",
    "Namibia",
    "Madagascar",
    "Chad",
    "Burkina Faso",
    "Mauritius",
    "Mali",
    "The Bahamas",
    "Papua New Guinea",
    "Nicaragua",
    "Haiti",
    "Benin",
    "alestinian flag West Bank and Gaza",
    "Jersey",
    "Fiji",
    "Guinea",
    "Moldova",
    "Niger",
    "Laos",
    "Mongolia",
    "French Polynesia",
    "Kyrgyzstan",
    "Barbados",
    "Tajikistan",
    "Malawi",
    "Liechtenstein",
    "New Caledonia",
    "Kosovo",
    "Rwanda",
    "Montenegro",
    "Swaziland",
    "Guam",
    "Mauritania",
    "Guernsey",
    "Isle of Man",
    "Togo",
    "Somalia",
    "Suriname",
    "Aruba",
    "North Korea",
    "Zimbabwe",
    "Central African Republic",
    "Faroe Islands",
    "Greenland",
    "Sierra Leone",
    "Lesotho",
    "Cape Verde",
    "Eritrea",
    "Bhutan",
    "Belize",
    "Antigua and Barbuda",
    "Gibraltar",
    "Maldives",
    "San Marino",
    "Guyana",
    "Burundi",
    "Saint Lucia",
    "Djibouti",
    "British Virgin Islands",
    "Liberia",
    "Seychelles",
    "The Gambia",
    "Northern Mariana Islands",
    "Grenada",
    "Saint Vincent and the Grenadines",
    "Saint Kitts and Nevis",
    "East Timor",
    "Vanuatu",
    "Comoros",
    "Samoa",
    "Solomon Islands",
    "Guinea-Bissau",
    "American Samoa",
    "Dominica",
    "Micronesia",
    "Tonga",
    "Cook Islands",
    "Palau",
    "Marshall Islands",
    "São Tomé and Príncipe",
    "Anguilla",
    "Kiribati",
    "Tuvalu",
    "Niue",
];

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.SortableSingleDocSource`
/// (`public class SortableSingleDocSource extends SingleDocSource`): adds
/// fields appropriate for sorting: country, random_string and sort_field
/// (int). Supports the following parameters:
///
/// * **sort.rng** - defines the range for sort-by-int field (default
///   **20000**).
/// * **rand.seed** - defines the seed to initialize Random with (default
///   **13**).
#[derive(Default)]
pub struct SortableSingleDocSource {
    inner: SingleDocSource,
    /// `(sortRange, r)`.
    state: Mutex<(i32, Option<JavaRandom>)>,
}

impl SortableSingleDocSource {
    /// `new SortableSingleDocSource()`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl ContentItemsSource for SortableSingleDocSource {
    fn items_base(&self) -> &ContentItemsSourceBase {
        self.inner.items_base()
    }

    fn java_simple_name(&self) -> &'static str {
        "SortableSingleDocSource"
    }

    fn close(&self) -> Result<()> {
        self.inner.close()
    }

    fn reset_inputs(&self) -> Result<()> {
        self.inner.reset_inputs()
    }

    fn set_config(&self, config: Arc<Config>) -> Result<()> {
        self.inner.set_config(Arc::clone(&config))?;
        let sort_range = config.get_int("sort.rng", 20000)?;
        let r = JavaRandom::new(i64::from(config.get_int("rand.seed", 13)?));
        *jdk::lock(&self.state) = (sort_range, Some(r));
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ContentSource for SortableSingleDocSource {
    fn get_next_doc_data(&self, doc_data: &mut DocData) -> Result<()> {
        self.inner.get_next_doc_data(doc_data)?;
        let mut props = Properties::new();
        let mut s = jdk::lock(&self.state);
        let sort_range = s.0;
        let r = s.1.as_mut().ok_or_else(LuceneError::null_pointer)?;

        // random int
        props.set_property("sort_field", r.next_int_bound(sort_range).to_string());

        // random string
        let len = 2 + r.next_int_bound(20 - 2); // nextInt(2, 20)
        let buffer: Vec<u16> = (0..len).map(|_| r.next_int_bound(0x80) as u16).collect();
        props.set_property("random_string", String::from_utf16_lossy(&buffer));

        // random country
        props.set_property(
            "country",
            COUNTRIES[r.next_int_bound(COUNTRIES.len() as i32) as usize],
        );
        doc_data.set_props(Some(props));
        Ok(())
    }
}
