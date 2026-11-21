// The suite of LuceneTestCaseOracle's task #349 rows (ltc_members.tsv): the
// remaining LuceneTestCase members under RandomizedRunner.
package org.apache.lucene.tests.util;

import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.FieldType;
import org.apache.lucene.document.IntPoint;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.TextField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.IndexReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.Term;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.store.Directory;
import org.apache.lucene.util.BytesRef;

public class TestLtcMembers extends LuceneTestCase {
  public void testMembers() throws Exception {
    StringBuilder sb = new StringBuilder();
    sb.append("name=").append(getTestName()).append(" thread=").append(isTestThread());
    for (int i = 0; i < 6; i++) {
      Field f = newStringField("s" + (i % 3), "v", i % 2 == 0 ? Field.Store.YES : Field.Store.NO);
      sb.append(" ns=").append(f.fieldType());
    }
    for (int i = 0; i < 6; i++) {
      Field f = newTextField("t" + (i % 4), "some text", i % 2 == 0 ? Field.Store.YES : Field.Store.NO);
      sb.append(" nt=").append(f.fieldType());
    }
    for (int i = 0; i < 4; i++) {
      Field f = newStringField(random(), "b" + i, new BytesRef(new byte[] {1, 2}), Field.Store.NO);
      sb.append(" nb=").append(f.fieldType()).append('|').append(f.binaryValue());
    }
    FieldType custom = new FieldType();
    custom.setIndexOptions(IndexOptions.DOCS_AND_FREQS_AND_POSITIONS);
    custom.setTokenized(false);
    for (int i = 0; i < 4; i++) {
      sb.append(" nf=").append(newField("c" + i, "x", custom).fieldType());
      sb.append(" nfs=").append(newField("d" + i, "x", StringField.TYPE_STORED).fieldType());
      sb.append(" nft=").append(newField(random(), "e" + i, "x", TextField.TYPE_NOT_STORED).fieldType());
    }
    sb.append(" lmp=").append(newLogMergePolicy(true));
    sb.append(" lmp2=").append(newLogMergePolicy(false, 7));
    sb.append(" lmp3=").append(newLogMergePolicy(9));
    sb.append(" tmp=").append(newTieredMergePolicy());
    sb.append(" lmp4=").append(newLogMergePolicy());

    IndexReader l = reader(new String[] {"a", "b"}, new int[] {1, 2}, new int[] {1, 2}, false, null);
    IndexReader l2 = reader(new String[] {"a", "b"}, new int[] {1, 2}, new int[] {1, 2}, false, null);
    IndexReader one = reader(new String[] {"a"}, new int[] {1}, new int[] {1}, false, null);
    IndexReader ids = reader(new String[] {"a", "c"}, new int[] {1, 2}, new int[] {1, 2}, false, null);
    IndexReader dvs = reader(new String[] {"a", "b"}, new int[] {1, 3}, new int[] {1, 2}, false, null);
    IndexReader extra = reader(new String[] {"a", "b"}, new int[] {1, 2}, new int[] {1, 2}, true, null);
    IndexReader pts = reader(new String[] {"a", "b"}, new int[] {1, 2}, new int[] {1, 5}, false, null);
    IndexReader delA = reader(new String[] {"a", "b"}, new int[] {1, 2}, new int[] {1, 2}, false, "a");
    IndexReader delB = reader(new String[] {"a", "b"}, new int[] {1, 2}, new int[] {1, 2}, false, "b");
    check(sb, "same", () -> assertReaderEquals("info", l, l2));
    check(sb, "stats", () -> assertReaderStatisticsEquals("info", l, one));
    check(sb, "stored", () -> assertStoredFieldsEquals("info", l, ids));
    check(sb, "terms", () -> assertTermsEquals("info", l, ids, true));
    check(sb, "dv", () -> assertDocValuesEquals("info", l, dvs));
    check(sb, "fieldInfos", () -> assertFieldInfosEquals("info", l, extra));
    check(sb, "termsFields", () -> assertTermsEquals("info", l, extra, false));
    check(sb, "norms", () -> assertNormsEquals("info", l, extra));
    check(sb, "dvFields", () -> assertDocValuesEquals("info", extra, l));
    check(sb, "points", () -> assertPointsEquals("info", l, pts));
    check(sb, "deleted", () -> assertDeletedDocsEquals("info", delA, delB));
    check(sb, "deletedCount", () -> assertDeletedDocsEquals("info", l, delB));
    check(sb, "tv", () -> assertTermVectorsEquals("info", l, ids));
    for (IndexReader r : new IndexReader[] {l, l2, one, ids, dvs, extra, pts, delA, delB}) r.close();
    sb.append(" last=").append(random().nextLong());
    LuceneTestCaseOracle.DRAWS.add(sb.toString());
  }

  interface Op {
    void run() throws Exception;
  }

  static void check(StringBuilder sb, String label, Op op) throws Exception {
    sb.append(' ').append(label).append('=');
    try {
      op.run();
      sb.append("ok");
    } catch (AssertionError e) {
      sb.append(e.getClass().getName()).append(':').append(String.valueOf(e.getMessage()).replace("\n", "\\n"));
    }
  }

  static IndexReader reader(String[] ids, int[] ns, int[] ps, boolean extra, String delete) throws Exception {
    Directory dir = new ByteBuffersDirectory();
    IndexWriter w = new IndexWriter(dir, new IndexWriterConfig());
    for (int i = 0; i < ids.length; i++) {
      Document d = new Document();
      d.add(new StringField("id", ids[i], Field.Store.YES));
      d.add(new NumericDocValuesField("n", ns[i]));
      d.add(new IntPoint("p", ps[i]));
      if (extra) d.add(new StringField("x", "y", Field.Store.NO));
      w.addDocument(d);
    }
    if (delete != null) w.deleteDocuments(new Term("id", delete));
    w.close();
    return DirectoryReader.open(dir);
  }
}
