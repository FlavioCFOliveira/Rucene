// Access to the package-private RamUsageEstimator for RunnerPartsOracle.
package com.carrotsearch.randomizedtesting.rules;

public final class RamOracle {
  private RamOracle() {}
  public static String human(long bytes) { return RamUsageEstimator.humanReadableUnits(bytes); }
  public static String layout() {
    return RamUsageEstimator.NUM_BYTES_OBJECT_REF + " " + RamUsageEstimator.NUM_BYTES_OBJECT_HEADER + " "
        + RamUsageEstimator.NUM_BYTES_ARRAY_HEADER + " " + RamUsageEstimator.NUM_BYTES_OBJECT_ALIGNMENT;
  }
  public static long sizeOf(Object o) { return RamUsageEstimator.sizeOf(o); }
  public static long align(long v) { return RamUsageEstimator.alignObjectSize(v); }
}
