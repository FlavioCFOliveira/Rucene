// The suite of LuceneTestCaseOracle's random-helper draws (a Lucene suite must
// be named Test*).
package org.apache.lucene.tests.util;

import org.apache.lucene.util.BytesRef;

public class TestLtcDraws extends LuceneTestCase {
  public void testDraws() throws Exception {
    StringBuilder sb = new StringBuilder();
    sb.append("class=").append(getTestClass().getName());
    for (int i : new int[] {1, 2, 3, 5, 10, 100, 1000}) sb.append(" atLeast(").append(i).append(")=").append(atLeast(i));
    sb.append(" atLeastR=").append(atLeast(random(), 7));
    sb.append(" rarely=");
    for (int i = 0; i < 64; i++) sb.append(rarely() ? '1' : '0');
    sb.append(" usually=");
    for (int i = 0; i < 64; i++) sb.append(usually() ? '1' : '0');
    sb.append(" maybeCache=");
    for (int i = 0; i < 16; i++) sb.append(MAYBE_CACHE_POLICY.shouldCache(null) ? '1' : '0');
    for (int i = 0; i < 12; i++) sb.append(" bytes=").append(br(newBytesRef("hello")));
    for (int i = 0; i < 4; i++) sb.append(" empty=").append(br(newBytesRef()));
    for (int i = 0; i < 4; i++) sb.append(" len3=").append(br(newBytesRef(3)));
    for (int i = 0; i < 4; i++) sb.append(" slice=").append(br(newBytesRef(new byte[] {1, 2, 3, 4, 5}, 1, 3)));
    for (int i = 0; i < 4; i++) sb.append(" copy=").append(br(newBytesRef(new BytesRef(new byte[] {9, 8, 7}, 1, 2))));
    for (int i = 0; i < 12; i++) sb.append(" locale=").append(randomLocale(random())).append('|').append(randomLocale(random()).toLanguageTag());
    for (int i = 0; i < 12; i++) sb.append(" tz=").append(randomTimeZone(random()).getID());
    sb.append(" last=").append(random().nextLong());
    LuceneTestCaseOracle.DRAWS.add(sb.toString());
  }

  static String br(BytesRef b) {
    StringBuilder sb = new StringBuilder();
    sb.append(b.offset).append('/').append(b.length).append('/').append(b.bytes.length).append('/');
    for (byte x : b.bytes) sb.append(String.format("%02x", x & 0xff));
    return sb.toString();
  }
}
