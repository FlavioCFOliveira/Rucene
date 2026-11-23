// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.VersionInfo`.

use std::cmp::Ordering;
use std::fmt;

use rucene::error::{LuceneError, Result};

/// A version number of the form `major.minor.milli.micro`, each part in
/// `0..=255`, packed into one `int`.
///
/// Equivalent to `com.ibm.icu.util.VersionInfo`. Java interns the instances
/// in a map keyed by the packed value, so that `equals` (which is `==`)
/// holds exactly when the values are equal; here a `VersionInfo` is the
/// packed value itself, compared by value, which is the same relation, and
/// the interning map has no counterpart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VersionInfo {
    /// `m_version_`: the packed version.
    m_version: i32,
}

impl VersionInfo {
    /// `UNICODE_1_0`: Unicode 1.0.0.
    pub const UNICODE_1_0: VersionInfo = VersionInfo::of(1, 0, 0, 0);
    /// `UNICODE_1_0_1`: Unicode 1.0.1.
    pub const UNICODE_1_0_1: VersionInfo = VersionInfo::of(1, 0, 1, 0);
    /// `UNICODE_1_1_0`: Unicode 1.1.0.
    pub const UNICODE_1_1_0: VersionInfo = VersionInfo::of(1, 1, 0, 0);
    /// `UNICODE_1_1_5`: Unicode 1.1.5.
    pub const UNICODE_1_1_5: VersionInfo = VersionInfo::of(1, 1, 5, 0);
    /// `UNICODE_2_0`: Unicode 2.0.0.
    pub const UNICODE_2_0: VersionInfo = VersionInfo::of(2, 0, 0, 0);
    /// `UNICODE_2_1_2`: Unicode 2.1.2.
    pub const UNICODE_2_1_2: VersionInfo = VersionInfo::of(2, 1, 2, 0);
    /// `UNICODE_2_1_5`: Unicode 2.1.5.
    pub const UNICODE_2_1_5: VersionInfo = VersionInfo::of(2, 1, 5, 0);
    /// `UNICODE_2_1_8`: Unicode 2.1.8.
    pub const UNICODE_2_1_8: VersionInfo = VersionInfo::of(2, 1, 8, 0);
    /// `UNICODE_2_1_9`: Unicode 2.1.9.
    pub const UNICODE_2_1_9: VersionInfo = VersionInfo::of(2, 1, 9, 0);
    /// `UNICODE_3_0`: Unicode 3.0.0.
    pub const UNICODE_3_0: VersionInfo = VersionInfo::of(3, 0, 0, 0);
    /// `UNICODE_3_0_1`: Unicode 3.0.1.
    pub const UNICODE_3_0_1: VersionInfo = VersionInfo::of(3, 0, 1, 0);
    /// `UNICODE_3_1_0`: Unicode 3.1.0.
    pub const UNICODE_3_1_0: VersionInfo = VersionInfo::of(3, 1, 0, 0);
    /// `UNICODE_3_1_1`: Unicode 3.1.1.
    pub const UNICODE_3_1_1: VersionInfo = VersionInfo::of(3, 1, 1, 0);
    /// `UNICODE_3_2`: Unicode 3.2.0.
    pub const UNICODE_3_2: VersionInfo = VersionInfo::of(3, 2, 0, 0);
    /// `UNICODE_4_0`: Unicode 4.0.0.
    pub const UNICODE_4_0: VersionInfo = VersionInfo::of(4, 0, 0, 0);
    /// `UNICODE_4_0_1`: Unicode 4.0.1.
    pub const UNICODE_4_0_1: VersionInfo = VersionInfo::of(4, 0, 1, 0);
    /// `UNICODE_4_1`: Unicode 4.1.0.
    pub const UNICODE_4_1: VersionInfo = VersionInfo::of(4, 1, 0, 0);
    /// `UNICODE_5_0`: Unicode 5.0.0.
    pub const UNICODE_5_0: VersionInfo = VersionInfo::of(5, 0, 0, 0);
    /// `UNICODE_5_1`: Unicode 5.1.0.
    pub const UNICODE_5_1: VersionInfo = VersionInfo::of(5, 1, 0, 0);
    /// `UNICODE_5_2`: Unicode 5.2.0.
    pub const UNICODE_5_2: VersionInfo = VersionInfo::of(5, 2, 0, 0);
    /// `UNICODE_6_0`: Unicode 6.0.0.
    pub const UNICODE_6_0: VersionInfo = VersionInfo::of(6, 0, 0, 0);
    /// `UNICODE_6_1`: Unicode 6.1.0.
    pub const UNICODE_6_1: VersionInfo = VersionInfo::of(6, 1, 0, 0);
    /// `UNICODE_6_2`: Unicode 6.2.0.
    pub const UNICODE_6_2: VersionInfo = VersionInfo::of(6, 2, 0, 0);
    /// `UNICODE_6_3`: Unicode 6.3.0.
    pub const UNICODE_6_3: VersionInfo = VersionInfo::of(6, 3, 0, 0);
    /// `UNICODE_7_0`: Unicode 7.0.0.
    pub const UNICODE_7_0: VersionInfo = VersionInfo::of(7, 0, 0, 0);
    /// `UNICODE_8_0`: Unicode 8.0.0.
    pub const UNICODE_8_0: VersionInfo = VersionInfo::of(8, 0, 0, 0);
    /// `UNICODE_9_0`: Unicode 9.0.0.
    pub const UNICODE_9_0: VersionInfo = VersionInfo::of(9, 0, 0, 0);
    /// `UNICODE_10_0`: Unicode 10.0.0.
    pub const UNICODE_10_0: VersionInfo = VersionInfo::of(10, 0, 0, 0);
    /// `UNICODE_11_0`: Unicode 11.0.0.
    pub const UNICODE_11_0: VersionInfo = VersionInfo::of(11, 0, 0, 0);
    /// `UNICODE_12_0`: Unicode 12.0.0.
    pub const UNICODE_12_0: VersionInfo = VersionInfo::of(12, 0, 0, 0);
    /// `UNICODE_12_1`: Unicode 12.1.0.
    pub const UNICODE_12_1: VersionInfo = VersionInfo::of(12, 1, 0, 0);
    /// `UNICODE_13_0`: Unicode 13.0.0.
    pub const UNICODE_13_0: VersionInfo = VersionInfo::of(13, 0, 0, 0);
    /// `UNICODE_14_0`: Unicode 14.0.0.
    pub const UNICODE_14_0: VersionInfo = VersionInfo::of(14, 0, 0, 0);
    /// `UNICODE_15_0`: Unicode 15.0.0.
    pub const UNICODE_15_0: VersionInfo = VersionInfo::of(15, 0, 0, 0);
    /// `UNICODE_15_1`: Unicode 15.1.0.
    pub const UNICODE_15_1: VersionInfo = VersionInfo::of(15, 1, 0, 0);
    /// `UNICODE_16_0`: Unicode 16.0.0.
    pub const UNICODE_16_0: VersionInfo = VersionInfo::of(16, 0, 0, 0);

    /// `ICU_VERSION`: 77.1.
    pub const ICU_VERSION: VersionInfo = VersionInfo::of(77, 1, 0, 0);

    /// `ICU_DATA_VERSION_PATH` (deprecated): the data path suffix `"77b"`.
    pub const ICU_DATA_VERSION_PATH: &'static str = "77b";

    /// `ICU_DATA_VERSION` (deprecated): `ICU_VERSION`.
    pub const ICU_DATA_VERSION: VersionInfo = Self::ICU_VERSION;

    /// `UCOL_RUNTIME_VERSION`: 9.
    pub const UCOL_RUNTIME_VERSION: VersionInfo = VersionInfo::of(9, 0, 0, 0);

    /// `UCOL_BUILDER_VERSION`: 9.
    pub const UCOL_BUILDER_VERSION: VersionInfo = VersionInfo::of(9, 0, 0, 0);

    /// `UCOL_TAILORINGS_VERSION` (deprecated): 1.
    pub const UCOL_TAILORINGS_VERSION: VersionInfo = VersionInfo::of(1, 0, 0, 0);

    /// `UNICODE_VERSION` (private): `UNICODE_16_0`.
    pub(crate) const UNICODE_VERSION: VersionInfo = Self::UNICODE_16_0;

    /// `LAST_BYTE_MASK_`.
    const LAST_BYTE_MASK: i32 = 0xFF;

    /// `INVALID_VERSION_NUMBER_`.
    const INVALID_VERSION_NUMBER: &'static str =
        "Invalid version number: Version number may be negative or greater than 255";

    /// A constant of known-valid parts.
    const fn of(major: i32, minor: i32, milli: i32, micro: i32) -> Self {
        Self {
            m_version: Self::get_int(major, minor, milli, micro),
        }
    }

    /// `getInstance(String version)`: parses up to four dot-separated
    /// decimal parts.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a character other than a digit or a
    /// dot, more than four parts, or a part above 255.
    pub fn get_instance_str(version: &[u16]) -> Result<VersionInfo> {
        let length = version.len();
        let mut array = [0i32; 4];
        let mut count = 0usize;
        let mut index = 0usize;

        while count < 4 && index < length {
            let mut c = version[index];
            if c == b'.' as u16 {
                count += 1;
            } else {
                c = c.wrapping_sub(b'0' as u16);
                // `c` is a char: `c < 0` never holds.
                if c > 9 {
                    return Err(LuceneError::illegal_argument(Self::INVALID_VERSION_NUMBER));
                }
                array[count] = array[count].wrapping_mul(10);
                array[count] = array[count].wrapping_add(c as i32);
            }
            index += 1;
        }
        if index != length {
            return Err(LuceneError::illegal_argument(format!(
                "Invalid version number: String '{}' exceeds version format",
                String::from_utf16_lossy(version)
            )));
        }
        for v in array {
            if !(0..=255).contains(&v) {
                return Err(LuceneError::illegal_argument(Self::INVALID_VERSION_NUMBER));
            }
        }

        Self::get_instance4(array[0], array[1], array[2], array[3])
    }

    /// `getInstance(int major, int minor, int milli, int micro)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when a part is outside `0..=255`.
    pub fn get_instance4(major: i32, minor: i32, milli: i32, micro: i32) -> Result<VersionInfo> {
        // checks if it is in the hashmap
        // else
        if !(0..=255).contains(&major)
            || !(0..=255).contains(&minor)
            || !(0..=255).contains(&milli)
            || !(0..=255).contains(&micro)
        {
            return Err(LuceneError::illegal_argument(Self::INVALID_VERSION_NUMBER));
        }
        Ok(Self::of(major, minor, milli, micro))
    }

    /// `getInstance(int major, int minor, int milli)`.
    ///
    /// # Errors
    ///
    /// As [`get_instance4`](Self::get_instance4).
    pub fn get_instance3(major: i32, minor: i32, milli: i32) -> Result<VersionInfo> {
        Self::get_instance4(major, minor, milli, 0)
    }

    /// `getInstance(int major, int minor)`.
    ///
    /// # Errors
    ///
    /// As [`get_instance4`](Self::get_instance4).
    pub fn get_instance2(major: i32, minor: i32) -> Result<VersionInfo> {
        Self::get_instance4(major, minor, 0, 0)
    }

    /// `getInstance(int major)`.
    ///
    /// # Errors
    ///
    /// As [`get_instance4`](Self::get_instance4).
    pub fn get_instance(major: i32) -> Result<VersionInfo> {
        Self::get_instance4(major, 0, 0, 0)
    }

    /// `getMajor()`.
    pub fn get_major(&self) -> i32 {
        (self.m_version >> 24) & Self::LAST_BYTE_MASK
    }

    /// `getMinor()`.
    pub fn get_minor(&self) -> i32 {
        (self.m_version >> 16) & Self::LAST_BYTE_MASK
    }

    /// `getMilli()`.
    pub fn get_milli(&self) -> i32 {
        (self.m_version >> 8) & Self::LAST_BYTE_MASK
    }

    /// `getMicro()`.
    pub fn get_micro(&self) -> i32 {
        self.m_version & Self::LAST_BYTE_MASK
    }

    /// `hashCode()`: the packed value.
    pub fn hash_code(&self) -> i32 {
        self.m_version
    }

    /// `compareTo(VersionInfo other)`: compares the packed values as
    /// unsigned numbers (the top 31 bits, then the lowest bit).
    pub fn compare_to(&self, other: &VersionInfo) -> i32 {
        // m_version_ is an int, a signed 32-bit integer.
        // When the major version is >=128, then the version int is negative.
        // Compare it in two steps to simulate an unsigned-int comparison.
        // (Alternatively we could turn each int into a long and reset the upper 32 bits.)
        // Compare the upper bits first, using logical shift right (unsigned).
        let diff = ((self.m_version as u32) >> 1) as i32 - ((other.m_version as u32) >> 1) as i32;
        if diff != 0 {
            return diff;
        }
        // Compare the remaining bits.
        (self.m_version & 1) - (other.m_version & 1)
    }

    /// `getVersionString(int minDigits, int maxDigits)` (deprecated): the
    /// parts joined with dots, trailing zero parts dropped down to
    /// `min_digits`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Invalid min/maxDigits range")` unless
    /// `1 <= min_digits <= max_digits <= 4`.
    pub fn get_version_string(&self, min_digits: i32, max_digits: i32) -> Result<String> {
        if min_digits < 1 || max_digits < 1 || min_digits > 4 || max_digits > 4 || min_digits > max_digits {
            return Err(LuceneError::illegal_argument("Invalid min/maxDigits range"));
        }

        let digits = [self.get_major(), self.get_minor(), self.get_milli(), self.get_micro()];

        let mut num_digits = max_digits;
        while num_digits > min_digits {
            if digits[(num_digits - 1) as usize] != 0 {
                break;
            }
            num_digits -= 1;
        }

        let mut ver_str = String::with_capacity(7);
        ver_str.push_str(&digits[0].to_string());
        for d in digits.iter().take(num_digits as usize).skip(1) {
            ver_str.push('.');
            ver_str.push_str(&d.to_string());
        }

        Ok(ver_str)
    }

    /// `main(String[] args)`: prints the ICU, Unicode, CLDR and time zone
    /// data versions (`System.out`), as the class's `main` method does.
    ///
    /// # Errors
    ///
    /// The errors of reading the CLDR and time zone data versions.
    pub fn main(_args: &[String]) -> Result<()> {
        let icu_version = Self::ICU_VERSION;
        let icu_api_ver = if icu_version.get_major() <= 4 {
            if icu_version.get_minor() % 2 != 0 {
                // Development mile stone
                let mut major = icu_version.get_major();
                let mut minor = icu_version.get_minor() + 1;
                if minor >= 10 {
                    minor -= 10;
                    major += 1;
                }
                format!("{}.{}M{}", major, minor, icu_version.get_milli())
            } else {
                icu_version.get_version_string(2, 2)?
            }
        } else if icu_version.get_minor() == 0 {
            // Development mile stone
            format!("{}M{}", icu_version.get_major(), icu_version.get_milli())
        } else {
            icu_version.get_version_string(2, 2)?
        };

        println!("International Components for Unicode for Java {icu_api_ver}");

        println!();
        println!("Implementation Version: {}", icu_version.get_version_string(2, 4)?);
        println!("Unicode Data Version:   {}", Self::UNICODE_VERSION.get_version_string(2, 4)?);
        println!(
            "CLDR Data Version:      {}",
            crate::util::locale_data::LocaleData::get_cldr_version()?.get_version_string(2, 4)?
        );
        println!("Time Zone Data Version: {}", Self::get_tz_data_version()?);
        Ok(())
    }

    /// `getTZDataVersion()` (package-private): the `TZVersion` of the
    /// `zoneinfo64` bundle, read once.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the bundle or the key is missing.
    pub(crate) fn get_tz_data_version() -> Result<String> {
        static TZDATA_VERSION: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
        let mut v = TZDATA_VERSION.lock().unwrap_or_else(|e| e.into_inner());
        if v.is_none() {
            let tzbundle = crate::util::u_resource_bundle::UResourceBundle::get_bundle_instance(
                Some(&format!("com/ibm/icu/impl/{}", crate::r#impl::icu_data::ICUData::ICU_BUNDLE)),
                "zoneinfo64",
            )?;
            *v = Some(String::from_utf16_lossy(&tzbundle.get_string_by_key("TZVersion")?));
        }
        Ok(v.clone().expect("set above"))
    }

    /// `getInt(int major, int minor, int milli, int micro)` (private).
    const fn get_int(major: i32, minor: i32, milli: i32, micro: i32) -> i32 {
        (major << 24) | (minor << 16) | (milli << 8) | micro
    }
}

impl fmt::Display for VersionInfo {
    /// `toString()`: `major.minor.milli.micro`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}.{}.{}.{}",
            self.get_major(),
            self.get_minor(),
            self.get_milli(),
            self.get_micro()
        )
    }
}

impl PartialOrd for VersionInfo {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for VersionInfo {
    /// The order of [`compare_to`](VersionInfo::compare_to).
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare_to(other).cmp(&0)
    }
}
