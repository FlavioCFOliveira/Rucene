//! Port of `lucene/core/src/test/org/apache/lucene/util/fst/TestUtil.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use rucene::error::Result;
use rucene::util::fst::{
    binary_search, Arc as FstArc, Builder, InputType, NoOutputs, Outputs, Util, FST,
};
use rucene::util::{BytesRef, IntsRefBuilder};

use crate::core_support::{assert_equals, class, test};

/// `TestUtil` (of `org.apache.lucene.util.fst`).
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testBinarySearch", test_binary_search),
        test("testContinuous", test_continuous),
        test(
            "testReadCeilArcPackedArray",
            test_read_ceil_arc_packed_array,
        ),
        test(
            "testReadCeilArcArrayWithGaps",
            test_read_ceil_arc_array_with_gaps,
        ),
        test("testReadCeilArcList", test_read_ceil_arc_list),
    ];
    class("org.apache.lucene.util.fst.TestUtil", methods)
}

fn c(ch: char) -> i32 {
    ch as i32
}

fn test_binary_search() -> Result<()> {
    // Create a node with 8 arcs spanning (z-A) and ensure it is encoded as a packed array
    // requiring binary search.
    let letters = ["A", "E", "J", "K", "L", "O", "T", "z"];
    let fst = build_fst(&letters, true, false)?;
    let mut first = FstArc::default();
    fst.get_first_arc(&mut first);
    let mut arc = FstArc::default();
    fst.read_first_target_arc(&first, &mut arc, fst.get_bytes_reader()?.as_mut())?;
    for (i, letter) in letters.iter().enumerate() {
        let label = c(letter.chars().next().unwrap());
        assert_equals(&(i as i32), &binary_search(&fst, &arc, label)?);
    }
    // before the first
    assert_equals(&-1, &binary_search(&fst, &arc, c(' '))?);
    // after the last
    assert_equals(
        &(-1 - letters.len() as i32),
        &binary_search(&fst, &arc, c('~'))?,
    );
    assert_equals(&-2, &binary_search(&fst, &arc, c('B'))?);
    assert_equals(&-2, &binary_search(&fst, &arc, c('C'))?);
    assert_equals(&-7, &binary_search(&fst, &arc, c('P'))?);
    Ok(())
}

/// `Util.readCeilArc(label, fst, follow, arc, in)`: the arc, or `null`.
fn read_ceil_arc(
    label: i32,
    fst: &FST<NoOutputs>,
    follow: &FstArc<()>,
    arc: &mut FstArc<()>,
    input: &mut dyn rucene::util::fst::BytesReader,
) -> Result<Option<FstArc<()>>> {
    if Util::read_ceil_arc(label, fst, follow, arc, input)? {
        Ok(Some(arc.clone()))
    } else {
        Ok(None)
    }
}

fn test_continuous() -> Result<()> {
    let letters = ["A", "B", "C", "D", "E", "F", "G", "H"];
    let fst = build_fst(&letters, true, false)?;
    let mut first = FstArc::default();
    fst.get_first_arc(&mut first);
    let mut arc = FstArc::default();
    let mut input = fst.get_bytes_reader()?;
    for letter in &letters {
        let ch = c(letter.chars().next().unwrap());
        let found = read_ceil_arc(ch, &fst, &first, &mut arc, input.as_mut())?;
        crate::core_support::assert_not_null(&found);
        assert_equals(&ch, &arc.label());
    }
    let found = read_ceil_arc(c('F'), &fst, &first, &mut arc, input.as_mut())?;
    assert_equals(&c('F'), &found.expect("F is present").label());
    let follow = arc.clone();
    crate::core_support::assert_null(
        &read_ceil_arc(c('A'), &fst, &follow, &mut arc, input.as_mut())?.map(|a| a.label()),
    );
    Ok(())
}

fn test_read_ceil_arc_packed_array() -> Result<()> {
    let letters = ["A", "E", "J", "K", "L", "O", "T", "z"];
    verify_read_ceil_arc(&letters, true, false)
}

fn test_read_ceil_arc_array_with_gaps() -> Result<()> {
    let letters = ["A", "E", "J", "K", "L", "O", "T"];
    verify_read_ceil_arc(&letters, true, true)
}

fn test_read_ceil_arc_list() -> Result<()> {
    let letters = ["A", "E", "J", "K", "L", "O", "T", "z"];
    verify_read_ceil_arc(&letters, false, false)
}

/// `verifyReadCeilArc(letters, allowArrayArcs, allowDirectAddressing)`.
fn verify_read_ceil_arc(
    letters: &[&str],
    allow_array_arcs: bool,
    allow_direct_addressing: bool,
) -> Result<()> {
    let fst = build_fst(letters, allow_array_arcs, allow_direct_addressing)?;
    let mut first = FstArc::default();
    fst.get_first_arc(&mut first);
    let mut arc = FstArc::default();
    let mut input = fst.get_bytes_reader()?;
    for letter in letters {
        let ch = c(letter.chars().next().unwrap());
        let found = read_ceil_arc(ch, &fst, &first, &mut arc, input.as_mut())?;
        crate::core_support::assert_not_null(&found);
        assert_equals(&ch, &arc.label());
    }
    // before the first
    assert_equals(
        &c('A'),
        &read_ceil_arc(c(' '), &fst, &first, &mut arc, input.as_mut())?
            .expect("A is present")
            .label(),
    );
    // after the last
    crate::core_support::assert_null(
        &read_ceil_arc(c('~'), &fst, &first, &mut arc, input.as_mut())?.map(|a| a.label()),
    );
    // in the middle
    assert_equals(
        &c('J'),
        &read_ceil_arc(c('F'), &fst, &first, &mut arc, input.as_mut())?
            .expect("J is present")
            .label(),
    );
    // no following arcs
    let follow = arc.clone();
    crate::core_support::assert_null(
        &read_ceil_arc(c('Z'), &fst, &follow, &mut arc, input.as_mut())?.map(|a| a.label()),
    );
    Ok(())
}

/// `buildFST(words, allowArrayArcs, allowDirectAddressing)`.
fn build_fst(
    words: &[&str],
    allow_array_arcs: bool,
    allow_direct_addressing: bool,
) -> Result<FST<NoOutputs>> {
    let outputs = NoOutputs::get_singleton();
    let mut builder =
        Builder::new(InputType::Byte1, outputs).allow_fixed_length_arcs(allow_array_arcs);
    if !allow_direct_addressing {
        builder = builder.direct_addressing_max_oversizing_factor(-1.0);
    }
    let mut fst_compiler = builder.build()?;

    for word in words {
        fst_compiler.add(
            &Util::to_ints_ref(
                &BytesRef::from_char_sequence(word),
                &mut IntsRefBuilder::new(),
            ),
            outputs.no_output(),
        )?;
    }
    let metadata = fst_compiler.compile()?;
    Ok(
        FST::from_fst_reader(metadata, fst_compiler.into_fst_reader()?)
            .expect("the words make a non-empty FST"),
    )
}
