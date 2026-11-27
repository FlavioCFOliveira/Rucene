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

import com.ibm.icu.text.Collator;
import com.ibm.icu.text.Normalizer2;
import com.ibm.icu.text.Transliterator;
import com.ibm.icu.util.ULocale;
import java.io.FileDescriptor;
import java.io.FileOutputStream;
import java.io.PrintStream;
import java.io.StringReader;
import java.nio.charset.StandardCharsets;
import java.util.Random;
import org.apache.lucene.analysis.Analyzer;
import org.apache.lucene.analysis.TokenStream;
import org.apache.lucene.analysis.Tokenizer;
import org.apache.lucene.analysis.cjk.CJKBigramFilter;
import org.apache.lucene.analysis.core.KeywordTokenizer;
import org.apache.lucene.analysis.icu.ICUCollationKeyAnalyzer;
import org.apache.lucene.analysis.icu.ICUFoldingFilter;
import org.apache.lucene.analysis.icu.ICUNormalizer2CharFilter;
import org.apache.lucene.analysis.icu.ICUNormalizer2Filter;
import org.apache.lucene.analysis.icu.ICUTransformFilter;
import org.apache.lucene.analysis.icu.segmentation.DefaultICUTokenizerConfig;
import org.apache.lucene.analysis.icu.segmentation.ICUTokenizer;
import org.apache.lucene.analysis.icu.tokenattributes.ScriptAttribute;
import org.apache.lucene.analysis.tokenattributes.CharTermAttribute;
import org.apache.lucene.analysis.tokenattributes.OffsetAttribute;
import org.apache.lucene.analysis.tokenattributes.PositionIncrementAttribute;
import org.apache.lucene.analysis.tokenattributes.TermToBytesRefAttribute;
import org.apache.lucene.analysis.tokenattributes.TypeAttribute;
import org.apache.lucene.util.BytesRef;

/**
 * The Java oracle of {@code lucene/analysis/icu} (rmp #642): every ICU tokenizer, filter,
 * normalizer and transform, and {@code ICUCollationKeyAnalyzer}, over a seeded multilingual
 * corpus. Its Rust twin is {@code crates/rucene-analysis-icu/tests/icu_oracle.rs}.
 */
public class AnalysisIcuOracle {
  /** The fragments of the multilingual corpus: one script (or mix) each. */
  static final String[] FRAGMENTS = {
    "The quick brown fox", "don't stop", "O'Reilly's 2.5 km", "résumé naïve café", "Ruß ELİF",
    "Ｔｅｓｔ １２３４", "我是中国人", "定義切分字串的基本單位", "それはまだ実験段階にあります", "カタカナ ﾊﾟﾅｿﾆｯｸ",
    "훈민정음 안녕하세요", "การที่ได้ต้องแสดงว่างานดี", "ພາສາລາວກວ່າດອກ", "ផ្ទះស្កឹមស្កៃបីបួនខ្នងនេះ",
    "သက်ဝင်လှုပ်ရှားစေပြီး", "الفيلم الوثائقي الأول", "ویکی پدیای انگلیسی ۲۵", "דנקנר תקף את הדו\"ח",
    "Γράφεται σε συνεργασία", "Российская Федерация", "💩 👩‍❤️‍👩 🇺🇸🇺🇸", "#️⃣ 3️⃣ 👨🏼‍⚕️", "簡化字 ヒラガナ",
    "℃ № ㈱ ㌘ ｻﾞ", "ﷺ ﴳﴺﰧ", "৭০৬ ๑๒๓๔", "क्‍ष चि", "𐐖𐐾 𩬅艱鍟", "a-b c_d e.f", "  ,;  "
  };

  /** The separators drawn between fragments. */
  static final String[] SEPARATORS = {" ", "", ". ", "\n"};

  /** {@code n} fragments drawn from {@code r}, each followed by a drawn separator but the last. */
  static String text(Random r, int n) {
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < n; i++) {
      if (i > 0) sb.append(SEPARATORS[r.nextInt(SEPARATORS.length)]);
      sb.append(FRAGMENTS[r.nextInt(FRAGMENTS.length)]);
    }
    return sb.toString();
  }

  /** Printable ASCII except the backslash as is, every other UTF-16 unit as a u-escape. */
  static String escape(String s) {
    if (s == null) return "null";
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < s.length(); i++) {
      char c = s.charAt(i);
      if (c >= 0x20 && c <= 0x7e && c != '\\') sb.append(c);
      else sb.append(String.format("\\u%04X", (int) c));
    }
    return sb.toString();
  }

  static String hex(BytesRef b) {
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < b.length; i++) {
      sb.append(String.format("%02x", b.bytes[b.offset + i] & 0xff));
    }
    return sb.toString();
  }

  public static void main(String[] args) throws Exception {
    long seed = Long.parseLong(args[0]);
    String scenario = args[1];
    PrintStream out =
        new PrintStream(new FileOutputStream(FileDescriptor.out), false, StandardCharsets.UTF_8);
    Random r = new Random(seed);
    switch (scenario) {
      case "tokens" -> tokens(r, out);
      case "collation" -> collation(r, out);
      default -> throw new IllegalArgumentException("unknown scenario: " + scenario);
    }
    out.flush();
  }

  /** The analyzer chains of the "tokens" scenario, by name. */
  static final String[] CHAINS = {
    "icu", "icu-nocjk", "icu-myanmar-syllable", "icu-cjkbigram", "folding", "nfkc_cf", "nfkc",
    "nfd", "nfkd", "charfilter-nfkc_cf", "charfilter-nfd"
  };

  /** The transliterator IDs of the "tokens" scenario, each over a keyword token. */
  static final String[] TRANSFORMS = {
    "Any-Latin", "Traditional-Simplified", "Katakana-Hiragana", "Fullwidth-Halfwidth",
    "Cyrillic-Latin", "Han-Latin", "NFD; [:Nonspacing Mark:] Remove", "Latin-ASCII",
    "Greek-Latin", "Arabic-Latin", "Hebrew-Latin", "Thai-Latin", "Any-Upper", "Any-Title"
  };

  static Normalizer2 norm(String name, Normalizer2.Mode mode) {
    return Normalizer2.getInstance(null, name, mode);
  }

  static Analyzer chain(String name) {
    return new Analyzer() {
      @Override
      protected TokenStreamComponents createComponents(String fieldName) {
        switch (name) {
          case "icu":
            return new TokenStreamComponents(new ICUTokenizer());
          case "icu-nocjk":
            return new TokenStreamComponents(
                new ICUTokenizer(new DefaultICUTokenizerConfig(false, true)));
          case "icu-myanmar-syllable":
            return new TokenStreamComponents(
                new ICUTokenizer(new DefaultICUTokenizerConfig(false, false)));
          case "icu-cjkbigram":
            {
              Tokenizer t = new ICUTokenizer(new DefaultICUTokenizerConfig(false, true));
              return new TokenStreamComponents(t, new CJKBigramFilter(t));
            }
          case "folding":
            {
              Tokenizer t = new ICUTokenizer();
              return new TokenStreamComponents(t, new ICUFoldingFilter(t));
            }
          case "nfkc_cf":
            {
              Tokenizer t = new ICUTokenizer();
              return new TokenStreamComponents(t, new ICUNormalizer2Filter(t));
            }
          case "nfkc":
            {
              Tokenizer t = new ICUTokenizer();
              return new TokenStreamComponents(
                  t, new ICUNormalizer2Filter(t, norm("nfkc", Normalizer2.Mode.COMPOSE)));
            }
          case "nfd":
            {
              Tokenizer t = new ICUTokenizer();
              return new TokenStreamComponents(
                  t, new ICUNormalizer2Filter(t, norm("nfc", Normalizer2.Mode.DECOMPOSE)));
            }
          case "nfkd":
            {
              Tokenizer t = new ICUTokenizer();
              return new TokenStreamComponents(
                  t, new ICUNormalizer2Filter(t, norm("nfkc", Normalizer2.Mode.DECOMPOSE)));
            }
          case "charfilter-nfkc_cf":
          case "charfilter-nfd":
            return new TokenStreamComponents(new ICUTokenizer());
          default:
            {
              Tokenizer t = new KeywordTokenizer();
              return new TokenStreamComponents(
                  t, new ICUTransformFilter(t, Transliterator.getInstance(name)));
            }
        }
      }

      @Override
      protected java.io.Reader initReader(String fieldName, java.io.Reader reader) {
        return switch (name) {
          case "charfilter-nfkc_cf" -> new ICUNormalizer2CharFilter(reader);
          case "charfilter-nfd" ->
              new ICUNormalizer2CharFilter(reader, norm("nfc", Normalizer2.Mode.DECOMPOSE));
          default -> reader;
        };
      }
    };
  }

  /** Every token of {@code text} through {@code a}, one line each, then the final offset. */
  static void dump(Analyzer a, String text, PrintStream out) throws Exception {
    try (TokenStream ts = a.tokenStream("f", new StringReader(text))) {
      CharTermAttribute term = ts.addAttribute(CharTermAttribute.class);
      OffsetAttribute offset = ts.addAttribute(OffsetAttribute.class);
      TypeAttribute type = ts.addAttribute(TypeAttribute.class);
      PositionIncrementAttribute posInc = ts.addAttribute(PositionIncrementAttribute.class);
      ScriptAttribute script =
          ts.hasAttribute(ScriptAttribute.class) ? ts.getAttribute(ScriptAttribute.class) : null;
      ts.reset();
      while (ts.incrementToken()) {
        out.println(
            "  "
                + escape(term.toString())
                + " "
                + offset.startOffset()
                + "-"
                + offset.endOffset()
                + " "
                + type.type()
                + " +"
                + posInc.getPositionIncrement()
                + (script == null ? "" : " s" + script.getCode()));
      }
      ts.end();
      out.println("  end " + offset.endOffset());
    }
  }

  /**
   * Forty seeded texts of 1 to 8 fragments, each through every chain of {@link #CHAINS} and every
   * transform of {@link #TRANSFORMS}.
   */
  static void tokens(Random r, PrintStream out) throws Exception {
    Analyzer[] chains = new Analyzer[CHAINS.length + TRANSFORMS.length];
    String[] names = new String[chains.length];
    for (int i = 0; i < CHAINS.length; i++) names[i] = CHAINS[i];
    for (int i = 0; i < TRANSFORMS.length; i++) names[CHAINS.length + i] = TRANSFORMS[i];
    for (int i = 0; i < chains.length; i++) chains[i] = chain(names[i]);
    for (int doc = 0; doc < 40; doc++) {
      String text = text(r, 1 + r.nextInt(8));
      out.println("text " + escape(text));
      for (int i = 0; i < chains.length; i++) {
        out.println(" " + names[i]);
        dump(chains[i], text, out);
      }
    }
    for (Analyzer a : chains) a.close();
  }

  /** The collation locales of the "collation" scenario ("" is the root locale). */
  static final String[] LOCALES = {
    "", "en", "de", "fr", "ja", "zh", "ar", "fa", "sv", "es", "th", "ko", "ru", "he", "el",
    "de@collation=phonebook", "zh@collation=stroke"
  };

  /**
   * For every locale of {@link #LOCALES} and strength (default, PRIMARY, IDENTICAL): the hex
   * collation key of twenty seeded texts through {@code ICUCollationKeyAnalyzer}.
   */
  static void collation(Random r, PrintStream out) throws Exception {
    String[] texts = new String[20];
    for (int i = 0; i < texts.length; i++) texts[i] = text(r, 1 + r.nextInt(3));
    int[] strengths = {-1, Collator.PRIMARY, Collator.IDENTICAL};
    for (String locale : LOCALES) {
      for (int strength : strengths) {
        Collator collator = Collator.getInstance(new ULocale(locale));
        if (strength >= 0) collator.setStrength(strength);
        out.println("locale " + locale + " strength " + strength);
        Analyzer a = new ICUCollationKeyAnalyzer(collator);
        for (String text : texts) {
          try (TokenStream ts = a.tokenStream("f", text)) {
            TermToBytesRefAttribute bytes = ts.addAttribute(TermToBytesRefAttribute.class);
            ts.reset();
            while (ts.incrementToken()) {
              out.println("  " + hex(bytes.getBytesRef()));
            }
            ts.end();
          }
        }
        a.close();
      }
    }
  }
}
