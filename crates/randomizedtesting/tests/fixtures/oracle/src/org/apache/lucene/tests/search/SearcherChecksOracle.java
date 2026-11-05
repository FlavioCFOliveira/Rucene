// Oracle rows of LuceneTestCase.newSearcher/wrapReader, QueryUtils, CheckHits and the Scorer
// branch of AssertingLeafCollector (rmp #345, block 3). Appended to search_wrappers.tsv after the
// rows of SearchWrappersOracle (after its "sim" rows).
package org.apache.lucene.tests.search;

import static org.apache.lucene.tests.search.SearchWrappersOracle.bool;
import static org.apache.lucene.tests.search.SearchWrappersOracle.rec;
import static org.apache.lucene.tests.search.SearchWrappersOracle.term;

import java.io.IOException;
import java.lang.reflect.Field;
import java.util.List;
import java.util.Random;
import java.util.concurrent.ExecutorService;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexReader;
import org.apache.lucene.index.LeafReader;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.search.BooleanClause.Occur;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.search.DocIdStream;
import org.apache.lucene.search.Explanation;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.LeafCollector;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.QueryCache;
import org.apache.lucene.search.Scorable;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.ScoreMode;
import org.apache.lucene.search.Scorer;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.search.Weight;
import org.apache.lucene.search.similarities.BM25Similarity;
import org.apache.lucene.tests.util.LuceneTestCase;

/**
 * The scenarios run without the class's shared executor (set to {@code null} while they run) and
 * without a default query cache, so that every draw happens on the test thread in a fixed order;
 * every row group ends with the context's next {@code nextLong()}.
 */
final class SearcherChecksOracle {
  private SearcherChecksOracle() {}

  static final String C = "nocfs";

  static String searcherShape(IndexSearcher s, IndexReader r) {
    StringBuilder sb = new StringBuilder();
    sb.append(s.toString().startsWith("AssertingIndexSearcher(") ? "asserting" : "plain");
    sb.append(s.getIndexReader() == r ? " same" : " wrapped");
    sb.append(" leaves=").append(s.getIndexReader().leaves().size());
    sb.append(" maxDoc=").append(s.getIndexReader().maxDoc());
    sb.append(" numDocs=").append(s.getIndexReader().numDocs());
    sb.append(" timeout=").append(s.getTimeout() != null);
    sb.append(" maybeCache=").append(s.getQueryCachingPolicy() == LuceneTestCase.MAYBE_CACHE_POLICY);
    sb.append(" slices=");
    for (IndexSearcher.LeafSlice slice : s.getSlices()) {
      sb.append('[');
      for (IndexSearcher.LeafReaderContextPartition p : slice.partitions) {
        sb.append(p.ctx.ord).append(':').append(p.minDocId).append('-').append(p.maxDocId).append(' ');
      }
      sb.append(']');
    }
    return sb.toString();
  }

  static String readerShape(IndexReader r) throws IOException {
    String kind =
        r instanceof LeafReader ? "leaf" : r instanceof DirectoryReader ? "directory" : "composite";
    return kind
        + " leaves="
        + r.leaves().size()
        + " maxDoc="
        + r.maxDoc()
        + " numDocs="
        + r.numDocs()
        + " w7="
        + new IndexSearcher(r).count(term("w7"));
  }

  static String next() {
    return Long.toString(LuceneTestCase.random().nextLong());
  }

  /** The rows of {@link #run}, which {@code SearchWrappersOracle.main} writes last. */
  static final java.util.ArrayList<String> ROWS = new java.util.ArrayList<>();

  static void run(String corpusCase) throws Exception {
    int from = SearchWrappersOracle.ROWS.size();
    try {
      runScenarios();
    } finally {
      List<String> ours = SearchWrappersOracle.ROWS.subList(from, SearchWrappersOracle.ROWS.size());
      ROWS.addAll(ours);
      ours.clear();
    }
  }

  static void runScenarios() throws Exception {
    Field executorField = LuceneTestCase.class.getDeclaredField("executor");
    executorField.setAccessible(true);
    ExecutorService executor = (ExecutorService) executorField.get(null);
    executorField.set(null, null);
    QueryCache defaultCache = IndexSearcher.getDefaultQueryCache();
    IndexSearcher.setDefaultQueryCache(null);
    try (DirectoryReader reader =
        DirectoryReader.open(SearchWrappersOracle.loadCorpus(SearchWrappersOracle.corpus, C))) {
      newSearcherGrid(reader);
      wrapReaderGrid(reader);
      queryUtilsChecks(reader);
      checkHits(reader);
      scorerBranch(reader);
    } finally {
      IndexSearcher.setDefaultQueryCache(defaultCache);
      executorField.set(null, executor);
    }
  }

  static void newSearcherGrid(DirectoryReader reader) {
    for (int i = 0; i < 12; i++) {
      rec(C, "new_searcher", "plain" + i, () -> searcherShape(LuceneTestCase.newSearcher(reader), reader));
    }
    for (int i = 0; i < 4; i++) {
      rec(C, "new_searcher", "no_wrap" + i, () -> searcherShape(LuceneTestCase.newSearcher(reader, false), reader));
      rec(C, "new_searcher", "no_assert" + i, () -> searcherShape(LuceneTestCase.newSearcher(reader, true, false), reader));
      rec(C, "new_searcher", "inter" + i, () -> searcherShape(
          LuceneTestCase.newSearcher(reader, true, true, LuceneTestCase.Concurrency.INTER_SEGMENT), reader));
      rec(C, "new_searcher", "intra" + i, () -> searcherShape(
          LuceneTestCase.newSearcher(reader, false, false, LuceneTestCase.Concurrency.INTRA_SEGMENT), reader));
      rec(C, "new_searcher", "none" + i, () -> searcherShape(
          LuceneTestCase.newSearcher(reader, true, true, LuceneTestCase.Concurrency.NONE), reader));
    }
    rec(C, "new_searcher", "next", SearcherChecksOracle::next);
  }

  static void wrapReaderGrid(DirectoryReader reader) {
    for (int i = 0; i < 10; i++) {
      rec(C, "wrap_reader", "directory" + i, () -> readerShape(LuceneTestCase.wrapReader(reader)));
      rec(C, "wrap_reader", "leaf" + i, () -> readerShape(LuceneTestCase.wrapReader(reader.leaves().get(0).reader())));
    }
    for (int i = 0; i < 20; i++) {
      rec(C, "wrap_reader", "maybe" + i, () -> readerShape(LuceneTestCase.maybeWrapReader(reader)));
    }
    rec(C, "wrap_reader", "next", SearcherChecksOracle::next);
  }

  static IndexSearcher fixedSearcher(DirectoryReader reader, long seed) {
    AssertingIndexSearcher s = new AssertingIndexSearcher(new Random(seed), reader);
    s.setSimilarity(new BM25Similarity());
    return s;
  }

  static void queryUtilsChecks(DirectoryReader reader) throws Exception {
    IndexSearcher s = fixedSearcher(reader, 21);
    for (Object[] nq : SearchWrappersOracle.queries(new Random(22))) {
      String name = (String) nq[0];
      Query q = (Query) nq[1];
      rec(C, "qu_check", name, () -> {
        QueryUtils.check(new Random(23), q, s);
        return "pass";
      });
      rec(C, "qu_check", name + "/next", SearcherChecksOracle::next);
    }
    rec(C, "qu_check", "null_searcher", () -> {
      QueryUtils.check(new Random(24), term("w7"), null);
      return "pass";
    });
  }

  static int[] hitDocs(IndexSearcher s, Query q) throws IOException {
    TopDocs td = s.search(q, 1000);
    int[] docs = new int[td.scoreDocs.length];
    for (int i = 0; i < docs.length; i++) {
      docs[i] = td.scoreDocs[i].doc;
    }
    return docs;
  }

  static void checkHits(DirectoryReader reader) throws Exception {
    IndexSearcher s = fixedSearcher(reader, 31);
    Query w7 = term("w7");
    Query should = bool(term("w1"), Occur.SHOULD, term("w2"), Occur.SHOULD);
    int[] w7Docs = hitDocs(s, w7);
    int[] wrong = new int[w7Docs.length + 1];
    System.arraycopy(w7Docs, 0, wrong, 0, w7Docs.length);
    wrong[w7Docs.length] = reader.maxDoc() - 1 == w7Docs[w7Docs.length - 1] ? 0 : reader.maxDoc() - 1;
    rec(C, "check_hits", "hit_collector", () -> {
      CheckHits.checkHitCollector(new Random(32), w7, "body", s, w7Docs);
      return "pass";
    });
    rec(C, "check_hits", "hit_collector/next", SearcherChecksOracle::next);
    rec(C, "check_hits", "hit_collector_wrong", () -> {
      CheckHits.checkHitCollector(new Random(33), w7, "body", s, wrong);
      return "pass";
    });
    rec(C, "check_hits", "hits", () -> {
      CheckHits.checkHits(new Random(34), w7, "body", s, w7Docs);
      return "pass";
    });
    rec(C, "check_hits", "hits/next", SearcherChecksOracle::next);
    rec(C, "check_hits", "hits_wrong", () -> {
      CheckHits.checkHits(new Random(35), w7, "body", s, wrong);
      return "pass";
    });
    ScoreDoc[] hits = s.search(should, 10).scoreDocs;
    int[] order = new int[hits.length];
    for (int i = 0; i < hits.length; i++) {
      order[i] = hits[i].doc;
    }
    rec(C, "check_hits", "doc_ids", () -> {
      CheckHits.checkDocIds("m", order, hits);
      return "pass";
    });
    rec(C, "check_hits", "doc_ids_count", () -> {
      CheckHits.checkDocIds("m", new int[] {order[0]}, hits);
      return "pass";
    });
    rec(C, "check_hits", "doc_ids_order", () -> {
      int[] swapped = order.clone();
      swapped[0] = order[1];
      swapped[1] = order[0];
      CheckHits.checkDocIds("m", swapped, hits);
      return "pass";
    });
    ScoreDoc[] shifted = new ScoreDoc[hits.length];
    for (int i = 0; i < hits.length; i++) {
      shifted[i] = new ScoreDoc(hits[i].doc, hits[i].score + (i == 2 ? 1e-3f : 0f));
    }
    rec(C, "check_hits", "hits_query", () -> {
      CheckHits.checkHitsQuery(should, hits, hits, order);
      return "pass";
    });
    rec(C, "check_hits", "equal_score", () -> {
      CheckHits.checkEqual(should, hits, shifted);
      return "pass";
    });
    rec(C, "check_hits", "equal_length", () -> {
      CheckHits.checkEqual(should, hits, new ScoreDoc[] {hits[0]});
      return "pass";
    });
    rec(C, "check_hits", "equal_doc", () -> {
      ScoreDoc[] other = hits.clone();
      other[1] = new ScoreDoc(hits[1].doc + 1, hits[1].score);
      CheckHits.checkEqual(should, hits, other);
      return "pass";
    });
    rec(C, "check_hits", "hits2str", () -> CheckHits.hits2str(hits, new ScoreDoc[] {hits[0]}, 0, 0));
    rec(C, "check_hits", "topdocs", () -> CheckHits.topdocsString(s.search(should, 5), 1, 3));
    rec(C, "check_hits", "no_match", () -> {
      CheckHits.checkNoMatchExplanations(w7, "body", s, w7Docs);
      return "pass";
    });
    rec(C, "check_hits", "no_match_wrong", () -> {
      CheckHits.checkNoMatchExplanations(w7, "body", s, new int[] {w7Docs[0]});
      return "pass";
    });
    for (Object[] nq : SearchWrappersOracle.queries(new Random(36))) {
      String name = (String) nq[0];
      Query q = (Query) nq[1];
      rec(C, "check_hits", "explanations/" + name, () -> {
        CheckHits.checkExplanations(q, "body", s, true);
        return "pass";
      });
      rec(C, "check_hits", "matches/" + name, () -> {
        CheckHits.checkMatches(q, s);
        return "pass";
      });
      rec(C, "check_hits", "top_scores/" + name, () -> {
        CheckHits.checkTopScores(new Random(37), q, s);
        return "pass";
      });
    }
    rec(C, "check_hits", "explanations/next", SearcherChecksOracle::next);
    Explanation a = Explanation.match(1f, "a");
    Explanation b = Explanation.match(2f, "b");
    Explanation c = Explanation.match(0.1f, "c");
    rec(C, "verify", "shallow", () -> {
      CheckHits.verifyExplanation("q", 3, 1f, false, a);
      return "pass";
    });
    rec(C, "verify", "shallow_wrong", () -> {
      CheckHits.verifyExplanation("q", 3, 0.1f, false, a);
      return "pass";
    });
    rec(C, "verify", "sum", () -> {
      CheckHits.verifyExplanation("q", 3, 3f, true, Explanation.match(3f, "sum of:", a, b));
      return "pass";
    });
    rec(C, "verify", "sum_wrong", () -> {
      CheckHits.verifyExplanation("q", 3, 3.5f, true, Explanation.match(3.5f, "sum of:", a, b));
      return "pass";
    });
    rec(C, "verify", "sum_ulp", () -> {
      float v = Math.nextUp(3.1f);
      CheckHits.verifyExplanation("q", 3, v, true, Explanation.match(v, "sum of:", a, b, c));
      return "pass";
    });
    rec(C, "verify", "product", () -> {
      CheckHits.verifyExplanation("q", 3, 0.2f, true, Explanation.match(0.2f, "product of:", b, c));
      return "pass";
    });
    rec(C, "verify", "max", () -> {
      CheckHits.verifyExplanation("q", 3, 2f, true, Explanation.match(2f, "max of:", a, b, c));
      return "pass";
    });
    rec(C, "verify", "max_plus", () -> {
      float v = (float) (2f + 0.5f * (3.1 - 2f));
      CheckHits.verifyExplanation("q", 3, v, true, Explanation.match(v, "max plus 0.5 times others of:", a, b, c));
      return "pass";
    });
    rec(C, "verify", "max_plus_wrong", () -> {
      CheckHits.verifyExplanation("q", 3, 2f, true, Explanation.match(2f, "max plus 0.5 times others of:", a, b, c));
      return "pass";
    });
    rec(C, "verify", "max_plus_unterminated", () -> {
      CheckHits.verifyExplanation("q", 3, 2f, true, Explanation.match(2f, "max plus 0.5", a, b));
      return "pass";
    });
    rec(C, "verify", "computed", () -> {
      CheckHits.verifyExplanation("q", 3, 7f, true, Explanation.match(7f, "weight, computed as w * x from:", a, b));
      return "pass";
    });
    rec(C, "verify", "bad_description", () -> {
      CheckHits.verifyExplanation("q", 3, 3f, true, Explanation.match(3f, "combined:", a, b));
      return "pass";
    });
    rec(C, "verify", "single", () -> {
      CheckHits.verifyExplanation("q", 3, 1f, true, Explanation.match(1f, "wrapper", a));
      return "pass";
    });
    rec(C, "verify", "single_wrong", () -> {
      CheckHits.verifyExplanation("q", 3, 1f, true, Explanation.match(1f, "wrapper", b));
      return "pass";
    });
    rec(C, "verify", "freq", () -> {
      CheckHits.verifyExplanation("q", 3, 1f, true, Explanation.match(1f, "x, with freq of:", b));
      return "pass";
    });
    rec(C, "verify", "child_docs", () -> {
      CheckHits.verifyExplanation("q", 3, 1f, true, Explanation.match(1f, "Score based on 3 child docs in range"));
      return "pass";
    });
    rec(C, "verify", "computed_from", () -> {
      CheckHits.verifyExplanation("q", 3, 1f, true, Explanation.match(1f, "computed from:", b, c));
      return "pass";
    });
    rec(C, "verify", "no_match", () -> {
      CheckHits.verifyExplanation("q", 3, 0f, true, Explanation.noMatch("nothing", b, c));
      return "pass";
    });
  }

  /** A leaf collector whose callbacks run the given action on the scorable they receive. */
  interface ScorableAction {
    void run(Scorable s) throws IOException;
  }

  static LeafCollector probe(ScorableAction onSetScorer, ScorableAction onCollect, ScorableAction onBatch) {
    return new LeafCollector() {
      Scorable scorer;

      @Override
      public void setScorer(Scorable s) throws IOException {
        scorer = s;
        onSetScorer.run(s);
      }

      @Override
      public void collect(int doc) throws IOException {
        onCollect.run(scorer);
      }

      @Override
      public void collect(DocIdStream stream) throws IOException {
        onBatch.run(scorer);
        stream.forEach(doc -> {});
      }

      @Override
      public void collectRange(int min, int max) throws IOException {
        onBatch.run(scorer);
      }
    };
  }

  static void scorerBranch(DirectoryReader reader) throws Exception {
    IndexSearcher plain = new IndexSearcher(reader);
    plain.setQueryCache(null);
    LeafReaderContext ctx = reader.leaves().get(0);
    int maxDoc = ctx.reader().maxDoc();
    ScorableAction none = x -> {};
    rec(C, "scorer_branch", "is_scorer", () -> {
      Scorer sc = SearchWrappersOracle.termScorer(plain, ctx, "w7");
      boolean[] seen = new boolean[2];
      LeafCollector lc = new AssertingLeafCollector(probe(x -> {
        seen[0] = x instanceof Scorer;
        seen[1] = x instanceof AssertingScorer;
      }, none, none), 0, maxDoc);
      lc.setScorer(sc);
      return seen[0] + " " + seen[1];
    });
    rec(C, "scorer_branch", "scorable_not_scorer", () -> {
      boolean[] seen = new boolean[1];
      LeafCollector lc = new AssertingLeafCollector(probe(x -> seen[0] = x instanceof Scorer, none, none), 0, maxDoc);
      lc.setScorer(new Scorable() {
        @Override
        public float score() {
          return 1f;
        }
      });
      return seen[0];
    });
    rec(C, "scorer_branch", "unpositioned_score", () -> {
      Scorer sc = SearchWrappersOracle.termScorer(plain, ctx, "w7");
      LeafCollector lc = new AssertingLeafCollector(probe(Scorable::score, none, none), 0, maxDoc);
      lc.setScorer(sc);
      return null;
    });
    rec(C, "scorer_branch", "score_in_range", () -> {
      Scorer sc = SearchWrappersOracle.termScorer(plain, ctx, "w7");
      sc.iterator().nextDoc();
      LeafCollector lc = new AssertingLeafCollector(probe(none, none, Scorable::score), 0, maxDoc);
      lc.setScorer(sc);
      lc.collectRange(0, 5);
      return null;
    });
    rec(C, "scorer_branch", "score_in_collect", () -> {
      Scorer sc = SearchWrappersOracle.termScorer(plain, ctx, "w7");
      int doc = sc.iterator().nextDoc();
      float[] score = new float[1];
      LeafCollector lc = new AssertingLeafCollector(probe(none, x -> score[0] = x.score(), none), 0, maxDoc);
      lc.setScorer(sc);
      lc.collect(doc);
      return Integer.toHexString(Float.floatToIntBits(score[0]));
    });
    rec(C, "scorer_branch", "min_competitive_across_calls", () -> {
      Scorer sc = SearchWrappersOracle.termScorer(plain, ctx, "w7");
      int doc = sc.iterator().nextDoc();
      LeafCollector lc = new AssertingLeafCollector(
          probe(x -> x.setMinCompetitiveScore(1f), x -> x.setMinCompetitiveScore(0.5f), none), 0, maxDoc);
      lc.setScorer(sc);
      lc.collect(doc);
      return null;
    });
    rec(C, "scorer_branch", "min_competitive_reset_by_set_scorer", () -> {
      Scorer sc = SearchWrappersOracle.termScorer(plain, ctx, "w7");
      int doc = sc.iterator().nextDoc();
      LeafCollector lc = new AssertingLeafCollector(
          probe(x -> {}, x -> x.setMinCompetitiveScore(1f), none), 0, maxDoc);
      lc.setScorer(sc);
      lc.collect(doc);
      lc.setScorer(sc);
      int doc2 = sc.iterator().nextDoc();
      lc.collect(doc2);
      return doc + " " + doc2;
    });
    rec(C, "scorer_branch", "asserting_scorer_unchanged", () -> {
      Scorer sc = AssertingScorer.wrap(SearchWrappersOracle.termScorer(plain, ctx, "w7"), true, false);
      sc.iterator().nextDoc();
      LeafCollector lc = new AssertingLeafCollector(
          probe(x -> x.setMinCompetitiveScore(1f), none, none), 0, maxDoc);
      lc.setScorer(sc);
      return null;
    });
    rec(C, "scorer_branch", "asserting_identity", () -> {
      Scorer sc = AssertingScorer.wrap(SearchWrappersOracle.termScorer(plain, ctx, "w7"), false, false);
      int doc = sc.iterator().advance(10);
      LeafCollector lc = new AssertingLeafCollector(
          probe(x -> ((Scorer) x).advanceShallow(doc - 1), none, none), 0, maxDoc);
      lc.setScorer(sc);
      return null;
    });
    rec(C, "scorer_branch", "nested", () -> {
      Scorer sc = SearchWrappersOracle.termScorer(plain, ctx, "w7");
      int doc = sc.iterator().nextDoc();
      float[] score = new float[1];
      LeafCollector lc = new AssertingLeafCollector(new AssertingLeafCollector(
          probe(none, x -> score[0] = x.score(), none), 0, maxDoc), 0, maxDoc);
      lc.setScorer(sc);
      lc.collect(doc);
      return Integer.toHexString(Float.floatToIntBits(score[0]));
    });
    rec(C, "scorer_branch", "nested_range", () -> {
      Scorer sc = SearchWrappersOracle.termScorer(plain, ctx, "w7");
      sc.iterator().nextDoc();
      LeafCollector lc = new AssertingLeafCollector(new AssertingLeafCollector(
          probe(none, none, Scorable::score), 0, maxDoc), 0, maxDoc);
      lc.setScorer(sc);
      lc.collectRange(0, 5);
      return null;
    });
    rec(C, "scorer_branch", "search", () -> {
      AssertingIndexSearcher s = new AssertingIndexSearcher(new Random(41), reader);
      s.setQueryCache(null);
      List<Query> qs = List.of(term("w7"), bool(term("w1"), Occur.SHOULD, term("w2"), Occur.SHOULD),
          new org.apache.lucene.search.ConstantScoreQuery(term("w7")));
      StringBuilder sb = new StringBuilder();
      for (Query q : qs) {
        sb.append(s.search(q, 3).totalHits).append(';');
      }
      return sb;
    });
    rec(C, "scorer_branch", "next", SearcherChecksOracle::next);
  }
}
