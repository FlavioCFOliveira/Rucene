/*
 * Writes crates/rucene-test-framework/src/jdk/time_zone_data.rs: the rules
 * java.util.TimeZone applies, on the running JDK (25), for every ID of
 * TimeZone.getAvailableIDs() — the IDs LuceneTestCase.randomTimeZone draws
 * from, and the deprecated three-letter ones it removes.
 *
 * TimeZone.getTimeZone(id) is a sun.util.calendar.ZoneInfo, the form
 * GregorianCalendar reads: ZoneInfoFile derives it from the tzdb rules
 * (java.time.zone.ZoneRules) as a transition table bounded to 1900..2037
 * plus, for zones that still observe daylight saving time, a SimpleTimeZone
 * "last rule" for the instants beyond the table. java.time.zone.ZoneRules
 * itself gives a different hour for some zones (measured: Africa/Casablanca,
 * Africa/El_Aaiun, Africa/Windhoek, Asia/Gaza, Asia/Hebron), so this
 * generator records ZoneInfo's own fields, read reflectively, and the last
 * rule's SimpleTimeZone fields as its constructor decoded them.
 *
 * Usage (JDK 25):
 *   java --add-opens java.base/sun.util.calendar=ALL-UNNAMED \
 *        --add-opens java.base/java.util=ALL-UNNAMED \
 *        tools/gen/TimeZoneRules.java [OUT.rs] [--check]
 * OUT.rs defaults to crates/rucene-test-framework/src/jdk/time_zone_data.rs;
 * --check compares instead of writing and exits 1 on a difference.
 */

import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.SimpleTimeZone;
import java.util.TimeZone;

public class TimeZoneRules {
  private static final String[] STZ_FIELDS = {
    "rawOffset", "dstSavings", "startYear",
    "startMode", "startMonth", "startDay", "startDayOfWeek", "startTime", "startTimeMode",
    "endMode", "endMonth", "endDay", "endDayOfWeek", "endTime", "endTimeMode"
  };

  private static Object get(Object o, Class<?> c, String name) throws Exception {
    Field f = c.getDeclaredField(name);
    f.setAccessible(true);
    return f.get(o);
  }

  public static void main(String[] args) throws Exception {
    Path out = Path.of("crates/rucene-test-framework/src/jdk/time_zone_data.rs");
    boolean check = false;
    for (String a : args) {
      if (a.equals("--check")) check = true;
      else out = Path.of(a);
    }
    Class<?> zoneInfo = Class.forName("sun.util.calendar.ZoneInfo");
    // One entry per distinct rule set, in first-seen order; IDs refer to it.
    Map<String, Integer> ruleIndex = new LinkedHashMap<>();
    List<String> rules = new ArrayList<>();
    StringBuilder ids = new StringBuilder();
    String[] available = TimeZone.getAvailableIDs();
    // The order of TimeZone.getAvailableIDs() is not stable for the legacy
    // three-letter IDs; the table is keyed by ID, sorted by UTF-16 units.
    String[] sorted = available.clone();
    Arrays.sort(sorted);
    for (String id : sorted) {
      TimeZone tz = TimeZone.getTimeZone(id);
      if (!zoneInfo.isInstance(tz)) {
        throw new IllegalStateException(id + " is a " + tz.getClass());
      }
      int rawOffset = (Integer) get(tz, zoneInfo, "rawOffset");
      int rawOffsetDiff = (Integer) get(tz, zoneInfo, "rawOffsetDiff");
      if (rawOffsetDiff != 0) {
        throw new IllegalStateException(id + " rawOffsetDiff=" + rawOffsetDiff);
      }
      long[] transitions = (long[]) get(tz, zoneInfo, "transitions");
      int[] offsets = (int[]) get(tz, zoneInfo, "offsets");
      SimpleTimeZone last =
          (SimpleTimeZone) zoneInfo.getMethod("getLastRuleInstance").invoke(tz);
      StringBuilder r = new StringBuilder();
      r.append("    ZoneInfo {\n        raw_offset: ").append(rawOffset).append(",\n");
      r.append("        transitions: ");
      if (transitions == null) {
        r.append("None,\n");
      } else {
        r.append("Some(&[");
        for (int i = 0; i < transitions.length; i++) {
          r.append(i % 4 == 0 ? "\n            " : " ");
          r.append(transitions[i]).append(',');
        }
        r.append("\n        ]),\n");
      }
      r.append("        offsets: &[");
      if (offsets != null) {
        for (int i = 0; i < offsets.length; i++) {
          r.append(i == 0 ? "" : ", ").append(offsets[i]);
        }
      }
      r.append("],\n        last_rule: ");
      if (last == null) {
        r.append("None,\n");
      } else {
        r.append("Some(SimpleTimeZone {\n");
        r.append("            use_daylight: ")
            .append(get(last, SimpleTimeZone.class, "useDaylight"))
            .append(",\n");
        for (String f : STZ_FIELDS) {
          r.append("            ")
              .append(f.replaceAll("([A-Z])", "_$1").toLowerCase())
              .append(": ")
              .append(get(last, SimpleTimeZone.class, f))
              .append(",\n");
        }
        r.append("        }),\n");
      }
      r.append("    },\n");
      String body = r.toString();
      Integer index = ruleIndex.get(body);
      if (index == null) {
        index = rules.size();
        ruleIndex.put(body, index);
        rules.add(body);
      }
      ids.append("    (\"").append(id).append("\", ").append(index).append("),\n");
    }
    StringBuilder rs = new StringBuilder();
    rs.append("// @generated by tools/gen/TimeZoneRules.java on ")
        .append(System.getProperty("java.vm.name"))
        .append(' ')
        .append(Runtime.version().feature())
        .append(" (tzdata ")
        .append(java.time.zone.ZoneRulesProvider.getVersions("UTC").lastKey())
        .append("). Do not edit.\n\n");
    rs.append("//! The `sun.util.calendar.ZoneInfo` of every ID of JDK 25's\n");
    rs.append("//! `TimeZone.getAvailableIDs()`, as `TimeZone.getTimeZone(id)` builds it.\n\n");
    rs.append("use super::time_zone::{SimpleTimeZone, ZoneInfo};\n\n");
    rs.append("/// The distinct rule sets, referred to by [`IDS`].\n");
    rs.append("pub(super) static RULES: &[ZoneInfo] = &[\n");
    for (String body : rules) rs.append(body);
    rs.append("];\n\n");
    rs.append("/// Every available ID, sorted by UTF-16 code units, with its index in [`RULES`].\n");
    rs.append("pub(super) static IDS: &[(&str, usize)] = &[\n").append(ids).append("];\n");
    byte[] bytes = rs.toString().getBytes(StandardCharsets.UTF_8);
    if (check) {
      byte[] old = Files.exists(out) ? Files.readAllBytes(out) : new byte[0];
      if (!Arrays.equals(old, bytes)) {
        System.err.println(out + " differs from the JDK's rules");
        System.exit(1);
      }
      System.err.println(out + " is current: " + available.length + " IDs, " + rules.size() + " rule sets");
    } else {
      Files.write(out, bytes);
      System.err.println("wrote " + out + ": " + available.length + " IDs, " + rules.size() + " rule sets");
    }
  }
}
