//! Port of `lucene/core/src/test/org/apache/lucene/util/compress/TestLowercaseAsciiCompression.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::store::ByteBuffersDataOutput;
use rucene::util::compress::LowercaseAsciiCompression;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_array_equals, class, random, test};

/// `TestLowercaseAsciiCompression`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testSimple", test_simple),
        test("testNotReallySimple", test_not_really_simple),
        test("testNotReallySimple2", test_not_really_simple2),
        test("testFarAwayExceptions", test_far_away_exceptions),
        test("testRandomAscii", test_random_ascii),
        test(
            "testRandomCompressibleAscii",
            test_random_compressible_ascii,
        ),
        test(
            "testRandomCompressibleAsciiWithExceptions",
            test_random_compressible_ascii_with_exceptions,
        ),
        test("testRandom", test_random),
        test(
            "testAsciiCompressionRandom2",
            test_ascii_compression_random2,
        ),
    ];
    class(
        "org.apache.lucene.util.compress.TestLowercaseAsciiCompression",
        methods,
    )
}

/// `doTestCompress(bytes)`.
fn do_test_compress(bytes: &[u8]) -> Result<bool> {
    do_test_compress_len(bytes, bytes.len())
}

/// `doTestCompress(bytes, len)`.
fn do_test_compress_len(bytes: &[u8], len: usize) -> Result<bool> {
    let mut compressed = ByteBuffersDataOutput::new();
    let mut tmp = vec![0u8; len + random().next_int_bound(10) as usize];
    random().next_bytes(&mut tmp);
    if LowercaseAsciiCompression::compress(bytes, len, &mut tmp, &mut compressed)? {
        Assert::assert_true(compressed.size() < len);
        let mut restored = vec![0u8; len + random().next_int_bound(10) as usize];
        LowercaseAsciiCompression::decompress(
            &mut compressed.to_data_input()?,
            &mut restored,
            len,
        )?;
        assert_array_equals(&bytes[..len], &restored[..len]);
        Ok(true)
    } else {
        Ok(false)
    }
}

fn test_simple() -> Result<()> {
    Assert::assert_false(do_test_compress("".as_bytes())?); // too short
    Assert::assert_false(do_test_compress("ab1".as_bytes())?); // too short
    Assert::assert_false(do_test_compress("ab1cdef".as_bytes())?); // too short
    Assert::assert_true(do_test_compress("ab1cdefg".as_bytes())?);
    Assert::assert_false(do_test_compress("ab1cdEfg".as_bytes())?); // too many exceptions
    Assert::assert_true(do_test_compress("ab1cdefg".as_bytes())?);
    // 1 exception, but enough chars to be worth encoding an exception
    Assert::assert_true(do_test_compress(
        "ab1.dEfg427hiogchio:'nwm un!94twxz".as_bytes(),
    )?);
    Ok(())
}

/// `testNotReallySimple()` (LUCENE-10551).
fn test_not_really_simple() -> Result<()> {
    do_test_compress(r#"cion1cion_desarrollociones_oraclecionesnaturacionesnatura2tedppsa-integrationdemotiontion cloud gen2tion instance - dev1tion instance - testtion-devbtion-instancetion-prdtion-promerication-qation064533tion535217tion697401tion761348tion892818tion_matrationcauto_simmonsintgic_testtioncloudprodictioncloudservicetiongateway10tioninstance-jtsundatamartprd??o"#.as_bytes())?;
    Ok(())
}

/// `testNotReallySimple2()` (LUCENE-10551).
fn test_not_really_simple2() -> Result<()> {
    do_test_compress(r#"analytics-platform-test/koala/cluster-tool:1.0-20220310151438.492,mesh_istio_examples-bookinfo-details-v1:1.16.2mesh_istio_examples-bookinfo-reviews-v3:1.16.2oce-clamav:1.0.219oce-tesseract:1.0.7oce-traefik:2.5.1oci-opensearch:1.2.4.8.103oda-digital-assistant-control-plane-train-pool-workflow-v6:22.02.14oke-coresvcs-k8s-dns-dnsmasq-nanny-amd64@sha256:41aa9160ceeaf712369ddb660d02e5ec06d1679965e6930351967c8cf5ed62d4oke-coresvcs-k8s-dns-kube-dns-amd64@sha256:2cf34b04106974952996c6ef1313f165ce65b4ad68a3051f51b1b8f91ba5f838oke-coresvcs-k8s-dns-sidecar-amd64@sha256:8a82c7288725cb4de9c7cd8d5a78279208e379f35751539b406077f9a3163dcdoke-coresvcs-node-problem-detector@sha256:9d54df11804a862c54276648702a45a6a0027a9d930a86becd69c34cc84bf510oke-coresvcs-oke-fluentd-lumberjack@sha256:5f3f10b187eb804ce4e84bc3672de1cf318c0f793f00dac01cd7da8beea8f269oke-etcd-operator@sha256:4353a2e5ef02bb0f6b046a8d6219b1af359a2c1141c358ff110e395f29d0bfc8oke-oke-hyperkube-amd64@sha256:3c734f46099400507f938090eb9a874338fa25cde425ac9409df4c885759752foke-public-busybox@sha256:4cee1979ba0bf7db9fc5d28fb7b798ca69ae95a47c5fecf46327720df4ff352doke-public-coredns@sha256:86f8cfc74497f04e181ab2e1d26d2fd8bd46c4b33ce24b55620efcdfcb214670oke-public-coredns@sha256:8cd974302f1f6108f6f31312f8181ae723b514e2022089cdcc3db10666c49228oke-public-etcd@sha256:b751e459bc2a8f079f6730dd8462671b253c7c8b0d0eb47c67888d5091c6bb77oke-public-etcd@sha256:d6a76200a6e9103681bc2cf7fefbcada0dd9372d52cf8964178d846b89959d14oke-public-etcd@sha256:fa056479342b45479ac74c58176ddad43687d5fc295375d705808f9dfb48439aoke-public-kube-proxy@sha256:93b2da69d03413671606e22294c59a69fe404088a5f6e74d6394a8641fdb899boke-public-tiller@sha256:c2eb6e580123622e1bc0ff3becae3a3a71ac36c98a2786d780590197839175e5osms/opcbuild-osms-agent-proxy-java:0.4.0-129rosms/opcbuild-osms-agent-proxy-nginx:0.4.0-129rosms/opcbuild-osms-ingestion-cert:0.4.0-129rscs-lcm/drift-detector:227scs-lcm/salt-state-sync:242streaming-alpine:30.10.183streaming-kafka:30.10.183vision-service-document-classification:1.1.55vision-service-image-classification:1.4.52"#.as_bytes())?;
    Ok(())
}

fn test_far_away_exceptions() -> Result<()> {
    let s = format!("01W{}W.", "a".repeat(300));
    Assert::assert_true(do_test_compress(s.as_bytes())?);
    Ok(())
}

fn test_random_ascii() -> Result<()> {
    for _ in 0..1000 {
        let len = random().next_int_bound(1000) as usize;
        let mut bytes = vec![0u8; len + random().next_int_bound(10) as usize];
        for b in bytes.iter_mut() {
            *b = TestUtil::next_int(&*random(), ' ' as i32, '~' as i32) as u8;
        }
        do_test_compress_len(&bytes, len)?;
    }
    Ok(())
}

fn test_random_compressible_ascii() -> Result<()> {
    for _ in 0..1000 {
        let len = TestUtil::next_int(&*random(), 8, 1000) as usize;
        let mut bytes = vec![0u8; len + random().next_int_bound(10) as usize];
        for v in bytes.iter_mut() {
            // only use always compressible bytes
            let mut b = random().next_int_bound(32);
            b = b | 0x20 | ((b & 0x20) << 1);
            b -= 1;
            *v = b as u8;
        }
        Assert::assert_true(do_test_compress_len(&bytes, len)?);
    }
    Ok(())
}

fn test_random_compressible_ascii_with_exceptions() -> Result<()> {
    for _ in 0..1000 {
        let len = TestUtil::next_int(&*random(), 8, 1000) as usize;
        let mut exceptions = 0;
        let max_exceptions = len >> 5;
        let mut bytes = vec![0u8; len + random().next_int_bound(10) as usize];
        for v in bytes.iter_mut() {
            if exceptions == max_exceptions || random().next_int_bound(100) != 0 {
                let mut b = random().next_int_bound(32);
                b = b | 0x20 | ((b & 0x20) << 1);
                b -= 1;
                *v = b as u8;
            } else {
                exceptions += 1;
                *v = random().next_int_bound(256) as u8;
            }
        }
        Assert::assert_true(do_test_compress_len(&bytes, len)?);
    }
    Ok(())
}

fn test_random() -> Result<()> {
    for _ in 0..1000 {
        let len = random().next_int_bound(1000) as usize;
        let mut bytes = vec![0u8; len + random().next_int_bound(10) as usize];
        random().next_bytes(&mut bytes);
        do_test_compress_len(&bytes, len)?;
    }
    Ok(())
}

fn test_ascii_compression_random2() -> Result<()> {
    let iters = LuceneTestCase::at_least(1000);
    for _ in 0..iters {
        let r = random();
        let word_length = LuceneTestCase::at_least(400);
        let simple = random().next_boolean();
        do_test_compress(TestUtil::random_sub_string(&*r, word_length, simple).as_bytes())?;
    }
    Ok(())
}
