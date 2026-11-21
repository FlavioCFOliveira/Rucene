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
import java.io.PrintWriter;
import java.io.Reader;
import java.io.StringReader;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Iterator;
import java.util.List;
import java.util.Map;
import org.apache.lucene.analysis.Analyzer;
import org.apache.lucene.analysis.CharArraySet;
import org.apache.lucene.analysis.TokenStream;
import org.apache.lucene.analysis.Tokenizer;
import org.apache.lucene.analysis.TokenizerFactory;
import org.apache.lucene.analysis.custom.CustomAnalyzer;
import org.apache.lucene.analysis.miscellaneous.DropIfFlaggedFilter;
import org.apache.lucene.analysis.miscellaneous.ProtectedTermFilter;
import org.apache.lucene.analysis.miscellaneous.TypeAsSynonymFilter;
import org.apache.lucene.analysis.tokenattributes.CharTermAttribute;
import org.apache.lucene.analysis.tokenattributes.OffsetAttribute;
import org.apache.lucene.analysis.tokenattributes.PositionIncrementAttribute;
import org.apache.lucene.util.Attribute;
import org.apache.lucene.util.AttributeFactory;

/**
 * Java side of {@code crates/rucene-analysis-common/tests/conditional_attribute_oracle.rs} (rmp
 * #451): a {@code ProtectedTermFilter} and a {@code CustomAnalyzer.Builder.whenTerm} condition over
 * a tokenizer that has no {@code CharTermAttribute}, which the conditional filter adds after its
 * wrapped filters were built. Prints, per scenario, every token's term, offsets and position
 * increment, the end state, and the attribute interfaces the stream holds. Run as a single source
 * file on the class path {@code lucene-core} + {@code lucene-analysis-common} 10.5.0.
 *
 * <p>The wrapped filter is a {@code DropIfFlaggedFilter} with {@code dropFlags} 0, which drops
 * every token it sees and keeps pulling its input: a wrapped filter that stops pulling before the
 * wrapper reports the end of its position ({@code LimitTokenCountFilter}) trips the {@code assert
 * exhausted} of {@code ConditionalTokenFilter.endDelegating} under {@code -ea}, which the port's
 * {@code debug_assert!} reproduces.
 */
public class ConditionalAttributeOracle {

  /** The input every scenario analyses. */
  static final String TEXT = "a bb ccc dddd";

  /** A tokenizer emitting one token per run of non-space characters, with offsets only. */
  public static final class NoTermTokenizer extends Tokenizer {
    private final OffsetAttribute offsetAtt = addAttribute(OffsetAttribute.class);
    private String text;
    private int pos;

    public NoTermTokenizer(AttributeFactory factory) {
      super(factory);
    }

    @Override
    public boolean incrementToken() throws IOException {
      clearAttributes();
      if (text == null) {
        StringBuilder b = new StringBuilder();
        char[] buf = new char[64];
        int n;
        while ((n = input.read(buf)) != -1) {
          b.append(buf, 0, n);
        }
        text = b.toString();
      }
      while (pos < text.length() && text.charAt(pos) == ' ') {
        pos++;
      }
      if (pos == text.length()) {
        return false;
      }
      int start = pos;
      while (pos < text.length() && text.charAt(pos) != ' ') {
        pos++;
      }
      offsetAtt.setOffset(correctOffset(start), correctOffset(pos));
      return true;
    }

    @Override
    public void end() throws IOException {
      super.end();
      int finalOffset = correctOffset(text == null ? 0 : text.length());
      offsetAtt.setOffset(finalOffset, finalOffset);
    }

    @Override
    public void reset() throws IOException {
      super.reset();
      text = null;
      pos = 0;
    }
  }

  /** The factory of {@link NoTermTokenizer}. */
  public static final class NoTermTokenizerFactory extends TokenizerFactory {
    public NoTermTokenizerFactory(Map<String, String> args) {
      super(args);
      if (!args.isEmpty()) {
        throw new IllegalArgumentException("Unknown parameters: " + args);
      }
    }

    @Override
    public Tokenizer create(AttributeFactory factory) {
      return new NoTermTokenizer(factory);
    }
  }

  static void dump(String name, TokenStream ts, PrintWriter out) throws IOException {
    out.println("scenario " + name);
    CharTermAttribute term = ts.addAttribute(CharTermAttribute.class);
    OffsetAttribute offset = ts.addAttribute(OffsetAttribute.class);
    PositionIncrementAttribute posInc = ts.addAttribute(PositionIncrementAttribute.class);
    ts.reset();
    while (ts.incrementToken()) {
      out.println(
          "tok term="
              + term
              + " offset="
              + offset.startOffset()
              + "-"
              + offset.endOffset()
              + " posInc="
              + posInc.getPositionIncrement());
    }
    ts.end();
    out.println(
        "end offset="
            + offset.startOffset()
            + "-"
            + offset.endOffset()
            + " posInc="
            + posInc.getPositionIncrement());
    List<String> names = new ArrayList<>();
    for (Iterator<Class<? extends Attribute>> it = ts.getAttributeClassesIterator();
        it.hasNext(); ) {
      names.add(it.next().getSimpleName());
    }
    Collections.sort(names);
    out.println("attrs " + String.join(",", names));
    ts.close();
  }

  static Tokenizer noTerm() {
    Tokenizer t = new NoTermTokenizer(TokenStream.DEFAULT_TOKEN_ATTRIBUTE_FACTORY);
    t.setReader(new StringReader(TEXT));
    return t;
  }

  static CharArraySet set(String... words) {
    return new CharArraySet(List.of(words), false);
  }

  public static void main(String[] args) throws Exception {
    PrintWriter out = new PrintWriter(System.out, false, StandardCharsets.UTF_8);
    dump(
        "protected-drop",
        new ProtectedTermFilter(set("zz"), noTerm(), in -> new DropIfFlaggedFilter(in, 0)),
        out);
    dump(
        "protected-empty-term",
        new ProtectedTermFilter(set(""), noTerm(), in -> new DropIfFlaggedFilter(in, 0)),
        out);
    dump(
        "protected-type-as-synonym",
        new ProtectedTermFilter(set("word"), noTerm(), TypeAsSynonymFilter::new),
        out);
    for (boolean empty : new boolean[] {true, false}) {
      Analyzer a =
          CustomAnalyzer.builder()
              .withTokenizer(NoTermTokenizerFactory.class)
              .whenTerm(t -> (t.length() == 0) == empty)
              .addTokenFilter("dropIfFlagged", "dropFlags", "0")
              .endwhen()
              .build();
      Reader reader = new StringReader(TEXT);
      dump("when-term-" + empty, a.tokenStream("f", reader), out);
      a.close();
    }
    out.flush();
  }
}
