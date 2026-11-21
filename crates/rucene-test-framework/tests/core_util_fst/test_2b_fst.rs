//! Ports of `lucene/core/src/test/org/apache/lucene/util/fst/Test2BFST.java`
//! and `Test2BFSTOffHeap.java` (`@Ignore`d in Lucene: the first requires
//! tons of heap, the second takes ~4.5 hours; `@SuppressSysoutChecks`,
//! `@TimeoutSuite(millis = 100 * TimeUnits.HOUR)`). The two classes share
//! their body; `Test2BFSTOffHeap` streams the FST to an `IndexOutput` and
//! reads it back through an `OffHeapFSTStore`, `Test2BFST` keeps it on heap
//! and saves and reloads it once between the two verification passes.

use std::sync::Arc;

use randomizedtesting::annotations::TimeoutSuite;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Ignore;
use rucene::error::Result;
use rucene::store::mmap::MMapDirectory;
use rucene::store::{ByteBuffersDataOutput, Directory, DEFAULT_IO_CONTEXT};
use rucene::util::fst::{
    Builder, ByteSequenceOutputs, FSTCompiler, FSTMetadata, FSTReader, FstOutputSink, InputType,
    IntsRefFSTEnum, NoOutputs, OffHeapFSTStore, Outputs, PositiveIntOutputs, Util, FST,
};
use rucene::util::{BytesRef, IntsRef};
use rucene_test_framework::util::lucene_test_case::SuppressSysoutChecks;
use rucene_test_framework::util::time_units::TimeUnits;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class_annotated, random, test};

/// `LIMIT`.
const LIMIT: i64 = 3 * 1024 * 1024 * 1024;

/// The class annotations both classes carry, with their `@Ignore` reason.
fn annotate(
    builder: randomizedtesting::jdk::reflect::ClassBuilder,
    ignore: &str,
) -> randomizedtesting::jdk::reflect::ClassBuilder {
    builder
        .annotate(Ignore {
            value: ignore.to_string(),
        })
        .annotate(SuppressSysoutChecks {
            bug_url: "test prints helpful progress reports with time".to_string(),
        })
        .annotate(TimeoutSuite {
            millis: 100 * TimeUnits::HOUR,
        })
}

/// `Test2BFST`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![test("test", test_on_heap)];
    class_annotated("org.apache.lucene.util.fst.Test2BFST", methods, |b| {
        annotate(
            b,
            "Requires tons of heap to run (30 GB hits OOME but 35 GB passes after ~4.5 hours)",
        )
    })
}

/// `Test2BFSTOffHeap`.
pub fn off_heap_class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![test("test", test_off_heap)];
    class_annotated(
        "org.apache.lucene.util.fst.Test2BFSTOffHeap",
        methods,
        |b| annotate(b, "Will take long time to run (~4.5 hours)"),
    )
}

fn test_on_heap() -> Result<()> {
    run(false, "2BFST")
}

fn test_off_heap() -> Result<()> {
    run(true, "2BFSTOffHeap")
}

/// `nextInput(r, ints)`.
fn next_input(r: &dyn Random, ints: &mut [i32]) {
    let mut down_to: i32 = 6;
    while down_to >= 0 {
        let d = down_to as usize;
        ints[d] += 1 + r.next_int_bound(10);
        if ints[d] < 256 {
            break;
        } else {
            ints[d] = 0;
            down_to -= 1;
        }
    }
}

/// The FST of a compiler: on heap (`FST.fromFSTReader(compile(),
/// getFSTReader())`), or off heap from `fst` in `dir`.
/// `fstCompiler.compile()`, with the compiler's node and arc counts and,
/// for an on-heap FST, its `getFSTReader()`.
#[allow(clippy::type_complexity)]
fn finish<O: Outputs + Clone>(
    mut fst_compiler: FSTCompiler<'_, O>,
    off_heap: bool,
) -> Result<(FSTMetadata<O>, Option<Box<dyn FSTReader>>, i64, i64)> {
    let metadata = fst_compiler.compile()?.expect("inputs were added");
    let node_count = fst_compiler.node_count();
    let arc_count = fst_compiler.arc_count();
    let reader = if off_heap {
        None
    } else {
        Some(fst_compiler.into_fst_reader()?)
    };
    Ok((metadata, reader, node_count, arc_count))
}

/// `FST.fromFSTReader(fstMetadata, reader)`: the on-heap reader, or a new
/// `OffHeapFSTStore(indexInput, indexInput.getFilePointer(), fstMetadata)`
/// over the streamed file.
fn open_fst<O: Outputs + Clone>(
    dir: &Arc<dyn Directory>,
    metadata: FSTMetadata<O>,
    reader: Option<Box<dyn FSTReader>>,
) -> Result<FST<O>> {
    Ok(match reader {
        Some(reader) => FST::from_reader(metadata, reader),
        None => {
            let index_input = dir.open_input("fst", &*DEFAULT_IO_CONTEXT)?;
            let fp = index_input.file_pointer()?;
            let store = OffHeapFSTStore::new(index_input, fp, &metadata);
            FST::from_reader(metadata, Box::new(store))
        }
    })
}

/// `fst.save(out, out)` to `fst`, then `new FST<>(FST.readMetadata(in,
/// outputs), in)`: the metadata, then the bytes, into one output.
fn save_and_load<O: Outputs + Clone>(
    dir: &Arc<dyn Directory>,
    fst: &FST<O>,
    outputs: O,
) -> Result<FST<O>> {
    println!("\nTEST: save/load FST and re-verify");
    let mut out = dir.create_output("fst", &*DEFAULT_IO_CONTEXT)?;
    let mut meta = ByteBuffersDataOutput::new();
    let mut body = ByteBuffersDataOutput::new();
    fst.save(&mut meta, &mut body)?;
    meta.copy_to(out.as_mut())?;
    body.copy_to(out.as_mut())?;
    out.close()?;
    let mut input = dir.open_input("fst", &*DEFAULT_IO_CONTEXT)?;
    let metadata = FST::read_metadata(input.as_mut(), outputs)?;
    let fst = FST::new(metadata, input.as_mut())?;
    input.close()?;
    Ok(fst)
}

/// The body of `test()` of both classes.
fn run(off_heap: bool, temp_dir_prefix: &str) -> Result<()> {
    let mut ints = vec![0i32; 7];
    let seed = random().next_long();
    let dir: Arc<dyn Directory> = Arc::new(MMapDirectory::open(
        LuceneTestCase::create_temp_dir_prefix(temp_dir_prefix),
    )?);

    // Build FST w/ NoOutputs and stop when nodeCount > 2.2B
    {
        println!("\nTEST: ~2.2B nodes; output=NO_OUTPUTS");
        let outputs = NoOutputs::get_singleton();
        let no_output = outputs.no_output();
        let mut index_output = if off_heap {
            Some(dir.create_output("fst", &*DEFAULT_IO_CONTEXT)?)
        } else {
            None
        };
        let mut builder = Builder::new(InputType::Byte1, outputs);
        if let Some(out) = index_output.as_mut() {
            builder = builder.data_output(FstOutputSink::Streaming(out.as_mut()));
        }
        let mut fst_compiler = builder.build()?;

        let mut count = 0i32;
        let mut r = JdkRandom::new(seed);
        let mut ints2 = vec![0i32; 200];
        let start_time = std::time::Instant::now();
        loop {
            // System.out.println("add: " + input + " -> " + output);
            for v in ints2.iter_mut().skip(10) {
                *v = r.next_int_bound(256);
            }
            fst_compiler.add(&IntsRef::new(ints2.clone()), no_output)?;
            count += 1;
            if count % 100000 == 0 {
                println!(
                    "{count}: {} RAM bytes used; {} FST bytes; {} nodes; took {} seconds",
                    fst_compiler.fst_ram_bytes_used(),
                    fst_compiler.fst_size_in_bytes(),
                    fst_compiler.node_count(),
                    start_time.elapsed().as_secs()
                );
            }
            if fst_compiler.node_count() > i32::MAX as i64 + 100 * 1024 * 1024 {
                break;
            }
            next_input(&r, &mut ints2);
        }

        let (metadata, reader, node_count, arc_count) = finish(fst_compiler, off_heap)?;
        if let Some(mut out) = index_output.take() {
            out.close()?;
        }
        let mut fst = open_fst(&dir, metadata, reader)?;

        for verify in 0..2 {
            println!(
                "\nTEST: now verify [fst size={}; nodeCount={node_count}; arcCount={arc_count}]",
                fst.num_bytes()
            );

            ints2.fill(0);
            r = JdkRandom::new(seed);
            let start_time = std::time::Instant::now();
            for i in 0..count {
                if i % 1000000 == 0 {
                    println!("{i}...: took {} seconds", start_time.elapsed().as_secs());
                }
                for v in ints2.iter_mut().skip(10) {
                    *v = r.next_int_bound(256);
                }
                assert_equals(
                    &Some(no_output),
                    &Util::get(&fst, &IntsRef::new(ints2.clone()))?,
                );
                next_input(&r, &mut ints2);
            }

            println!("\nTEST: enum all input/outputs");
            let mut fst_enum = IntsRefFSTEnum::new(&fst)?;

            ints2.fill(0);
            r = JdkRandom::new(seed);
            let mut upto = 0;
            loop {
                let pair = fst_enum.next()?;
                let Some(pair) = pair else { break };
                for v in ints2.iter_mut().skip(10) {
                    *v = r.next_int_bound(256);
                }
                assert_equals(&IntsRef::new(ints2.clone()), pair.input);
                assert_equals(&no_output, pair.output);
                upto += 1;
                next_input(&r, &mut ints2);
            }
            assert_equals(&count, &upto);
            drop(fst_enum);

            if off_heap {
                continue;
            }
            if verify == 0 {
                fst = save_and_load(&dir, &fst, outputs)?;
            } else {
                dir.delete_file("fst")?;
            }
        }
        if off_heap {
            drop(fst);
            dir.delete_file("fst")?;
        }
    }

    // Build FST w/ ByteSequenceOutputs and stop when FST
    // size = 3GB
    {
        println!("\nTEST: 3 GB size; outputs=bytes");
        let outputs = ByteSequenceOutputs::get_singleton();
        let mut index_output = if off_heap {
            Some(dir.create_output("fst", &*DEFAULT_IO_CONTEXT)?)
        } else {
            None
        };
        let mut builder = Builder::new(InputType::Byte1, outputs);
        if let Some(out) = index_output.as_mut() {
            builder = builder.data_output(FstOutputSink::Streaming(out.as_mut()));
        }
        let mut fst_compiler = builder.build()?;

        let mut output_bytes = vec![0u8; 20];
        ints.fill(0);
        let mut count = 0i32;
        let mut r = JdkRandom::new(seed);
        loop {
            r.next_bytes(&mut output_bytes);
            fst_compiler.add(
                &IntsRef::new(ints.clone()),
                BytesRef::new(output_bytes.clone()),
            )?;
            count += 1;
            if count % 10000 == 0 {
                let size = fst_compiler.fst_size_in_bytes();
                if count % 1000000 == 0 {
                    println!("{count}...: {size} bytes");
                }
                if size > LIMIT {
                    break;
                }
            }
            next_input(&r, &mut ints);
        }

        let (metadata, reader, node_count, arc_count) = finish(fst_compiler, off_heap)?;
        if let Some(mut out) = index_output.take() {
            out.close()?;
        }
        let mut fst = open_fst(&dir, metadata, reader)?;
        for verify in 0..2 {
            println!(
                "\nTEST: now verify [fst size={}; nodeCount={node_count}; arcCount={arc_count}]",
                fst.num_bytes()
            );

            r = JdkRandom::new(seed);
            ints.fill(0);

            let start_time = std::time::Instant::now();
            for i in 0..count {
                if i % 1000000 == 0 {
                    println!("{i}...: took {} seconds", start_time.elapsed().as_secs());
                }
                r.next_bytes(&mut output_bytes);
                assert_equals(
                    &Some(BytesRef::new(output_bytes.clone())),
                    &Util::get(&fst, &IntsRef::new(ints.clone()))?,
                );
                next_input(&r, &mut ints);
            }

            println!("\nTEST: enum all input/outputs");
            let mut fst_enum = IntsRefFSTEnum::new(&fst)?;

            ints.fill(0);
            r = JdkRandom::new(seed);
            let mut upto = 0;
            while let Some(pair) = fst_enum.next()? {
                assert_equals(&IntsRef::new(ints.clone()), pair.input);
                r.next_bytes(&mut output_bytes);
                assert_equals(&BytesRef::new(output_bytes.clone()), pair.output);
                upto += 1;
                next_input(&r, &mut ints);
            }
            assert_equals(&count, &upto);
            drop(fst_enum);

            if off_heap {
                continue;
            }
            if verify == 0 {
                fst = save_and_load(&dir, &fst, outputs)?;
            } else {
                dir.delete_file("fst")?;
            }
        }
        if off_heap {
            drop(fst);
            dir.delete_file("fst")?;
        }
    }

    // Build FST w/ PositiveIntOutputs and stop when FST
    // size = 3GB
    {
        println!("\nTEST: 3 GB size; outputs=long");
        let outputs = PositiveIntOutputs::get_singleton();
        let mut index_output = if off_heap {
            Some(dir.create_output("fst", &*DEFAULT_IO_CONTEXT)?)
        } else {
            None
        };
        let mut builder = Builder::new(InputType::Byte1, outputs);
        if let Some(out) = index_output.as_mut() {
            builder = builder.data_output(FstOutputSink::Streaming(out.as_mut()));
        }
        let mut fst_compiler = builder.build()?;

        let mut output: i64 = 1;
        ints.fill(0);
        let mut count = 0i32;
        let mut r = JdkRandom::new(seed);
        loop {
            fst_compiler.add(&IntsRef::new(ints.clone()), output)?;
            output += 1 + r.next_int_bound(10) as i64;
            count += 1;
            if count % 10000 == 0 {
                let size = fst_compiler.fst_size_in_bytes();
                if count % 1000000 == 0 {
                    println!("{count}...: {size} bytes");
                }
                if size > LIMIT {
                    break;
                }
            }
            next_input(&r, &mut ints);
        }

        let (metadata, reader, node_count, arc_count) = finish(fst_compiler, off_heap)?;
        if let Some(mut out) = index_output.take() {
            out.close()?;
        }
        let mut fst = open_fst(&dir, metadata, reader)?;

        for verify in 0..2 {
            println!(
                "\nTEST: now verify [fst size={}; nodeCount={node_count}; arcCount={arc_count}]",
                fst.num_bytes()
            );

            ints.fill(0);

            output = 1;
            r = JdkRandom::new(seed);
            let start_time = std::time::Instant::now();
            for i in 0..count {
                if i % 1000000 == 0 {
                    println!("{i}...: took {} seconds", start_time.elapsed().as_secs());
                }

                // forward lookup:
                assert_equals(
                    &Some(output),
                    &Util::get(&fst, &IntsRef::new(ints.clone()))?,
                );
                // reverse lookup:
                // assertEquals(input, Util.getByOutput(fst, output));
                output += 1 + r.next_int_bound(10) as i64;
                next_input(&r, &mut ints);
            }

            println!("\nTEST: enum all input/outputs");
            let mut fst_enum = IntsRefFSTEnum::new(&fst)?;

            ints.fill(0);
            r = JdkRandom::new(seed);
            let mut upto = 0;
            output = 1;
            while let Some(pair) = fst_enum.next()? {
                assert_equals(&IntsRef::new(ints.clone()), pair.input);
                assert_equals(&output, pair.output);
                output += 1 + r.next_int_bound(10) as i64;
                upto += 1;
                next_input(&r, &mut ints);
            }
            assert_equals(&count, &upto);
            drop(fst_enum);

            if off_heap {
                continue;
            }
            if verify == 0 {
                fst = save_and_load(&dir, &fst, outputs)?;
            } else {
                dir.delete_file("fst")?;
            }
        }
        if off_heap {
            drop(fst);
            dir.delete_file("fst")?;
        }
    }
    Ok(())
}
