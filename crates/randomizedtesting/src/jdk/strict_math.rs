//! `java.lang.StrictMath.log` and `StrictMath.sqrt`.
//!
//! `java.util.Random.nextGaussian` is specified with `StrictMath`, whose
//! results are defined bit for bit as those of fdlibm 5.3. `f64::ln` calls the
//! platform's `log`, which may differ from fdlibm in the last bit, so the port
//! carries fdlibm's `e_log.c` algorithm, transcribed from fdlibm 5.3
//! (Copyright (C) 1993 by Sun Microsystems, Inc.; "Permission to use, copy,
//! modify, and distribute this software is freely granted, provided that this
//! notice is preserved."; see the crate's NOTICE). Its results are pinned
//! against JDK 25 `StrictMath.log` by `tests/fixtures/strict_math_log.tsv`.

const LN2_HI: f64 = f64::from_bits(0x3fe6_2e42_fee0_0000);
const LN2_LO: f64 = f64::from_bits(0x3dea_39ef_3579_3c76);
/// fdlibm's `0.33333333333333333`, as the double it denotes.
const ONE_THIRD: f64 = f64::from_bits(0x3fd5_5555_5555_5555);
const TWO54: f64 = f64::from_bits(0x4350_0000_0000_0000);
const LG1: f64 = f64::from_bits(0x3fe5_5555_5555_5593);
const LG2: f64 = f64::from_bits(0x3fd9_9999_9997_fa04);
const LG3: f64 = f64::from_bits(0x3fd2_4924_9422_9359);
const LG4: f64 = f64::from_bits(0x3fcc_71c5_1d8e_78af);
const LG5: f64 = f64::from_bits(0x3fc7_4664_96cb_03de);
const LG6: f64 = f64::from_bits(0x3fc3_9a09_d078_c69f);
const LG7: f64 = f64::from_bits(0x3fc2_f112_df3e_5244);

fn high_word(x: f64) -> i32 {
    (x.to_bits() >> 32) as i32
}

fn low_word(x: f64) -> u32 {
    x.to_bits() as u32
}

fn with_high_word(x: f64, hi: i32) -> f64 {
    f64::from_bits((u64::from(hi as u32) << 32) | u64::from(low_word(x)))
}

/// `StrictMath.log(x)`: fdlibm's `__ieee754_log`.
///
/// Method (fdlibm): reduce `x = 2^k * (1+f)` with `sqrt(2)/2 < 1+f < sqrt(2)`,
/// approximate `log(1+f)` through `s = f/(2+f)` and a degree-14 minimax
/// polynomial `R(s)`, then add `k*ln2` split in a high and a low part.
pub fn log(x: f64) -> f64 {
    let mut x = x;
    let mut hx = high_word(x);
    let lx = low_word(x);
    let mut k: i32 = 0;
    if hx < 0x0010_0000 {
        // x < 2^-1022
        if ((hx & 0x7fff_ffff) as u32 | lx) == 0 {
            return f64::NEG_INFINITY; // log(+-0) = -inf
        }
        if hx < 0 {
            // log(-#) = NaN, computed as fdlibm computes it: the NaN's bits
            // are the platform's default NaN, as the JVM's are.
            #[allow(clippy::eq_op)] // `x - x` is fdlibm's NaN, not zero: x is negative.
            return (x - x) / 0.0;
        }
        // Subnormal: scale x up.
        k -= 54;
        x *= TWO54;
        hx = high_word(x);
    }
    if hx >= 0x7ff0_0000 {
        return x + x;
    }
    k += (hx >> 20) - 1023;
    hx &= 0x000f_ffff;
    let i = (hx + 0x95f64) & 0x0010_0000;
    // Normalize x or x/2.
    x = with_high_word(x, hx | (i ^ 0x3ff0_0000));
    k += i >> 20;
    let f = x - 1.0;
    if (0x000f_ffff & (2 + hx)) < 3 {
        // |f| < 2^-20
        if f == 0.0 {
            if k == 0 {
                return 0.0;
            }
            let dk = f64::from(k);
            return dk * LN2_HI + dk * LN2_LO;
        }
        let r = f * f * (0.5 - ONE_THIRD * f);
        if k == 0 {
            return f - r;
        }
        let dk = f64::from(k);
        return dk * LN2_HI - ((r - dk * LN2_LO) - f);
    }
    let s = f / (2.0 + f);
    let dk = f64::from(k);
    let z = s * s;
    let mut i = hx - 0x6147a;
    let w = z * z;
    let j = 0x6b851 - hx;
    let t1 = w * (LG2 + w * (LG4 + w * LG6));
    let t2 = z * (LG1 + w * (LG3 + w * (LG5 + w * LG7)));
    i |= j;
    let r = t2 + t1;
    if i > 0 {
        let hfsq = 0.5 * f * f;
        if k == 0 {
            f - (hfsq - s * (hfsq + r))
        } else {
            dk * LN2_HI - ((hfsq - (s * (hfsq + r) + dk * LN2_LO)) - f)
        }
    } else if k == 0 {
        f - s * (f - r)
    } else {
        dk * LN2_HI - ((s * (f - r) - dk * LN2_LO) - f)
    }
}

/// `StrictMath.sqrt(x)`: the correctly rounded square root, which IEEE 754
/// fixes and `f64::sqrt` computes.
pub fn sqrt(x: f64) -> f64 {
    x.sqrt()
}
