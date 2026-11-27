// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.locale.LSR`.

use std::fmt;
use std::hash::{Hash, Hasher};

use rucene::error::{LuceneError, Result};

use crate::lang::u_script::UScript;
use crate::r#impl::locale::ascii_util::len16;

/// `StringBuilder.append((char) c)`: the low 16 bits of `c` as a unit.
fn push_char(res: &mut String, c: i32) {
    res.push_str(&String::from_utf16_lossy(&[c as u16]));
}

/// A language, script and region triple, with flags of which fields were
/// explicit.
///
/// Equivalent to `com.ibm.icu.impl.locale.LSR`.
#[derive(Debug, Clone)]
pub struct LSR {
    /// `language`.
    pub language: String,
    /// `script`.
    pub script: String,
    /// `region`.
    pub region: String,
    /// `regionIndex` (package-private).
    pub region_index: i32,
    /// `flags`.
    pub flags: i32,
}

impl LSR {
    /// `REGION_INDEX_LIMIT`.
    pub const REGION_INDEX_LIMIT: i32 = 1001 + 26 * 26;
    /// `EXPLICIT_LSR`.
    pub const EXPLICIT_LSR: i32 = 7;
    /// `EXPLICIT_LANGUAGE`.
    pub const EXPLICIT_LANGUAGE: i32 = 4;
    /// `EXPLICIT_SCRIPT`.
    pub const EXPLICIT_SCRIPT: i32 = 2;
    /// `EXPLICIT_REGION`.
    pub const EXPLICIT_REGION: i32 = 1;
    /// `IMPLICIT_LSR`.
    pub const IMPLICIT_LSR: i32 = 0;
    /// `DONT_CARE_FLAGS`.
    pub const DONT_CARE_FLAGS: i32 = 0;
    /// `DEBUG_OUTPUT`.
    pub const DEBUG_OUTPUT: bool = false;

    /// `new LSR(String language, String script, String region, int flags)`.
    pub fn new(language: &str, script: &str, region: &str, flags: i32) -> Self {
        Self {
            language: language.to_string(),
            script: script.to_string(),
            region: region.to_string(),
            region_index: Self::index_for_region(region),
            flags,
        }
    }

    /// `indexForRegion(String region)`: a positive index (>0) for a
    /// well-formed region code, `0` for ill-formed strings.
    pub fn index_for_region(region: &str) -> i32 {
        let u: Vec<u16> = region.encode_utf16().collect();
        if u.len() == 2 {
            let a = u[0] as i32 - 'A' as i32;
            if !(0..=25).contains(&a) {
                return 0;
            }
            let b = u[1] as i32 - 'A' as i32;
            if !(0..=25).contains(&b) {
                return 0;
            }
            return 26 * a + b + 1001;
        } else if u.len() == 3 {
            let a = u[0] as i32 - '0' as i32;
            if !(0..=9).contains(&a) {
                return 0;
            }
            let b = u[1] as i32 - '0' as i32;
            if !(0..=9).contains(&b) {
                return 0;
            }
            let c = u[2] as i32 - '0' as i32;
            if !(0..=9).contains(&c) {
                return 0;
            }
            return (10 * a + b) * 10 + c + 1;
        }
        0
    }

    /// `isEquivalentTo(LSR other)`: equal fields, ignoring the flags.
    pub fn is_equivalent_to(&self, other: &LSR) -> bool {
        self.language == other.language
            && self.script == other.script
            && self.region == other.region
    }

    /// `hashCode()`: `Objects.hash(language, script, region, flags)`.
    pub fn hash_code(&self) -> i32 {
        use crate::r#impl::locale::ascii_util::string_hash;
        let mut h: i32 = 1;
        for v in [
            string_hash(&self.language),
            string_hash(&self.script),
            string_hash(&self.region),
            self.flags,
        ] {
            h = h.wrapping_mul(31).wrapping_add(v);
        }
        h
    }

    // This method is added only to support encodeToIntForResource()
    // It only support [a-z]{2,3} and will not work for other cases.
    fn encode_language_to_int(&self) -> i32 {
        let l = self.language.as_bytes();
        debug_assert!((2..=3).contains(&l.len()));
        (l[0] as i32 - 'a' as i32 + 1)
            + 27 * (l[1] as i32 - 'a' as i32 + 1)
            + if l.len() == 2 {
                0
            } else {
                27 * 27 * (l[2] as i32 - 'a' as i32 + 1)
            }
    }

    // This method is added only to support encodeToIntForResource()
    // It only support [A-Z][a-z]{3} which defined in UScript and does not work for other cases.
    fn encode_script_to_int(&self) -> Result<i32> {
        let ret = UScript::get_code_from_name(&self.script)?;
        debug_assert!(ret != UScript::INVALID_CODE);
        Ok(ret)
    }

    // This method is added only to support encodeToIntForResource()
    // It only support [A-Z]{2} and the code in m49 but does not work for other cases.
    fn encode_region_to_int(&self, m49: &[String]) -> Result<i32> {
        debug_assert!((2..=3).contains(&len16(&self.region)));
        if len16(&self.region) == 3 {
            let index = m49
                .iter()
                .position(|r| *r == self.region)
                .map_or(-1, |i| i as i32);
            debug_assert!(index >= 0);
            if index < 0 {
                return Err(LuceneError::illegal_state(format!(
                    "Please add '{}' to M49 in LocaleDistanceMapper.java",
                    self.region
                )));
            }
            return Ok(index);
        }
        let r = self.region.as_bytes();
        // 'AA' => 1+27*1  = 28
        // ...
        // 'AZ' => 1+27*26 = 703
        // 'BA' => 2+27*1  = 29
        // ...
        // 'IN' => 9+27*14 = 387
        // 'ZZ' => 26+27*26 = 728
        Ok((r[0] as i32 - 'A' as i32 + 1) + 27 * (r[1] as i32 - 'A' as i32 + 1))
    }

    /// `encodeToIntForResource(List<String> m49)`: designed to only support
    /// encoding some LSR into resources but not for other cases.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` for a numeric region missing from `m49`.
    pub fn encode_to_int_for_resource(&self, m49: &[String]) -> Result<i32> {
        Ok(
            (self.encode_language_to_int() + (27 * 27 * 27) * self.encode_region_to_int(m49)?)
                | (self.encode_script_to_int()? << 24),
        )
    }

    fn to_language(encoded: i32) -> String {
        if encoded == 0 {
            return String::new();
        }
        if encoded == 1 {
            return "skip".to_string();
        }
        let mut encoded = encoded & 0x00ffffff;
        encoded %= 27 * 27 * 27;
        let mut res = String::with_capacity(3);
        push_char(&mut res, 'a' as i32 + ((encoded % 27) - 1));
        push_char(&mut res, 'a' as i32 + (((encoded / 27) % 27) - 1));
        if encoded / (27 * 27) != 0 {
            push_char(&mut res, 'a' as i32 + ((encoded / (27 * 27)) - 1));
        }
        res
    }

    fn to_script(encoded: i32) -> Result<String> {
        if encoded == 0 {
            return Ok(String::new());
        }
        if encoded == 1 {
            return Ok("script".to_string());
        }
        let encoded = (encoded >> 24) & 0x000000ff;
        Ok(UScript::get_short_name(encoded)?.unwrap_or_else(|| "null".to_string()))
    }

    fn to_region(encoded: i32, m49: &[String]) -> String {
        if encoded == 0 || encoded == 1 {
            return String::new();
        }
        let mut encoded = encoded & 0x00ffffff;
        encoded /= 27 * 27 * 27;
        encoded %= 27 * 27;
        if encoded < 27 {
            return m49[encoded as usize].clone();
        }
        let mut res = String::with_capacity(3);
        push_char(&mut res, 'A' as i32 + ((encoded % 27) - 1));
        push_char(&mut res, 'A' as i32 + (((encoded / 27) % 27) - 1));
        res
    }

    /// `decodeInts(int[] nums, String[] m49)`.
    ///
    /// # Errors
    ///
    /// The errors of the script names lookup.
    pub fn decode_ints(nums: &[i32], m49: &[String]) -> Result<Vec<LSR>> {
        let mut lsrs = Vec::with_capacity(nums.len());
        for &n in nums {
            lsrs.push(LSR::new(
                &Self::to_language(n),
                &Self::to_script(n)?,
                &Self::to_region(n, m49),
                LSR::IMPLICIT_LSR,
            ));
        }
        Ok(lsrs)
    }
}

/// `toString()`: `language[-script][-region]`.
impl fmt::Display for LSR {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut result = self.language.clone();
        if !self.script.is_empty() {
            result.push('-');
            result.push_str(&self.script);
        }
        if !self.region.is_empty() {
            result.push('-');
            result.push_str(&self.region);
        }
        f.write_str(&result)
    }
}

/// `equals(Object obj)`: equal fields and flags.
impl PartialEq for LSR {
    fn eq(&self, other: &Self) -> bool {
        self.language == other.language
            && self.script == other.script
            && self.region == other.region
            && self.flags == other.flags
    }
}

impl Eq for LSR {}

impl Hash for LSR {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash_code().hash(state);
    }
}
