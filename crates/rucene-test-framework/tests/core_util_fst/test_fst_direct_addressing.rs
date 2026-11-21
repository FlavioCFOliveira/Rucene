//! Port of `lucene/core/src/test/org/apache/lucene/util/fst/TestFSTDirectAddressing.java`.
//!
//! The class's `main` entry point and the command-line tools it dispatches
//! to (`countFSTArcs`, `measureFSTOversizing`, `recompileAndWalk`, `walk`,
//! `recompile`, `newInputStream`, `printStats`) are not test members: JUnit
//! never calls them, so they are not ported.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::fst::{
    Builder, BytesRefFSTEnum, FSTCompiler, InputType, NoOutputs, Outputs, Util,
    DIRECT_ADDRESSING_MAX_OVERSIZING_FACTOR, FST,
};
use rucene::util::{Accountable, BytesRef, IntsRefBuilder};
use rucene_test_framework::util::lucene_test_case::Nightly;

use crate::core_support::{class, random, test, JavaHashSet};

/// `TestFSTDirectAddressing`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testDenseWithGap", test_dense_with_gap),
        test("testDeDupTails", test_de_dup_tails),
        test(
            "testWorstCaseForDirectAddressing",
            test_worst_case_for_direct_addressing,
        )
        .annotate(Nightly),
    ];
    class(
        "org.apache.lucene.util.fst.TestFSTDirectAddressing",
        methods,
    )
}

fn test_dense_with_gap() -> Result<()> {
    let words = ["ah", "bi", "cj", "dk", "fl", "gm"];
    let mut entries = Vec::new();
    for word in &words {
        entries.push(BytesRef::new(word.as_bytes().to_vec()));
    }
    let fst = build_fst(&entries)?;
    let mut fst_enum = BytesRefFSTEnum::new(&fst)?;
    for entry in &entries {
        let found = fst_enum.seek_exact(entry)?.is_some();
        Assert::assert_true_msg(&format!("{} not found", entry.utf8_to_string()?), found);
    }
    Ok(())
}

fn test_de_dup_tails() -> Result<()> {
    let mut entries = Vec::new();
    let mut i = 0;
    while i < 1000000 {
        let mut b = [0u8; 3];
        let mut val = i;
        for j in (0..b.len()).rev() {
            b[j] = (val & 0xff) as u8;
            val >>= 8;
        }
        entries.push(BytesRef::new(b.to_vec()));
        i += 4;
    }
    let size = build_fst(&entries)?.num_bytes();
    // Size is 1648 when we use only list-encoding. We were previously failing to ever de-dup
    // direct addressing, which led this case to blow up.
    // This test will fail if there is more than 1% size increase with direct addressing.
    Assert::assert_true_msg(
        &format!("FST size = {size} B"),
        size as f64 <= 1648.0 * 1.01f64,
    );
    Ok(())
}

fn test_worst_case_for_direct_addressing() -> Result<()> {
    // This test will fail if there is more than 1% memory increase with direct addressing in this
    // worst case.
    let memory_increase_limit_percent = 1f64;
    let num_words = 1000000;

    // Generate words with specially crafted bytes.
    let mut word_set: JavaHashSet<BytesRef> = JavaHashSet::new();
    for _ in 0..num_words {
        let mut b = [0u8; 5];
        random().next_bytes(&mut b);
        for v in b.iter_mut() {
            *v &= 0xfc; // Make this byte a multiple of 4.
        }
        word_set.add(BytesRef::new(b.to_vec()));
    }
    let mut word_list = word_set.to_vec();
    word_list.sort();

    // Disable direct addressing and measure the FST size.
    let fst = build_fst_with(&word_list, create_fst_compiler(-1.0)?)?;
    let ram_bytes_used_no_direct_addressing = fst.ram_bytes_used();

    // Enable direct addressing and measure the FST size.
    let fst = build_fst_with(
        &word_list,
        create_fst_compiler(DIRECT_ADDRESSING_MAX_OVERSIZING_FACTOR)?,
    )?;
    let ram_bytes_used = fst.ram_bytes_used();

    // Compute the size increase in percents.
    let direct_addressing_memory_increase_percent =
        (ram_bytes_used as f64 / ram_bytes_used_no_direct_addressing as f64 - 1.0) * 100.0;

    //    printStats(builder, ramBytesUsed, directAddressingMemoryIncreasePercent);

    // Verify the FST size does not exceed the limit.
    Assert::assert_true_msg(
        &format!(
            "FST size exceeds limit, size = {ram_bytes_used}, increase = {direct_addressing_memory_increase_percent} %, limit = {memory_increase_limit_percent} %"
        ),
        direct_addressing_memory_increase_percent < memory_increase_limit_percent,
    );
    Ok(())
}

/// `createFSTCompiler(directAddressingMaxOversizingFactor)`.
fn create_fst_compiler(
    direct_addressing_max_oversizing_factor: f32,
) -> Result<FSTCompiler<'static, NoOutputs>> {
    Builder::new(InputType::Byte1, NoOutputs::get_singleton())
        .direct_addressing_max_oversizing_factor(direct_addressing_max_oversizing_factor)
        .build()
}

/// `buildFST(entries)`.
fn build_fst(entries: &[BytesRef]) -> Result<FST<NoOutputs>> {
    build_fst_with(
        entries,
        create_fst_compiler(DIRECT_ADDRESSING_MAX_OVERSIZING_FACTOR)?,
    )
}

/// `buildFST(entries, fstCompiler)`.
fn build_fst_with(
    entries: &[BytesRef],
    mut fst_compiler: FSTCompiler<'static, NoOutputs>,
) -> Result<FST<NoOutputs>> {
    let mut last: Option<&BytesRef> = None;
    for entry in entries {
        if last != Some(entry) {
            fst_compiler.add(
                &Util::to_ints_ref(entry, &mut IntsRefBuilder::new()),
                NoOutputs::get_singleton().no_output(),
            )?;
        }
        last = Some(entry);
    }
    let metadata = fst_compiler.compile()?;
    Ok(
        FST::from_fst_reader(metadata, fst_compiler.into_fst_reader()?)
            .expect("the entries make a non-empty FST"),
    )
}
