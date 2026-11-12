//! Port of `lucene/core/src/test/org/apache/lucene/index/TestSegmentInfos.java`.

use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::codecs::codec_util;
use rucene::error::Result;
use rucene::index::index_file_names::SEGMENTS;
use rucene::index::segment_infos::SegmentInfos;
use rucene::index::{SegmentCommitInfo, SegmentInfo};
use rucene::search::sort::{Sort, SortField};
use rucene::store::{Directory, DEFAULT_IO_CONTEXT, READONCE_IO_CONTEXT};
use rucene::util::java_collections::JavaStringMap;
use rucene::util::string_helper::StringHelper;
use rucene::util::Version;
use rucene_test_framework::mockfile::extras_fs::ExtrasFS;
use rucene_test_framework::util::java_exceptions::{
    expect_throws, expect_throws_any_of, is_instance,
};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_equals, assert_null, class, random, test, verbose,
    IAE,
};
use crate::support::set_check_index_on_close;

/// `TestSegmentInfos`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestSegmentInfos",
        vec![
            test("testIllegalCreatedVersion", test_illegal_created_version),
            test("testVersionsNoSegments", test_versions_no_segments),
            test("testVersionsOneSegment", test_versions_one_segment),
            test("testVersionsTwoSegments", test_versions_two_segments),
            test("testToString", test_to_string),
            test("testIDChangesOnAdvance", test_id_changes_on_advance),
            test(
                "testBitFlippedTriggersCorruptIndexException",
                test_bit_flipped_triggers_corrupt_index_exception,
            ),
            test("testAddDiagnostics", test_add_diagnostics),
        ],
    )
}

/// `Version.LATEST.major`.
fn latest_major() -> i32 {
    i32::from(Version::LATEST.major)
}

/// `Codec.getDefault()`.
fn default_codec() -> Arc<dyn rucene::codecs::Codec> {
    rucene::codecs::get_default().unwrap_or_else(|| crate::support::npe("Codec.getDefault()"))
}

/// `Map.of(k1, v1, ...)`: an immutable map.
fn map_of(entries: &[(&str, &str)]) -> HashMap<String, String> {
    entries
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// `map.toString()` of an immutable `Map.of` map, which `SegmentInfo` keeps
/// as is (`Map.copyOf` of an immutable map is that map).
fn map_to_string(map: &HashMap<String, String>) -> String {
    JavaStringMap::immutable_copy_of(map.clone()).to_string()
}

/// `new Sort(SortField.FIELD_DOC)`: `Sort.INDEXORDER`.
fn index_order() -> Result<Sort> {
    Sort::new_fields(vec![SortField::FIELD_DOC.clone()])
}

/// `new SegmentInfo(dir, version, minVersion, name, 1, false, false,
/// Codec.getDefault(), emptyMap(), id, emptyMap(), null)`; Java's `null`
/// index sort is `Sort::new()` in Rucene (see `SegmentInfo::to_string_verbose`).
fn one_doc_info(
    dir: &Arc<dyn Directory>,
    version: Version,
    name: &str,
    id: [u8; 16],
) -> Result<SegmentInfo> {
    SegmentInfo::new(
        Arc::clone(dir),
        version,
        Some(version),
        name.to_string(),
        1,
        false,
        false,
        default_codec(),
        HashMap::new(),
        id,
        HashMap::new(),
        None,
    )
}

/// `info.setFiles(emptySet()); codec.segmentInfoFormat().write(dir, info,
/// IOContext.DEFAULT); new SegmentCommitInfo(info, 0, 0, -1, -1, -1,
/// StringHelper.randomId())`.
fn write_and_commit_info(
    dir: &Arc<dyn Directory>,
    codec: &Arc<dyn rucene::codecs::Codec>,
    info: SegmentInfo,
) -> Result<SegmentCommitInfo> {
    info.set_files(std::iter::empty::<String>())?;
    codec
        .segment_info_format()
        .write(&**dir, &info, &*DEFAULT_IO_CONTEXT)?;
    SegmentCommitInfo::new(info, 0, 0, -1, -1, -1, StringHelper::random_id())
}

/// Port of `TestSegmentInfos.testIllegalCreatedVersion`.
fn test_illegal_created_version() -> Result<()> {
    let e = expect_throws(IAE, || SegmentInfos::new(5));
    assert_equals(
        &Some("indexCreatedVersionMajor must be >= 6, got: 5"),
        &e.message(),
    );
    let e = expect_throws(IAE, || SegmentInfos::new(latest_major() + 1));
    assert_equals(
        &Some(format!(
            "indexCreatedVersionMajor is in the future: {}",
            latest_major() + 1
        )),
        &e.message().map(str::to_string),
    );
    Ok(())
}

/// LUCENE-5954.
///
/// Port of `TestSegmentInfos.testVersionsNoSegments`.
fn test_versions_no_segments() -> Result<()> {
    let mut sis = SegmentInfos::new(latest_major())?;
    let dir = LuceneTestCase::new_directory();
    set_check_index_on_close(&dir, false);
    sis.commit(&*dir.as_directory())?;
    let sis = SegmentInfos::read_latest_commit(dir.as_directory())?;
    assert_null(&sis.min_segment_lucene_version());
    assert_equals(&Version::LATEST, &sis.commit_lucene_version());
    dir.close()
}

/// LUCENE-5954.
///
/// Port of `TestSegmentInfos.testVersionsOneSegment`.
fn test_versions_one_segment() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    set_check_index_on_close(&dir, false);
    let d = dir.as_directory();
    let id = StringHelper::random_id();
    let codec = default_codec();

    let mut sis = SegmentInfos::new(latest_major())?;
    let info = one_doc_info(&d, Version::LUCENE_10_0_0, "_0", id)?;
    let commit_info = write_and_commit_info(&d, &codec, info)?;

    sis.add(commit_info)?;
    sis.commit(&*d)?;
    let sis = SegmentInfos::read_latest_commit(Arc::clone(&d))?;
    assert_equals(
        &Some(Version::LUCENE_10_0_0),
        &sis.min_segment_lucene_version(),
    );
    assert_equals(&Version::LATEST, &sis.commit_lucene_version());
    drop(d);
    dir.close()
}

/// LUCENE-5954.
///
/// Port of `TestSegmentInfos.testVersionsTwoSegments`.
fn test_versions_two_segments() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    set_check_index_on_close(&dir, false);
    let d = dir.as_directory();
    let id = StringHelper::random_id();
    let codec = default_codec();

    let mut sis = SegmentInfos::new(latest_major())?;
    let info = one_doc_info(&d, Version::LUCENE_10_0_0, "_0", id)?;
    let commit_info = write_and_commit_info(&d, &codec, info)?;
    sis.add(commit_info)?;

    let info = one_doc_info(&d, Version::LUCENE_10_0_0, "_1", id)?;
    let commit_info = write_and_commit_info(&d, &codec, info)?;
    sis.add(commit_info)?;

    sis.commit(&*d)?;
    let commit_info_id0 = sis.info(0).id();
    let commit_info_id1 = sis.info(1).id();
    let sis = SegmentInfos::read_latest_commit(Arc::clone(&d))?;
    assert_equals(
        &Some(Version::LUCENE_10_0_0),
        &sis.min_segment_lucene_version(),
    );
    assert_equals(&Version::LATEST, &sis.commit_lucene_version());
    assert_equals(
        &StringHelper::id_to_string(Some(&commit_info_id0)),
        &StringHelper::id_to_string(Some(&sis.info(0).id())),
    );
    assert_equals(
        &StringHelper::id_to_string(Some(&commit_info_id1)),
        &StringHelper::id_to_string(Some(&sis.info(1).id())),
    );
    drop(d);
    dir.close()
}

/// `new SegmentInfo(dir, LATEST, LATEST, "TEST", 10000, false, false, codec,
/// diagnostics, randomId(), attributes, Sort.INDEXORDER)`.
fn test_info(
    dir: &Arc<dyn Directory>,
    codec: &Arc<dyn rucene::codecs::Codec>,
    diagnostics: HashMap<String, String>,
    attributes: HashMap<String, String>,
) -> Result<SegmentInfo> {
    SegmentInfo::new(
        Arc::clone(dir),
        Version::LATEST,
        Some(Version::LATEST),
        "TEST".to_string(),
        10000,
        false,
        false,
        Arc::clone(codec),
        diagnostics,
        StringHelper::random_id(),
        attributes,
        Some(index_order()?),
    )
}

/// Test toString method.
///
/// Port of `TestSegmentInfos.testToString`.
fn test_to_string() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let codec = default_codec();

    // diagnostics map
    let diagnostics = map_of(&[("key1", "value1"), ("key2", "value2")]);

    // attributes map
    let attributes = map_of(&[("akey1", "value1"), ("akey2", "value2")]);

    let latest = Version::LATEST.to_string();

    // diagnostics X, attributes X
    let si = test_info(&d, &codec, HashMap::new(), HashMap::new())?;
    assert_equals(
        &format!("TEST({latest}):C10000:[indexSort=<doc>]"),
        &si.to_string_verbose(),
    );
    assert_equals(
        &format!("TEST({latest}):C10000"),
        &si.to_string_with_del_count(0),
    );

    // diagnostics O, attributes X
    let si = test_info(&d, &codec, diagnostics.clone(), HashMap::new())?;
    assert_equals(
        &format!(
            "TEST({latest}):C10000:[indexSort=<doc>]:[diagnostics={}]",
            map_to_string(&diagnostics)
        ),
        &si.to_string_verbose(),
    );
    assert_equals(
        &format!("TEST({latest}):C10000"),
        &si.to_string_with_del_count(0),
    );

    // diagnostics X, attributes O
    let si = test_info(&d, &codec, HashMap::new(), attributes.clone())?;
    assert_equals(
        &format!(
            "TEST({latest}):C10000:[indexSort=<doc>]:[attributes={}]",
            map_to_string(&attributes)
        ),
        &si.to_string_verbose(),
    );
    assert_equals(
        &format!("TEST({latest}):C10000"),
        &si.to_string_with_del_count(0),
    );

    // diagnostics O, attributes O
    let si = test_info(&d, &codec, diagnostics.clone(), attributes.clone())?;
    assert_equals(
        &format!(
            "TEST({latest}):C10000:[indexSort=<doc>]:[diagnostics={}]:[attributes={}]",
            map_to_string(&diagnostics),
            map_to_string(&attributes)
        ),
        &si.to_string_verbose(),
    );
    assert_equals(
        &format!("TEST({latest}):C10000"),
        &si.to_string_with_del_count(0),
    );

    drop(d);
    dir.close()
}

/// `StringHelper.idToString(id)`.
fn id_str(id: &[u8]) -> String {
    StringHelper::id_to_string(Some(id))
}

/// Port of `TestSegmentInfos.testIDChangesOnAdvance`.
fn test_id_changes_on_advance() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        set_check_index_on_close(&dir, false);
        let id = StringHelper::random_id();
        let info = SegmentInfo::new(
            dir.as_directory(),
            Version::LUCENE_9_0_0,
            Some(Version::LUCENE_9_0_0),
            "_0".to_string(),
            1,
            false,
            false,
            default_codec(),
            HashMap::new(),
            StringHelper::random_id(),
            HashMap::new(),
            None,
        )?;
        let commit_info = SegmentCommitInfo::new(info, 0, 0, -1, -1, -1, id)?;
        assert_equals(&id_str(&id), &id_str(&commit_info.id()));
        commit_info.advance_del_gen();
        assert_not_equals(&id_str(&id), &id_str(&commit_info.id()));

        let id = commit_info.id();
        commit_info.advance_doc_values_gen();
        assert_not_equals(&id_str(&id), &id_str(&commit_info.id()));

        let id = commit_info.id();
        commit_info.advance_field_infos_gen();
        assert_not_equals(&id_str(&id), &id_str(&commit_info.id()));
        let clone = commit_info.java_clone();
        let id = commit_info.id();
        assert_equals(&id_str(&id), &id_str(&commit_info.id()));
        assert_equals(&id_str(&id), &id_str(&clone.id()));

        commit_info.advance_field_infos_gen();
        assert_not_equals(&id_str(&id), &id_str(&commit_info.id()));
        assert_equals_msg(
            Some("clone changed but shouldn't"),
            &id_str(&id),
            &id_str(&clone.id()),
        );
        Ok(())
    })();
    let closed = dir.close();
    result.and(closed)
}

/// Port of `TestSegmentInfos.testBitFlippedTriggersCorruptIndexException`.
fn test_bit_flipped_triggers_corrupt_index_exception() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    set_check_index_on_close(&dir, false);
    let d = dir.as_directory();
    let id = StringHelper::random_id();
    let codec = default_codec();

    let mut sis = SegmentInfos::new(latest_major())?;
    let info = one_doc_info(&d, Version::LATEST, "_0", id)?;
    let commit_info = write_and_commit_info(&d, &codec, info)?;
    sis.add(commit_info)?;

    let info = one_doc_info(&d, Version::LATEST, "_1", id)?;
    let commit_info = write_and_commit_info(&d, &codec, info)?;
    sis.add(commit_info)?;

    sis.commit(&*d)?;

    let corrupt_dir = LuceneTestCase::new_directory();
    set_check_index_on_close(&corrupt_dir, false);
    let cd = corrupt_dir.as_directory();
    let mut corrupt = false;
    for file in d.list_all()? {
        if file.starts_with(SEGMENTS) {
            {
                let mut r#in = d.open_input(&file, &*READONCE_IO_CONTEXT)?;
                let mut out = cd.create_output(&file, &*DEFAULT_IO_CONTEXT)?;
                let result = (|| -> Result<()> {
                    let corrupt_index = TestUtil::next_long(&*random(), 0, r#in.length() - 1);
                    out.copy_bytes(&mut *r#in, corrupt_index)?;
                    let b =
                        i32::from(r#in.read_byte()?) + TestUtil::next_int(&*random(), 0x01, 0xff);
                    out.write_byte(b as u8)?;
                    let remaining = r#in.length() - r#in.file_pointer()?;
                    out.copy_bytes(&mut *r#in, remaining)?;
                    Ok(())
                })();
                let closed_out = out.close();
                let closed_in = r#in.close();
                result.and(closed_out).and(closed_in)?;
            }
            {
                let mut r#in = cd.open_input(&file, &*READONCE_IO_CONTEXT)?;
                let checked = codec_util::checksum_entire_file(&mut *r#in);
                let closed = r#in.close();
                match checked {
                    Ok(_) => {
                        closed?;
                        if verbose() {
                            println!(
                                "TEST: Altering the file did not update the checksum, aborting..."
                            );
                        }
                        return Ok(());
                    }
                    Err(e) if is_instance(&e, "org.apache.lucene.index.CorruptIndexException") => {
                        // ok
                        closed?;
                    }
                    Err(e) => return Err(e),
                }
            }
            corrupt = true;
        } else if !ExtrasFS::is_extra(&file) {
            cd.copy_from(&*d, &file, &file, &*DEFAULT_IO_CONTEXT)?;
        }
    }
    Assert::assert_true_msg("No segments file found", corrupt);

    expect_throws_any_of(
        &[
            "org.apache.lucene.index.CorruptIndexException",
            "org.apache.lucene.index.IndexFormatTooOldException",
            "org.apache.lucene.index.IndexFormatTooNewException",
        ],
        || SegmentInfos::read_latest_commit(Arc::clone(&cd)),
    );
    drop((d, cd));
    dir.close()?;
    corrupt_dir.close()
}

/// Test addDiagnostics method.
///
/// Port of `TestSegmentInfos.testAddDiagnostics`.
fn test_add_diagnostics() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let codec = default_codec();

    // diagnostics map
    let diagnostics = map_of(&[("key1", "value1"), ("key2", "value2")]);

    // adds an additional key/value pair
    let mut si = test_info(&d, &codec, diagnostics.clone(), HashMap::new())?;
    si.add_diagnostics(map_of(&[("key3", "value3")]));
    assert_equals(
        &map_of(&[("key1", "value1"), ("key2", "value2"), ("key3", "value3")]),
        &si.get_diagnostics().to_std(),
    );

    // modifies an existing key/value pair
    let mut si = test_info(&d, &codec, diagnostics.clone(), HashMap::new())?;
    si.add_diagnostics(map_of(&[("key2", "foo")]));
    assert_equals(
        &map_of(&[("key1", "value1"), ("key2", "foo")]),
        &si.get_diagnostics().to_std(),
    );

    drop(d);
    dir.close()
}
