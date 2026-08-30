//! Fast approximate math ported from `org.apache.lucene.util.SloppyMath`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`SloppyMath`] | `SloppyMath` |
//!
//! **Divergence from Lucene 10.5.0.** The lookup tables are seeded at first use
//! from `sin`, `cos`, `asin` and `sqrt`. Java seeds them from `StrictMath`,
//! whose results are pinned to the fdlibm algorithms and therefore identical on
//! every JVM; Rust's `f64` methods call the platform libm, which is
//! correctly-rounded-or-nearly on mainstream targets but is not guaranteed to
//! agree with fdlibm in the last unit in the last place. Every other operation
//! here — the table indexing, the polynomial evaluation, the fdlibm-derived
//! `asin` tail, the bit-masking in [`SloppyMath::haversin_sort_key`] — is
//! reproduced exactly, so any difference is bounded by the seeding error and
//! is far below the ~1e-7 error the class already documents. Closing it would
//! require vendoring fdlibm, which is a dependency decision for the user.

#![deny(unsafe_code)]

use std::sync::LazyLock;

/// Fast, approximate trigonometry and geodesic distance.
///
/// Port of `org.apache.lucene.util.SloppyMath`.
pub struct SloppyMath;

/// Equatorial radius in metres. `SloppyMath.TO_METERS`.
const TO_METERS: f64 = 6_371_008.771_4;

const ONE_DIV_F2: f64 = 1.0 / 2.0;
const ONE_DIV_F3: f64 = 1.0 / 6.0;
const ONE_DIV_F4: f64 = 1.0 / 24.0;

/// `SloppyMath.SIN_COS_TABS_SIZE`.
const SIN_COS_TABS_SIZE: usize = (1 << 11) + 1;
/// `SloppyMath.ASIN_TABS_SIZE`.
const ASIN_TABS_SIZE: usize = (1 << 13) + 1;

/// The tables and derived constants Java builds in a static initialiser.
struct Tables {
    sin_cos_indexer: f64,
    sin_cos_delta_hi: f64,
    sin_cos_delta_lo: f64,
    sin_cos_max_value_for_int_modulo: f64,
    sin_tab: Vec<f64>,
    cos_tab: Vec<f64>,

    asin_max_value_for_tabs: f64,
    asin_delta: f64,
    asin_indexer: f64,
    asin_tab: Vec<f64>,
    asin_der1_div_f1_tab: Vec<f64>,
    asin_der2_div_f2_tab: Vec<f64>,
    asin_der3_div_f3_tab: Vec<f64>,
    asin_der4_div_f4_tab: Vec<f64>,

    asin_pio2_hi: f64,
    asin_pio2_lo: f64,
    asin_ps0: f64,
    asin_ps1: f64,
    asin_ps2: f64,
    asin_ps3: f64,
    asin_ps4: f64,
    asin_ps5: f64,
    asin_qs1: f64,
    asin_qs2: f64,
    asin_qs3: f64,
    asin_qs4: f64,
}

/// `java.lang.Math.toRadians`.
fn to_radians(angdeg: f64) -> f64 {
    angdeg / 180.0 * std::f64::consts::PI
}

static TABLES: LazyLock<Tables> = LazyLock::new(|| {
    let pio2_hi = f64::from_bits(0x3FF9_21FB_5440_0000);
    let pio2_lo = f64::from_bits(0x3DD0_B461_1A62_6331);
    let twopi_hi = 4.0 * pio2_hi;
    let twopi_lo = 4.0 * pio2_lo;
    let sin_cos_delta_hi = twopi_hi / (SIN_COS_TABS_SIZE - 1) as f64;
    let sin_cos_delta_lo = twopi_lo / (SIN_COS_TABS_SIZE - 1) as f64;
    let sin_cos_indexer = 1.0 / (sin_cos_delta_hi + sin_cos_delta_lo);
    let sin_cos_max_value_for_int_modulo = (((i32::MAX >> 9) as f64) / sin_cos_indexer) * 0.99;

    let mut sin_tab = vec![0.0f64; SIN_COS_TABS_SIZE];
    let mut cos_tab = vec![0.0f64; SIN_COS_TABS_SIZE];

    let sin_cos_pi_index = (SIN_COS_TABS_SIZE - 1) / 2;
    let sin_cos_pi_mul_2_index = 2 * sin_cos_pi_index;
    let sin_cos_pi_mul_0_5_index = sin_cos_pi_index / 2;
    let sin_cos_pi_mul_1_5_index = 3 * sin_cos_pi_index / 2;
    for i in 0..SIN_COS_TABS_SIZE {
        let angle = i as f64 * sin_cos_delta_hi + i as f64 * sin_cos_delta_lo;
        let mut sin_angle = angle.sin();
        let mut cos_angle = angle.cos();
        // Java writes four separate branches; the four indices are distinct,
        // so pairing them changes nothing.
        if i == sin_cos_pi_index || i == sin_cos_pi_mul_2_index {
            sin_angle = 0.0;
        } else if i == sin_cos_pi_mul_0_5_index || i == sin_cos_pi_mul_1_5_index {
            cos_angle = 0.0;
        }
        sin_tab[i] = sin_angle;
        cos_tab[i] = cos_angle;
    }

    let asin_max_value_for_tabs = to_radians(73.0).sin();
    let asin_delta = asin_max_value_for_tabs / (ASIN_TABS_SIZE - 1) as f64;
    let asin_indexer = 1.0 / asin_delta;

    let mut asin_tab = vec![0.0f64; ASIN_TABS_SIZE];
    let mut asin_der1_div_f1_tab = vec![0.0f64; ASIN_TABS_SIZE];
    let mut asin_der2_div_f2_tab = vec![0.0f64; ASIN_TABS_SIZE];
    let mut asin_der3_div_f3_tab = vec![0.0f64; ASIN_TABS_SIZE];
    let mut asin_der4_div_f4_tab = vec![0.0f64; ASIN_TABS_SIZE];
    for i in 0..ASIN_TABS_SIZE {
        let x = i as f64 * asin_delta;
        asin_tab[i] = x.asin();
        let one_minus_x_sq_inv = 1.0 / (1.0 - x * x);
        let one_minus_x_sq_inv_0_5 = one_minus_x_sq_inv.sqrt();
        let one_minus_x_sq_inv_1_5 = one_minus_x_sq_inv_0_5 * one_minus_x_sq_inv;
        let one_minus_x_sq_inv_2_5 = one_minus_x_sq_inv_1_5 * one_minus_x_sq_inv;
        let one_minus_x_sq_inv_3_5 = one_minus_x_sq_inv_2_5 * one_minus_x_sq_inv;
        asin_der1_div_f1_tab[i] = one_minus_x_sq_inv_0_5;
        asin_der2_div_f2_tab[i] = (x * one_minus_x_sq_inv_1_5) * ONE_DIV_F2;
        asin_der3_div_f3_tab[i] = ((1.0 + 2.0 * x * x) * one_minus_x_sq_inv_2_5) * ONE_DIV_F3;
        asin_der4_div_f4_tab[i] =
            ((5.0 + 2.0 * x * (2.0 + x * (5.0 - 2.0 * x))) * one_minus_x_sq_inv_3_5) * ONE_DIV_F4;
    }

    Tables {
        sin_cos_indexer,
        sin_cos_delta_hi,
        sin_cos_delta_lo,
        sin_cos_max_value_for_int_modulo,
        sin_tab,
        cos_tab,
        asin_max_value_for_tabs,
        asin_delta,
        asin_indexer,
        asin_tab,
        asin_der1_div_f1_tab,
        asin_der2_div_f2_tab,
        asin_der3_div_f3_tab,
        asin_der4_div_f4_tab,
        asin_pio2_hi: f64::from_bits(0x3FF9_21FB_5444_2D18),
        asin_pio2_lo: f64::from_bits(0x3C91_A626_3314_5C07),
        asin_ps0: f64::from_bits(0x3fc5_5555_5555_5555),
        asin_ps1: f64::from_bits(0xbfd4_d612_03eb_6f7d),
        asin_ps2: f64::from_bits(0x3fc9_c155_0e88_4455),
        asin_ps3: f64::from_bits(0xbfa4_8228_b568_8f3b),
        asin_ps4: f64::from_bits(0x3f49_efe0_7501_b288),
        asin_ps5: f64::from_bits(0x3f02_3de1_0dfd_f709),
        asin_qs1: f64::from_bits(0xc003_3a27_1c8a_2d4b),
        asin_qs2: f64::from_bits(0x4000_2ae5_9c59_8ac8),
        asin_qs3: f64::from_bits(0xbfe6_066c_1b8d_0159),
        asin_qs4: f64::from_bits(0x3fb3_b8c5_b12e_9282),
    }
});

impl SloppyMath {
    /// Returns the distance in metres between two points on the earth's
    /// surface, using the haversine formula.
    ///
    /// Equivalent to `SloppyMath.haversinMeters(double, double, double, double)`.
    pub fn haversin_meters(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
        Self::haversin_meters_from_sort_key(Self::haversin_sort_key(lat1, lon1, lat2, lon2))
    }

    /// Converts a sort key returned by [`Self::haversin_sort_key`] into metres.
    ///
    /// Equivalent to `SloppyMath.haversinMeters(double)`.
    pub fn haversin_meters_from_sort_key(sort_key: f64) -> f64 {
        TO_METERS * 2.0 * Self::asin(1.0f64.min((sort_key * 0.5).sqrt()))
    }

    /// Returns a sort key that orders points by haversine distance without
    /// paying for the final conversion to metres.
    ///
    /// Equivalent to `SloppyMath.haversinSortKey`, including the masking of the
    /// three low mantissa bits, which makes the key stable under the small
    /// perturbations the approximation introduces.
    pub fn haversin_sort_key(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
        let x1 = to_radians(lat1);
        let x2 = to_radians(lat2);
        let h1 = 1.0 - Self::cos(x1 - x2);
        let h2 = 1.0 - Self::cos(to_radians(lon1 - lon2));
        let h = h1 + Self::cos(x1) * Self::cos(x2) * h2;
        f64::from_bits(h.to_bits() & 0xFFFF_FFFF_FFFF_FFF8)
    }

    /// Returns the cosine of `a`, with an error around 1e-15.
    ///
    /// Equivalent to `SloppyMath.cos`.
    pub fn cos(a: f64) -> f64 {
        let t = &*TABLES;
        let a = if a < 0.0 { -a } else { a };
        if a > t.sin_cos_max_value_for_int_modulo {
            return a.cos();
        }
        // The index is possibly outside the table's range.
        let mut index = (a * t.sin_cos_indexer + 0.5) as i32;
        let delta = (a - index as f64 * t.sin_cos_delta_hi) - index as f64 * t.sin_cos_delta_lo;
        // Bring the index inside the table's range. The last value of each table
        // repeats the first, so it is ignored for the modulo.
        index &= (SIN_COS_TABS_SIZE - 2) as i32;
        let index = index as usize;
        let index_cos = t.cos_tab[index];
        let index_sin = t.sin_tab[index];
        index_cos
            + delta
                * (-index_sin
                    + delta
                        * (-index_cos * ONE_DIV_F2
                            + delta * (index_sin * ONE_DIV_F3 + delta * index_cos * ONE_DIV_F4)))
    }

    /// Returns the arc sine of `a` in `[-pi/2, pi/2]`, with an error around
    /// 1e-7. Returns NaN when `|a| > 1` or `a` is NaN.
    ///
    /// Equivalent to `SloppyMath.asin`.
    pub fn asin(a: f64) -> f64 {
        let t = &*TABLES;
        let (a, negate_result) = if a < 0.0 { (-a, true) } else { (a, false) };
        if a <= t.asin_max_value_for_tabs {
            let index = (a * t.asin_indexer + 0.5) as i32 as usize;
            let delta = a - index as f64 * t.asin_delta;
            let result = t.asin_tab[index]
                + delta
                    * (t.asin_der1_div_f1_tab[index]
                        + delta
                            * (t.asin_der2_div_f2_tab[index]
                                + delta
                                    * (t.asin_der3_div_f3_tab[index]
                                        + delta * t.asin_der4_div_f4_tab[index])));
            return if negate_result { -result } else { result };
        }
        // `a > ASIN_MAX_VALUE_FOR_TABS`, or `a` is NaN. This part is derived
        // from fdlibm.
        if a < 1.0 {
            let x = (1.0 - a) * 0.5;
            let p = x
                * (t.asin_ps0
                    + x * (t.asin_ps1
                        + x * (t.asin_ps2 + x * (t.asin_ps3 + x * (t.asin_ps4 + x * t.asin_ps5)))));
            let q = 1.0 + x * (t.asin_qs1 + x * (t.asin_qs2 + x * (t.asin_qs3 + x * t.asin_qs4)));
            let s = x.sqrt();
            let z = s + s * (p / q);
            let result = t.asin_pio2_hi - ((z + z) - t.asin_pio2_lo);
            if negate_result {
                -result
            } else {
                result
            }
        } else if a == 1.0 {
            if negate_result {
                -std::f64::consts::FRAC_PI_2
            } else {
                std::f64::consts::FRAC_PI_2
            }
        } else {
            f64::NAN
        }
    }

    /// Returns the sine of `a`, with an error around 1e-15.
    ///
    /// Equivalent to `SloppyMath.sin`, which is defined as `cos(a - pi/2)`.
    pub fn sin(a: f64) -> f64 {
        Self::cos(a - std::f64::consts::FRAC_PI_2)
    }
}
