//! The `java.lang.StrictMath` functions `org.apache.lucene.util.SloppyMath`
//! seeds its lookup tables from.
//!
//! | Rucene | JDK |
//! | --- | --- |
//! | [`StrictMath`] | `java.lang.StrictMath` (fdlibm 5.3) |
//!
//! `StrictMath` pins its results to the fdlibm algorithms, so they are the same
//! on every JVM. The functions here are a line-by-line Rust translation of the
//! fdlibm 5.3 C sources published at <https://netlib.org/fdlibm/>: `s_sin.c`,
//! `s_cos.c`, `k_sin.c`, `k_cos.c`, `e_rem_pio2.c`, `k_rem_pio2.c`,
//! `e_asin.c`, `e_sqrt.c` and `s_scalbn.c`. Each of those files carries the
//! following notice, which is preserved here as its terms require:
//!
//! ```text
//! ====================================================
//! Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.
//!
//! Developed at SunSoft, a Sun Microsystems, Inc. business.
//! Permission to use, copy, modify, and distribute this
//! software is freely granted, provided that this notice
//! is preserved.
//! ====================================================
//! ```
//!
//! The C macros `__HI(x)` and `__LO(x)` (the high and low 32-bit words of a
//! `double`) are `hi` and `lo`; writing them is `from_words`. C `int`
//! arithmetic is reproduced with `i32` and C `unsigned` arithmetic with
//! wrapping `u32`. `floor` and `fabs` are the exact IEEE operations
//! [`f64::floor`] and [`f64::abs`].

#![deny(unsafe_code)]
// fdlibm produces NaN as `x - x` and `(x - x) / (x - x)` on purpose (the
// IEEE invalid operation); the translation keeps those expressions.
#![allow(clippy::eq_op)]

/// `__HI(x)`: the high 32-bit word of `x`, as a C `int`.
#[inline]
fn hi(x: f64) -> i32 {
    (x.to_bits() >> 32) as i32
}

/// `__LO(x)`: the low 32-bit word of `x`, as a C `unsigned`.
#[inline]
fn lo(x: f64) -> u32 {
    x.to_bits() as u32
}

/// The `double` whose high word is `hi` and whose low word is `lo`: the
/// result of assigning `__HI(x) = hi; __LO(x) = lo;`.
#[inline]
fn from_words(hi: i32, lo: u32) -> f64 {
    f64::from_bits((u64::from(hi as u32) << 32) | u64::from(lo))
}

/// `copysign(x, y)` (fdlibm `s_copysign.c`): `x` with the sign of `y`.
#[inline]
fn copysign(x: f64, y: f64) -> f64 {
    from_words(
        (hi(x) & 0x7fff_ffff) | (hi(y) & (0x8000_0000_u32 as i32)),
        lo(x),
    )
}

/// `scalbn(x, n)` (fdlibm `s_scalbn.c`): `x * 2**n`, computed by exponent
/// manipulation.
fn scalbn(mut x: f64, n: i32) -> f64 {
    const TWO54: f64 = f64::from_bits(0x43500000_00000000); // 1.801_439_850_948_198_4e16
    const TWOM54: f64 = f64::from_bits(0x3C900000_00000000); // 5.551_115_123_125_782_702_12e-17
    const HUGE: f64 = 1.0e300;
    const TINY: f64 = 1.0e-300;

    let mut hx = hi(x);
    let lx = lo(x);
    let mut k = (hx & 0x7ff0_0000) >> 20; // extract exponent
    if k == 0 {
        // 0 or subnormal x
        if (lx | (hx & 0x7fff_ffff) as u32) == 0 {
            return x; // +-0
        }
        x *= TWO54;
        hx = hi(x);
        k = ((hx & 0x7ff0_0000) >> 20) - 54;
        if n < -50000 {
            return TINY * x; // underflow
        }
    }
    if k == 0x7ff {
        return x + x; // NaN or Inf
    }
    k = k.wrapping_add(n);
    if k > 0x7fe {
        return HUGE * copysign(HUGE, x); // overflow
    }
    if k > 0 {
        // normal result
        return from_words((hx & (0x800f_ffff_u32 as i32)) | (k << 20), lo(x));
    }
    if k <= -54 {
        return if n > 50000 {
            // in case integer overflow in n+k
            HUGE * copysign(HUGE, x) // overflow
        } else {
            TINY * copysign(TINY, x) // underflow
        };
    }
    k += 54; // subnormal result
    from_words((hx & (0x800f_ffff_u32 as i32)) | (k << 20), lo(x)) * TWOM54
}

/// `__kernel_sin(x, y, iy)` (fdlibm `k_sin.c`): the sine on `[-pi/4, pi/4]`
/// of `x + y`, where `y` is the tail of `x` and `iy == 0` means `y` is zero.
fn kernel_sin(x: f64, y: f64, iy: i32) -> f64 {
    const HALF: f64 = f64::from_bits(0x3FE00000_00000000); // 5.000_000_000_000_000_000_00e-01
    const S1: f64 = f64::from_bits(0xBFC55555_55555549); // -1.666_666_666_666_663_243_48e-01
    const S2: f64 = f64::from_bits(0x3F811111_1110F8A6); // 8.333_333_333_322_489_461_24e-03
    const S3: f64 = f64::from_bits(0xBF2A01A0_19C161D5); // -1.984_126_982_985_794_931_34e-04
    const S4: f64 = f64::from_bits(0x3EC71DE3_57B1FE7D); // 2.755_731_370_707_006_767_89e-06
    const S5: f64 = f64::from_bits(0xBE5AE5E6_8A2B9CEB); // -2.505_076_025_340_686_341_95e-08
    const S6: f64 = f64::from_bits(0x3DE5D93A_5ACFD57C); // 1.589_690_995_211_550_102_21e-10

    let ix = hi(x) & 0x7fff_ffff; // high word of x
    if ix < 0x3e40_0000 {
        // |x| < 2**-27
        if x as i32 == 0 {
            return x; // generate inexact
        }
    }
    let z = x * x;
    let v = z * x;
    let r = S2 + z * (S3 + z * (S4 + z * (S5 + z * S6)));
    if iy == 0 {
        x + v * (S1 + z * r)
    } else {
        x - ((z * (HALF * y - v * r) - y) - v * S1)
    }
}

/// `__kernel_cos(x, y)` (fdlibm `k_cos.c`): the cosine on `[-pi/4, pi/4]` of
/// `x + y`, where `y` is the tail of `x`.
fn kernel_cos(x: f64, y: f64) -> f64 {
    const ONE: f64 = f64::from_bits(0x3FF00000_00000000); // 1.000_000_000_000_000_000_00e+00
    const C1: f64 = f64::from_bits(0x3FA55555_5555554C); // 4.166_666_666_666_660_190_37e-02
    const C2: f64 = f64::from_bits(0xBF56C16C_16C15177); // -1.388_888_888_887_410_957_49e-03
    const C3: f64 = f64::from_bits(0x3EFA01A0_19CB1590); // 2.480_158_728_947_672_941_78e-05
    const C4: f64 = f64::from_bits(0xBE927E4F_809C52AD); // -2.755_731_435_139_066_330_35e-07
    const C5: f64 = f64::from_bits(0x3E21EE9E_BDB4B1C4); // 2.087_572_321_298_174_827_90e-09
    const C6: f64 = f64::from_bits(0xBDA8FAE9_BE8838D4); // -1.135_964_755_778_819_482_65e-11

    let ix = hi(x) & 0x7fff_ffff; // ix = |x|'s high word
    if ix < 0x3e40_0000 {
        // if x < 2**27
        if x as i32 == 0 {
            return ONE; // generate inexact
        }
    }
    let z = x * x;
    let r = z * (C1 + z * (C2 + z * (C3 + z * (C4 + z * (C5 + z * C6)))));
    if ix < 0x3FD3_3333 {
        // if |x| < 0.3
        ONE - (0.5 * z - (z * r - x * y))
    } else {
        let qx = if ix > 0x3fe9_0000 {
            // x > 0.78125
            0.28125
        } else {
            from_words(ix - 0x0020_0000, 0) // x/4
        };
        let hz = 0.5 * z - qx;
        let a = ONE - qx;
        a - (hz - (z * r - x * y))
    }
}

/// Table of constants for 2/pi, 396 hex digits (476 decimal) of 2/pi
/// (`e_rem_pio2.c`, `two_over_pi`).
const TWO_OVER_PI: [i32; 66] = [
    0xA2F983, 0x6E4E44, 0x1529FC, 0x2757D1, 0xF534DD, 0xC0DB62, 0x95993C, 0x439041, 0xFE5163,
    0xABDEBB, 0xC561B7, 0x246E3A, 0x424DD2, 0xE00649, 0x2EEA09, 0xD1921C, 0xFE1DEB, 0x1CB129,
    0xA73EE8, 0x8235F5, 0x2EBB44, 0x84E99C, 0x7026B4, 0x5F7E41, 0x3991D6, 0x398353, 0x39F49C,
    0x845F8B, 0xBDF928, 0x3B1FF8, 0x97FFDE, 0x05980F, 0xEF2F11, 0x8B5A0A, 0x6D1F6D, 0x367ECF,
    0x27CB09, 0xB74F46, 0x3F669E, 0x5FEA2D, 0x7527BA, 0xC7EBE5, 0xF17B3D, 0x0739F7, 0x8A5292,
    0xEA6BFB, 0x5FB11F, 0x8D5D08, 0x560330, 0x46FC7B, 0x6BABF0, 0xCFBC20, 0x9AF436, 0x1DA9E3,
    0x91615E, 0xE61B08, 0x659985, 0x5F14A0, 0x68408D, 0xFFD880, 0x4D7327, 0x310606, 0x1556CA,
    0x73A8C9, 0x60E27B, 0xC08C6B,
];

/// High words of `n * pi/2` for `n` in `1..=32` (`e_rem_pio2.c`, `npio2_hw`).
const NPIO2_HW: [i32; 32] = [
    0x3FF921FB, 0x400921FB, 0x4012D97C, 0x401921FB, 0x401F6A7A, 0x4022D97C, 0x4025FDBB, 0x402921FB,
    0x402C463A, 0x402F6A7A, 0x4031475C, 0x4032D97C, 0x40346B9C, 0x4035FDBB, 0x40378FDB, 0x403921FB,
    0x403AB41B, 0x403C463A, 0x403DD85A, 0x403F6A7A, 0x40407E4C, 0x4041475C, 0x4042106C, 0x4042D97C,
    0x4043A28C, 0x40446B9C, 0x404534AC, 0x4045FDBB, 0x4046C6CB, 0x40478FDB, 0x404858EB, 0x404921FB,
];

/// `__ieee754_rem_pio2(x, y)` (fdlibm `e_rem_pio2.c`): returns `n` and stores
/// the remainder of `x` rem pi/2 in `y[0] + y[1]`.
fn ieee754_rem_pio2(x: f64, y: &mut [f64; 2]) -> i32 {
    const ZERO: f64 = f64::from_bits(0x00000000_00000000); // 0.000_000_000_000_000_000_00e+00
    const HALF: f64 = f64::from_bits(0x3FE00000_00000000); // 5.000_000_000_000_000_000_00e-01
    const TWO24: f64 = f64::from_bits(0x41700000_00000000); // 1.677_721_600_000_000_000_00e+07
    const INVPIO2: f64 = f64::from_bits(0x3FE45F30_6DC9C883); // 6.366_197_723_675_813_824_33e-01
    const PIO2_1: f64 = f64::from_bits(0x3FF921FB_54400000); // 1.570_796_326_734_125_614_17e+00
    const PIO2_1T: f64 = f64::from_bits(0x3DD0B461_1A626331); // 6.077_100_506_506_192_249_32e-11
    const PIO2_2: f64 = f64::from_bits(0x3DD0B461_1A600000); // 6.077_100_506_303_965_976_60e-11
    const PIO2_2T: f64 = f64::from_bits(0x3BA3198A_2E037073); // 2.022_266_248_795_950_631_54e-21
    const PIO2_3: f64 = f64::from_bits(0x3BA3198A_2E000000); // 2.022_266_248_711_166_455_80e-21
    const PIO2_3T: f64 = f64::from_bits(0x397B839A_252049C1); // 8.478_427_660_368_899_569_97e-32

    let hx = hi(x); // high word of x
    let ix = hx & 0x7fff_ffff;
    if ix <= 0x3fe9_21fb {
        // |x| ~<= pi/4, no need for reduction
        y[0] = x;
        y[1] = 0.0;
        return 0;
    }
    if ix < 0x4002_d97c {
        // |x| < 3pi/4, special case with n=+-1
        if hx > 0 {
            let mut z = x - PIO2_1;
            if ix != 0x3ff9_21fb {
                // 33+53 bit pi is good enough
                y[0] = z - PIO2_1T;
                y[1] = (z - y[0]) - PIO2_1T;
            } else {
                // near pi/2, use 33+33+53 bit pi
                z -= PIO2_2;
                y[0] = z - PIO2_2T;
                y[1] = (z - y[0]) - PIO2_2T;
            }
            return 1;
        } else {
            // negative x
            let mut z = x + PIO2_1;
            if ix != 0x3ff9_21fb {
                // 33+53 bit pi is good enough
                y[0] = z + PIO2_1T;
                y[1] = (z - y[0]) + PIO2_1T;
            } else {
                // near pi/2, use 33+33+53 bit pi
                z += PIO2_2;
                y[0] = z + PIO2_2T;
                y[1] = (z - y[0]) + PIO2_2T;
            }
            return -1;
        }
    }
    if ix <= 0x4139_21fb {
        // |x| ~<= 2^19*(pi/2), medium size
        let t = x.abs();
        let n = (t * INVPIO2 + HALF) as i32;
        let fn_ = f64::from(n);
        let mut r = t - fn_ * PIO2_1;
        let mut w = fn_ * PIO2_1T; // 1st round good to 85 bit
        if n < 32 && ix != NPIO2_HW[(n - 1) as usize] {
            y[0] = r - w; // quick check no cancellation
        } else {
            let j = ix >> 20;
            y[0] = r - w;
            let i = j - ((hi(y[0]) >> 20) & 0x7ff);
            if i > 16 {
                // 2nd iteration needed, good to 118
                let t = r;
                w = fn_ * PIO2_2;
                r = t - w;
                w = fn_ * PIO2_2T - ((t - r) - w);
                y[0] = r - w;
                let i = j - ((hi(y[0]) >> 20) & 0x7ff);
                if i > 49 {
                    // 3rd iteration need, 151 bits acc
                    let t = r; // will cover all possible cases
                    w = fn_ * PIO2_3;
                    r = t - w;
                    w = fn_ * PIO2_3T - ((t - r) - w);
                    y[0] = r - w;
                }
            }
        }
        y[1] = (r - y[0]) - w;
        if hx < 0 {
            y[0] = -y[0];
            y[1] = -y[1];
            return -n;
        }
        return n;
    }
    // all other (large) arguments
    if ix >= 0x7ff0_0000 {
        // x is inf or NaN
        y[0] = x - x;
        y[1] = y[0];
        return 0;
    }
    // set z = scalbn(|x|,ilogb(x)-23)
    let e0 = (ix >> 20) - 1046; // e0 = ilogb(z)-23;
    let mut z = from_words(ix - (e0 << 20), lo(x));
    let mut tx = [0.0f64; 3];
    for txi in tx.iter_mut().take(2) {
        *txi = f64::from(z as i32);
        z = (z - *txi) * TWO24;
    }
    tx[2] = z;
    let mut nx = 3usize;
    while tx[nx - 1] == ZERO {
        nx -= 1; // skip zero term
    }
    let n = kernel_rem_pio2(&tx, y, e0, nx as i32, 2, &TWO_OVER_PI);
    if hx < 0 {
        y[0] = -y[0];
        y[1] = -y[1];
        return -n;
    }
    n
}

/// `__kernel_rem_pio2(x, y, e0, nx, prec, ipio2)` (fdlibm `k_rem_pio2.c`):
/// the remainder of a large `x` rem pi/2, for the precisions `prec` of
/// `0..=3`; [`ieee754_rem_pio2`] calls it with `prec = 2`, so `y` holds two
/// terms, and the `prec = 3` branch (three terms) is not reachable here.
fn kernel_rem_pio2(x: &[f64], y: &mut [f64; 2], e0: i32, nx: i32, prec: i32, ipio2: &[i32]) -> i32 {
    // initial value for jk
    const INIT_JK: [i32; 4] = [2, 3, 4, 6];
    const PIO2: [f64; 8] = [
        f64::from_bits(0x3FF921FB_40000000), // 1.570_796_251_296_997_070_31e+00
        f64::from_bits(0x3E74442D_00000000), // 7.549_789_415_861_596_353_35e-08
        f64::from_bits(0x3CF84698_80000000), // 5.390_302_529_957_764_765_54e-15
        f64::from_bits(0x3B78CC51_60000000), // 3.282_003_415_807_912_941_23e-22
        f64::from_bits(0x39F01B83_80000000), // 1.270_655_753_080_676_073_49e-29
        f64::from_bits(0x387A2520_40000000), // 1.229_333_089_811_113_289_32e-36
        f64::from_bits(0x36E38222_80000000), // 2.733_700_538_164_645_596_24e-44
        f64::from_bits(0x3569F31D_00000000), // 2.167_416_838_778_048_194_44e-51
    ];
    const ZERO: f64 = 0.0;
    const ONE: f64 = 1.0;
    const TWO24: f64 = f64::from_bits(0x41700000_00000000); // 1.677_721_600_000_000_000_00e+07
    const TWON24: f64 = f64::from_bits(0x3E700000_00000000); // 5.960_464_477_539_062_500_00e-08

    let mut iq = [0i32; 20];
    let mut f = [0.0f64; 20];
    let mut fq = [0.0f64; 20];
    let mut q = [0.0f64; 20];

    // initialize jk
    let jk = INIT_JK[prec as usize];
    let jp = jk;

    // determine jx,jv,q0, note that 3>q0
    let jx = nx - 1;
    let mut jv = (e0 - 3) / 24;
    if jv < 0 {
        jv = 0;
    }
    let mut q0 = e0 - 24 * (jv + 1);

    // set up f[0] to f[jx+jk] where f[jx+jk] = ipio2[jv+jk]
    let m = jx + jk;
    for (i, j) in (0..=m).zip(jv - jx..) {
        f[i as usize] = if j < 0 {
            ZERO
        } else {
            f64::from(ipio2[j as usize])
        };
    }

    // compute q[0],q[1],...q[jk]
    for i in 0..=jk {
        let mut fw = 0.0;
        for j in 0..=jx {
            fw += x[j as usize] * f[(jx + i - j) as usize];
        }
        q[i as usize] = fw;
    }

    let mut jz = jk;
    let mut z;
    let mut n;
    let mut ih;
    loop {
        // recompute:
        // distill q[] into iq[] reversingly
        z = q[jz as usize];
        let mut i = 0i32;
        let mut j = jz;
        while j > 0 {
            let fw = f64::from((TWON24 * z) as i32);
            iq[i as usize] = (z - TWO24 * fw) as i32;
            z = q[(j - 1) as usize] + fw;
            i += 1;
            j -= 1;
        }

        // compute n
        z = scalbn(z, q0); // actual value of z
        z -= 8.0 * (z * 0.125).floor(); // trim off integer >= 8
        n = z as i32;
        z -= f64::from(n);
        ih = 0;
        if q0 > 0 {
            // need iq[jz-1] to determine n
            let i = iq[(jz - 1) as usize] >> (24 - q0);
            n += i;
            iq[(jz - 1) as usize] -= i << (24 - q0);
            ih = iq[(jz - 1) as usize] >> (23 - q0);
        } else if q0 == 0 {
            ih = iq[(jz - 1) as usize] >> 23;
        } else if z >= 0.5 {
            ih = 2;
        }

        if ih > 0 {
            // q > 0.5
            n += 1;
            let mut carry = 0;
            for i in 0..jz {
                // compute 1-q
                let j = iq[i as usize];
                if carry == 0 {
                    if j != 0 {
                        carry = 1;
                        iq[i as usize] = 0x0100_0000 - j;
                    }
                } else {
                    iq[i as usize] = 0x00ff_ffff - j;
                }
            }
            if q0 > 0 {
                // rare case: chance is 1 in 12
                match q0 {
                    1 => iq[(jz - 1) as usize] &= 0x007f_ffff,
                    2 => iq[(jz - 1) as usize] &= 0x003f_ffff,
                    _ => {}
                }
            }
            if ih == 2 {
                z = ONE - z;
                if carry != 0 {
                    z -= scalbn(ONE, q0);
                }
            }
        }

        // check if recomputation is needed
        if z == ZERO {
            let mut j = 0;
            let mut i = jz - 1;
            while i >= jk {
                j |= iq[i as usize];
                i -= 1;
            }
            if j == 0 {
                // need recomputation
                let mut k = 1;
                while iq[(jk - k) as usize] == 0 {
                    k += 1; // k = no. of terms needed
                }
                for i in (jz + 1)..=(jz + k) {
                    // add q[jz+1] to q[jz+k]
                    f[(jx + i) as usize] = f64::from(ipio2[(jv + i) as usize]);
                    let mut fw = 0.0;
                    for j in 0..=jx {
                        fw += x[j as usize] * f[(jx + i - j) as usize];
                    }
                    q[i as usize] = fw;
                }
                jz += k;
                continue; // goto recompute
            }
        }
        break;
    }

    // chop off zero terms
    if z == 0.0 {
        jz -= 1;
        q0 -= 24;
        while iq[jz as usize] == 0 {
            jz -= 1;
            q0 -= 24;
        }
    } else {
        // break z into 24-bit if necessary
        z = scalbn(z, -q0);
        if z >= TWO24 {
            let fw = f64::from((TWON24 * z) as i32);
            iq[jz as usize] = (z - TWO24 * fw) as i32;
            jz += 1;
            q0 += 24;
            iq[jz as usize] = fw as i32;
        } else {
            iq[jz as usize] = z as i32;
        }
    }

    // convert integer "bit" chunk to floating-point value
    let mut fw = scalbn(ONE, q0);
    let mut i = jz;
    while i >= 0 {
        q[i as usize] = fw * f64::from(iq[i as usize]);
        fw *= TWON24;
        i -= 1;
    }

    // compute PIo2[0,...,jp]*q[jz,...,0]
    let mut i = jz;
    while i >= 0 {
        let mut fw = 0.0;
        let mut k = 0;
        while k <= jp && k <= jz - i {
            fw += PIO2[k as usize] * q[(i + k) as usize];
            k += 1;
        }
        fq[(jz - i) as usize] = fw;
        i -= 1;
    }

    // compress fq[] into y[]
    match prec {
        0 => {
            let mut fw = 0.0;
            let mut i = jz;
            while i >= 0 {
                fw += fq[i as usize];
                i -= 1;
            }
            y[0] = if ih == 0 { fw } else { -fw };
        }
        1 | 2 => {
            let mut fw = 0.0;
            let mut i = jz;
            while i >= 0 {
                fw += fq[i as usize];
                i -= 1;
            }
            y[0] = if ih == 0 { fw } else { -fw };
            fw = fq[0] - fw;
            for i in 1..=jz {
                fw += fq[i as usize];
            }
            y[1] = if ih == 0 { fw } else { -fw };
        }
        // `case 3` writes a third term, `y[2]`; the only caller passes
        // `prec = 2`.
        _ => unreachable!("INVARIANT: ieee754_rem_pio2 passes prec = 2"),
    }
    n & 7
}

/// The fdlibm functions `SloppyMath` uses.
///
/// Equivalent to the matching static methods of `java.lang.StrictMath`.
pub struct StrictMath;

impl StrictMath {
    /// The sine of `x`, from fdlibm `s_sin.c` (with `k_sin.c`, `k_cos.c` and
    /// `e_rem_pio2.c`).
    ///
    /// Equivalent to `java.lang.StrictMath.sin(double)`.
    pub fn sin(x: f64) -> f64 {
        let mut y = [0.0f64; 2];
        let z = 0.0;

        // High word of x.
        let mut ix = hi(x);

        // |x| ~< pi/4
        ix &= 0x7fff_ffff;
        if ix <= 0x3fe9_21fb {
            kernel_sin(x, z, 0)
        } else if ix >= 0x7ff0_0000 {
            // sin(Inf or NaN) is NaN
            x - x
        } else {
            // argument reduction needed
            let n = ieee754_rem_pio2(x, &mut y);
            match n & 3 {
                0 => kernel_sin(y[0], y[1], 1),
                1 => kernel_cos(y[0], y[1]),
                2 => -kernel_sin(y[0], y[1], 1),
                _ => -kernel_cos(y[0], y[1]),
            }
        }
    }

    /// The cosine of `x`, from fdlibm `s_cos.c` (with `k_sin.c`, `k_cos.c`
    /// and `e_rem_pio2.c`).
    ///
    /// Equivalent to `java.lang.StrictMath.cos(double)`.
    pub fn cos(x: f64) -> f64 {
        let mut y = [0.0f64; 2];
        let z = 0.0;

        // High word of x.
        let mut ix = hi(x);

        // |x| ~< pi/4
        ix &= 0x7fff_ffff;
        if ix <= 0x3fe9_21fb {
            kernel_cos(x, z)
        } else if ix >= 0x7ff0_0000 {
            // cos(Inf or NaN) is NaN
            x - x
        } else {
            // argument reduction needed
            let n = ieee754_rem_pio2(x, &mut y);
            match n & 3 {
                0 => kernel_cos(y[0], y[1]),
                1 => -kernel_sin(y[0], y[1], 1),
                2 => -kernel_cos(y[0], y[1]),
                _ => kernel_sin(y[0], y[1], 1),
            }
        }
    }

    /// The arc sine of `x`, from fdlibm `e_asin.c`.
    ///
    /// Equivalent to `java.lang.StrictMath.asin(double)`.
    pub fn asin(x: f64) -> f64 {
        const ONE: f64 = f64::from_bits(0x3FF00000_00000000); // 1.000_000_000_000_000_000_00e+00
        const HUGE: f64 = 1.000e+300;
        const PIO2_HI: f64 = f64::from_bits(0x3FF921FB_54442D18); // 1.570_796_326_794_896_558_00e+00
        const PIO2_LO: f64 = f64::from_bits(0x3C91A626_33145C07); // 6.123_233_995_736_766_035_87e-17
        const PIO4_HI: f64 = f64::from_bits(0x3FE921FB_54442D18); // 7.853_981_633_974_482_789_99e-01
                                                                  // coefficient for R(x^2)
        const PS0: f64 = f64::from_bits(0x3FC55555_55555555); // 1.666_666_666_666_666_574_15e-01
        const PS1: f64 = f64::from_bits(0xBFD4D612_03EB6F7D); // -3.255_658_186_224_009_154_05e-01
        const PS2: f64 = f64::from_bits(0x3FC9C155_0E884455); // 2.012_125_321_348_629_258_81e-01
        const PS3: f64 = f64::from_bits(0xBFA48228_B5688F3B); // -4.005_553_450_067_941_140_27e-02
        const PS4: f64 = f64::from_bits(0x3F49EFE0_7501B288); // 7.915_349_942_898_145_321_76e-04
        const PS5: f64 = f64::from_bits(0x3F023DE1_0DFDF709); // 3.479_331_075_960_211_675_70e-05
        const QS1: f64 = f64::from_bits(0xC0033A27_1C8A2D4B); // -2.403_394_911_734_414_218_78e+00
        const QS2: f64 = f64::from_bits(0x40002AE5_9C598AC8); // 2.020_945_760_233_505_694_71e+00
        const QS3: f64 = f64::from_bits(0xBFE6066C_1B8D0159); // -6.882_839_716_054_532_930_30e-01
        const QS4: f64 = f64::from_bits(0x3FB3B8C5_B12E9282); // 7.703_815_055_590_193_527_91e-02

        let hx = hi(x);
        let ix = hx & 0x7fff_ffff;
        if ix >= 0x3ff0_0000 {
            // |x|>= 1
            if ((ix - 0x3ff0_0000) as u32 | lo(x)) == 0 {
                // asin(1)=+-pi/2 with inexact
                return x * PIO2_HI + x * PIO2_LO;
            }
            return (x - x) / (x - x); // asin(|x|>1) is NaN
        } else if ix < 0x3fe0_0000 {
            // |x|<0.5
            if ix < 0x3e40_0000 {
                // if |x| < 2**-27
                if HUGE + x > ONE {
                    return x; // return x with inexact if x!=0
                }
            }
            // The C source's dangling `else t = x*x;` is reached by every
            // argument that has not returned above.
            let t = x * x;
            let p = t * (PS0 + t * (PS1 + t * (PS2 + t * (PS3 + t * (PS4 + t * PS5)))));
            let q = ONE + t * (QS1 + t * (QS2 + t * (QS3 + t * QS4)));
            let w = p / q;
            return x + x * w;
        }
        // 1> |x|>= 0.5
        let mut w = ONE - x.abs();
        let mut t = w * 0.5;
        let mut p = t * (PS0 + t * (PS1 + t * (PS2 + t * (PS3 + t * (PS4 + t * PS5)))));
        let mut q = ONE + t * (QS1 + t * (QS2 + t * (QS3 + t * QS4)));
        let s = Self::sqrt(t);
        if ix >= 0x3FEF_3333 {
            // if |x| > 0.975
            w = p / q;
            t = PIO2_HI - (2.0 * (s + s * w) - PIO2_LO);
        } else {
            w = from_words(hi(s), 0);
            let c = (t - w * w) / (s + w);
            let r = p / q;
            p = 2.0 * s * r - (PIO2_LO - 2.0 * c);
            q = PIO4_HI - 2.0 * w;
            t = PIO4_HI - (p - q);
        }
        if hx > 0 {
            t
        } else {
            -t
        }
    }

    /// The correctly rounded square root of `x`, from fdlibm `e_sqrt.c`
    /// (bit-by-bit computation).
    ///
    /// Equivalent to `java.lang.StrictMath.sqrt(double)`.
    pub fn sqrt(x: f64) -> f64 {
        const ONE: f64 = 1.0;
        const TINY: f64 = 1.0e-300;
        const SIGN: u32 = 0x8000_0000;

        let mut ix0 = hi(x); // high word of x
        let mut ix1 = lo(x); // low word of x

        // take care of Inf and NaN
        if (ix0 & 0x7ff0_0000) == 0x7ff0_0000 {
            // sqrt(NaN)=NaN, sqrt(+inf)=+inf, sqrt(-inf)=sNaN
            return x * x + x;
        }
        // take care of zero
        if ix0 <= 0 {
            if ((ix0 as u32 & !SIGN) | ix1) == 0 {
                return x; // sqrt(+-0) = +-0
            } else if ix0 < 0 {
                return (x - x) / (x - x); // sqrt(-ve) = sNaN
            }
        }
        // normalize x
        let mut m = ix0 >> 20;
        if m == 0 {
            // subnormal x
            while ix0 == 0 {
                m -= 21;
                ix0 |= (ix1 >> 11) as i32;
                ix1 <<= 21;
            }
            let mut i = 0u32;
            while (ix0 & 0x0010_0000) == 0 {
                ix0 <<= 1;
                i += 1;
            }
            m -= i as i32 - 1;
            // C's `ix1>>(32-i)` with `i == 0` shifts by the word width, which
            // C leaves undefined; the bits it must contribute are none.
            ix0 |= ix1.checked_shr(32 - i).unwrap_or(0) as i32;
            ix1 = ix1.checked_shl(i).unwrap_or(0);
        }
        m -= 1023; // unbias exponent
        ix0 = (ix0 & 0x000f_ffff) | 0x0010_0000;
        if (m & 1) != 0 {
            // odd m, double x to make it even
            ix0 = ix0.wrapping_add(ix0.wrapping_add(((ix1 & SIGN) >> 31) as i32));
            ix1 = ix1.wrapping_add(ix1);
        }
        m >>= 1; // m = [m/2]

        // generate sqrt(x) bit by bit
        ix0 = ix0.wrapping_add(ix0.wrapping_add(((ix1 & SIGN) >> 31) as i32));
        ix1 = ix1.wrapping_add(ix1);
        let mut q: i32 = 0; // [q,q1] = sqrt(x)
        let mut q1: u32 = 0;
        let mut s0: i32 = 0;
        let mut s1: u32 = 0;
        let mut r: u32 = 0x0020_0000; // r = moving bit from right to left

        while r != 0 {
            let t = s0.wrapping_add(r as i32);
            if t <= ix0 {
                s0 = t.wrapping_add(r as i32);
                ix0 = ix0.wrapping_sub(t);
                q = q.wrapping_add(r as i32);
            }
            ix0 = ix0.wrapping_add(ix0.wrapping_add(((ix1 & SIGN) >> 31) as i32));
            ix1 = ix1.wrapping_add(ix1);
            r >>= 1;
        }

        r = SIGN;
        while r != 0 {
            let t1 = s1.wrapping_add(r);
            let t = s0;
            if (t < ix0) || ((t == ix0) && (t1 <= ix1)) {
                s1 = t1.wrapping_add(r);
                if ((t1 & SIGN) == SIGN) && (s1 & SIGN) == 0 {
                    s0 = s0.wrapping_add(1);
                }
                ix0 = ix0.wrapping_sub(t);
                if ix1 < t1 {
                    ix0 = ix0.wrapping_sub(1);
                }
                ix1 = ix1.wrapping_sub(t1);
                q1 = q1.wrapping_add(r);
            }
            ix0 = ix0.wrapping_add(ix0.wrapping_add(((ix1 & SIGN) >> 31) as i32));
            ix1 = ix1.wrapping_add(ix1);
            r >>= 1;
        }

        // use floating add to find out rounding direction
        if (ix0 as u32 | ix1) != 0 {
            let z = std::hint::black_box(ONE) - std::hint::black_box(TINY); // trigger inexact flag
            if z >= ONE {
                let z = std::hint::black_box(ONE) + std::hint::black_box(TINY);
                if q1 == 0xffff_ffff {
                    q1 = 0;
                    q = q.wrapping_add(1);
                } else if z > ONE {
                    if q1 == 0xffff_fffe {
                        q = q.wrapping_add(1);
                    }
                    q1 = q1.wrapping_add(2);
                } else {
                    q1 = q1.wrapping_add(q1 & 1);
                }
            }
        }
        ix0 = (q >> 1) + 0x3fe0_0000;
        ix1 = q1 >> 1;
        if (q & 1) == 1 {
            ix1 |= SIGN;
        }
        ix0 = ix0.wrapping_add(m << 20);
        from_words(ix0, ix1)
    }
}

/// `__kernel_tan(x, y, iy)` (fdlibm `k_tan.c`): the tangent on
/// `[-pi/4, pi/4]` of `x + y`, where `y` is the tail of `x`; `iy == 1`
/// returns `tan`, `iy == -1` returns `-1/tan`.
fn kernel_tan(mut x: f64, mut y: f64, iy: i32) -> f64 {
    const T: [f64; 13] = [
        f64::from_bits(0x3FD55555_55555563), // 3.33333333333334091986e-01
        f64::from_bits(0x3FC11111_1110FE7A), // 1.33333333333201242699e-01
        f64::from_bits(0x3FABA1BA_1BB341FE), // 5.39682539762260521377e-02
        f64::from_bits(0x3F9664F4_8406D637), // 2.18694882948595424599e-02
        f64::from_bits(0x3F8226E3_E96E8493), // 8.86323982359930005737e-03
        f64::from_bits(0x3F6D6D22_C9560328), // 3.59207910759131235356e-03
        f64::from_bits(0x3F57DBC8_FEE08315), // 1.45620945432529025516e-03
        f64::from_bits(0x3F4344D8_F2F26501), // 5.88041240820264096874e-04
        f64::from_bits(0x3F3026F7_1A8D1068), // 2.46463134818469906812e-04
        f64::from_bits(0x3F147E88_A03792A6), // 7.81794442939557092300e-05
        f64::from_bits(0x3F12B80F_32F0A7E9), // 7.14072491382608190305e-05
        f64::from_bits(0xBEF375CB_DB605373), // -1.85586374855275456654e-05
        f64::from_bits(0x3EFB2A70_74BF7AD4), // 2.59073051863633712884e-05
    ];
    const ONE: f64 = 1.0;
    const PIO4: f64 = f64::from_bits(0x3FE921FB_54442D18); // 7.85398163397448278999e-01
    const PIO4LO: f64 = f64::from_bits(0x3C81A626_33145C07); // 3.06161699786838301793e-17

    let hx = hi(x); // high word of x
    let ix = hx & 0x7fff_ffff; // high word of |x|
    if ix < 0x3e30_0000 {
        // x < 2**-28
        if x as i32 == 0 {
            // generate inexact
            if ((ix as u32 | lo(x)) | (iy + 1) as u32) == 0 {
                return ONE / x.abs();
            } else if iy == 1 {
                return x;
            } else {
                // compute -1 / (x+y) carefully
                let w = x + y;
                let z = from_words(hi(w), 0);
                let v = y - (z - x);
                let a = -ONE / w;
                let t = from_words(hi(a), 0);
                let s = ONE + t * z;
                return t + a * (s + t * v);
            }
        }
    }
    if ix >= 0x3FE5_9428 {
        // |x| >= 0.6744
        if hx < 0 {
            x = -x;
            y = -y;
        }
        let z = PIO4 - x;
        let w = PIO4LO - y;
        x = z + w;
        y = 0.0;
    }
    let z = x * x;
    let w = z * z;
    // Break x^5*(T[1]+x^2*T[2]+...) into
    // x^5(T[1]+x^4*T[3]+...+x^20*T[11]) +
    // x^5(x^2*(T[2]+x^4*T[4]+...+x^22*[T12]))
    let mut r = T[1] + w * (T[3] + w * (T[5] + w * (T[7] + w * (T[9] + w * T[11]))));
    let v = z * (T[2] + w * (T[4] + w * (T[6] + w * (T[8] + w * (T[10] + w * T[12])))));
    let s = z * x;
    r = y + z * (s * (r + v) + y);
    r += T[0] * s;
    let w = x + r;
    if ix >= 0x3FE5_9428 {
        let v = f64::from(iy);
        return f64::from(1 - ((hx >> 30) & 2)) * (v - 2.0 * (x - (w * w / (w + v) - r)));
    }
    if iy == 1 {
        w
    } else {
        // compute -1.0 / (x+r) accurately
        let z = from_words(hi(w), 0);
        let v = r - (z - x); // z+v = r+x
        let a = -1.0 / w; // a = -1.0/w
        let t = from_words(hi(a), 0);
        let s = 1.0 + t * z;
        t + a * (s + t * v)
    }
}

/// The fdlibm functions the spatial modules (`spatial3d`, `spatial4j`,
/// `s2-geometry`, `spatial-extras`) reach through `java.lang.Math`.
impl StrictMath {
    /// The tangent of `x`, from fdlibm `s_tan.c` (with `k_tan.c` and
    /// `e_rem_pio2.c`).
    ///
    /// Equivalent to `java.lang.StrictMath.tan(double)`.
    pub fn tan(x: f64) -> f64 {
        let mut y = [0.0f64; 2];
        let z = 0.0;

        // High word of x.
        let mut ix = hi(x);

        // |x| ~< pi/4
        ix &= 0x7fff_ffff;
        if ix <= 0x3fe9_21fb {
            kernel_tan(x, z, 1)
        } else if ix >= 0x7ff0_0000 {
            // tan(Inf or NaN) is NaN
            x - x
        } else {
            // argument reduction needed
            let n = ieee754_rem_pio2(x, &mut y);
            // 1 -- n even, -1 -- n odd
            kernel_tan(y[0], y[1], 1 - ((n & 1) << 1))
        }
    }

    /// The arc cosine of `x`, from fdlibm `e_acos.c`.
    ///
    /// Equivalent to `java.lang.StrictMath.acos(double)`.
    pub fn acos(x: f64) -> f64 {
        const ONE: f64 = 1.0;
        const PI: f64 = f64::from_bits(0x400921FB_54442D18); // 3.14159265358979311600e+00
        const PIO2_HI: f64 = f64::from_bits(0x3FF921FB_54442D18); // 1.57079632679489655800e+00
        const PIO2_LO: f64 = f64::from_bits(0x3C91A626_33145C07); // 6.12323399573676603587e-17
        const PS0: f64 = f64::from_bits(0x3FC55555_55555555); // 1.66666666666666657415e-01
        const PS1: f64 = f64::from_bits(0xBFD4D612_03EB6F7D); // -3.25565818622400915405e-01
        const PS2: f64 = f64::from_bits(0x3FC9C155_0E884455); // 2.01212532134862925881e-01
        const PS3: f64 = f64::from_bits(0xBFA48228_B5688F3B); // -4.00555345006794114027e-02
        const PS4: f64 = f64::from_bits(0x3F49EFE0_7501B288); // 7.91534994289814532176e-04
        const PS5: f64 = f64::from_bits(0x3F023DE1_0DFDF709); // 3.47933107596021167570e-05
        const QS1: f64 = f64::from_bits(0xC0033A27_1C8A2D4B); // -2.40339491173441421878e+00
        const QS2: f64 = f64::from_bits(0x40002AE5_9C598AC8); // 2.02094576023350569471e+00
        const QS3: f64 = f64::from_bits(0xBFE6066C_1B8D0159); // -6.88283971605453293030e-01
        const QS4: f64 = f64::from_bits(0x3FB3B8C5_B12E9282); // 7.70381505559019352791e-02

        let hx = hi(x);
        let ix = hx & 0x7fff_ffff;
        if ix >= 0x3ff0_0000 {
            // |x| >= 1
            if ((ix - 0x3ff0_0000) as u32 | lo(x)) == 0 {
                // |x|==1
                if hx > 0 {
                    return 0.0; // acos(1) = 0
                } else {
                    return PI + 2.0 * PIO2_LO; // acos(-1)= pi
                }
            }
            return (x - x) / (x - x); // acos(|x|>1) is NaN
        }
        if ix < 0x3fe0_0000 {
            // |x| < 0.5
            if ix <= 0x3c60_0000 {
                return PIO2_HI + PIO2_LO; // if|x|<2**-57
            }
            let z = x * x;
            let p = z * (PS0 + z * (PS1 + z * (PS2 + z * (PS3 + z * (PS4 + z * PS5)))));
            let q = ONE + z * (QS1 + z * (QS2 + z * (QS3 + z * QS4)));
            let r = p / q;
            PIO2_HI - (x - (PIO2_LO - x * r))
        } else if hx < 0 {
            // x < -0.5
            let z = (ONE + x) * 0.5;
            let p = z * (PS0 + z * (PS1 + z * (PS2 + z * (PS3 + z * (PS4 + z * PS5)))));
            let q = ONE + z * (QS1 + z * (QS2 + z * (QS3 + z * QS4)));
            let s = Self::sqrt(z);
            let r = p / q;
            let w = r * s - PIO2_LO;
            PI - 2.0 * (s + w)
        } else {
            // x > 0.5
            let z = (ONE - x) * 0.5;
            let s = Self::sqrt(z);
            let df = from_words(hi(s), 0);
            let c = (z - df * df) / (s + df);
            let p = z * (PS0 + z * (PS1 + z * (PS2 + z * (PS3 + z * (PS4 + z * PS5)))));
            let q = ONE + z * (QS1 + z * (QS2 + z * (QS3 + z * QS4)));
            let r = p / q;
            let w = r * s + c;
            2.0 * (df + w)
        }
    }

    /// The arc tangent of `x`, from fdlibm `s_atan.c`.
    ///
    /// Equivalent to `java.lang.StrictMath.atan(double)`.
    pub fn atan(mut x: f64) -> f64 {
        const ATANHI: [f64; 4] = [
            f64::from_bits(0x3FDDAC67_0561BB4F), // atan(0.5)hi 4.63647609000806093515e-01
            f64::from_bits(0x3FE921FB_54442D18), // atan(1.0)hi 7.85398163397448278999e-01
            f64::from_bits(0x3FEF730B_D281F69B), // atan(1.5)hi 9.82793723247329054082e-01
            f64::from_bits(0x3FF921FB_54442D18), // atan(inf)hi 1.57079632679489655800e+00
        ];
        const ATANLO: [f64; 4] = [
            f64::from_bits(0x3C7A2B7F_222F65E2), // atan(0.5)lo 2.26987774529616870924e-17
            f64::from_bits(0x3C81A626_33145C07), // atan(1.0)lo 3.06161699786838301793e-17
            f64::from_bits(0x3C700788_7AF0CBBD), // atan(1.5)lo 1.39033110312309984516e-17
            f64::from_bits(0x3C91A626_33145C07), // atan(inf)lo 6.12323399573676603587e-17
        ];
        const AT: [f64; 11] = [
            f64::from_bits(0x3FD55555_5555550D), // 3.33333333333329318027e-01
            f64::from_bits(0xBFC99999_9998EBC4), // -1.99999999998764832476e-01
            f64::from_bits(0x3FC24924_920083FF), // 1.42857142725034663711e-01
            f64::from_bits(0xBFBC71C6_FE231671), // -1.11111104054623557880e-01
            f64::from_bits(0x3FB745CD_C54C206E), // 9.09088713343650656196e-02
            f64::from_bits(0xBFB3B0F2_AF749A6D), // -7.69187620504482999495e-02
            f64::from_bits(0x3FB10D66_A0D03D51), // 6.66107313738753120669e-02
            f64::from_bits(0xBFADDE2D_52DEFD9A), // -5.83357013379057348645e-02
            f64::from_bits(0x3FA97B4B_24760DEB), // 4.97687799461593236017e-02
            f64::from_bits(0xBFA2B444_2C6A6C2F), // -3.65315727442169155270e-02
            f64::from_bits(0x3F90AD3A_E322DA11), // 1.62858201153657823623e-02
        ];
        const ONE: f64 = 1.0;
        const HUGE: f64 = 1.0e300;

        let hx = hi(x);
        let ix = hx & 0x7fff_ffff;
        let id: i32;
        if ix >= 0x4410_0000 {
            // if |x| >= 2^66
            if ix > 0x7ff0_0000 || (ix == 0x7ff0_0000 && lo(x) != 0) {
                return x + x; // NaN
            }
            if hx > 0 {
                return ATANHI[3] + ATANLO[3];
            } else {
                return -ATANHI[3] - ATANLO[3];
            }
        }
        if ix < 0x3fdc_0000 {
            // |x| < 0.4375
            if ix < 0x3e20_0000 {
                // |x| < 2^-29
                if HUGE + x > ONE {
                    return x; // raise inexact
                }
            }
            id = -1;
        } else {
            x = x.abs();
            if ix < 0x3ff3_0000 {
                // |x| < 1.1875
                if ix < 0x3fe6_0000 {
                    // 7/16 <=|x|<11/16
                    id = 0;
                    x = (2.0 * x - ONE) / (2.0 + x);
                } else {
                    // 11/16<=|x|< 19/16
                    id = 1;
                    x = (x - ONE) / (x + ONE);
                }
            } else if ix < 0x4003_8000 {
                // |x| < 2.4375
                id = 2;
                x = (x - 1.5) / (ONE + 1.5 * x);
            } else {
                // 2.4375 <= |x| < 2^66
                id = 3;
                x = -1.0 / x;
            }
        }
        // end of argument reduction
        let z = x * x;
        let w = z * z;
        // break sum from i=0 to 10 aT[i]z**(i+1) into odd and even poly
        let s1 = z * (AT[0] + w * (AT[2] + w * (AT[4] + w * (AT[6] + w * (AT[8] + w * AT[10])))));
        let s2 = w * (AT[1] + w * (AT[3] + w * (AT[5] + w * (AT[7] + w * AT[9]))));
        if id < 0 {
            x - x * (s1 + s2)
        } else {
            let id = id as usize;
            let z = ATANHI[id] - ((x * (s1 + s2) - ATANLO[id]) - x);
            if hx < 0 {
                -z
            } else {
                z
            }
        }
    }

    /// The angle of the point `(x, y)`, from fdlibm `e_atan2.c`.
    ///
    /// Equivalent to `java.lang.StrictMath.atan2(double, double)`.
    pub fn atan2(y: f64, x: f64) -> f64 {
        const TINY: f64 = 1.0e-300;
        const ZERO: f64 = 0.0;
        const PI_O_4: f64 = f64::from_bits(0x3FE921FB_54442D18); // 7.8539816339744827900E-01
        const PI_O_2: f64 = f64::from_bits(0x3FF921FB_54442D18); // 1.5707963267948965580E+00
        const PI: f64 = f64::from_bits(0x400921FB_54442D18); // 3.1415926535897931160E+00
        const PI_LO: f64 = f64::from_bits(0x3CA1A626_33145C07); // 1.2246467991473531772E-16

        let hx = hi(x);
        let ix = hx & 0x7fff_ffff;
        let lx = lo(x);
        let hy = hi(y);
        let iy = hy & 0x7fff_ffff;
        let ly = lo(y);
        if (ix as u32 | ((lx | lx.wrapping_neg()) >> 31)) > 0x7ff0_0000
            || (iy as u32 | ((ly | ly.wrapping_neg()) >> 31)) > 0x7ff0_0000
        {
            // x or y is NaN
            return x + y;
        }
        if (hx.wrapping_sub(0x3ff0_0000) as u32 | lx) == 0 {
            return Self::atan(y); // x=1.0
        }
        let m = ((hy >> 31) & 1) | ((hx >> 30) & 2); // 2*sign(x)+sign(y)

        // when y = 0
        if (iy as u32 | ly) == 0 {
            match m {
                0 | 1 => return y, // atan(+-0,+anything)=+-0
                2 => return PI + TINY, // atan(+0,-anything) = pi
                _ => return -PI - TINY, // atan(-0,-anything) =-pi
            }
        }
        // when x = 0
        if (ix as u32 | lx) == 0 {
            return if hy < 0 { -PI_O_2 - TINY } else { PI_O_2 + TINY };
        }
        // when x is INF
        if ix == 0x7ff0_0000 {
            if iy == 0x7ff0_0000 {
                return match m {
                    0 => PI_O_4 + TINY,        // atan(+INF,+INF)
                    1 => -PI_O_4 - TINY,       // atan(-INF,+INF)
                    2 => 3.0 * PI_O_4 + TINY,  // atan(+INF,-INF)
                    _ => -3.0 * PI_O_4 - TINY, // atan(-INF,-INF)
                };
            } else {
                return match m {
                    0 => ZERO,       // atan(+...,+INF)
                    1 => -ZERO,      // atan(-...,+INF)
                    2 => PI + TINY,  // atan(+...,-INF)
                    _ => -PI - TINY, // atan(-...,-INF)
                };
            }
        }
        // when y is INF
        if iy == 0x7ff0_0000 {
            return if hy < 0 { -PI_O_2 - TINY } else { PI_O_2 + TINY };
        }

        // compute y/x
        let k = (iy - ix) >> 20;
        let mut z = if k > 60 {
            PI_O_2 + 0.5 * PI_LO // |y/x| >  2**60
        } else if hx < 0 && k < -60 {
            0.0 // |y|/x < -2**60
        } else {
            Self::atan((y / x).abs()) // safe to do y/x
        };
        match m {
            0 => z, // atan(+,+)
            1 => {
                z = from_words(hi(z) ^ (0x8000_0000_u32 as i32), lo(z));
                z // atan(-,+)
            }
            2 => PI - (z - PI_LO), // atan(+,-)
            _ => (z - PI_LO) - PI, // atan(-,-)
        }
    }

    /// `x` raised to the power `y`, from fdlibm `e_pow.c`.
    ///
    /// Equivalent to `java.lang.StrictMath.pow(double, double)`.
    pub fn pow(x: f64, y: f64) -> f64 {
        const BP: [f64; 2] = [1.0, 1.5];
        const DP_H: [f64; 2] = [0.0, f64::from_bits(0x3FE2B803_40000000)]; // 5.84962487220764160156e-01
        const DP_L: [f64; 2] = [0.0, f64::from_bits(0x3E4CFDEB_43CFD006)]; // 1.35003920212974897128e-08
        const ZERO: f64 = 0.0;
        const ONE: f64 = 1.0;
        const TWO: f64 = 2.0;
        const TWO53: f64 = f64::from_bits(0x43400000_00000000); // 9007199254740992.0
        const HUGE: f64 = 1.0e300;
        const TINY: f64 = 1.0e-300;
        // poly coefs for (3/2)*(log(x)-2s-2/3*s**3
        const L1: f64 = f64::from_bits(0x3FE33333_33333303); // 5.99999999999994648725e-01
        const L2: f64 = f64::from_bits(0x3FDB6DB6_DB6FABFF); // 4.28571428578550184252e-01
        const L3: f64 = f64::from_bits(0x3FD55555_518F264D); // 3.33333329818377432918e-01
        const L4: f64 = f64::from_bits(0x3FD17460_A91D4101); // 2.72728123808534006489e-01
        const L5: f64 = f64::from_bits(0x3FCD864A_93C9DB65); // 2.30660745775561754067e-01
        const L6: f64 = f64::from_bits(0x3FCA7E28_4A454EEF); // 2.06975017800338417784e-01
        const P1: f64 = f64::from_bits(0x3FC55555_5555553E); // 1.66666666666666019037e-01
        const P2: f64 = f64::from_bits(0xBF66C16C_16BEBD93); // -2.77777777770155933842e-03
        const P3: f64 = f64::from_bits(0x3F11566A_AF25DE2C); // 6.61375632143793436117e-05
        const P4: f64 = f64::from_bits(0xBEBBBD41_C5D26BF1); // -1.65339022054652515390e-06
        const P5: f64 = f64::from_bits(0x3E663769_72BEA4D0); // 4.13813679705723846039e-08
        const LG2: f64 = f64::from_bits(0x3FE62E42_FEFA39EF); // 6.93147180559945286227e-01
        const LG2_H: f64 = f64::from_bits(0x3FE62E43_00000000); // 6.93147182464599609375e-01
        const LG2_L: f64 = f64::from_bits(0xBE205C61_0CA86C39); // -1.90465429995776804525e-09
        const OVT: f64 = 8.008_566_259_537_294_437_2e-17; // -(1024-log2(ovfl+.5ulp))
        const CP: f64 = f64::from_bits(0x3FEEC709_DC3A03FD); // 9.61796693925975554329e-01 =2/(3ln2)
        const CP_H: f64 = f64::from_bits(0x3FEEC709_E0000000); // 9.61796700954437255859e-01 =(float)cp
        const CP_L: f64 = f64::from_bits(0xBE3E2FE0_145B01F5); // -7.02846165095275826516e-09 =tail of cp_h
        const IVLN2: f64 = f64::from_bits(0x3FF71547_652B82FE); // 1.44269504088896338700e+00 =1/ln2
        const IVLN2_H: f64 = f64::from_bits(0x3FF71547_60000000); // 1.44269502162933349609e+00 =24b 1/ln2
        const IVLN2_L: f64 = f64::from_bits(0x3E54AE0B_F85DDF44); // 1.92596299112661746887e-08 =1/ln2 tail

        let hx = hi(x);
        let lx = lo(x);
        let hy = hi(y);
        let ly = lo(y);
        let mut ix = hx & 0x7fff_ffff;
        let iy = hy & 0x7fff_ffff;

        // y==zero: x**0 = 1
        if (iy as u32 | ly) == 0 {
            return ONE;
        }

        // +-NaN return x+y
        if ix > 0x7ff0_0000
            || (ix == 0x7ff0_0000 && lx != 0)
            || iy > 0x7ff0_0000
            || (iy == 0x7ff0_0000 && ly != 0)
        {
            return x + y;
        }

        // determine if y is an odd int when x < 0
        // yisint = 0 ... y is not an integer
        // yisint = 1 ... y is an odd int
        // yisint = 2 ... y is an even int
        let mut yisint = 0;
        if hx < 0 {
            if iy >= 0x4340_0000 {
                yisint = 2; // even integer y
            } else if iy >= 0x3ff0_0000 {
                let k = (iy >> 20) - 0x3ff; // exponent
                if k > 20 {
                    let j = ly >> (52 - k);
                    if (j << (52 - k)) == ly {
                        yisint = 2 - (j & 1) as i32;
                    }
                } else if ly == 0 {
                    let j = iy >> (20 - k);
                    if (j << (20 - k)) == iy {
                        yisint = 2 - (j & 1);
                    }
                }
            }
        }

        // special value of y
        if ly == 0 {
            if iy == 0x7ff0_0000 {
                // y is +-inf
                if (ix.wrapping_sub(0x3ff0_0000) as u32 | lx) == 0 {
                    return y - y; // inf**+-1 is NaN
                } else if ix >= 0x3ff0_0000 {
                    // (|x|>1)**+-inf = inf,0
                    return if hy >= 0 { y } else { ZERO };
                } else {
                    // (|x|<1)**-,+inf = inf,0
                    return if hy < 0 { -y } else { ZERO };
                }
            }
            if iy == 0x3ff0_0000 {
                // y is  +-1
                return if hy < 0 { ONE / x } else { x };
            }
            if hy == 0x4000_0000 {
                return x * x; // y is  2
            }
            if hy == 0x3fe0_0000 && hx >= 0 {
                // y is  0.5, x >= +0
                return Self::sqrt(x);
            }
        }

        let mut ax = x.abs();
        // special value of x
        if lx == 0 && (ix == 0x7ff0_0000 || ix == 0 || ix == 0x3ff0_0000) {
            let mut z = ax; // x is +-0,+-inf,+-1
            if hy < 0 {
                z = ONE / z; // z = (1/|x|)
            }
            if hx < 0 {
                if (ix.wrapping_sub(0x3ff0_0000) | yisint) == 0 {
                    z = (z - z) / (z - z); // (-1)**non-int is NaN
                } else if yisint == 1 {
                    z = -z; // (x<0)**odd = -(|x|**odd)
                }
            }
            return z;
        }

        let mut n = (hx >> 31) + 1;

        // (x<0)**(non-int) is NaN
        if (n | yisint) == 0 {
            return (x - x) / (x - x);
        }

        let mut s = ONE; // s (sign of result -ve**odd) = -1 else = 1
        if (n | (yisint - 1)) == 0 {
            s = -ONE; // (-ve)**(odd int)
        }

        let t1: f64;
        let t2: f64;
        // |y| is huge
        if iy > 0x41e0_0000 {
            // if |y| > 2**31
            if iy > 0x43f0_0000 {
                // if |y| > 2**64, must o/uflow
                if ix <= 0x3fef_ffff {
                    return if hy < 0 { HUGE * HUGE } else { TINY * TINY };
                }
                if ix >= 0x3ff0_0000 {
                    return if hy > 0 { HUGE * HUGE } else { TINY * TINY };
                }
            }
            // over/underflow if x is not close to one
            if ix < 0x3fef_ffff {
                return if hy < 0 { s * HUGE * HUGE } else { s * TINY * TINY };
            }
            if ix > 0x3ff0_0000 {
                return if hy > 0 { s * HUGE * HUGE } else { s * TINY * TINY };
            }
            // now |1-x| is tiny <= 2**-20, suffice to compute
            // log(x) by x-x^2/2+x^3/3-x^4/4
            let t = ax - ONE; // t has 20 trailing zeros
            let w = (t * t) * (0.5 - t * (0.333_333_333_333_333_333_333_3 - t * 0.25));
            let u = IVLN2_H * t; // ivln2_h has 21 sig. bits
            let v = t * IVLN2_L - w * IVLN2;
            t1 = from_words(hi(u + v), 0);
            t2 = v - (t1 - u);
        } else {
            n = 0;
            // take care subnormal number
            if ix < 0x0010_0000 {
                ax *= TWO53;
                n -= 53;
                ix = hi(ax);
            }
            n += (ix >> 20) - 0x3ff;
            let j = ix & 0x000f_ffff;
            // determine interval
            ix = j | 0x3ff0_0000; // normalize ix
            let k: usize;
            if j <= 0x3988E {
                k = 0; // |x|<sqrt(3/2)
            } else if j < 0xBB67A {
                k = 1; // |x|<sqrt(3)
            } else {
                k = 0;
                n += 1;
                ix -= 0x0010_0000;
            }
            ax = from_words(ix, lo(ax));

            // compute ss = s_h+s_l = (x-1)/(x+1) or (x-1.5)/(x+1.5)
            let mut u = ax - BP[k]; // bp[0]=1.0, bp[1]=1.5
            let mut v = ONE / (ax + BP[k]);
            let ss = u * v;
            let s_h = from_words(hi(ss), 0);
            // t_h=ax+bp[k] High
            let mut t_h = from_words(((ix >> 1) | 0x2000_0000) + 0x0008_0000 + ((k as i32) << 18), 0);
            let mut t_l = ax - (t_h - BP[k]);
            let s_l = v * ((u - s_h * t_h) - s_h * t_l);
            // compute log(ax)
            let mut s2 = ss * ss;
            let mut r = s2 * s2 * (L1 + s2 * (L2 + s2 * (L3 + s2 * (L4 + s2 * (L5 + s2 * L6)))));
            r += s_l * (s_h + ss);
            s2 = s_h * s_h;
            t_h = from_words(hi(3.0 + s2 + r), 0);
            t_l = r - ((t_h - 3.0) - s2);
            // u+v = ss*(1+...)
            u = s_h * t_h;
            v = s_l * t_h + t_l * ss;
            // 2/(3log2)*(ss+...)
            let p_h = from_words(hi(u + v), 0);
            let p_l = v - (p_h - u);
            let z_h = CP_H * p_h; // cp_h+cp_l = 2/(3*log2)
            let z_l = CP_L * p_h + p_l * CP + DP_L[k];
            // log2(ax) = (ss+..)*2/(3*log2) = n + dp_h + z_h + z_l
            let t = f64::from(n);
            t1 = from_words(hi(((z_h + z_l) + DP_H[k]) + t), 0);
            t2 = z_l - (((t1 - t) - DP_H[k]) - z_h);
        }

        // split up y into y1+y2 and compute (y1+y2)*(t1+t2)
        let y1 = from_words(hi(y), 0);
        let p_l = (y - y1) * t1 + y * t2;
        let mut p_h = y1 * t1;
        let mut z = p_l + p_h;
        let mut j = hi(z);
        let i = lo(z) as i32;
        if j >= 0x4090_0000 {
            // z >= 1024
            if (j.wrapping_sub(0x4090_0000) | i) != 0 {
                return s * HUGE * HUGE; // overflow
            } else if p_l + OVT > z - p_h {
                return s * HUGE * HUGE; // overflow
            }
        } else if (j & 0x7fff_ffff) >= 0x4090_cc00 {
            // z <= -1075
            if (j.wrapping_sub(0xc090_cc00_u32 as i32) | i) != 0 {
                return s * TINY * TINY; // underflow
            } else if p_l <= z - p_h {
                return s * TINY * TINY; // underflow
            }
        }

        // compute 2**(p_h+p_l)
        let i = j & 0x7fff_ffff;
        let mut k = (i >> 20) - 0x3ff;
        n = 0;
        if i > 0x3fe0_0000 {
            // if |z| > 0.5, set n = [z+0.5]
            n = j + (0x0010_0000 >> (k + 1));
            k = ((n & 0x7fff_ffff) >> 20) - 0x3ff; // new k for n
            let t = from_words(n & !(0x000f_ffff >> k), 0);
            n = ((n & 0x000f_ffff) | 0x0010_0000) >> (20 - k);
            if j < 0 {
                n = -n;
            }
            p_h -= t;
        }
        let t = from_words(hi(p_l + p_h), 0);
        let u = t * LG2_H;
        let v = (p_l - (t - p_h)) * LG2 + t * LG2_L;
        z = u + v;
        let w = v - (z - u);
        let t = z * z;
        let t1 = z - t * (P1 + t * (P2 + t * (P3 + t * (P4 + t * P5))));
        let r = (z * t1) / (t1 - TWO) - (w + z * w);
        z = ONE - (r - z);
        j = hi(z);
        j = j.wrapping_add(n << 20);
        if (j >> 20) <= 0 {
            z = scalbn(z, n); // subnormal output
        } else {
            z = from_words(hi(z).wrapping_add(n << 20), lo(z));
        }
        s * z
    }

    /// The IEEE 754 remainder of `x` and `p`, from fdlibm `e_remainder.c`.
    /// Its `__ieee754_fmod` is the exact IEEE operation Rust's `%` computes
    /// on `f64` (C `fmod`), so it is not translated.
    ///
    /// Equivalent to `java.lang.StrictMath.IEEEremainder(double, double)`.
    pub fn ieee_remainder(mut x: f64, mut p: f64) -> f64 {
        const ZERO: f64 = 0.0;

        let mut hx = hi(x); // high word of x
        let lx = lo(x); // low  word of x
        let mut hp = hi(p); // high word of p
        let lp = lo(p); // low  word of p
        let sx = hx as u32 & 0x8000_0000;
        hp &= 0x7fff_ffff;
        hx &= 0x7fff_ffff;

        // purge off exception values
        if (hp as u32 | lp) == 0 {
            return (x * p) / (x * p); // p = 0
        }
        if hx >= 0x7ff0_0000 // x not finite
            || (hp >= 0x7ff0_0000 // p is NaN
                && ((hp - 0x7ff0_0000) as u32 | lp) != 0)
        {
            return (x * p) / (x * p);
        }

        if hp <= 0x7fdf_ffff {
            x %= p + p; // now x < 2p
        }
        if ((hx.wrapping_sub(hp)) as u32 | lx.wrapping_sub(lp)) == 0 {
            return ZERO * x;
        }
        x = x.abs();
        p = p.abs();
        if hp < 0x0020_0000 {
            if x + x > p {
                x -= p;
                if x + x >= p {
                    x -= p;
                }
            }
        } else {
            let p_half = 0.5 * p;
            if x > p_half {
                x -= p;
                if x >= p_half {
                    x -= p;
                }
            }
        }
        from_words(hi(x) ^ sx as i32, lo(x))
    }
}
