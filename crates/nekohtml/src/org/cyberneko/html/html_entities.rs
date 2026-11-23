/* 
 * Copyright 2002-2009 Andy Clark, Marc Guillemot
 * 
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `HTMLEntities.java`, of the Java package `org.cyberneko.html` (NekoHTML 1.9.17).

use std::collections::HashMap;
use std::sync::LazyLock;

use rucene_queryparser::jdk::resource_bundle::load;

/// Pre-defined HTML entities.
///
/// Equivalent to `org.cyberneko.html.HTMLEntities`: its static methods are
/// the functions of this module; the entity tables are loaded once from
/// NekoHTML 1.9.17's `res/*.properties` resources, as its static
/// initializer loads them.
#[derive(Debug)]
pub struct HTMLEntities;

/// `Properties.load(InputStream)` of one resource: ISO-8859-1 text.
fn load0(props: &mut HashMap<String, String>, data: &[u8]) {
    let text: Vec<u16> = data.iter().map(|&b| b as u16).collect();
    // IOException: "error: unable to load resource" is never met, the
    // resources are embedded
    match load(&text) {
        Ok(map) => props.extend(map),
        Err(_) => eprintln!("error: unable to load resource"),
    }
}

/// Entities: `ENTITIES`.
static ENTITIES: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    let mut props = HashMap::new();
    // load entities
    load0(&mut props, include_bytes!("res/HTMLlat1.properties"));
    load0(&mut props, include_bytes!("res/HTMLspecial.properties"));
    load0(&mut props, include_bytes!("res/HTMLsymbol.properties"));
    load0(&mut props, include_bytes!("res/XMLbuiltin.properties"));
    props
});

/// Reverse mapping from characters to names: `SEITITNE`.
static SEITITNE: LazyLock<IntProperties> = LazyLock::new(|| {
    let mut seititne = IntProperties::new();
    // store reverse mappings
    for (key, value) in ENTITIES.iter() {
        let units: Vec<u16> = value.encode_utf16().collect();
        if units.len() == 1 {
            seititne.put(units[0] as i32, key);
        }
    }
    seititne
});

/// Returns the character associated to the given entity name, or -1 if the
/// name is not known.
///
/// Equivalent to `get(String)`.
pub fn get(name: &str) -> i32 {
    match ENTITIES.get(name) {
        Some(value) => value.encode_utf16().next().map_or(-1, |c| c as i32),
        None => -1,
    }
}

/// Returns the name associated to the given character or null if the
/// character is not known.
///
/// Equivalent to `get(int)`.
pub fn get_name(c: i32) -> Option<String> {
    SEITITNE.get(c).map(str::to_string)
}

/// Equivalent to the package-private nested class
/// `HTMLEntities.IntProperties`: a chained table of 101 buckets.
#[derive(Debug)]
struct IntProperties {
    entries: Vec<Vec<(i32, String)>>,
}

impl IntProperties {
    fn new() -> Self {
        Self {
            entries: (0..101).map(|_| Vec::new()).collect(),
        }
    }

    fn put(&mut self, key: i32, value: &str) {
        let hash = (key % self.entries.len() as i32) as usize;
        self.entries[hash].insert(0, (key, value.to_string()));
    }

    fn get(&self, key: i32) -> Option<&str> {
        let hash = (key % self.entries.len() as i32) as usize;
        self.entries[hash].iter().find(|e| e.0 == key).map(|e| e.1.as_str())
    }
}
