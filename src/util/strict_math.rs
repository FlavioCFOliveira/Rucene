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
//! `double`) are [`hi`] and [`lo`]; writing them is [`from_words`]. C `int`
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
