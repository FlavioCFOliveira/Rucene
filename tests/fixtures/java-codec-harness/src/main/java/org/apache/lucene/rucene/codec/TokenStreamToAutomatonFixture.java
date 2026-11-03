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

import java.util.ArrayList;
import java.util.List;
import org.apache.lucene.analysis.TokenStream;
import org.apache.lucene.analysis.TokenStreamToAutomaton;
import org.apache.lucene.analysis.tokenattributes.CharTermAttribute;
import org.apache.lucene.analysis.tokenattributes.OffsetAttribute;
import org.apache.lucene.analysis.tokenattributes.PositionIncrementAttribute;
import org.apache.lucene.analysis.tokenattributes.PositionLengthAttribute;
import org.apache.lucene.util.automaton.Automaton;
import org.apache.lucene.util.automaton.Transition;

/**
 * Runs {@code TokenStreamToAutomaton} over scripted token graphs and prints the
 * automaton it builds — state count, accept states and every transition in
 * sorted order — so the Rust portability test can compare.
 *
 * <p>The scripts mirror {@code tests/portability/token_stream_to_automaton.rs}.
 */
public final class TokenStreamToAutomatonFixture {
  private TokenStreamToAutomatonFixture() {}

  record Tok(String term, int posInc, int posLen, int start, int end) {}

  record Case(
      String name,
      List<Tok> tokens,
      int endPosInc,
      int finalOffset,
      boolean preservePositionIncrements,
      boolean finalOffsetGapAsHole,
      boolean unicodeArcs) {}

  static final class Scripted extends TokenStream {
    final CharTermAttribute term = addAttribute(CharTermAttribute.class);
    final PositionIncrementAttribute posInc = addAttribute(PositionIncrementAttribute.class);
    final PositionLengthAttribute posLen = addAttribute(PositionLengthAttribute.class);
    final OffsetAttribute offset = addAttribute(OffsetAttribute.class);
    final Case c;
    int upto;

    Scripted(Case c) {
      this.c = c;
    }

    @Override
    public boolean incrementToken() {
      if (upto == c.tokens().size()) {
        return false;
      }
      clearAttributes();
      Tok t = c.tokens().get(upto++);
      term.append(t.term());
      posInc.setPositionIncrement(t.posInc());
      posLen.setPositionLength(t.posLen());
      offset.setOffset(t.start(), t.end());
      return true;
    }

    @Override
    public void end() throws java.io.IOException {
      super.end();
      posInc.setPositionIncrement(c.endPosInc());
      offset.setOffset(c.finalOffset(), c.finalOffset());
    }

    @Override
    public void reset() throws java.io.IOException {
      super.reset();
      upto = 0;
    }
  }

  static List<Case> cases() {
    List<Case> cases = new ArrayList<>();
    cases.add(
        new Case(
            "linear",
            List.of(new Tok("a", 1, 1, 0, 1), new Tok("b", 1, 1, 2, 3), new Tok("c", 1, 1, 4, 5)),
            0, 5, true, false, false));
    cases.add(
        new Case(
            "synonym",
            List.of(
                new Tok("wifi", 1, 2, 0, 5),
                new Tok("wi", 0, 1, 0, 2),
                new Tok("fi", 1, 1, 3, 5),
                new Tok("network", 1, 1, 6, 13)),
            0, 13, true, false, false));
    cases.add(
        new Case(
            "holes",
            List.of(new Tok("a", 1, 1, 0, 1), new Tok("b", 3, 1, 6, 7), new Tok("c", 1, 1, 8, 9)),
            0, 9, true, false, false));
    cases.add(
        new Case(
            "holesflattened",
            List.of(new Tok("a", 1, 1, 0, 1), new Tok("b", 3, 1, 6, 7), new Tok("c", 1, 1, 8, 9)),
            2, 12, false, false, false));
    cases.add(
        new Case(
            "trailing",
            List.of(new Tok("a", 1, 1, 0, 1), new Tok("b", 1, 1, 2, 3)),
            2, 9, true, false, false));
    cases.add(
        new Case(
            "spanhole",
            List.of(new Tok("a", 1, 3, 0, 1), new Tok("b", 3, 1, 6, 7), new Tok("c", 1, 1, 8, 9)),
            0, 9, true, false, false));
    cases.add(
        new Case(
            "finalgap",
            List.of(new Tok("a", 1, 1, 0, 1), new Tok("b", 1, 1, 2, 3)),
            0, 7, true, true, false));
    cases.add(
        new Case(
            "unicode",
            List.of(new Tok("é€", 1, 1, 0, 2), new Tok("😀x", 1, 1, 3, 6)),
            0, 6, true, false, true));
    cases.add(
        new Case(
            "emptyterm",
            List.of(new Tok("a", 1, 1, 0, 1), new Tok("", 1, 1, 2, 2), new Tok("c", 1, 1, 3, 4)),
            0, 4, true, false, false));
    List<Tok> longTokens = new ArrayList<>();
    for (int i = 0; i < 60; i++) {
      int posInc = (i % 7 == 6) ? 2 : 1;
      longTokens.add(new Tok("t" + i, posInc, (i % 5 == 0) ? 2 : 1, i * 3, i * 3 + 2));
      if (i % 5 == 0) {
        longTokens.add(new Tok("s" + i, 0, 1, i * 3, i * 3 + 2));
      }
    }
    cases.add(new Case("long", longTokens, 1, 200, true, false, false));
    return cases;
  }

  public static void main(String[] args) throws Exception {
    for (Case c : cases()) {
      TokenStreamToAutomaton tsta = new TokenStreamToAutomaton();
      tsta.setPreservePositionIncrements(c.preservePositionIncrements());
      tsta.setFinalOffsetGapAsHole(c.finalOffsetGapAsHole());
      tsta.setUnicodeArcs(c.unicodeArcs());
      Automaton a;
      try (Scripted ts = new Scripted(c)) {
        a = tsta.toAutomaton(ts);
      }
      System.out.println("case=" + c.name());
      System.out.println("states=" + a.getNumStates());
      StringBuilder accept = new StringBuilder("accept=");
      for (int s = 0; s < a.getNumStates(); s++) {
        if (a.isAccept(s)) {
          accept.append(s).append(',');
        }
      }
      System.out.println(accept);
      for (Transition[] perState : a.getSortedTransitions()) {
        for (Transition t : perState) {
          System.out.println("t=" + t.source + "," + t.dest + "," + t.min + "," + t.max);
        }
      }
    }
    System.out.println("done=true");
  }
}
