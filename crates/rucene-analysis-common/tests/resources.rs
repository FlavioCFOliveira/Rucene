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

//! The module's embedded resource files (`rucene_analysis_common::resources`,
//! rmp #446) against Lucene 10.5.0.
//!
//! * Every one of the 43 data files of `lucene/analysis/common/src/resources`
//!   (outside `META-INF/services`) is embedded with the SHA-1 of the 10.5.0
//!   original ([`LUCENE_SHA1`], the census list of the sprint, checked against
//!   `releases/lucene/10.5.0`), and no other file is.
//! * Names resolve as Java's class loader resolves them, and a missing one
//!   fails as Java fails (observed on JDK 25 with `lucene-analysis-common`
//!   10.5.0 on the class path): `Class.getResourceAsStream` relative to the
//!   class's package or absolute with a leading `/`, `null` when absent;
//!   `ClasspathResourceLoader` over the module's class loader by absolute
//!   name only (a leading `/` is not found), `IOException("Resource not found
//!   (...): <name>")` when absent.

use std::collections::BTreeSet;
use std::io::Read;

use rucene::util::resource_loader::{ClasspathResourceLoader, ResourceLoader};
use rucene_analysis_common::resources::{
    class_loader_resources, get_resource, get_resource_as_stream,
};

/// Every resource file of `lucene-analysis-common` 10.5.0 outside
/// `META-INF/services`, with its SHA-1 (`sha1sum` of
/// `lucene/analysis/common/src/resources/` at `releases/lucene/10.5.0`).
const LUCENE_SHA1: [(&str, &str); 43] = [
    (
        "org/apache/lucene/analysis/ar/stopwords.txt",
        "755b7aa4410c5adb6756c8b75fd09a4202ce84cb",
    ),
    (
        "org/apache/lucene/analysis/bg/stopwords.txt",
        "9c9eb54eabafae070d4b8fe8ed6cefd333cc7f20",
    ),
    (
        "org/apache/lucene/analysis/bn/stopwords.txt",
        "e789cad0b655561325ac4f6c45e4216f0c3211fd",
    ),
    (
        "org/apache/lucene/analysis/br/stopwords.txt",
        "761776a39f2092b8fb7d6424fa85c3c79cfd432d",
    ),
    (
        "org/apache/lucene/analysis/ca/stopwords.txt",
        "94ec5dab31cd57070e774d5a51d164c4cc8240b6",
    ),
    (
        "org/apache/lucene/analysis/cjk/stopwords.txt",
        "72c1c1974f1f008f8820b685d8310de2301323d5",
    ),
    (
        "org/apache/lucene/analysis/ckb/stopwords.txt",
        "bb6336b6e3f005728be87f3e135f2c6120e07060",
    ),
    (
        "org/apache/lucene/analysis/compound/hyphenation/hyphenation.dtd",
        "dcd2690d772ee6ae0cc77289b0cfcfe2d74a61f5",
    ),
    (
        "org/apache/lucene/analysis/cz/stopwords.txt",
        "d960d5a10c8e902e8c0d8fa594083c3160f30dfd",
    ),
    (
        "org/apache/lucene/analysis/el/stopwords.txt",
        "8c917a977840a2ac517fe8432fca3019471acc34",
    ),
    (
        "org/apache/lucene/analysis/et/stopwords.txt",
        "9ced6cbd36bb84ac989ab9e41ecbc1411d52d0dd",
    ),
    (
        "org/apache/lucene/analysis/eu/stopwords.txt",
        "d35025c8ab7ff60ff1706e6438dcf8c4ac2def9f",
    ),
    (
        "org/apache/lucene/analysis/fa/stopwords.txt",
        "0334a095461092d2eb9574a70fad1c71cde294a6",
    ),
    (
        "org/apache/lucene/analysis/gl/galician.rslp",
        "af86a20a8a4c24c901e872aa179026baf219a4eb",
    ),
    (
        "org/apache/lucene/analysis/gl/stopwords.txt",
        "557f3dc0b56ea65590feebe7780005d137f22243",
    ),
    (
        "org/apache/lucene/analysis/hi/stopwords.txt",
        "1a01efd38eb60a64bc1f8a3333db68658e6ffa2f",
    ),
    (
        "org/apache/lucene/analysis/hy/stopwords.txt",
        "e0e645e97fff83522ea6b1295e25c1afb664ef5d",
    ),
    (
        "org/apache/lucene/analysis/id/stopwords.txt",
        "e80cc594bfcde39c3b36de8aeb960d9f2871a5a8",
    ),
    (
        "org/apache/lucene/analysis/lt/stopwords.txt",
        "3f23b2db646c15be1b7e899bb3b80d134bc767f4",
    ),
    (
        "org/apache/lucene/analysis/lv/stopwords.txt",
        "313ecd90a576364fa565836347b2ee50531cc96b",
    ),
    (
        "org/apache/lucene/analysis/ne/stopwords.txt",
        "8e03cc3fa3e887bf7f83a8fedda399dc90f84922",
    ),
    (
        "org/apache/lucene/analysis/pt/portuguese.rslp",
        "32adae91a1005ff9fad04d2e994f82aa7bbdd714",
    ),
    (
        "org/apache/lucene/analysis/ro/stopwords.txt",
        "0f014e31e915fd91e3f2fdea84c91f31bf1763be",
    ),
    (
        "org/apache/lucene/analysis/snowball/danish_stop.txt",
        "f3627b9037da730534aa008b9edf32582c16866e",
    ),
    (
        "org/apache/lucene/analysis/snowball/dutch_stop.txt",
        "3f84f1c2ae282bd1350fa246baaded095bebdecc",
    ),
    (
        "org/apache/lucene/analysis/snowball/english_stop.txt",
        "5822d03de10b6403512774492508f586ea55a7c7",
    ),
    (
        "org/apache/lucene/analysis/snowball/finnish_stop.txt",
        "f161b9b0e757fa4793272bb5d11fd0c0546f9129",
    ),
    (
        "org/apache/lucene/analysis/snowball/french_stop.txt",
        "315826e66bd1f9ae294f5a1fc7738887706fc9e1",
    ),
    (
        "org/apache/lucene/analysis/snowball/german_stop.txt",
        "301fa879d91631bc30169c12f4045f294aefd7ec",
    ),
    (
        "org/apache/lucene/analysis/snowball/hungarian_stop.txt",
        "81837d5b861a876314b32fdaa2a4cee7c09fbbf8",
    ),
    (
        "org/apache/lucene/analysis/snowball/indonesian_stop.txt",
        "b5e23c7a1b9893ed472bdbf27f952b9ace53d2d6",
    ),
    (
        "org/apache/lucene/analysis/snowball/irish_stop.txt",
        "99dea155818fe3fb3c31a29ff7f67e77a76ccc0d",
    ),
    (
        "org/apache/lucene/analysis/snowball/italian_stop.txt",
        "4df230f3d0f3898d9812b8d08c7b08bfc55477c3",
    ),
    (
        "org/apache/lucene/analysis/snowball/norwegian_stop.txt",
        "b612f3c7df6d3f6fca1aff5d8d9e5c353df423cd",
    ),
    (
        "org/apache/lucene/analysis/snowball/portuguese_stop.txt",
        "e7579ef190edfc939348353dca480d9cedae1d83",
    ),
    (
        "org/apache/lucene/analysis/snowball/russian_stop.txt",
        "cbe87679fbabae13c8ea16be8a11fd78151f8bf7",
    ),
    (
        "org/apache/lucene/analysis/snowball/spanish_stop.txt",
        "852c0b6520a1d2ac9d2ec189c296c2707b5060aa",
    ),
    (
        "org/apache/lucene/analysis/snowball/swedish_stop.txt",
        "2fcc6906c3d55f8087030c28f0f118be4aed6596",
    ),
    (
        "org/apache/lucene/analysis/sr/stopwords.txt",
        "7aefbfec85669665b93ad8d62f5a718c9d05ef77",
    ),
    (
        "org/apache/lucene/analysis/ta/stopwords.txt",
        "bf8e4d8f004fdf868bbbcf47849236c0bcca7938",
    ),
    (
        "org/apache/lucene/analysis/te/stopwords.txt",
        "75c85e6f902b42d87549de216e207960e26ba548",
    ),
    (
        "org/apache/lucene/analysis/th/stopwords.txt",
        "04bc1cda20438031564712ba41f1764b75685ecb",
    ),
    (
        "org/apache/lucene/analysis/tr/stopwords.txt",
        "de10aae26825df4706c346a9e57872fde064190c",
    ),
];

/// SHA-1 (FIPS 180-4), for the digests of [`LUCENE_SHA1`].
fn sha1(data: &[u8]) -> String {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xefcd_ab89,
        0x98ba_dcfe,
        0x1032_5476,
        0xc3d2_e1f0,
    ];
    let mut message = data.to_vec();
    let bit_length = (data.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_length.to_be_bytes());
    for block in message.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5a82_7999),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

/// The digest itself, on the FIPS 180-4 test vectors.
#[test]
fn sha1_matches_the_fips_vectors() {
    assert_eq!(sha1(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    assert_eq!(sha1(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert_eq!(
        sha1(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
    );
    // Two blocks of padding (a 56-byte message) and a multi-block message.
    assert_eq!(
        sha1(&[b'a'; 1000]),
        "291e9a6c66994949b57ba5e650361e98fc36b1ba"
    );
}

/// Every resource is embedded byte for byte, and only those.
#[test]
fn every_resource_has_its_lucene_sha1() {
    for (name, digest) in LUCENE_SHA1 {
        let bytes = get_resource(name).unwrap_or_else(|| panic!("{name} is not embedded"));
        assert_eq!(sha1(bytes), digest, "{name}");
    }
    let embedded: BTreeSet<String> = class_loader_resources().into_keys().collect();
    let lucene: BTreeSet<String> = LUCENE_SHA1.iter().map(|(n, _)| n.to_string()).collect();
    assert_eq!(embedded, lucene, "the embedded set is Lucene's");
    for (name, bytes) in class_loader_resources() {
        assert_eq!(get_resource(&name), Some(bytes.as_slice()), "{name}");
    }
}

fn read(mut r: Box<dyn Read + Send>) -> Vec<u8> {
    let mut out = Vec::new();
    r.read_to_end(&mut out).unwrap();
    out
}

/// `Class.getResourceAsStream`: relative to the class's package, absolute
/// with a leading `/`, `None` (Java's `null`) when absent.
#[test]
fn class_relative_names_resolve_as_java() {
    let turkish = "org.apache.lucene.analysis.tr.TurkishAnalyzer";
    assert_eq!(
        read(get_resource_as_stream(turkish, "stopwords.txt").expect("relative")),
        get_resource("org/apache/lucene/analysis/tr/stopwords.txt").unwrap()
    );
    assert_eq!(
        read(
            get_resource_as_stream(
                turkish,
                "/org/apache/lucene/analysis/snowball/english_stop.txt"
            )
            .expect("absolute")
        ),
        get_resource("org/apache/lucene/analysis/snowball/english_stop.txt").unwrap()
    );
    assert!(get_resource_as_stream(turkish, "nope.txt").is_none());
    assert!(get_resource("/org/apache/lucene/analysis/tr/stopwords.txt").is_none());
}

/// `new ClasspathResourceLoader(CustomAnalyzer.class.getClassLoader())`:
/// absolute names only; Java's `IOException` when absent.
#[test]
fn the_class_loader_table_resolves_as_java() {
    let loader = ClasspathResourceLoader::new(class_loader_resources());
    let ar = read(
        loader
            .open_resource("org/apache/lucene/analysis/ar/stopwords.txt")
            .expect("found"),
    );
    assert_eq!(ar.len(), 1248);
    for name in [
        "/org/apache/lucene/analysis/ar/stopwords.txt",
        "missing.txt",
    ] {
        let error = loader.open_resource(name).map(|_| ()).unwrap_err();
        assert!(error.is_io_exception(), "{error:?}");
        assert!(
            error.to_string().contains(&format!(
                "Resource not found (if you use Java Module System, make sure to open module \
                 and package containing resources to 'org.apache.lucene.core' module): {name}"
            )),
            "{error}"
        );
    }
}
