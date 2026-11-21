// The BaseMergePolicyTestCase suite of BaseFormatSuitesOracle (rmp #355): the members of
// TestTieredMergePolicy that bind it (mergePolicy(), assertSegmentInfos, assertMerge and the two
// simulations), recording after each test (in tearDown, before super.tearDown()) the next
// random().nextLong() of the test's randomness, and after the class that of the class randomness.
// It lives in org.apache.lucene.index, as TestTieredMergePolicy does, for the package-private
// TieredMergePolicy.getMaxAllowedDocs.
package org.apache.lucene.index;

import java.io.IOException;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.List;
import java.util.Locale;
import org.apache.lucene.index.MergePolicy.MergeSpecification;
import org.apache.lucene.index.MergePolicy.OneMerge;
import org.apache.lucene.tests.index.BaseFormatSuitesOracle;
import org.apache.lucene.tests.index.BaseMergePolicyTestCase;
import org.apache.lucene.tests.util.TestUtil;
import org.junit.AfterClass;

public class TestMergePolicySuiteDraws extends BaseMergePolicyTestCase {
  private record DocCountAndSizeInBytes(int docCount, long sizeInBytes) {}

  @Override
  public TieredMergePolicy mergePolicy() {
    return newTieredMergePolicy();
  }

  @Override
  protected void assertSegmentInfos(MergePolicy policy, SegmentInfos infos) throws IOException {
    TieredMergePolicy tmp = (TieredMergePolicy) policy;

    final long maxMergedSegmentBytes = (long) (tmp.getMaxMergedSegmentMB() * 1024 * 1024);

    long minSegmentBytes = Long.MAX_VALUE;
    int totalDelCount = 0;
    int totalMaxDoc = 0;
    long totalBytes = 0;
    List<DocCountAndSizeInBytes> segmentSizes = new ArrayList<>();
    for (SegmentCommitInfo sci : infos) {
      totalDelCount += sci.getDelCount();
      totalMaxDoc += sci.info.maxDoc();
      long byteSize = sci.sizeInBytes();
      double liveRatio = 1 - (double) sci.getDelCount() / sci.info.maxDoc();
      long weightedByteSize = (long) (liveRatio * byteSize);
      totalBytes += weightedByteSize;
      segmentSizes.add(
          new DocCountAndSizeInBytes(sci.info.maxDoc() - sci.getDelCount(), weightedByteSize));
      minSegmentBytes = Math.min(minSegmentBytes, weightedByteSize);
    }
    Collections.sort(segmentSizes, Comparator.comparingLong(DocCountAndSizeInBytes::sizeInBytes));

    final double delPercentage = 100.0 * totalDelCount / totalMaxDoc;
    assertTrue(
        "Percentage of deleted docs "
            + delPercentage
            + " is larger than the target: "
            + tmp.getDeletesPctAllowed(),
        delPercentage <= tmp.getDeletesPctAllowed());

    long levelSizeBytes = Math.max(minSegmentBytes, (long) (tmp.getFloorSegmentMB() * 1024 * 1024));
    long bytesLeft = totalBytes;
    double allowedSegCount = 0;
    List<DocCountAndSizeInBytes> biggestSegments = segmentSizes;
    if (biggestSegments.size() > tmp.getTargetSearchConcurrency() - 1) {
      biggestSegments =
          biggestSegments.subList(
              biggestSegments.size() - tmp.getTargetSearchConcurrency() + 1,
              biggestSegments.size());
    }
    // Allow whole segments for the targetSearchConcurrency-1 biggest segments
    for (DocCountAndSizeInBytes size : biggestSegments) {
      bytesLeft -= size.sizeInBytes();
      allowedSegCount++;
    }

    int tooBigCount = 0;
    for (DocCountAndSizeInBytes size : segmentSizes) {
      if (size.sizeInBytes() >= maxMergedSegmentBytes / 2) {
        tooBigCount++;
      }
    }

    // below we make the assumption that segments that reached the max segment
    // size divided by 2 don't need merging anymore
    int mergeFactor = (int) Math.min(tmp.getSegmentsPerTier(), tmp.getMaxMergeAtOnce());
    while (true) {
      final double segCountLevel = bytesLeft / (double) levelSizeBytes;
      if (segCountLevel <= tmp.getSegmentsPerTier()
          || levelSizeBytes >= maxMergedSegmentBytes / 2) {
        allowedSegCount += Math.ceil(segCountLevel);
        break;
      }
      allowedSegCount += tmp.getSegmentsPerTier();
      bytesLeft -= tmp.getSegmentsPerTier() * levelSizeBytes;
      levelSizeBytes = Math.min(levelSizeBytes * mergeFactor, maxMergedSegmentBytes / 2);
    }
    // Allow at least a full tier in addition of the too big segments.
    allowedSegCount = Math.max(allowedSegCount, tooBigCount + tmp.getSegmentsPerTier());
    // Allow at least `targetSearchConcurrency` segments.
    allowedSegCount = Math.max(allowedSegCount, tmp.getTargetSearchConcurrency());

    // It's ok to be over the allowed segment count if none of the merges are legal, because they
    // are either not balanced or because they exceed the max merged segment doc count.
    // We only check pairwise merges instead of every possible merge to keep things simple. If none
    // of the pairwise merges are legal, chances are high that no merge is legal.
    int maxDocsPerSegment = tmp.getMaxAllowedDocs(infos.totalMaxDoc(), totalDelCount);
    boolean hasLegalMerges = false;
    for (int i = 0; i < segmentSizes.size() - 1; ++i) {
      DocCountAndSizeInBytes size1 = segmentSizes.get(i);
      DocCountAndSizeInBytes size2 = segmentSizes.get(i + 1);
      long mergedSegmentSizeInBytes = size1.sizeInBytes() + size2.sizeInBytes();
      int mergedSegmentDocCount = size1.docCount() + size2.docCount();

      if (mergedSegmentSizeInBytes <= maxMergedSegmentBytes
          && size2.sizeInBytes() * 1.5 <= mergedSegmentSizeInBytes
          && mergedSegmentDocCount <= maxDocsPerSegment) {
        hasLegalMerges = true;
        break;
      }
    }

    int numSegments = infos.asList().size();
    assertTrue(
        String.format(
            Locale.ROOT,
            "mergeFactor=%d minSegmentBytes=%,d maxMergedSegmentBytes=%,d segmentsPerTier=%g maxMergeAtOnce=%d numSegments=%d allowed=%g totalBytes=%,d delPercentage=%g deletesPctAllowed=%g targetNumSegments=%d",
            mergeFactor,
            minSegmentBytes,
            maxMergedSegmentBytes,
            tmp.getSegmentsPerTier(),
            tmp.getMaxMergeAtOnce(),
            numSegments,
            allowedSegCount,
            totalBytes,
            delPercentage,
            tmp.getDeletesPctAllowed(),
            tmp.getTargetSearchConcurrency()),
        numSegments <= allowedSegCount || hasLegalMerges == false);
  }

  @Override
  protected void assertMerge(MergePolicy policy, MergeSpecification merges) {
    TieredMergePolicy tmp = (TieredMergePolicy) policy;
    for (OneMerge merge : merges.merges) {
      assertTrue(merge.segments.size() <= tmp.getMaxMergeAtOnce());
    }
  }

  @Override
  public void testSimulateAppendOnly() throws IOException {
    TieredMergePolicy mergePolicy = mergePolicy();
    // Avoid low values of the max merged segment size which prevent this merge policy from scaling
    // well
    mergePolicy.setMaxMergedSegmentMB(TestUtil.nextInt(random(), 1024, 10 * 1024));
    doTestSimulateAppendOnly(mergePolicy, 100_000_000, 10_000);
  }
  @Override
  public void testSimulateUpdates() throws IOException {
    TieredMergePolicy mergePolicy = mergePolicy();
    // Avoid low values of the max merged segment size which prevent this merge policy from scaling
    // well
    mergePolicy.setMaxMergedSegmentMB(TestUtil.nextInt(random(), 1024, 10 * 1024));
    int numDocs = TEST_NIGHTLY ? atLeast(10_000_000) : atLeast(1_000_000);
    doTestSimulateUpdates(mergePolicy, numDocs, 2500);
  }

  @Override
  public void tearDown() throws Exception {
    BaseFormatSuitesOracle.DRAWS.add(getTestName() + "\tnext\t" + random().nextLong());
    super.tearDown();
  }

  @AfterClass
  public static void recordClassDraw() {
    BaseFormatSuitesOracle.DRAWS.add("#class\tnext\t" + random().nextLong());
  }
}
