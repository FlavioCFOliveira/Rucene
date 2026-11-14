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
import java.io.OutputStream;
import java.io.OutputStreamWriter;
import java.io.PrintWriter;
import java.io.StringReader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import org.apache.lucene.analysis.morph.BinaryDictionary;
import org.apache.lucene.analysis.morph.BinaryDictionaryWriter;
import org.apache.lucene.analysis.morph.CharacterDefinition;
import org.apache.lucene.analysis.morph.CharacterDefinitionWriter;
import org.apache.lucene.analysis.morph.ConnectionCosts;
import org.apache.lucene.analysis.morph.ConnectionCostsWriter;
import org.apache.lucene.analysis.morph.Dictionary;
import org.apache.lucene.analysis.morph.DictionaryEntryWriter;
import org.apache.lucene.analysis.morph.MorphData;
import org.apache.lucene.analysis.morph.Token;
import org.apache.lucene.analysis.morph.TokenInfoFST;
import org.apache.lucene.analysis.morph.TokenType;
import org.apache.lucene.analysis.morph.Viterbi;
import org.apache.lucene.store.DataOutput;
import org.apache.lucene.util.IntsRef;
import org.apache.lucene.util.IntsRefBuilder;
import org.apache.lucene.util.fst.FST;
import org.apache.lucene.util.fst.FSTCompiler;
import org.apache.lucene.util.fst.PositiveIntOutputs;
import org.apache.lucene.util.fst.Util;

/**
 * The Java side of the morph oracle of rmp task #466 (Rust counterpart {@code
 * crates/rucene-analysis-common/tests/analysis_oracle_hunspell_compound_morph_collation.rs}).
 * {@code analysis.morph} is abstract: Lucene's concrete dictionaries and tokenizers live in the
 * kuromoji and nori modules, which the port does not hold. The oracle therefore builds the same
 * minimal concrete world on both sides: an entry writer and a binary dictionary of {@code (leftId,
 * rightId, cost)} shorts, a connection cost matrix, a character definition, a token info FST over
 * the surfaces, and a {@code Viterbi} subclass whose unknown-word and backtrace steps follow
 * kuromoji's {@code JapaneseTokenizer} in their simplest form.
 *
 * <p>Usage: {@code MorphOracle <spec> <out-dir>}. The spec holds {@code entry <surface> <left>
 * <right> <cost> <pos>} lines (sorted by surface, as the FST needs) and {@code input <text>}
 * lines, escaped as {@code AnalysisOracle.escape}. The program writes the dictionary files under
 * {@code <out-dir>} and dumps (stdout) what the readers read back and the tokens of each input.
 */
public final class MorphOracle {

  private MorphOracle() {}

  static final int FORWARD_SIZE = 4;
  static final int BACKWARD_SIZE = 5;
  static final String[] CLASSES = {"DEFAULT", "ALPHA", "NUMERIC", "SYMBOL"};

  static String escape(CharSequence s) {
    StringBuilder out = new StringBuilder();
    for (int i = 0; i < s.length(); i++) {
      char ch = s.charAt(i);
      if (ch >= 0x20 && ch <= 0x7e && ch != '\\') {
        out.append(ch);
      } else {
        out.append(String.format(Locale.ROOT, "\\u%04X", (int) ch));
      }
    }
    return out.toString();
  }

  static String unescape(String s) {
    StringBuilder out = new StringBuilder();
    int i = 0;
    while (i < s.length()) {
      char ch = s.charAt(i);
      if (ch == '\\') {
        out.append((char) Integer.parseInt(s.substring(i + 2, i + 6), 16));
        i += 6;
      } else {
        out.append(ch);
        i++;
      }
    }
    return out.toString();
  }

  static byte lookup(String name) {
    for (byte i = 0; i < CLASSES.length; i++) {
      if (CLASSES[i].equals(name)) return i;
    }
    throw new IllegalArgumentException("unknown class " + name);
  }

  /** The entry writer: three shorts per entry, the part of speech in the POS dictionary. */
  static final class MiniEntryWriter extends DictionaryEntryWriter {
    MiniEntryWriter() {
      super(1 << 12);
    }

    @Override
    protected int putEntry(String[] entry) {
      int start = buffer.position();
      buffer.putShort(Short.parseShort(entry[1]));
      buffer.putShort(Short.parseShort(entry[2]));
      buffer.putShort(Short.parseShort(entry[3]));
      posDict.add(entry[4]);
      return start;
    }

    @Override
    protected void writePosDict(OutputStream bos, DataOutput out) throws IOException {
      out.writeVInt(posDict.size());
      for (String s : posDict) {
        out.writeString(s);
      }
    }
  }

  static final class MiniDictionaryWriter extends BinaryDictionaryWriter<MiniBinaryDictionary> {
    MiniDictionaryWriter() {
      super(MiniBinaryDictionary.class, new MiniEntryWriter());
    }

    void map(int sourceId, int wordId) {
      addMapping(sourceId, wordId);
    }

    @Override
    public void write(Path baseDir) throws IOException {
      super.write(baseDir, "mini_tm", "mini_pos", "mini_dict", 1);
    }
  }

  /** The binary dictionary of {@link MiniEntryWriter}'s entries. */
  static final class MiniBinaryDictionary extends BinaryDictionary<MiniMorphData> {
    final MiniMorphData morph = new MiniMorphData(this);

    MiniBinaryDictionary(Path dir) throws IOException {
      super(
          () -> Files.newInputStream(dir.resolve(base(MiniBinaryDictionary.class) + TARGETMAP_FILENAME_SUFFIX)),
          () -> Files.newInputStream(dir.resolve(base(MiniBinaryDictionary.class) + DICT_FILENAME_SUFFIX)),
          "mini_tm",
          "mini_dict",
          1);
    }

    short shortAt(int i) {
      return buffer.getShort(i);
    }

    @Override
    public MiniMorphData getMorphAttributes() {
      return morph;
    }
  }

  static final class MiniMorphData implements MorphData {
    final MiniBinaryDictionary dict;

    MiniMorphData(MiniBinaryDictionary dict) {
      this.dict = dict;
    }

    @Override
    public int getLeftId(int morphId) {
      return dict.shortAt(morphId);
    }

    @Override
    public int getRightId(int morphId) {
      return dict.shortAt(morphId + 2);
    }

    @Override
    public int getWordCost(int morphId) {
      return dict.shortAt(morphId + 4);
    }
  }

  /** The unknown-word data: one entry per character class. */
  static final class UnknownMorphData implements MorphData {
    @Override
    public int getLeftId(int morphId) {
      return morphId + 1;
    }

    @Override
    public int getRightId(int morphId) {
      return morphId;
    }

    @Override
    public int getWordCost(int morphId) {
      return 1000 + 100 * morphId;
    }
  }

  static final class UnknownDictionary implements Dictionary<UnknownMorphData> {
    final UnknownMorphData morph = new UnknownMorphData();

    @Override
    public UnknownMorphData getMorphAttributes() {
      return morph;
    }
  }

  static final class MiniConnectionCosts extends ConnectionCosts {
    MiniConnectionCosts(Path dir) throws IOException {
      super(
          () -> Files.newInputStream(dir.resolve(base(MiniConnectionCosts.class) + FILENAME_SUFFIX)),
          "mini_cc",
          1);
    }
  }

  static final class MiniCharacterDefinition extends CharacterDefinition {
    MiniCharacterDefinition(Path dir) throws IOException {
      super(
          () -> Files.newInputStream(dir.resolve(base(MiniCharacterDefinition.class) + FILENAME_SUFFIX)),
          "mini_cd",
          1,
          CLASSES.length);
    }
  }

  static final class MiniTokenInfoFST extends TokenInfoFST {
    MiniTokenInfoFST(FST<Long> fst) throws IOException {
      super(fst, 0x7a, 0x61);
    }
  }

  static final class MiniToken extends Token {
    final int wordId;

    MiniToken(char[] surface, int start, int length, TokenType type, int wordId) {
      super(surface, 0, length, start, start + length, type);
      this.wordId = wordId;
    }
  }

  static final class MiniViterbi extends Viterbi<MiniToken, Viterbi.Position> {
    final MiniCharacterDefinition charDef;
    final UnknownDictionary unknown = new UnknownDictionary();
    final MiniBinaryDictionary dictionary;

    MiniViterbi(
        MiniTokenInfoFST fst,
        MiniBinaryDictionary dictionary,
        MiniConnectionCosts costs,
        MiniCharacterDefinition charDef) {
      super(fst, fst.getBytesReader(), dictionary, null, null, null, costs, Viterbi.Position.class);
      this.charDef = charDef;
      this.dictionary = dictionary;
    }

    @Override
    protected int processUnknownWord(boolean anyMatches, Position posData) throws IOException {
      final char firstCharacter = (char) buffer.get(pos);
      if (!anyMatches || charDef.isInvoke(firstCharacter)) {
        final byte characterClass = charDef.getCharacterClass(firstCharacter);
        int unknownWordLength = 1;
        if (charDef.isGroup(firstCharacter)) {
          for (int posAhead = pos + 1; unknownWordLength < MAX_UNKNOWN_WORD_LENGTH; posAhead++) {
            final int ch = buffer.get(posAhead);
            if (ch == -1 || characterClass != charDef.getCharacterClass((char) ch)) {
              break;
            }
            unknownWordLength++;
          }
        }
        add(
            unknown.getMorphAttributes(),
            posData,
            pos,
            pos + unknownWordLength,
            characterClass,
            TokenType.UNKNOWN,
            false);
        return unknownWordLength;
      }
      return 0;
    }

    @Override
    protected void backtrace(Position endPosData, int fromIDX) throws IOException {
      final int endPos = endPosData.getPos();
      int pos = endPos;
      int bestIDX = fromIDX;
      while (pos > lastBackTracePos) {
        final Position posData = positions.get(pos);
        final int backPos = posData.getBackPos(bestIDX);
        final int backWordPos = posData.getBackWordPos(bestIDX);
        final int length = pos - backWordPos;
        final TokenType backType = posData.getBackType(bestIDX);
        final int backID = posData.getBackID(bestIDX);
        final int nextBestIDX = posData.getBackIndex(bestIDX);
        final char[] fragment = buffer.get(backWordPos, length);
        pending.add(new MiniToken(fragment, backWordPos, length, backType, backID));
        pos = backPos;
        bestIDX = nextBestIDX;
      }
      lastBackTracePos = endPos;
      buffer.freeBefore(endPos);
      positions.freeBefore(endPos);
    }
  }

  static String base(Class<?> c) {
    return c.getName().replace('.', '/');
  }

  public static void main(String[] args) throws Exception {
    List<String[]> entries = new ArrayList<>();
    List<String> inputs = new ArrayList<>();
    for (String line : Files.readAllLines(Path.of(args[0]), StandardCharsets.UTF_8)) {
      String[] p = line.split("\t", -1);
      if (p[0].equals("entry")) {
        entries.add(new String[] {unescape(p[1]), p[2], p[3], p[4], p[5]});
      } else if (p[0].equals("input")) {
        inputs.add(unescape(p[1]));
      }
    }
    Path dir = Path.of(args[1]);
    PrintWriter out =
        new PrintWriter(new OutputStreamWriter(System.out, StandardCharsets.UTF_8), false);

    // writers
    ConnectionCostsWriter<MiniConnectionCosts> ccw =
        new ConnectionCostsWriter<>(MiniConnectionCosts.class, FORWARD_SIZE, BACKWARD_SIZE);
    for (int f = 0; f < FORWARD_SIZE; f++) {
      for (int b = 0; b < BACKWARD_SIZE; b++) {
        if ((f + b) % 5 != 3) {
          ccw.add(f, b, ((f * 7 + b * 13) % 41) * 37 - 600);
        }
      }
    }
    // ConnectionCostsWriter creates baseDir, not the directories of the class name
    try {
      ccw.write(dir, "mini_cc", 1);
      out.println("ccw written");
    } catch (IOException e) {
      out.println("ccw-error " + e.getClass().getSimpleName());
    }
    Files.createDirectories(dir.resolve(base(MiniConnectionCosts.class)).getParent());
    ccw.write(dir, "mini_cc", 1);

    CharacterDefinitionWriter<MiniCharacterDefinition> cdw =
        new CharacterDefinitionWriter<>(MiniCharacterDefinition.class, (byte) 0, CLASSES.length, MorphOracle::lookup);
    for (int c = 'a'; c <= 'z'; c++) cdw.putCharacterCategory(c, "ALPHA");
    for (int c = 'A'; c <= 'Z'; c++) cdw.putCharacterCategory(c, "ALPHA EXTRA");
    for (int c = '0'; c <= '9'; c++) cdw.putCharacterCategory(c, "NUMERIC");
    for (char c : "!?.,;:-".toCharArray()) cdw.putCharacterCategory(c, "SYMBOL");
    cdw.putCharacterCategory(0x30FB, "ALPHA");
    cdw.putCharacterCategory(0x65E5, "ALPHA");
    cdw.putInvokeDefinition("ALPHA", 1, 1, 0);
    cdw.putInvokeDefinition("NUMERIC", 1, 1, 2);
    cdw.putInvokeDefinition("SYMBOL", 0, 1, 0);
    cdw.putInvokeDefinition("DEFAULT", 0, 0, 0);
    cdw.write(dir, "mini_cd", 1);

    MiniDictionaryWriter dw = new MiniDictionaryWriter();
    FSTCompiler<Long> compiler =
        new FSTCompiler.Builder<>(FST.INPUT_TYPE.BYTE2, PositiveIntOutputs.getSingleton()).build();
    IntsRefBuilder scratch = new IntsRefBuilder();
    String last = null;
    int sourceId = -1;
    for (String[] e : entries) {
      int wordId = dw.put(new String[] {e[0], e[1], e[2], e[3], e[4]});
      if (!e[0].equals(last)) {
        sourceId++;
        compiler.add(Util.toUTF16(e[0], scratch), (long) sourceId);
        last = e[0];
      }
      dw.map(sourceId, wordId);
      out.println("put " + escape(e[0]) + " " + wordId);
    }
    dw.write(dir);
    FST<Long> fst = FST.fromFSTReader(compiler.compile(), compiler.getFSTReader());

    // readers
    MiniBinaryDictionary dict = new MiniBinaryDictionary(dir);
    for (int s = 0; s <= sourceId; s++) {
      IntsRef ref = new IntsRef();
      dict.lookupWordIds(s, ref);
      StringBuilder b = new StringBuilder("ids " + s + ":");
      for (int i = 0; i < ref.length; i++) {
        int id = ref.ints[ref.offset + i];
        b.append(' ')
            .append(id)
            .append('/')
            .append(dict.getLeftId(id))
            .append('/')
            .append(dict.getRightId(id))
            .append('/')
            .append(dict.getWordCost(id));
      }
      out.println(b);
    }
    MiniConnectionCosts costs = new MiniConnectionCosts(dir);
    for (int f = 0; f < FORWARD_SIZE; f++) {
      StringBuilder b = new StringBuilder("cost " + f + ":");
      for (int bw = 0; bw < BACKWARD_SIZE; bw++) b.append(' ').append(costs.get(f, bw));
      out.println(b);
    }
    MiniCharacterDefinition charDef = new MiniCharacterDefinition(dir);
    for (char c : "aZ09!?x・日\u0000￿ ".toCharArray()) {
      out.println(
          "char "
              + escape(String.valueOf(c))
              + " "
              + charDef.getCharacterClass(c)
              + " "
              + charDef.isInvoke(c)
              + " "
              + charDef.isGroup(c));
    }

    // tokenization
    MiniViterbi viterbi = new MiniViterbi(new MiniTokenInfoFST(fst), dict, costs, charDef);
    for (String input : inputs) {
      out.println("input " + escape(input));
      viterbi.resetBuffer(new StringReader(input));
      viterbi.resetState();
      while (true) {
        List<MiniToken> pending = viterbi.getPending();
        if (pending.isEmpty()) {
          if (viterbi.isEnd()) break;
          viterbi.forward();
          continue;
        }
        MiniToken t = pending.remove(pending.size() - 1);
        out.println(
            "tok "
                + escape(new String(t.getSurfaceForm(), t.getOffset(), t.getLength()))
                + " "
                + t.getStartOffset()
                + "-"
                + t.getEndOffset()
                + " "
                + t.getType()
                + " "
                + t.wordId
                + " "
                + t.getPositionIncrement()
                + " "
                + t.getPositionLength());
      }
    }
    out.flush();
  }
}
