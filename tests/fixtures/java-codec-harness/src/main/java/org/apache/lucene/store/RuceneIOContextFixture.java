/*
 * Measures, from lucene-core-10.5.0, every observable value that the Rucene
 * portability test `tests/portability/io_context_contracts.rs` asserts about
 * org.apache.lucene.store.IOContext and the things that read it.
 *
 * Lives in org.apache.lucene.store because DefaultIOContext is package-private
 * (`DefaultIOContext.java:25`), so the validation it performs on duplicate
 * hints can only be provoked from inside the package.
 *
 * Prints key=value lines on stdout. Hint sets are printed sorted, because
 * IOContext.hints() is a java.util.Set whose iteration order is deliberately
 * unspecified and varies between JVM runs.
 */
package org.apache.lucene.store;

import java.util.ArrayList;
import java.util.List;
import java.util.stream.Collectors;
import org.apache.lucene.util.Constants;

public final class RuceneIOContextFixture {

  private static String hintName(IOContext.FileOpenHint hint) {
    return hint.getClass().getSimpleName() + "." + hint;
  }

  private static String hints(IOContext context) {
    return context.hints().stream()
        .map(RuceneIOContextFixture::hintName)
        .sorted()
        .collect(Collectors.joining(","));
  }

  /** Prints the five accessors the IOContext interface declares. */
  private static void dump(String prefix, IOContext context) {
    System.out.println(prefix + ".context=" + context.context());
    System.out.println(prefix + ".merge_info=" + context.mergeInfo());
    System.out.println(prefix + ".flush_info=" + context.flushInfo());
    System.out.println(prefix + ".hints=" + hints(context));
    System.out.println(prefix + ".hint_count=" + context.hints().size());
  }

  public static void main(String[] args) {
    MergeInfo mergeInfo = new MergeInfo(100, 1024, false, 1);
    FlushInfo flushInfo = new FlushInfo(10, 512);

    // ---- the two static factories, IOContext.java:64 and :95 ----------------
    IOContext merge = IOContext.merge(mergeInfo);
    dump("merge", merge);
    System.out.println("merge.total_max_doc=" + merge.mergeInfo().totalMaxDoc());
    System.out.println(
        "merge.estimated_merge_bytes=" + merge.mergeInfo().estimatedMergeBytes());
    System.out.println("merge.is_external=" + merge.mergeInfo().isExternal());
    System.out.println(
        "merge.merge_max_num_segments=" + merge.mergeInfo().mergeMaxNumSegments());

    IOContext flush = IOContext.flush(flushInfo);
    dump("flush", flush);
    System.out.println("flush.num_docs=" + flush.flushInfo().numDocs());
    System.out.println(
        "flush.estimated_segment_size=" + flush.flushInfo().estimatedSegmentSize());

    // ---- withHints is ignored by the merge and flush contexts ---------------
    IOContext mergeHinted = merge.withHints(DataAccessHint.RANDOM, FileTypeHint.DATA);
    dump("merge_hinted", mergeHinted);
    System.out.println("merge_hinted.same_instance=" + (mergeHinted == merge));

    IOContext flushHinted = flush.withHints(DataAccessHint.SEQUENTIAL, PreloadHint.INSTANCE);
    dump("flush_hinted", flushHinted);
    System.out.println("flush_hinted.same_instance=" + (flushHinted == flush));

    // ---- the two constants, IOContext.java:52 and :60 -----------------------
    dump("default", IOContext.DEFAULT);
    dump("readonce", IOContext.READONCE);

    // ---- DEFAULT.withHints builds a new context and leaves DEFAULT alone ----
    IOContext defaultHinted =
        IOContext.DEFAULT.withHints(DataAccessHint.RANDOM, FileTypeHint.INDEX, FileDataHint.POSTINGS);
    dump("default_hinted", defaultHinted);
    System.out.println("default_hinted.same_instance=" + (defaultHinted == IOContext.DEFAULT));
    dump("default_after", IOContext.DEFAULT);

    // withHints REPLACES the hint set; it does not merge with the receiver's.
    IOContext readonceHinted = IOContext.READONCE.withHints(FileTypeHint.DATA);
    dump("readonce_hinted", readonceHinted);
    dump("readonce_after", IOContext.READONCE);

    // ---- the default hints(Class) filter, IOContext.java:136 ----------------
    System.out.println(
        "default_hinted.filter_data_access="
            + defaultHinted.hints(DataAccessHint.class).map(Object::toString).sorted()
                .collect(Collectors.joining(",")));
    System.out.println(
        "default_hinted.filter_read_once_count="
            + defaultHinted.hints(ReadOnceHint.class).count());
    System.out.println(
        "merge.filter_data_access_count=" + merge.hints(DataAccessHint.class).count());

    // ---- DefaultIOContext rejects two hints of one type, DefaultIOContext.java:30 ----
    try {
      new DefaultIOContext(DataAccessHint.RANDOM, DataAccessHint.SEQUENTIAL);
      System.out.println("two_of_one_type.threw=false");
    } catch (IllegalArgumentException e) {
      System.out.println("two_of_one_type.threw=true");
      System.out.println("two_of_one_type.exception=" + e.getClass().getSimpleName());
    }
    // The same constant twice is rejected earlier, by Set.of.
    try {
      new DefaultIOContext(DataAccessHint.RANDOM, DataAccessHint.RANDOM);
      System.out.println("same_hint_twice.threw=false");
    } catch (IllegalArgumentException e) {
      System.out.println("same_hint_twice.threw=true");
      System.out.println("same_hint_twice.exception=" + e.getClass().getSimpleName());
    }
    // Two hints of different types are accepted.
    IOContext twoTypes = new DefaultIOContext(DataAccessHint.RANDOM, ReadOnceHint.INSTANCE);
    dump("two_types", twoTypes);

    // A context that is MERGE *and* carries a RANDOM hint. No Lucene type
    // produces one -- the merge and flush factories return contexts with an
    // empty hint set, and DefaultIOContext is always DEFAULT -- but IOContext
    // is a public interface, so an implementor outside the package can. It is
    // the only way to observe that ADVISE_BY_CONTEXT tests the context before
    // it tests the hints.
    IOContext customMergeRandom =
        new IOContext() {
          @Override
          public Context context() {
            return Context.MERGE;
          }

          @Override
          public MergeInfo mergeInfo() {
            return mergeInfo;
          }

          @Override
          public FlushInfo flushInfo() {
            return null;
          }

          @Override
          public java.util.Set<FileOpenHint> hints() {
            return java.util.Set.of(DataAccessHint.RANDOM);
          }

          @Override
          public IOContext withHints(FileOpenHint... hints) {
            return this;
          }
        };
    dump("custom_merge_random", customMergeRandom);

    // ---- MMapDirectory.ADVISE_BY_CONTEXT, MMapDirectory.java:157 -----------
    List<String> adviceCases = new ArrayList<>();
    adviceCases.add("merge");
    adviceCases.add("flush");
    adviceCases.add("default");
    adviceCases.add("default_random");
    adviceCases.add("default_sequential");
    adviceCases.add("readonce");
    adviceCases.add("custom_merge_random");
    List<IOContext> adviceContexts = new ArrayList<>();
    adviceContexts.add(merge);
    adviceContexts.add(flush);
    adviceContexts.add(IOContext.DEFAULT);
    adviceContexts.add(IOContext.DEFAULT.withHints(DataAccessHint.RANDOM));
    adviceContexts.add(IOContext.DEFAULT.withHints(DataAccessHint.SEQUENTIAL));
    adviceContexts.add(IOContext.READONCE);
    adviceContexts.add(customMergeRandom);
    for (int i = 0; i < adviceCases.size(); i++) {
      System.out.println(
          "advise_by_context."
              + adviceCases.get(i)
              + "="
              + MMapDirectory.ADVISE_BY_CONTEXT
                  .apply("_0.dat", adviceContexts.get(i))
                  .orElseThrow());
    }
    System.out.println("constants.default_read_advice=" + Constants.DEFAULT_READADVICE);

    // ---- MMapDirectory.PRELOAD_HINT, MMapDirectory.java:103 ----------------
    System.out.println(
        "preload_hint.default=" + MMapDirectory.PRELOAD_HINT.test("_0.dat", IOContext.DEFAULT));
    System.out.println(
        "preload_hint.default_preload="
            + MMapDirectory.PRELOAD_HINT.test(
                "_0.dat", IOContext.DEFAULT.withHints(PreloadHint.INSTANCE)));
    System.out.println("preload_hint.merge=" + MMapDirectory.PRELOAD_HINT.test("_0.dat", merge));
    System.out.println(
        "preload_hint.custom_merge_preload="
            + MMapDirectory.PRELOAD_HINT.test(
                "_0.dat",
                new IOContext() {
                  @Override
                  public Context context() {
                    return Context.MERGE;
                  }

                  @Override
                  public MergeInfo mergeInfo() {
                    return mergeInfo;
                  }

                  @Override
                  public FlushInfo flushInfo() {
                    return null;
                  }

                  @Override
                  public java.util.Set<FileOpenHint> hints() {
                    return java.util.Set.of(PreloadHint.INSTANCE);
                  }

                  @Override
                  public IOContext withHints(FileOpenHint... hints) {
                    return this;
                  }
                }));

    // ---- MergeInfo / FlushInfo are records: equals, hashCode, toString ------
    System.out.println("merge_info.to_string=" + mergeInfo);
    System.out.println("flush_info.to_string=" + flushInfo);
    System.out.println(
        "merge_info.equals_same=" + mergeInfo.equals(new MergeInfo(100, 1024, false, 1)));
    System.out.println(
        "merge_info.equals_other=" + mergeInfo.equals(new MergeInfo(100, 1024, true, 1)));
    System.out.println(
        "flush_info.equals_same=" + flushInfo.equals(new FlushInfo(10, 512)));

    System.out.println("read_ok=true");
  }
}
