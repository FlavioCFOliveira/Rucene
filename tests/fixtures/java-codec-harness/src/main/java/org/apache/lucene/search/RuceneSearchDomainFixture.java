/*
 * Reference values for the Rucene portability test `portability_search_domain`.
 *
 * This class lives in `org.apache.lucene.search` on purpose: `DisjunctionScorer`,
 * `DisjunctionSumScorer` and `DisjunctionMaxScorer` are package-private in Apache
 * Lucene Core 10.5.0, so the only way to drive the real classes -- rather than a
 * transcription of them -- is from inside their own package. Nothing here is part
 * of the Rucene crate; it is test-fixture code that prints reference values on
 * stdout for the Rust side to assert against.
 */
package org.apache.lucene.search;

import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.List;
import org.apache.lucene.document.IntPoint;
import org.apache.lucene.document.LongPoint;
import org.apache.lucene.index.Term;
import org.apache.lucene.search.comparators.DoubleComparator;
import org.apache.lucene.search.comparators.FloatComparator;
import org.apache.lucene.search.comparators.IntComparator;
import org.apache.lucene.search.comparators.LongComparator;
import org.apache.lucene.util.NumericUtils;

public final class RuceneSearchDomainFixture {

  private RuceneSearchDomainFixture() {}

  // -------------------------------------------------------------------------
  // Scorers
  // -------------------------------------------------------------------------

  /** A scorer matching only document 0, whose score is a fixed value. */
  static final class Fixed extends Scorer {
    private final float value;
    private final DocIdSetIterator disi = DocIdSetIterator.all(1);

    Fixed(float value) {
      this.value = value;
    }

    @Override
    public int docID() {
      return disi.docID();
    }

    @Override
    public DocIdSetIterator iterator() {
      return disi;
    }

    @Override
    public float getMaxScore(int upTo) {
      return value;
    }

    @Override
    public float score() {
      return value;
    }
  }

  /** A scorer matching only document 0 through a two-phase iterator. */
  static final class FixedTwoPhase extends Scorer {
    private final float value;
    private final TwoPhaseIterator twoPhase;
    private final DocIdSetIterator disi;

    FixedTwoPhase(float value) {
      this.value = value;
      this.twoPhase =
          new TwoPhaseIterator(DocIdSetIterator.all(1)) {
            @Override
            public boolean matches() {
              return true;
            }

            @Override
            public float matchCost() {
              return 7f;
            }
          };
      this.disi = TwoPhaseIterator.asDocIdSetIterator(twoPhase);
    }

    @Override
    public int docID() {
      return disi.docID();
    }

    @Override
    public DocIdSetIterator iterator() {
      return disi;
    }

    @Override
    public TwoPhaseIterator twoPhaseIterator() {
      return twoPhase;
    }

    @Override
    public float getMaxScore(int upTo) {
      return value;
    }

    @Override
    public float score() {
      return value;
    }
  }

  static List<Scorer> fixed(float[] scores) {
    List<Scorer> subs = new ArrayList<>();
    for (float score : scores) {
      subs.add(new Fixed(score));
    }
    return subs;
  }

  static String bits(float[] values) {
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < values.length; i++) {
      if (i > 0) {
        sb.append(',');
      }
      sb.append(Float.floatToRawIntBits(values[i]));
    }
    return sb.toString();
  }

  /** Drives the real `DisjunctionSumScorer` end to end and prints its score. */
  static void sum(float[] scores) throws Exception {
    DisjunctionSumScorer scorer =
        new DisjunctionSumScorer(fixed(scores), ScoreMode.COMPLETE, Long.MAX_VALUE);
    int doc = scorer.iterator().nextDoc();
    float score = scorer.score();
    System.out.println(
        "sum|" + bits(scores) + "|" + doc + "|" + Float.floatToRawIntBits(score) + "|" + score);
  }

  /** Drives the real `DisjunctionMaxScorer` end to end and prints its score. */
  static void max(float tie, float[] scores) throws Exception {
    DisjunctionMaxScorer scorer =
        new DisjunctionMaxScorer(tie, fixed(scores), ScoreMode.COMPLETE, Long.MAX_VALUE);
    int doc = scorer.iterator().nextDoc();
    float score = scorer.score();
    System.out.println(
        "max|"
            + Float.floatToRawIntBits(tie)
            + "|"
            + bits(scores)
            + "|"
            + doc
            + "|"
            + Float.floatToRawIntBits(score)
            + "|"
            + score);
  }

  /**
   * Prints whether `DisjunctionScorer.twoPhaseIterator()` is null, and its match cost when it is
   * not, for a disjunction whose sub-scorers do or do not support two-phase iteration.
   */
  static void twoPhase(String label, boolean firstTwoPhase, boolean secondTwoPhase)
      throws Exception {
    List<Scorer> subs = new ArrayList<>();
    subs.add(firstTwoPhase ? new FixedTwoPhase(1f) : new Fixed(1f));
    subs.add(secondTwoPhase ? new FixedTwoPhase(2f) : new Fixed(2f));
    DisjunctionSumScorer scorer =
        new DisjunctionSumScorer(subs, ScoreMode.COMPLETE, Long.MAX_VALUE);
    TwoPhaseIterator twoPhase = scorer.twoPhaseIterator();
    System.out.println(
        "twophase|"
            + label
            + "|"
            + (twoPhase == null ? "null" : "present")
            + "|"
            + (twoPhase == null ? "-" : Float.floatToRawIntBits(twoPhase.matchCost())));
  }

  // -------------------------------------------------------------------------
  // Comparators
  // -------------------------------------------------------------------------

  static void decode(String name, FieldComparator<?> comparator, byte[] encoded) throws Exception {
    Method method =
        comparator.getClass().getDeclaredMethod("sortableBytesToLong", byte[].class);
    method.setAccessible(true);
    long decoded = (Long) method.invoke(comparator, (Object) encoded);
    StringBuilder hex = new StringBuilder();
    for (byte b : encoded) {
      hex.append(String.format("%02x", b));
    }
    System.out.println("cmp|" + name + "|" + hex + "|" + decoded);
  }

  static void comparators() throws Exception {
    byte[] four = new byte[4];
    for (int value : new int[] {0, 1, -1, Integer.MIN_VALUE, Integer.MAX_VALUE, -42, 123456789}) {
      NumericUtils.intToSortableBytes(value, four, 0);
      decode("Int", new IntComparator(1, "f", null, false, Pruning.NONE), four.clone());
      decode("Float", new FloatComparator(1, "f", null, false, Pruning.NONE), four.clone());
    }
    byte[] eight = new byte[8];
    for (long value : new long[] {0L, 1L, -1L, Long.MIN_VALUE, Long.MAX_VALUE, -42L, 1234567890123L}) {
      NumericUtils.longToSortableBytes(value, eight, 0);
      decode("Long", new LongComparator(1, "f", null, false, Pruning.NONE), eight.clone());
      decode("Double", new DoubleComparator(1, "f", null, false, Pruning.NONE), eight.clone());
    }
  }

  // -------------------------------------------------------------------------
  // LateInteractionRescorer.combine
  // -------------------------------------------------------------------------

  static void combine(String label, DoubleValuesSourceRescorer rescorer, float firstPass,
      boolean present, double sourceValue) throws Exception {
    Method method = null;
    for (Class<?> c = rescorer.getClass(); c != null; c = c.getSuperclass()) {
      try {
        method = c.getDeclaredMethod("combine", float.class, boolean.class, double.class);
        break;
      } catch (NoSuchMethodException ignored) {
        // keep walking up
      }
    }
    method.setAccessible(true);
    float combined = (Float) method.invoke(rescorer, firstPass, present, sourceValue);
    System.out.println(
        "combine|"
            + label
            + "|"
            + Float.floatToRawIntBits(firstPass)
            + "|"
            + present
            + "|"
            + Double.doubleToRawLongBits(sourceValue)
            + "|"
            + Float.floatToRawIntBits(combined)
            + "|"
            + combined);
  }

  static void combines() throws Exception {
    float[][] vectors = new float[][] {{1f, 0f}};
    DoubleValuesSourceRescorer zero = LateInteractionRescorer.create("f", vectors);
    DoubleValuesSourceRescorer fallback =
        LateInteractionRescorer.withFallbackToFirstPassScore(
            "f", vectors, org.apache.lucene.index.VectorSimilarityFunction.COSINE);

    double[] values = {
      0d, 1d, -1d, 0.1d, Double.MAX_VALUE, -Double.MAX_VALUE, Double.MIN_VALUE, 1e-46d,
      16777217d, 0.30000000000000004d, Double.NaN, Double.POSITIVE_INFINITY
    };
    for (double value : values) {
      combine("zero", zero, 2.5f, true, value);
      combine("fallback", fallback, 2.5f, true, value);
    }
    combine("zero", zero, 2.5f, false, 9d);
    combine("fallback", fallback, 2.5f, false, 9d);
    combine("zero", zero, Float.NaN, false, 9d);
    combine("fallback", fallback, Float.NaN, false, 9d);
  }

  // -------------------------------------------------------------------------
  // equalsTo helpers
  // -------------------------------------------------------------------------

  static void pair(String label, Query a, Query b) {
    System.out.println(
        "eq|" + label + "|" + a.equals(b) + "|" + (a.hashCode() == b.hashCode()));
  }

  static TermQuery tq(String text) {
    return new TermQuery(new Term("f", text));
  }

  static BooleanQuery bq(int minShouldMatch, Object[][] clauses) {
    BooleanQuery.Builder builder = new BooleanQuery.Builder();
    builder.setMinimumNumberShouldMatch(minShouldMatch);
    for (Object[] clause : clauses) {
      builder.add((Query) clause[0], (BooleanClause.Occur) clause[1]);
    }
    return builder.build();
  }

  static void booleanEquals() {
    Object[][] ab = {{tq("a"), BooleanClause.Occur.SHOULD}, {tq("b"), BooleanClause.Occur.SHOULD}};
    Object[][] ba = {{tq("b"), BooleanClause.Occur.SHOULD}, {tq("a"), BooleanClause.Occur.SHOULD}};
    Object[][] aab = {
      {tq("a"), BooleanClause.Occur.SHOULD},
      {tq("a"), BooleanClause.Occur.SHOULD},
      {tq("b"), BooleanClause.Occur.SHOULD}
    };
    Object[][] filterAA = {
      {tq("a"), BooleanClause.Occur.FILTER}, {tq("a"), BooleanClause.Occur.FILTER}
    };
    Object[][] filterA = {{tq("a"), BooleanClause.Occur.FILTER}};
    Object[][] mustNotAA = {
      {tq("a"), BooleanClause.Occur.MUST_NOT}, {tq("a"), BooleanClause.Occur.MUST_NOT}
    };
    Object[][] mustNotA = {{tq("a"), BooleanClause.Occur.MUST_NOT}};
    Object[][] mustAA = {
      {tq("a"), BooleanClause.Occur.MUST}, {tq("a"), BooleanClause.Occur.MUST}
    };
    Object[][] mustA = {{tq("a"), BooleanClause.Occur.MUST}};

    pair("bool.should_order", bq(0, ab), bq(0, ba));
    pair("bool.should_duplicate", bq(0, ab), bq(0, aab));
    pair("bool.min_should_match", bq(0, ab), bq(1, ab));
    pair("bool.filter_duplicate", bq(0, filterAA), bq(0, filterA));
    pair("bool.must_not_duplicate", bq(0, mustNotAA), bq(0, mustNotA));
    pair("bool.must_duplicate", bq(0, mustAA), bq(0, mustA));
    pair("bool.same", bq(2, aab), bq(2, aab));
    pair("bool.occur_differs", bq(0, filterA), bq(0, mustA));
  }

  static void disjunctionMaxEquals() {
    List<Query> ab = List.of(tq("a"), tq("b"));
    List<Query> ba = List.of(tq("b"), tq("a"));
    pair("dismax.same", new DisjunctionMaxQuery(ab, 0.3f), new DisjunctionMaxQuery(ab, 0.3f));
    pair("dismax.order", new DisjunctionMaxQuery(ab, 0.3f), new DisjunctionMaxQuery(ba, 0.3f));
    pair("dismax.tie", new DisjunctionMaxQuery(ab, 0.3f), new DisjunctionMaxQuery(ab, 0.4f));
    pair(
        "dismax.tie_zero_sign",
        new DisjunctionMaxQuery(ab, 0.0f),
        new DisjunctionMaxQuery(ab, -0.0f));
  }

  static void logOddsEquals() {
    float nan1 = Float.intBitsToFloat(0x7fc00000);
    float nan2 = Float.intBitsToFloat(0x7fc00001);
    List<Query> clauses = List.of(tq("a"), tq("b"));

    pair(
        "logodds.same",
        new LogOddsFusionQuery(clauses, 0.5f, new float[] {0.25f, 0.75f}, null, null),
        new LogOddsFusionQuery(clauses, 0.5f, new float[] {0.25f, 0.75f}, null, null));
    pair(
        "logodds.weights_differ",
        new LogOddsFusionQuery(clauses, 0.5f, new float[] {0.25f, 0.75f}, null, null),
        new LogOddsFusionQuery(clauses, 0.5f, new float[] {0.5f, 0.5f}, null, null));
    pair(
        "logodds.alpha_differs",
        new LogOddsFusionQuery(clauses, 0.5f, null, null, null),
        new LogOddsFusionQuery(clauses, 0.25f, null, null, null));
    pair(
        "logodds.nan_same_bits",
        new LogOddsFusionQuery(clauses, 0.5f, null, new float[] {nan1, 1f}, new float[] {2f, 3f}),
        new LogOddsFusionQuery(clauses, 0.5f, null, new float[] {nan1, 1f}, new float[] {2f, 3f}));
    pair(
        "logodds.nan_other_bits",
        new LogOddsFusionQuery(clauses, 0.5f, null, new float[] {nan1, 1f}, new float[] {2f, 3f}),
        new LogOddsFusionQuery(clauses, 0.5f, null, new float[] {nan2, 1f}, new float[] {2f, 3f}));
    pair(
        "logodds.zero_sign_weights",
        new LogOddsFusionQuery(clauses, 0.5f, new float[] {0.0f, 1.0f}, null, null),
        new LogOddsFusionQuery(clauses, 0.5f, new float[] {-0.0f, 1.0f}, null, null));
    pair(
        "logodds.zero_sign_logit",
        new LogOddsFusionQuery(clauses, 0.5f, null, new float[] {0.0f, 1f}, new float[] {2f, 3f}),
        new LogOddsFusionQuery(clauses, 0.5f, null, new float[] {-0.0f, 1f}, new float[] {2f, 3f}));
    pair(
        "logodds.logit_absent_vs_present",
        new LogOddsFusionQuery(clauses, 0.5f, null, null, null),
        new LogOddsFusionQuery(clauses, 0.5f, null, new float[] {0f, 1f}, new float[] {2f, 3f}));
  }

  static void pointRangeEquals() {
    pair(
        "point.same",
        IntPoint.newRangeQuery("f", new int[] {1, 2}, new int[] {3, 4}),
        IntPoint.newRangeQuery("f", new int[] {1, 2}, new int[] {3, 4}));
    pair(
        "point.bounds_differ",
        IntPoint.newRangeQuery("f", new int[] {1, 2}, new int[] {3, 4}),
        IntPoint.newRangeQuery("f", new int[] {1, 2}, new int[] {3, 5}));
    pair(
        "point.field_differs",
        IntPoint.newRangeQuery("f", new int[] {1}, new int[] {3}),
        IntPoint.newRangeQuery("g", new int[] {1}, new int[] {3}));
    pair(
        "point.dims_differ",
        IntPoint.newRangeQuery("f", new int[] {1}, new int[] {3}),
        IntPoint.newRangeQuery("f", new int[] {1, 1}, new int[] {3, 3}));
    // Same packed bytes and same field, different concrete subclass: Java's
    // `sameClassAs` separates them before `equalsTo` ever runs.
    pair(
        "point.class_differs",
        IntPoint.newRangeQuery("f", new int[] {1}, new int[] {3}),
        LongPoint.newRangeQuery("f", new long[] {1}, new long[] {3}));
  }

  // -------------------------------------------------------------------------
  // Sort.getSort / SortField.getType
  // -------------------------------------------------------------------------

  static void sorts() {
    Sort sort =
        new Sort(
            new SortField("a", SortField.Type.STRING, true),
            new SortField("b", SortField.Type.INT),
            new SortField(null, SortField.Type.SCORE),
            new SortField(null, SortField.Type.DOC),
            new SortField("c", SortField.Type.LONG),
            new SortField("d", SortField.Type.FLOAT),
            new SortField("e", SortField.Type.DOUBLE),
            new SortField("g", SortField.Type.STRING_VAL));
    System.out.println("sort|length|" + sort.getSort().length);
    for (SortField field : sort.getSort()) {
      System.out.println(
          "sortfield|" + field.getField() + "|" + field.getType() + "|" + field.getReverse());
    }
  }

  public static void main(String[] args) throws Exception {
    sum(new float[] {1f, 2f});
    sum(new float[] {0.1f, 0.2f, 0.3f});
    sum(new float[] {16777216f, 1f, 1f});
    sum(new float[] {Float.MAX_VALUE, Float.MAX_VALUE});
    sum(new float[] {0.1f, 0.1f, 0.1f, 0.1f, 0.1f, 0.1f, 0.1f, 0.1f, 0.1f, 0.1f});
    sum(new float[] {1e20f, 1f, -1e20f});

    max(0f, new float[] {1f, 2f, 3f});
    max(1f, new float[] {1f, 2f, 3f});
    max(0.5f, new float[] {1f, 2f, 3f});
    max(0.3f, new float[] {3f, 2f, 1f});
    max(0.1f, new float[] {0.1f, 0.2f, 0.30000001f});
    max(1f, new float[] {Float.MAX_VALUE, Float.MAX_VALUE});
    max(0.7f, new float[] {2f, 2f, 2f});
    max(0.5f, new float[] {16777216f, 1f, 1f});

    twoPhase("none", false, false);
    twoPhase("first", true, false);
    twoPhase("both", true, true);

    comparators();
    combines();
    booleanEquals();
    disjunctionMaxEquals();
    logOddsEquals();
    pointRangeEquals();
    sorts();

    System.out.println("fixture_ok=true");
  }
}
