// Prints, one per line in sorted order, ASCIIFoldingFilter.foldToASCII of
// every word of PortugueseAnalyzer.getDefaultStopSet() (Lucene's
// snowball/portuguese_stop.txt): the stop file of the pt-PT chain of
// rucene-cli-base, derived by Lucene 10.5.0 itself for the analysis oracle
// test (tests/analysis_oracle_pt.rs).

import java.util.TreeSet;
import org.apache.lucene.analysis.miscellaneous.ASCIIFoldingFilter;
import org.apache.lucene.analysis.pt.PortugueseAnalyzer;

public class PortugueseStopFolded {
  public static void main(String[] args) {
    TreeSet<String> folded = new TreeSet<>();
    for (Object entry : PortugueseAnalyzer.getDefaultStopSet()) {
      char[] word = (char[]) entry;
      char[] out = new char[4 * word.length];
      int length = ASCIIFoldingFilter.foldToASCII(word, 0, out, 0, word.length);
      folded.add(new String(out, 0, length));
    }
    StringBuilder text = new StringBuilder();
    for (String word : folded) {
      text.append(word).append('\n');
    }
    System.out.print(text);
  }
}
