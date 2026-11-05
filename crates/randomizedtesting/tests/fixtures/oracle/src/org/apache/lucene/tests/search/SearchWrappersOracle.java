// Oracle of the search wrappers of org.apache.lucene.tests.search (rmp #345).
package org.apache.lucene.tests.search;

import java.io.BufferedReader;
import java.io.File;
import java.io.FileInputStream;
import java.io.InputStreamReader;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;
import java.util.Random;
import org.apache.lucene.document.IntPoint;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.Term;
import org.apache.lucene.search.BooleanClause.Occur;
import org.apache.lucene.search.BooleanQuery;
import org.apache.lucene.search.BoostQuery;
import org.apache.lucene.search.BulkScorer;
import org.apache.lucene.search.ConstantScoreQuery;
import org.apache.lucene.search.DisjunctionMaxQuery;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.search.Explanation;
import org.apache.lucene.search.FieldExistsQuery;
import org.apache.lucene.search.FilterWeight;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.KnnFloatVectorQuery;
import org.apache.lucene.search.LeafCollector;
import org.apache.lucene.search.MatchAllDocsQuery;
import org.apache.lucene.search.MatchNoDocsQuery;
import org.apache.lucene.search.Matches;
import org.apache.lucene.search.MatchesIterator;
import org.apache.lucene.search.PhraseQuery;
import org.apache.lucene.search.PrefixQuery;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.Scorable;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.ScoreMode;
import org.apache.lucene.search.Scorer;
import org.apache.lucene.search.ScorerSupplier;
import org.apache.lucene.search.TermQuery;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.search.TwoPhaseIterator;
import org.apache.lucene.search.Weight;
import org.apache.lucene.search.WildcardQuery;
import org.apache.lucene.search.similarities.BM25Similarity;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.store.IndexOutput;
import org.apache.lucene.tests.search.similarities.AssertingSimilarity;
import org.apache.lucene.tests.search.similarities.RandomSimilarity;
import org.junit.runner.JUnitCore;
import org.junit.runner.Result;
import org.junit.runner.notification.Failure;

/**
 * Writes {@code rucene-test-framework/search_wrappers.tsv}: per corpus case ({@code nocfs},
 * {@code cfs}: the default-codec corpus of {@code mdw_corpus.tsv}, which {@code
 * MockDirectoryWrapperOracle} writes into the output directory first), scenario and step,
 * {@code ok<TAB>value} or {@code err<TAB>class<TAB>message}. The scenarios run inside {@link
 * TestSearchWrappersScenarios#testScenarios} under {@code RandomizedRunner} ({@code tests.seed}
 * {@link #SEED}), because {@code AssertingLeafCollector} draws {@code rarely()} from the test
 * context. They record: the top hits (doc, score bits), total hits, count and top explanation of a
 * fixed set of core queries (and the test-framework query wrappers) through {@code
 * AssertingIndexSearcher} (three seeds, each ending with the searcher's next {@code nextLong}),
 * through {@code ScorerIndexSearcher}, and through a searcher using {@code
 * AssertingSimilarity(RandomSimilarity)}; the misuse of each asserting class with Java's message;
 * and {@code RandomSimilarity}'s pick per field, its {@code toString()} and next draw over a seed
 * grid (outside the runner: {@code RandomSimilarity} draws only from its own {@code Random}).
 */
public final class SearchWrappersOracle {
  private SearchWrappersOracle() {}

  static final String SEED = "5EA4C4E5";

  static final List<String> ROWS = new ArrayList<>();
  static File corpus;

  static String m(Object o) {
    String s = String.valueOf(o);
    return s.replaceAll("@[0-9a-f]+", "@HASH").replace("\n", "\\n").replace("\t", "\\t");
  }

  static String err(Throwable t) {
    return "err\t"
        + t.getClass().getName()
        + "\t"
        + (t.getMessage() == null ? "<null>" : m(t.getMessage()));
  }

  interface Step {
    Object run() throws Throwable;
  }

  static void rec(String c, String scenario, String step, Step s) {
    String value;
    try {
      value = "ok\t" + m(s.run());
    } catch (Throwable t) {
      value = err(t);
    }
    ROWS.add(c + "\t" + scenario + "\t" + step + "\t" + value);
  }

  static Directory loadCorpus(File tsv, String c) throws Exception {
    Directory dir = new ByteBuffersDirectory();
    try (BufferedReader r =
        new BufferedReader(
            new InputStreamReader(new FileInputStream(tsv), StandardCharsets.UTF_8))) {
      for (String line; (line = r.readLine()) != null; ) {
        if (line.startsWith("#") || line.isEmpty()) {
          continue;
        }
        String[] f = line.split("\t", -1);
        if (!f[0].equals(c)) {
          continue;
        }
        byte[] b = new byte[f[2].length() / 2];
        for (int i = 0; i < b.length; i++) {
          b[i] = (byte) Integer.parseInt(f[2].substring(2 * i, 2 * i + 2), 16);
        }
        try (IndexOutput out = dir.createOutput(f[1], IOContext.DEFAULT)) {
          out.writeBytes(b, b.length);
        }
      }
    }
    return dir;
  }

  static Query term(String text) {
    return new TermQuery(new Term("body", text));
  }

  static Query bool(Object... clauses) {
    BooleanQuery.Builder b = new BooleanQuery.Builder();
    for (int i = 0; i < clauses.length; i += 2) {
      b.add((Query) clauses[i], (Occur) clauses[i + 1]);
    }
    return b.build();
  }

  /** The queries of the search scenarios, by name; {@code random} feeds the random wrappers. */
  static List<Object[]> queries(Random random) throws Exception {
    List<Object[]> q = new ArrayList<>();
    q.add(new Object[] {"term", term("w7")});
    q.add(new Object[] {"should", bool(term("w1"), Occur.SHOULD, term("w2"), Occur.SHOULD)});
    q.add(new Object[] {"must_should", bool(term("w3"), Occur.MUST, term("w4"), Occur.SHOULD)});
    q.add(new Object[] {"must_filter", bool(term("w5"), Occur.MUST, term("w12"), Occur.FILTER)});
    q.add(new Object[] {"should_not", bool(term("w8"), Occur.SHOULD, term("w9"), Occur.MUST_NOT)});
    q.add(new Object[] {"phrase", new PhraseQuery("body", "w0", "w17")});
    q.add(new Object[] {"prefix", new PrefixQuery(new Term("body", "w1"))});
    q.add(new Object[] {"wildcard", new WildcardQuery(new Term("body", "w?3"))});
    q.add(new Object[] {"point_range", IntPoint.newRangeQuery("p_int", 10, 50)});
    q.add(new Object[] {"exists", new FieldExistsQuery("dv_num")});
    q.add(new Object[] {"constant", new ConstantScoreQuery(term("w7"))});
    q.add(new Object[] {"boost", new BoostQuery(term("w7"), 2.5f)});
    q.add(
        new Object[] {
          "dismax", new DisjunctionMaxQuery(List.of(term("w1"), term("w2")), 0.1f)
        });
    q.add(new Object[] {"all", new MatchAllDocsQuery()});
    q.add(new Object[] {"none", new MatchNoDocsQuery("none")});
    q.add(
        new Object[] {"knn", new KnnFloatVectorQuery("vf", new float[] {1f, -0.5f, 1.5f, 1f}, 5)});
    q.add(new Object[] {"asserting", new AssertingQuery(new Random(random.nextLong()), term("w7"))});
    q.add(
        new Object[] {
          "random_approx",
          new RandomApproximationQuery(
              bool(term("w1"), Occur.SHOULD, term("w2"), Occur.SHOULD),
              new Random(random.nextLong()))
        });
    q.add(
        new Object[] {
          "block_score",
          new BlockScoreQueryWrapper(bool(term("w1"), Occur.SHOULD, term("w2"), Occur.SHOULD), 2)
        });
    q.add(
        new Object[] {
          "disabling_bulk",
          new DisablingBulkScorerQuery(bool(term("w1"), Occur.SHOULD, term("w2"), Occur.SHOULD))
        });
    return q;
  }

  static String hits(TopDocs td) {
    StringBuilder sb = new StringBuilder().append(td.totalHits);
    for (ScoreDoc sd : td.scoreDocs) {
      sb.append(' ').append(sd.doc).append(':').append(Integer.toHexString(Float.floatToIntBits(sd.score)));
    }
    return sb.toString();
  }

  static String expl(Explanation e) {
    return e.isMatch()
        + " "
        + Integer.toHexString(Float.floatToIntBits(e.getValue().floatValue()))
        + " "
        + e.getDescription();
  }

  static void searches(String c, String scenario, IndexSearcher s, Random random) throws Exception {
    s.setQueryCache(null);
    for (Object[] nq : queries(random)) {
      String name = (String) nq[0];
      Query q = (Query) nq[1];
      int[] top = {-1};
      rec(
          c,
          scenario,
          name + "/top",
          () -> {
            TopDocs td = s.search(q, 10);
            if (td.scoreDocs.length > 0) top[0] = td.scoreDocs[0].doc;
            return hits(td);
          });
      rec(c, scenario, name + "/count", () -> s.count(q));
      if (top[0] >= 0) {
        rec(c, scenario, name + "/explain", () -> expl(s.explain(q, top[0])));
      }
    }
  }

  /** The body of {@link TestSearchWrappersScenarios#testScenarios}. */
  static void scenarios() throws Exception {
    for (String c : new String[] {"nocfs", "cfs"}) {
      try (DirectoryReader reader = DirectoryReader.open(loadCorpus(corpus, c))) {
        for (long seed : new long[] {1, 2, 3}) {
          Random r = new Random(seed);
          AssertingIndexSearcher s = new AssertingIndexSearcher(r, reader);
          searches(c, "asserting" + seed, s, new Random(seed + 100));
          rec(c, "asserting" + seed, "next", () -> s.random.nextLong());
          rec(c, "asserting" + seed, "toString", () -> s.toString().startsWith("AssertingIndexSearcher(IndexSearcher("));
        }
        searches(c, "scorer", new ScorerIndexSearcher(reader), new Random(7));
        AssertingIndexSearcher sim = new AssertingIndexSearcher(new Random(11), reader);
        RandomSimilarity rs = new RandomSimilarity(new Random(12));
        sim.setSimilarity(new AssertingSimilarity(rs));
        searches(c, "random_sim", sim, new Random(13));
        rec(c, "random_sim", "toString", () -> rs.toString());
        rec(c, "random_sim", "next", () -> sim.random.nextLong());
        misuse(c, reader);
      }
    }
  }

  static Scorer termScorer(IndexSearcher s, LeafReaderContext ctx, String text) throws Exception {
    Weight w = s.createWeight(s.rewrite(term(text)), ScoreMode.COMPLETE, 1);
    return w.scorer(ctx);
  }

  /** {@code Weight.DefaultBulkScorer} over {@code scorer}: the default {@code bulkScorer()}. */
  static BulkScorer defaultBulk(Scorer scorer) throws Exception {
    return new ScorerSupplier() {
      @Override
      public Scorer get(long leadCost) {
        return scorer;
      }

      @Override
      public long cost() {
        return scorer.iterator().cost();
      }
    }.bulkScorer();
  }

  static final LeafCollector NOOP =
      new LeafCollector() {
        @Override
        public void setScorer(Scorable scorer) {}

        @Override
        public void collect(int doc) {}
      };

  static void misuse(String c, DirectoryReader reader) throws Exception {
    IndexSearcher plain = new IndexSearcher(reader);
    plain.setQueryCache(null);
    LeafReaderContext ctx = reader.leaves().get(0);
    int maxDoc = ctx.reader().maxDoc();

    // AssertingScorer
    Scorer sc1 = AssertingScorer.wrap(termScorer(plain, ctx, "w7"), true, false);
    rec(c, "scorer", "score_unpositioned", () -> sc1.score());
    rec(c, "scorer", "max_score_unpositioned", () -> sc1.getMaxScore(10));
    rec(c, "scorer", "shallow_decreasing", () -> sc1.advanceShallow(-5));
    rec(c, "scorer", "next", () -> sc1.iterator().nextDoc());
    rec(c, "scorer", "score", () -> Integer.toHexString(Float.floatToIntBits(sc1.score())));
    rec(c, "scorer", "min_competitive", () -> {
      sc1.setMinCompetitiveScore(1f);
      return null;
    });
    rec(c, "scorer", "advance_same", () -> sc1.iterator().advance(sc1.docID()));
    rec(c, "scorer", "children", () -> sc1.getChildren().size() + " " + sc1.getChildren().iterator().next().relationship());
    rec(c, "scorer", "exhaust", () -> sc1.iterator().advance(DocIdSetIterator.NO_MORE_DOCS));
    rec(c, "scorer", "next_after_end", () -> sc1.iterator().nextDoc());
    rec(c, "scorer", "advance_after_end", () -> sc1.iterator().advance(5));
    Scorer sc2 = AssertingScorer.wrap(termScorer(plain, ctx, "w7"), false, false);
    rec(c, "scorer", "cannot_score", () -> {
      sc2.iterator().nextDoc();
      return sc2.score();
    });
    rec(c, "scorer", "cannot_shallow", () -> sc2.advanceShallow(100));
    Scorer sc3 = AssertingScorer.wrap(termScorer(plain, ctx, "w7"), true, true);
    rec(c, "scorer", "shallow", () -> sc3.advanceShallow(3));
    rec(c, "scorer", "shallow_below_doc", () -> {
      sc3.iterator().advance(40);
      return sc3.advanceShallow(sc3.docID() - 1);
    });
    rec(c, "scorer", "min_competitive_nan", () -> {
      sc3.setMinCompetitiveScore(Float.NaN);
      return null;
    });
    rec(c, "scorer", "min_competitive_decreasing", () -> {
      sc3.setMinCompetitiveScore(0.5f);
      sc3.setMinCompetitiveScore(0.25f);
      return null;
    });
    // AssertingScorer over a two-phase scorer
    Weight phraseWeight =
        plain.createWeight(plain.rewrite(new PhraseQuery("body", "w0", "w17")), ScoreMode.COMPLETE, 1);
    Scorer sc4 = AssertingScorer.wrap(phraseWeight.scorer(ctx), true, false);
    TwoPhaseIterator tpi = sc4.twoPhaseIterator();
    rec(c, "scorer", "two_phase", () -> tpi != null);
    rec(c, "scorer", "matches_unpositioned", () -> tpi.matches());
    rec(c, "scorer", "approx_next", () -> tpi.approximation().nextDoc());
    rec(c, "scorer", "approx_matches", () -> tpi.matches());
    rec(c, "scorer", "approx_backwards", () -> tpi.approximation().advance(0));
    rec(c, "scorer", "approx_end", () -> tpi.approximation().advance(DocIdSetIterator.NO_MORE_DOCS));
    rec(c, "scorer", "approx_after_end", () -> tpi.approximation().nextDoc());

    // AssertingCollector / AssertingLeafCollector
    AssertingCollector ac = AssertingCollector.wrap(new DummyTotalHitCountCollector());
    rec(c, "collector", "leaf_before_weight", () -> ac.getLeafCollector(ctx) != null);
    Weight allWeight = plain.createWeight(new MatchAllDocsQuery(), ScoreMode.COMPLETE_NO_SCORES, 1);
    rec(c, "collector", "weight", () -> {
      ac.setWeight(allWeight);
      return null;
    });
    rec(c, "collector", "weight_twice", () -> {
      ac.setWeight(allWeight);
      return null;
    });
    LeafCollector lc = ac.getLeafCollector(reader.leaves().get(reader.leaves().size() - 1));
    rec(c, "collector", "collect5", () -> {
      lc.collect(5);
      return null;
    });
    rec(c, "collector", "collect3", () -> {
      lc.collect(3);
      return null;
    });
    rec(c, "collector", "finish", () -> {
      lc.finish();
      return ac.hasFinishedCollectingPreviousLeaf;
    });
    rec(c, "collector", "finish_twice", () -> {
      lc.finish();
      return null;
    });
    rec(c, "collector", "leaf_out_of_order", () -> ac.getLeafCollector(ctx) != null);
    AssertingLeafCollector l1 = new AssertingLeafCollector(NOOP, 0, 10);
    rec(c, "leaf", "out_of_range_max", () -> {
      l1.collect(10);
      return null;
    });
    AssertingLeafCollector l2 = new AssertingLeafCollector(NOOP, 2, 10);
    rec(c, "leaf", "out_of_range_min", () -> {
      l2.collect(1);
      return null;
    });
    AssertingLeafCollector l3 = new AssertingLeafCollector(NOOP, 0, 10);
    rec(c, "leaf", "out_of_order", () -> {
      l3.collect(4);
      l3.collect(4);
      return null;
    });
    AssertingLeafCollector l4 = new AssertingLeafCollector(NOOP, 0, 10);
    rec(c, "leaf", "range_empty", () -> {
      l4.collectRange(5, 5);
      return null;
    });
    AssertingLeafCollector l5 = new AssertingLeafCollector(NOOP, 0, 10);
    rec(c, "leaf", "range_beyond", () -> {
      l5.collectRange(3, 12);
      return null;
    });
    AssertingLeafCollector l6 = new AssertingLeafCollector(NOOP, 0, 10);
    rec(c, "leaf", "range_then_collect", () -> {
      l6.collectRange(3, 6);
      l6.collect(5);
      return null;
    });
    AssertingLeafCollector l7 = new AssertingLeafCollector(NOOP, 0, 10);
    rec(c, "leaf", "finish_twice", () -> {
      l7.finish();
      l7.finish();
      return null;
    });

    // AssertingBulkScorer
    BulkScorer bs =
        AssertingBulkScorer.wrap(
            new Random(21), defaultBulk(termScorer(plain, ctx, "w7")), maxDoc);
    rec(c, "bulk", "rewrap", () -> AssertingBulkScorer.wrap(new Random(1), bs, maxDoc) == bs);
    int[] collected = {0};
    LeafCollector counting =
        new LeafCollector() {
          @Override
          public void setScorer(Scorable scorer) {}

          @Override
          public void collect(int doc) {
            collected[0]++;
          }
        };
    rec(c, "bulk", "score_first", () -> bs.score(counting, null, 0, 100) + " " + collected[0]);
    rec(c, "bulk", "score_rest", () -> bs.score(counting, null, 100, DocIdSetIterator.NO_MORE_DOCS) + " " + collected[0]);
    BulkScorer bs2 =
        AssertingBulkScorer.wrap(
            new Random(22), defaultBulk(termScorer(plain, ctx, "w7")), maxDoc);
    rec(c, "bulk", "window", () -> bs2.score(NOOP, null, 10, 20));
    rec(c, "bulk", "backward", () -> bs2.score(NOOP, null, 5, 30));
    rec(c, "bulk", "inverted", () -> bs2.score(NOOP, null, 40, 35));

    // AssertingWeight / AssertingMatches
    AssertingIndexSearcher as = new AssertingIndexSearcher(new Random(31), reader);
    as.setQueryCache(null);
    Weight mw = as.createWeight(as.rewrite(term("w7")), ScoreMode.COMPLETE, 1);
    int doc = termScorer(plain, ctx, "w7").iterator().nextDoc();
    Matches matches = mw.matches(ctx, doc);
    rec(c, "matches", "class", () -> matches.getClass().getSimpleName() + " " + (AssertingMatches.unWrap(matches) != matches));
    MatchesIterator mi = matches.getMatches("body");
    rec(c, "matches", "start_unpositioned", () -> mi.startPosition());
    rec(c, "matches", "offset_unpositioned", () -> mi.startOffset());
    rec(c, "matches", "next", () -> mi.next());
    rec(c, "matches", "position", () -> mi.startPosition() + "-" + mi.endPosition());
    rec(c, "matches", "drain", () -> {
      int n = 0;
      while (mi.next()) n++;
      return n;
    });
    rec(c, "matches", "next_exhausted", () -> mi.next());
    rec(c, "matches", "query_exhausted", () -> mi.getQuery());
    rec(c, "matches", "other_field", () -> matches.getMatches("id"));
    Weight countWeight =
        new AssertingWeight(
            new Random(32),
            new FilterWeight(allWeight) {
              @Override
              public int count(LeafReaderContext context) {
                return context.reader().numDocs() + 1;
              }
            },
            ScoreMode.COMPLETE_NO_SCORES);
    rec(c, "weight", "count_too_large", () -> countWeight.count(ctx));
    rec(c, "weight", "count", () -> mw.count(ctx));
    rec(c, "weight", "supplier_get_twice", () -> {
      var supplier = mw.scorerSupplier(ctx);
      supplier.get(1);
      return supplier.get(1);
    });
    rec(c, "weight", "supplier_negative_lead", () -> mw.scorerSupplier(ctx).get(-1));
    rec(c, "weight", "supplier_top_level_after_get", () -> {
      var supplier = mw.scorerSupplier(ctx);
      supplier.get(1);
      supplier.setTopLevelScoringClause();
      return null;
    });
    rec(c, "query", "negative_boost", () -> new AssertingQuery(new Random(33), term("w7")).createWeight(plain, ScoreMode.COMPLETE, -1f));
    rec(c, "query", "wrap_twice", () -> {
      Query aq = AssertingQuery.wrap(new Random(34), term("w7"));
      return AssertingQuery.wrap(new Random(35), aq) == aq;
    });
    rec(c, "searcher", "search_plain_weight", () -> {
      as.search(new LeafReaderContextPartitionsHolder(reader).partitions, allWeight, new DummyTotalHitCountCollector());
      return null;
    });

    // RandomTwoPhaseView
    RandomApproximationQuery.RandomTwoPhaseView view =
        new RandomApproximationQuery.RandomTwoPhaseView(new Random(41), DocIdSetIterator.all(20));
    rec(c, "two_phase_view", "match_cost", () -> Integer.toHexString(Float.floatToIntBits(view.matchCost())));
    rec(c, "two_phase_view", "matches_unpositioned", () -> view.matches());
    rec(c, "two_phase_view", "next", () -> view.approximation().nextDoc());
    rec(c, "two_phase_view", "matches", () -> view.matches());
    rec(c, "two_phase_view", "matches_twice", () -> view.matches());
    rec(c, "two_phase_view", "advance", () -> view.approximation().advance(7) + " " + view.matches());
    rec(c, "two_phase_view", "run_end", () -> view.docIDRunEnd());

    // The small collectors and wrappers
    rec(c, "small", "fixed_bit_set", () -> {
      var bits = plain.search(term("w7"), FixedBitSetCollector.createManager(reader.maxDoc()));
      StringBuilder sb = new StringBuilder().append(bits.cardinality());
      for (int i = bits.nextSetBit(0); i != DocIdSetIterator.NO_MORE_DOCS; i = i + 1 >= bits.length() ? DocIdSetIterator.NO_MORE_DOCS : bits.nextSetBit(i + 1)) {
        sb.append(' ').append(i);
      }
      return sb;
    });
    rec(c, "small", "dummy_total", () -> plain.search(term("w7"), DummyTotalHitCountCollector.createManager()));
    rec(c, "small", "disabling_no_match", () -> plain.search(new DisablingBulkScorerQuery(term("zzz")), 10).totalHits);
    rec(c, "small", "bulk_wrapper_iterate", () -> {
      Weight w = plain.createWeight(plain.rewrite(should()), ScoreMode.COMPLETE, 1);
      BulkScorerWrapperScorer bw = new BulkScorerWrapperScorer(w.bulkScorer(ctx), 3);
      StringBuilder sb = new StringBuilder().append(bw.iterator().cost());
      for (int d = bw.iterator().nextDoc(); d != DocIdSetIterator.NO_MORE_DOCS; d = bw.iterator().nextDoc()) {
        sb.append(' ').append(d).append(':').append(Integer.toHexString(Float.floatToIntBits(bw.score())));
      }
      return sb;
    });
    rec(c, "small", "bulk_wrapper_advance", () -> {
      Weight w = plain.createWeight(plain.rewrite(should()), ScoreMode.COMPLETE, 1);
      BulkScorerWrapperScorer bw = new BulkScorerWrapperScorer(w.bulkScorer(ctx), 3);
      int a = bw.iterator().advance(50);
      int b = bw.iterator().nextDoc();
      int e = bw.iterator().advance(DocIdSetIterator.NO_MORE_DOCS - 1);
      return a + " " + b + " " + e + " " + bw.docID();
    });
  }

  static Query should() {
    return bool(term("w1"), Occur.SHOULD, term("w2"), Occur.SHOULD);
  }

  /** The partitions of every leaf, as the protected {@code search} receives them. */
  static final class LeafReaderContextPartitionsHolder {
    final IndexSearcher.LeafReaderContextPartition[] partitions;

    LeafReaderContextPartitionsHolder(DirectoryReader reader) {
      partitions = new IndexSearcher.LeafReaderContextPartition[reader.leaves().size()];
      for (int i = 0; i < partitions.length; i++) {
        partitions[i] =
            IndexSearcher.LeafReaderContextPartition.createForEntireSegment(reader.leaves().get(i));
      }
    }
  }

  static void similarities() {
    String[] fields = {"id", "body", "tv", "p_int", "dv_num", "f0", "f1", "f2", "f3", "f4", "f5"};
    for (long seed : new long[] {0, 1, 2, 3, 42, -7, 123456789L}) {
      Random r = new Random(seed);
      RandomSimilarity rs = new RandomSimilarity(r);
      for (String f : fields) {
        rec("sim", "random" + seed, f, () -> rs.get(f).toString());
      }
      rec("sim", "random" + seed, "toString", () -> rs.toString());
      rec("sim", "random" + seed, "next", () -> r.nextLong());
    }
    rec("sim", "asserting", "toString", () -> new AssertingSimilarity(new BM25Similarity()).toString());
  }

  public static void main(String[] args) throws Exception {
    File out = new File(args[0], "rucene-test-framework");
    out.mkdirs();
    corpus = new File(out, "mdw_corpus.tsv");
    ROWS.clear();
    System.setProperty("tests.seed", SEED);
    Result r = new JUnitCore().run(TestSearchWrappersScenarios.class);
    System.clearProperty("tests.seed");
    similarities();
    try (PrintWriter w = new PrintWriter(new File(out, "search_wrappers.tsv"), "UTF-8")) {
      w.println("# case\tscenario\tstep\toutcome (ok\\tvalue | err\\tclass\\tmessage)");
      for (Failure f : r.getFailures()) {
        w.println("# failure " + f.toString().replace("\n", "\\n"));
      }
      for (String row : ROWS) {
        w.println(row);
      }
    }
  }
}
