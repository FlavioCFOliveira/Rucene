/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

package org.apache.lucene.rucene.codec;

import java.io.IOException;
import java.net.InetAddress;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import org.apache.lucene.analysis.standard.StandardAnalyzer;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.InetAddressPoint;
import org.apache.lucene.document.StringField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.SerialMergeScheduler;
import org.apache.lucene.index.StoredFields;
import org.apache.lucene.index.Term;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.BytesRef;

/**
 * Oracle of {@code tests/portability/inet_address_point.rs} (rmp task #313).
 *
 * <p>Modes:
 *
 * <ul>
 *   <li>{@code values}: prints, for every address of {@link #ADDRESSES}, its host address, its
 *       {@code encode}/{@code decode}, {@code nextUp}/{@code nextDown}, the field's {@code toString}
 *       and the {@code toString} of its exact, prefix (every length of {@link #PREFIXES}) and set
 *       queries, then the boundary values, range and set queries over several addresses and the
 *       errors of {@code decode} and {@code setBytesValue}.
 *   <li>{@code write <root>}: writes the corpus of {@link #doc(int)} into {@code <root>} (two
 *       commits, deletions in the second).
 *   <li>{@code query <root>}: runs {@link #queries()} and prints every hit.
 * </ul>
 *
 * <p>The Rust test defines the same address list, corpus and queries, and prints the same lines.
 */
public class InetAddressPointFixture {

  static final String[] ADDRESSES = {
    "0.0.0.0",
    "1.2.3.4",
    "1.2.3.255",
    "10.0.0.1",
    "255.255.255.255",
    "::",
    "::1",
    "::ffff",
    "::1:0",
    "::fffe:ffff:ffff",
    "::ffff:1.2.3.4",
    "::1.2.3.4",
    "::1:0:0:0",
    "fec0::f66d",
    "2001:db8::a",
    "2001::a6bd:fc80",
    "fdc8:57ed:f042:0ad1:f66d:4ff:fe90:ce0c",
    "ffff:ffff:ffff:ffff:ffff:ffff:ffff:fffe",
    "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
  };

  static final int[] PREFIXES = {
    -1, 0, 1, 7, 8, 9, 16, 24, 25, 31, 32, 33, 64, 96, 100, 127, 128, 129
  };

  static final int DOCS = 400;
  static final int TOP_N = 1000;

  static InetAddress ip(String literal) {
    try {
      return InetAddress.getByName(literal);
    } catch (java.net.UnknownHostException e) {
      throw new IllegalArgumentException(e);
    }
  }

  static InetAddress inet(byte[] b) {
    try {
      return InetAddress.getByAddress(b);
    } catch (java.net.UnknownHostException e) {
      throw new IllegalArgumentException(e);
    }
  }

  static String hex(byte[] b) {
    StringBuilder sb = new StringBuilder();
    for (byte x : b) {
      sb.append(String.format("%02x", x & 0xFF));
    }
    return sb.toString();
  }

  static String error(RuntimeException e) {
    return "! " + e.getClass().getSimpleName() + " " + e.getMessage();
  }

  interface Op {
    String run();
  }

  static String attempt(Op op) {
    try {
      return op.run();
    } catch (RuntimeException e) {
      return error(e);
    }
  }

  static void values() {
    StringBuilder out = new StringBuilder();
    for (String literal : ADDRESSES) {
      InetAddress a = ip(literal);
      out.append("addr ").append(literal).append('\n');
      out.append("  host ")
          .append(a.getHostAddress())
          .append(" len ")
          .append(a.getAddress().length)
          .append('\n');
      byte[] encoded = InetAddressPoint.encode(a);
      out.append("  encode ").append(hex(encoded)).append('\n');
      InetAddress decoded = InetAddressPoint.decode(encoded);
      out.append("  decode ")
          .append(decoded.getHostAddress())
          .append(" len ")
          .append(decoded.getAddress().length)
          .append('\n');
      out.append("  nextUp ")
          .append(attempt(() -> InetAddressPoint.nextUp(a).getHostAddress()))
          .append('\n');
      out.append("  nextDown ")
          .append(attempt(() -> InetAddressPoint.nextDown(a).getHostAddress()))
          .append('\n');
      out.append("  field ").append(new InetAddressPoint("f", a)).append('\n');
      out.append("  exact ").append(InetAddressPoint.newExactQuery("f", a)).append('\n');
      for (int p : PREFIXES) {
        out.append("  prefix ")
            .append(p)
            .append(' ')
            .append(attempt(() -> InetAddressPoint.newPrefixQuery("f", a, p).toString()))
            .append('\n');
      }
      out.append("  set ").append(InetAddressPoint.newSetQuery("f", a)).append('\n');
    }
    out.append("min ").append(InetAddressPoint.MIN_VALUE.getHostAddress()).append('\n');
    out.append("max ").append(InetAddressPoint.MAX_VALUE.getHostAddress()).append('\n');
    for (int i = 0; i + 1 < ADDRESSES.length; i += 3) {
      out.append("range ")
          .append(ADDRESSES[i])
          .append(' ')
          .append(ADDRESSES[i + 1])
          .append(' ')
          .append(InetAddressPoint.newRangeQuery("f", ip(ADDRESSES[i]), ip(ADDRESSES[i + 1])))
          .append('\n');
    }
    InetAddress[] all = new InetAddress[ADDRESSES.length];
    for (int i = 0; i < ADDRESSES.length; i++) {
      all[i] = ip(ADDRESSES[i]);
    }
    out.append("set all ").append(InetAddressPoint.newSetQuery("f", all)).append('\n');
    out.append("set empty ").append(InetAddressPoint.newSetQuery("f")).append('\n');
    for (int len : new int[] {0, 3, 5, 15, 17}) {
      out.append("decode len ")
          .append(len)
          .append(' ')
          .append(attempt(() -> InetAddressPoint.decode(new byte[len]).getHostAddress()))
          .append('\n');
    }
    out.append("setBytesValue ")
        .append(
            attempt(
                () -> {
                  new InetAddressPoint("f", ip("1.2.3.4")).setBytesValue(new BytesRef("x"));
                  return "ok";
                }))
        .append('\n');
    System.out.print(out);
  }

  static String id(int i) {
    return String.format("id-%04d", i);
  }

  static byte[] v4(int i) {
    return new byte[] {10, (byte) (i % 3), (byte) (i / 16), (byte) ((i * 37) % 256)};
  }

  static byte[] v6(int i) {
    byte[] b = new byte[16];
    b[0] = 0x20;
    b[1] = 0x01;
    b[2] = 0x0d;
    b[3] = (byte) 0xb8;
    b[13] = (byte) (i % 7);
    int low = (i * 13) % 65536;
    b[14] = (byte) (low >> 8);
    b[15] = (byte) low;
    return b;
  }

  static final String[] SPECIAL = {
    "::", "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", "255.255.255.255", "0.0.0.0",
    "::fffe:ffff:ffff", "::1:0:0:0", "::1.2.3.4"
  };

  static Document doc(int i) {
    Document d = new Document();
    d.add(new StringField("id", id(i), Field.Store.YES));
    switch (i % 5) {
      case 1 -> d.add(new InetAddressPoint("ip", inet(v4(i))));
      case 2 -> d.add(new InetAddressPoint("ip", inet(v6(i))));
      case 3 -> {
        d.add(new InetAddressPoint("ip", inet(v4(i))));
        d.add(new InetAddressPoint("ip", inet(v6(i))));
      }
      case 4 -> {
        d.add(new InetAddressPoint("ip", inet(v4(i))));
        d.add(new InetAddressPoint("ip", inet(v4(i + 1))));
      }
      default -> {}
    }
    if (i < SPECIAL.length * 10 && i % 10 == 0) {
      d.add(new InetAddressPoint("ip", ip(SPECIAL[i / 10])));
    }
    return d;
  }

  static void write(String root) throws IOException {
    IndexWriterConfig config = new IndexWriterConfig(new StandardAnalyzer());
    config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
    config.setMergeScheduler(new SerialMergeScheduler());
    try (Directory dir = FSDirectory.open(Paths.get(root));
        IndexWriter w = new IndexWriter(dir, config)) {
      for (int i = 0; i < DOCS / 2; i++) {
        w.addDocument(doc(i));
      }
      w.commit();
      for (int i = DOCS / 2; i < DOCS; i++) {
        w.addDocument(doc(i));
      }
      for (int i = 7; i < DOCS; i += 50) {
        w.deleteDocuments(new Term("id", id(i)));
      }
      w.commit();
    }
  }

  static List<Object[]> queries() {
    List<Object[]> q = new ArrayList<>();
    for (String a :
        new String[] {
          "10.1.0.37", "2001:db8::2:1a", "::", "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
          "0.0.0.0", "255.255.255.255", "::fffe:ffff:ffff", "::1.2.3.4", "9.9.9.9"
        }) {
      q.add(new Object[] {"exact " + a, InetAddressPoint.newExactQuery("ip", ip(a))});
    }
    String[][] prefixes = {
      {"10.0.0.0", "8"},
      {"10.1.0.0", "16"},
      {"10.2.3.0", "24"},
      {"10.0.1.0", "20"},
      {"0.0.0.0", "0"},
      {"2001:db8::", "32"},
      {"2001:db8::3:0", "112"},
      {"2001:db8::3:27", "128"},
      {"::", "0"},
      {"::", "96"},
      {"::1:0:0:0", "80"},
    };
    for (String[] p : prefixes) {
      q.add(
          new Object[] {
            "prefix " + p[0] + "/" + p[1],
            InetAddressPoint.newPrefixQuery("ip", ip(p[0]), Integer.parseInt(p[1]))
          });
    }
    String[][] ranges = {
      {"10.0.0.0", "10.1.255.255"},
      {"::", "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"},
      {"::", "::fffe:ffff:ffff"},
      {"0.0.0.0", "255.255.255.255"},
      {"2001:db8::", "2001:db8::4:0"},
      {"10.2.0.0", "10.0.0.0"},
    };
    for (String[] r : ranges) {
      q.add(
          new Object[] {
            "range " + r[0] + " " + r[1], InetAddressPoint.newRangeQuery("ip", ip(r[0]), ip(r[1]))
          });
    }
    q.add(
        new Object[] {
          "range exclusive 10.0.0.37 10.2.255.255",
          InetAddressPoint.newRangeQuery(
              "ip",
              InetAddressPoint.nextUp(ip("10.0.0.37")),
              InetAddressPoint.nextDown(ip("10.2.255.255")))
        });
    q.add(
        new Object[] {
          "set mixed",
          InetAddressPoint.newSetQuery(
              "ip",
              inet(v4(1)),
              inet(v4(6)),
              inet(v6(2)),
              ip("ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
              ip("9.9.9.9"))
        });
    q.add(new Object[] {"set empty", InetAddressPoint.newSetQuery("ip")});
    q.add(
        new Object[] {
          "set boundaries",
          InetAddressPoint.newSetQuery(
              "ip", ip("0.0.0.0"), ip("255.255.255.255"), ip("::fffe:ffff:ffff"))
        });
    q.add(
        new Object[] {
          "set duplicates", InetAddressPoint.newSetQuery("ip", inet(v4(11)), inet(v4(11)))
        });
    return q;
  }

  static void query(String root) throws IOException {
    StringBuilder out = new StringBuilder();
    try (Directory dir = FSDirectory.open(Paths.get(root));
        DirectoryReader reader = DirectoryReader.open(dir)) {
      out.append("reader maxDoc=")
          .append(reader.maxDoc())
          .append(" numDocs=")
          .append(reader.numDocs())
          .append(" leaves=")
          .append(reader.leaves().size())
          .append('\n');
      IndexSearcher searcher = new IndexSearcher(reader);
      StoredFields stored = searcher.storedFields();
      for (Object[] q : queries()) {
        Query query = (Query) q[1];
        TopDocs top = searcher.search(query, TOP_N);
        out.append("query ")
            .append(q[0])
            .append(" | ")
            .append(query)
            .append(" | total=")
            .append(top.totalHits.value())
            .append(' ')
            .append(top.totalHits.relation())
            .append('\n');
        for (ScoreDoc hit : top.scoreDocs) {
          out.append("  hit doc=")
              .append(hit.doc)
              .append(" id=")
              .append(stored.document(hit.doc).get("id"))
              .append('\n');
        }
      }
    }
    System.out.print(out);
  }

  public static void main(String[] args) throws Exception {
    switch (args[0]) {
      case "values" -> values();
      case "write" -> write(args[1]);
      case "query" -> query(args[1]);
      default -> throw new IllegalArgumentException("unknown mode " + args[0]);
    }
  }
}
