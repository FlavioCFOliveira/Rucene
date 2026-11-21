//! Port of `org.apache.lucene.util.automaton.Lev2TParametricDescription`.
//!
//! The tables in this file are code-generated in Lucene by
//! `gradle/generation/moman/createAutomata.py` (the moman/finenight package) and are
//! reproduced here verbatim: the packed data *is* the automaton, so any change to it
//! changes which terms a fuzzy query matches.

use super::levenshtein_automata::{unpack, ParametricDescription};

/// Packed transition data, 1 bits per value.
#[rustfmt::skip]
const OFFSET_INCRS0: &[u64] = &[
    0x0,
];

/// Packed transition data, 1 bits per value.
#[rustfmt::skip]
const OFFSET_INCRS1: &[u64] = &[
    0x3e0,
];

/// Packed transition data, 2 bits per value.
#[rustfmt::skip]
const OFFSET_INCRS2: &[u64] = &[
    0x5558208800080000, 0x5555555555,
];

/// Packed transition data, 2 bits per value.
#[rustfmt::skip]
const OFFSET_INCRS3: &[u64] = &[
    0xc0c83000080000, 0x2200fcff300f3c30, 0x3c2200a8caa00a08, 0x55555555a8fea00a,
    0x5555555555555555, 0x5555555555555555, 0x5555555555555555,
];

/// Packed transition data, 3 bits per value.
#[rustfmt::skip]
const OFFSET_INCRS4: &[u64] = &[
    0xc0000010000000, 0x40000060061, 0x8001000800000000, 0x8229048249248a4, 0x6c360300002092,
    0x6db6036db61b6c30, 0x361b0180000db6c0, 0xdb11b71b91b72000, 0x100820006db6236,
    0x2492490612480012, 0x8041000248200049, 0x4924a48924000900, 0x2080012510822492,
    0x9241b69200048360, 0x4000926806da4924, 0x291b49000241b010, 0x494934236d249249,
    0x2492492492492492, 0x9249249249249249, 0x4924924924924924, 0x2492492492492492,
    0x9249249249249249, 0x4924924924924924, 0x2492492492492492, 0x9249249249249249,
    0x4924924924924924, 0x2492492492492492, 0x9249249249249249, 0x4924924924924924,
    0x2492492492492492, 0x9249249249249249, 0x4924924924924924, 0x2492492492492492,
    0x249249249249,
];

/// Packed transition data, 3 bits per value.
#[rustfmt::skip]
const OFFSET_INCRS5: &[u64] = &[
    0xc0000010000000, 0x40000060061, 0x6000000800000000, 0xdb6ab6db6b003080, 0x80040000002db6,
    0x1148241249245240, 0x4002000000104904, 0xa4b2592492292000, 0xd80c00009649658,
    0x80db6d86db0c001b, 0xc06000036db01b6d, 0x6db6c36d86000d86, 0x300001b6ddadb6ed,
    0xe37236e40006c360, 0xdb6c46db6236, 0xb91b72000361b018, 0x6db7636dbb1b71,
    0x6124800120100820, 0x2482000492492490, 0x9240009008041000, 0x555b6a4924924830,
    0x2000480402080012, 0x8411249249252449, 0x24020104000928, 0x5892492492922490,
    0x120d808200049456, 0x6924924906da4800, 0x6c041000249a01b, 0x924924836d240009,
    0x6020800124d5adb4, 0x2492523692000483, 0x104000926846da49, 0x49291b49000241b0,
    0x92494935636d2492, 0x4924924924924924, 0x2492492492492492, 0x9249249249249249,
    0x4924924924924924, 0x2492492492492492, 0x9249249249249249, 0x4924924924924924,
    0x2492492492492492, 0x9249249249249249, 0x4924924924924924, 0x2492492492492492,
    0x9249249249249249, 0x4924924924924924, 0x2492492492492492, 0x9249249249249249,
    0x4924924924924924, 0x2492492492492492, 0x9249249249249249, 0x4924924924924924,
    0x2492492492492492, 0x9249249249249249, 0x4924924924924924, 0x2492492492492492,
    0x9249249249249249, 0x4924924924924924, 0x2492492492492492, 0x9249249249249249,
    0x4924924924924924, 0x2492492492492492, 0x9249249249249249, 0x4924924924924924,
    0x2492492492492492, 0x9249249249249249, 0x24924924,
];

/// Packed transition data, 2 bits per value.
#[rustfmt::skip]
const TO_STATES0: &[u64] = &[
    0xe,
];

/// Packed transition data, 3 bits per value.
#[rustfmt::skip]
const TO_STATES1: &[u64] = &[
    0x1a688a2c,
];

/// Packed transition data, 4 bits per value.
#[rustfmt::skip]
const TO_STATES2: &[u64] = &[
    0xdc0703570707054, 0x2323213a03dd3a3a, 0x2254543215435223, 0x5435,
];

/// Packed transition data, 5 bits per value.
#[rustfmt::skip]
const TO_STATES3: &[u64] = &[
    0x700a5701c0380a4, 0x180a000ca529c0, 0xc5498e60a80af180, 0x8c4300e85a546398,
    0xd8d43501ac18c601, 0x51976d6a863500ad, 0xc3501ac28ca0180a, 0x76dda8a5b0c5be16,
    0xc41294a018c4519, 0x1086520ce248d231, 0x13946358ce31ac42, 0x6732d4942d0348c4,
    0xd635ad4b1ad224a5, 0xce24948520c4139, 0x58ce729d22110a52, 0x941cc520c41394e3,
    0x4729d22490e732d4, 0x39ce35ad,
];

/// Packed transition data, 6 bits per value.
#[rustfmt::skip]
const TO_STATES4: &[u64] = &[
    0x1453803801c0144, 0xc000514514700038, 0x1400001401, 0x140000, 0x6301f00700510000,
    0xa186178301f00d1, 0xc20c30c20ca0c3, 0xc00c00cd0c30030c, 0x4c054014f0c00c30,
    0x55150c34c30944c3, 0x430c014308300550, 0xc30850c00050c31, 0x50053c50c3143000,
    0x850d30c25130d301, 0xc21441430a08608, 0x2145003143142145, 0x4c1431451400c314,
    0x28014d6c32832803, 0x1c50c76cd34a0c3, 0x430c30c31c314014, 0xc30050000001431,
    0xd36d0e40ca00d303, 0xcb2abb2c90b0e400, 0x2c32ca2c70c20ca1, 0x31c00c00cd2c70cb,
    0x558328034c2c32c, 0x6cd6ca14558309b7, 0x51c51401430850c7, 0xc30871430c714,
    0xca00d3071451450, 0xb9071560c26dc156, 0xc70c21441cb2abb2, 0x1421c70cb1c51ca1,
    0x30811c51c51c00c3, 0xc51031c224324308, 0x5c33830d70820820, 0x30c30c30c33850c3,
    0x451450c30c30c31c, 0xda0920d20c20c20, 0x365961145145914f, 0xd964365351965865,
    0x51964364365a6590, 0x920b203243081505, 0xd72422492c718b28, 0x2cb3872c35cb28b0,
    0xb0c32cb2972c30d7, 0xc80c90c204e1c75c, 0x4504171c62ca2482, 0x33976585d65d9610,
    0x4b5ca5d70d95cb5d, 0x1030813873975c36, 0x41451031c2245105, 0xc35c338714e24208,
    0x1c51c51451453851, 0x20451450c70c30c3, 0x4f0da09214f1440c, 0x6533944d04513d41,
    0xe15450551350e658, 0x551938364365a50, 0x2892071851030815, 0x714e2422441c718b,
    0x4e1c73871c35cb28, 0x5c70c32cb28e1c51, 0x81c61440c204e1c7, 0xd04503ce1c62ca24,
    0x39338e6585d63944, 0x364b5ca38e154387, 0x38739738,
];

/// Packed transition data, 6 bits per value.
#[rustfmt::skip]
const TO_STATES5: &[u64] = &[
    0x1453803801c0144, 0xc000514514700038, 0x1400001401, 0x140000, 0x4e00e00700510000,
    0x3451451c000e0051, 0x30cd00000d015000, 0xc30c30d40c30c30c, 0x7c01c01440c30c30,
    0x185e0c07c03458c0, 0x830c30832830c286, 0x33430c00c30030, 0x70051030030c3003,
    0x8301f00d16301f00, 0xc20ca0c30a18617, 0xb1450c51431420c3, 0x4f14314514314315,
    0x4c30944c34c05401, 0x30830055055150c3, 0xc00050c31430c014, 0xc31430000c30850,
    0x25130d30150053c5, 0xc03541545430d30c, 0x1cb2cd0c300d0c90, 0x72c30cb2c91cb0c3,
    0xc34c054014f1cb2c, 0x8218221434c30944, 0x50851430851050c2, 0x30c50851400c50c,
    0x150053c50c51450, 0x8850d30c25130d3, 0x450c21441430a086, 0x1c91c70c51cb1c21,
    0x34c1cb1c71c314b, 0xc328014d6c328328, 0x1401c50c76cd34a0, 0x31430c30c31c3140,
    0x30c300500000014, 0x535b0ca0ca00d3, 0x514369b34d2830ca, 0x5965965a0c500d01,
    0x6435030c30d46546, 0xdb4390328034c659, 0xcaaecb242c390034, 0xcb28b1c30832872,
    0x700300334b1c32cb, 0xe40ca00d30b0cb0c, 0xb2c90b0e400d36d0, 0xa2c70c20ca1cb2ab,
    0x4315b5ce6575d95c, 0x28034c5d95c53831, 0xa14558309b705583, 0x401430850c76cd6c,
    0x871430c71451c51, 0xd3071451450000c3, 0x560c26dc1560ca00, 0xc914369b35b2851,
    0x465939451a14500d, 0x945075030cb2c939, 0x9b70558328034c3, 0x72caaecae41c5583,
    0xc71472871c308510, 0x1470030c50871c32, 0xc1560ca00d307147, 0xabb2b9071560c26d,
    0x38a1c70c21441cb2, 0x314b1c938e657394, 0x4308308139438738, 0x820c51031c22432,
    0x50c35c33830d7082, 0xc31c30c30c30c338, 0xc20451450c30c30, 0x31440c70890c90c2,
    0xea0df0c3a8208208, 0xa28a28a28a231430, 0x1861868a28a28a1e, 0xc368248348308308,
    0x4d96584514516453, 0x36590d94d4659619, 0x546590d90d969964, 0x920d20c20c20541,
    0x961145145914f0da, 0xe89d351965865365, 0x9e89e89e99e7a279, 0xb203243081821827,
    0x422492c718b28920, 0x3872c35cb28b0d72, 0x32cb2972c30d72cb, 0xc90c204e1c75cb0c,
    0x24b1c62ca2482c80, 0xb0ea2e42c3a89089, 0xa4966a289669a31c, 0x8175e7a59a8a269,
    0x718b28920b203243, 0x175976584114105c, 0x5c36572d74ce5d96, 0xe1ce5d70d92d7297,
    0xca2482c80c90c204, 0x5d96104504171c62, 0x79669533976585d6, 0x659689e6964965a2,
    0x24510510308175e7, 0xe2420841451031c2, 0x453851c35c338714, 0xc30c31c51c51451,
    0x41440c20451450c7, 0x821051440c708914, 0x1470ea0df1c58c90, 0x8a1e85e861861863,
    0x30818618687a8a2, 0x5053c36824853c51, 0x96194ce51341144f, 0x943855141544d439,
    0x5415464e0d90d96, 0xf0da09214f1440c2, 0x533944d04513d414, 0x86082181350e6586,
    0x18277689e89e981d, 0x8920718510308182, 0x14e2422441c718b2, 0xe1c73871c35cb287,
    0xc70c32cb28e1c514, 0x1c61440c204e1c75, 0x90891071c62ca248, 0xa31c70ea2e41c58c,
    0xa269a475e86175e7, 0x510308175e7a57a8, 0xf38718b28920718, 0x39961758e5134114,
    0x728e38550e1ce4ce, 0xc204e1ce5ce0d92d, 0x1c62ca2481c61440, 0x85d63944d04503ce,
    0x5d86075e75338e65, 0x75e7657689e69647,
];

/// Minimal number of errors for each parametric state.
#[rustfmt::skip]
const MIN_ERRORS: &[i32] = &[0, 1, 2, 0, 1, -1, 0, -1, 0, -1, 0, -1, 0, -1, -1, -1, -1, -1, -2, -1, -1, -2, -1, -2, -1, -1, -1, -2, -2, -2, -2, -2, -2, -2, -2, -2, -2, -2, -2, -2, -2, -2, -2, -2, -2];

/// Parametric description for generating a Levenshtein automaton of degree 2, with
/// transpositions as primitive edits.
///
/// Equivalent to `org.apache.lucene.util.automaton.Lev2TParametricDescription`.
#[derive(Clone, Debug)]
pub struct Lev2TParametricDescription {
    w: i32,
}

impl Lev2TParametricDescription {
    /// Creates the parametric description for a word of length `w`.
    pub fn new(w: i32) -> Self {
        Self { w }
    }
}

impl ParametricDescription for Lev2TParametricDescription {
    fn w(&self) -> i32 {
        self.w
    }

    fn n(&self) -> i32 {
        2
    }

    fn min_errors(&self) -> &'static [i32] {
        MIN_ERRORS
    }

    fn transition(&self, abs_state: i32, position: i32, vector: i32) -> i32 {
        debug_assert!(abs_state != -1, "null absState should never be passed in");

        // decode absState -> state, offset
        let w = self.w;
        let mut state = abs_state / (w + 1);
        let mut offset = abs_state % (w + 1);
        debug_assert!(offset >= 0);

        if position == w {
            if state < 3 {
                let loc = vector * 3 + state;
                offset += unpack(OFFSET_INCRS0, loc, 1);
                state = unpack(TO_STATES0, loc, 2) - 1;
            }
        } else if position == w - 1 {
            if state < 5 {
                let loc = vector * 5 + state;
                offset += unpack(OFFSET_INCRS1, loc, 1);
                state = unpack(TO_STATES1, loc, 3) - 1;
            }
        } else if position == w - 2 {
            if state < 13 {
                let loc = vector * 13 + state;
                offset += unpack(OFFSET_INCRS2, loc, 2);
                state = unpack(TO_STATES2, loc, 4) - 1;
            }
        } else if position == w - 3 {
            if state < 28 {
                let loc = vector * 28 + state;
                offset += unpack(OFFSET_INCRS3, loc, 2);
                state = unpack(TO_STATES3, loc, 5) - 1;
            }
        } else if position == w - 4 {
            if state < 45 {
                let loc = vector * 45 + state;
                offset += unpack(OFFSET_INCRS4, loc, 3);
                state = unpack(TO_STATES4, loc, 6) - 1;
            }
        } else if state < 45 {
            let loc = vector * 45 + state;
            offset += unpack(OFFSET_INCRS5, loc, 3);
            state = unpack(TO_STATES5, loc, 6) - 1;
        }

        if state == -1 {
            // null state
            -1
        } else {
            // translate back to abs
            state * (w + 1) + offset
        }
    }
}
