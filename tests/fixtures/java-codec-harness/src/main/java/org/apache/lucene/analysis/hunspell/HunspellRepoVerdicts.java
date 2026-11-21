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

package org.apache.lucene.analysis.hunspell;

import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.TreeSet;
import java.util.stream.Collectors;
import java.util.stream.Stream;
import org.apache.lucene.store.ByteBuffersDirectory;

/**
 * Lucene's verdict on each test case of a Hunspell repository checkout: the checks of {@code
 * TestSpellChecking.checkSpellCheckerExpectations} (Lucene 10.5.0, test sources), which {@code
 * TestHunspellRepositoryTestCases} runs per case, reproduced here on the main classes because the
 * test classes are not in a published jar. Usage: {@code HunspellRepoVerdicts <repo> [<case>...]};
 * prints {@code <case> ok} or {@code <case> FAIL <simple class>: <message>} per case, in name
 * order.
 */
public final class HunspellRepoVerdicts {

  private HunspellRepoVerdicts() {}

  static void fail(String message) {
    throw new AssertionError(message);
  }

  static void check(Path basePath) throws Exception {
    check(basePath, SortingStrategy.offline(new ByteBuffersDirectory(), "dictionary"));
    check(basePath, SortingStrategy.inMemory());
  }

  static void check(Path basePath, SortingStrategy strategy) throws Exception {
    Path affFile = Path.of(basePath + ".aff");
    Path dicFile = Path.of(basePath + ".dic");
    Hunspell speller;
    Map<String, Suggester> suggesters = new LinkedHashMap<>();
    try (InputStream affixStream = Files.newInputStream(affFile);
        InputStream dictStream = Files.newInputStream(dicFile)) {
      Dictionary dictionary = new Dictionary(affixStream, List.of(dictStream), false, strategy);
      speller = new Hunspell(dictionary, TimeoutPolicy.NO_TIMEOUT, () -> {});
      Suggester suggester = new Suggester(dictionary);
      suggesters.put("default", suggester);
      suggesters.put("caching", suggester.withSuggestibleEntryCache());
      if (dictionary.compoundRules == null
          && dictionary.compoundBegin == Dictionary.FLAG_UNSET
          && dictionary.compoundFlag == Dictionary.FLAG_UNSET) {
        for (int n = 2; n <= 4; n++) {
          var checker = NGramFragmentChecker.fromAllSimpleWords(n, dictionary, () -> {});
          suggesters.put("ngram" + n, suggester.withFragmentChecker(checker));
        }
      }
    }

    Path good = Path.of(basePath + ".good");
    if (Files.exists(good)) {
      for (String word : Files.readAllLines(good)) {
        if (!speller.spell(word.trim())) fail("Unexpectedly considered misspelled: " + word);
      }
    }

    Path wrong = Path.of(basePath + ".wrong");
    Path sug = Path.of(basePath + ".sug");
    if (Files.exists(wrong)) {
      List<String> wrongWords = Files.readAllLines(wrong);
      for (String word : wrongWords) {
        if (speller.spell(word.trim())) fail("Unexpectedly considered correct: " + word);
      }
      if (Files.exists(sug)) {
        String sugLines = Files.readString(sug).trim();
        for (Map.Entry<String, Suggester> e : suggesters.entrySet()) {
          String actual =
              wrongWords.stream()
                  .map(s -> String.join(", ", e.getValue().suggestNoTimeout(s, () -> {})))
                  .filter(s -> !s.isEmpty())
                  .collect(Collectors.joining("\n"));
          if (!sugLines.equals(actual)) fail("Suggester=" + e.getKey());
        }
      }
    } else if (Files.exists(sug)) {
      fail(".sug file without .wrong file!");
    }

    Set<String> everythingGenerated = new HashSet<>();
    boolean generatedEverything = true;
    try (Stream<String> lines = Files.lines(dicFile, speller.dictionary.decoder.charset())) {
      for (String line : lines.skip(1).toList()) {
        int len = (int) line.chars().takeWhile(c -> !Character.isWhitespace(c) && c != '/').count();
        String word = line.substring(0, len).trim();
        if (word.isEmpty() || word.contains("\\")) {
          generatedEverything = false;
          continue;
        }
        List<AffixedWord> expanded = speller.getAllWordForms(word);
        for (AffixedWord w : expanded) {
          if (!speller.spell(w.getWord()) || speller.analyzeSimpleWord(w.getWord()).isEmpty()) {
            fail("Misspelled words generated in " + dicFile);
          }
        }
        if (expanded.stream().anyMatch(e -> e.getWord().equals(word))) {
          EntrySuggestion suggestion =
              speller.compress(expanded.stream().map(AffixedWord::getWord).toList());
          if (suggestion != null
              && suggestion.getEntriesToEdit().stream().noneMatch(e -> e.getStem().equals(word))) {
            fail("Compression suggests a different stem from the original " + word);
          }
        }
        expanded.forEach(w -> everythingGenerated.add(w.getWord().toLowerCase(Locale.ROOT)));
      }
    }
    if (generatedEverything && !speller.dictionary.mayNeedInputCleaning()) {
      Set<String> goodWords = new HashSet<>();
      if (Files.exists(good)) {
        Files.readAllLines(good).stream().map(String::trim).forEach(goodWords::add);
      }
      if (Files.exists(sug)) {
        Files.readAllLines(sug).stream()
            .flatMap(line -> Stream.of(line.split(", ")))
            .map(String::trim)
            .filter(s -> !s.contains(" "))
            .forEach(goodWords::add);
      }
      goodWords.removeAll(everythingGenerated);
      goodWords.removeIf(s -> !s.equals(s.toLowerCase(Locale.ROOT)));
      goodWords.removeIf(s -> speller.analyzeSimpleWord(s).isEmpty());
      if (!goodWords.isEmpty()) fail("Some *.good/sug words weren't generated: " + goodWords);
    }
  }

  public static void main(String[] args) throws Exception {
    Path tests = Path.of(args[0]).resolve("tests");
    Set<String> names = new TreeSet<>();
    if (args.length > 1) {
      names.addAll(List.of(args).subList(1, args.length));
    } else {
      try (var files = Files.newDirectoryStream(tests)) {
        for (Path file : files) {
          String name = file.getFileName().toString();
          if (name.endsWith(".aff")) names.add(name.substring(0, name.length() - 4));
        }
      }
    }
    List<String> out = new ArrayList<>();
    for (String name : names) {
      String verdict;
      try {
        check(tests.resolve(name));
        verdict = "ok";
      } catch (Throwable t) {
        String msg = String.valueOf(t.getMessage()).lines().findFirst().orElse("");
        verdict = "FAIL " + t.getClass().getSimpleName() + ": " + msg;
      }
      System.out.println(name + " " + verdict);
      System.out.flush();
    }
  }
}
