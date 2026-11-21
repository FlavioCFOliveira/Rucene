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

//! Port of the test classes of Lucene 10.5.0 `lucene/analysis/common/src/test`
//! that rmp task #465 lists, each run as a suite by the ported runner:
//! `org.apache.lucene.analysis.email.TestUAX29URLEmailAnalyzer`,
//! `TestUAX29URLEmailTokenizer` and `TestUAX29URLEmailTokenizerFactory`, with
//! the package's resources (`LuceneResourcesWikiPage.html`, its URL list, the
//! random texts with URLs and e-mail addresses and their lists, `TLDs.txt`)
//! copied byte for byte, and the test framework's
//! `WordBreakTestUnicode_12_1_0` and `EmojiTokenizationTestUnicode_12_1`.
//!
//! `TestUAX29URLEmailTokenizer.testLongEMAILatomText` is `@Nightly`: Lucene
//! runs it only with `tests.nightly=true`. The port runs it in every run, in
//! its own suite of the class without the annotation
//! ([`test_uax29_url_email_tokenizer_nightly`]): setting `tests.nightly`
//! would change process-wide state (`TEST_NIGHTLY` is read once), which a
//! test run in parallel with others must not do.

mod support;

use std::cell::RefCell;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::tokenattributes::TypeAttribute;
use rucene::analysis::{
    Analyzer, CharReader, StringCharReader, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::Result;
use rucene::util::AttributeSource;
use rucene_analysis_common::analysis::email::uax29_url_email_analyzer::{
    UAX29URLEmailAnalyzer, DEFAULT_MAX_TOKEN_LENGTH,
};
use rucene_analysis_common::analysis::email::uax29_url_email_tokenizer::{
    new_uax29_url_email_tokenizer, new_uax29_url_email_tokenizer_with_factory, EMAIL,
    MAX_TOKEN_LENGTH_LIMIT, TOKEN_TYPES, URL,
};
use rucene_analysis_common::jdk::io::InputStreamReader;
use rucene_test_framework::analysis::standard::emoji_tokenization_test_unicode_12_1::EmojiTokenizationTestUnicode_12_1;
use rucene_test_framework::analysis::standard::word_break_test_unicode_12_1_0::WordBreakTestUnicode_12_1_0;
use rucene_test_framework::analysis::{
    components, shared, sink, tokenizer_components, BaseTokenStreamFactoryTestCase,
    BaseTokenStreamTestCase,
};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::RANDOM_MULTIPLIER;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};
use support::{btstc_class, factory_class, hook, own, run, t, FnAnalyzer};

type B = BaseTokenStreamTestCase;
type F = BaseTokenStreamFactoryTestCase;

/// The bytes of a test resource of the `email` package.
fn resource(name: &str) -> &'static [u8] {
    let full = format!("org/apache/lucene/analysis/email/{name}");
    support::TEST_RESOURCES
        .iter()
        .find(|(n, _)| *n == full)
        .unwrap_or_else(|| panic!("resource {full}"))
        .1
}

/// `" ".repeat(4094) + "testing 1234"`.
fn huge_doc_input() -> String {
    format!("{}testing 1234", " ".repeat(4094))
}

/// Reads the whole UTF-8 resource as Java's `read(char[])` loop does.
fn read_resource(name: &str) -> Result<String> {
    let mut reader = InputStreamReader::new_utf8(resource(name));
    let mut units: Vec<u16> = Vec::new();
    let mut buffer = [0u16; 1024];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        units.extend_from_slice(&buffer[..n]);
    }
    reader.close()?;
    Ok(String::from_utf16_lossy(&units))
}

/// The trimmed, non-empty lines of a UTF-8 resource (`BufferedReader.readLine`
/// then `String.trim()`).
fn resource_lines(name: &str) -> Result<Vec<String>> {
    let text = read_resource(name)?;
    let mut lines = Vec::new();
    // `readLine` ends a line at "\n", "\r" or "\r\n"
    for line in text.split('\n').flat_map(|l| l.split('\r')) {
        let line = line.trim_matches(|c: char| c <= ' ');
        if !line.is_empty() {
            lines.push(line.to_string());
        }
    }
    Ok(lines)
}

// -----------------------------------------------------------------------------
// TestUAX29URLEmailAnalyzer
// -----------------------------------------------------------------------------

thread_local! {
    /// The analyzer fields `setUp` sets: `a` (and, for the tokenizer test,
    /// `urlAnalyzer` and `emailAnalyzer`).
    static FIELDS: RefCell<Vec<Arc<dyn Analyzer>>> = const { RefCell::new(Vec::new()) };
}

fn field(i: usize) -> Arc<dyn Analyzer> {
    FIELDS.with(|f| Arc::clone(&f.borrow()[i]))
}

fn close_fields_tear_down() -> Result<()> {
    for a in FIELDS.with(|f| std::mem::take(&mut *f.borrow_mut())) {
        a.close();
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}

fn ea_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    let a: Arc<dyn Analyzer> = Arc::new(UAX29URLEmailAnalyzer::new());
    FIELDS.with(|f| *f.borrow_mut() = vec![a]);
    Ok(())
}

fn ea_test_huge_doc() -> Result<()> {
    let a = field(0);
    let input = huge_doc_input();
    B::assert_analyzes_to(&*a, &input, &["testing", "1234"])?;
    Ok(())
}

fn ea_test_armenian() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "\u{54E}\u{56B}\u{584}\u{56B}\u{57A}\u{565}\u{564}\u{56B}\u{561}\u{575}\u{56B} 13 \u{574}\u{56B}\u{56C}\u{56B}\u{578}\u{576} \u{570}\u{578}\u{564}\u{57E}\u{561}\u{56E}\u{576}\u{565}\u{580}\u{568} (4,600` \u{570}\u{561}\u{575}\u{565}\u{580}\u{565}\u{576} \u{57E}\u{56B}\u{584}\u{56B}\u{57A}\u{565}\u{564}\u{56B}\u{561}\u{575}\u{578}\u{582}\u{574}) \u{563}\u{580}\u{57E}\u{565}\u{56C} \u{565}\u{576} \u{56F}\u{561}\u{574}\u{561}\u{57E}\u{578}\u{580}\u{576}\u{565}\u{580}\u{56B} \u{56F}\u{578}\u{572}\u{574}\u{56B}\u{581} \u{578}\u{582} \u{570}\u{561}\u{574}\u{561}\u{580}\u{575}\u{561} \u{562}\u{578}\u{56C}\u{578}\u{580} \u{570}\u{578}\u{564}\u{57E}\u{561}\u{56E}\u{576}\u{565}\u{580}\u{568} \u{56F}\u{561}\u{580}\u{578}\u{572} \u{567} \u{56D}\u{574}\u{562}\u{561}\u{563}\u{580}\u{565}\u{56C} \u{581}\u{561}\u{576}\u{56F}\u{561}\u{581} \u{574}\u{561}\u{580}\u{564} \u{578}\u{57E} \u{56F}\u{561}\u{580}\u{578}\u{572} \u{567} \u{562}\u{561}\u{581}\u{565}\u{56C} \u{54E}\u{56B}\u{584}\u{56B}\u{57A}\u{565}\u{564}\u{56B}\u{561}\u{575}\u{56B} \u{56F}\u{561}\u{575}\u{584}\u{568}\u{589}", &["\u{57E}\u{56B}\u{584}\u{56B}\u{57A}\u{565}\u{564}\u{56B}\u{561}\u{575}\u{56B}", "13", "\u{574}\u{56B}\u{56C}\u{56B}\u{578}\u{576}", "\u{570}\u{578}\u{564}\u{57E}\u{561}\u{56E}\u{576}\u{565}\u{580}\u{568}", "4,600", "\u{570}\u{561}\u{575}\u{565}\u{580}\u{565}\u{576}", "\u{57E}\u{56B}\u{584}\u{56B}\u{57A}\u{565}\u{564}\u{56B}\u{561}\u{575}\u{578}\u{582}\u{574}", "\u{563}\u{580}\u{57E}\u{565}\u{56C}", "\u{565}\u{576}", "\u{56F}\u{561}\u{574}\u{561}\u{57E}\u{578}\u{580}\u{576}\u{565}\u{580}\u{56B}", "\u{56F}\u{578}\u{572}\u{574}\u{56B}\u{581}", "\u{578}\u{582}", "\u{570}\u{561}\u{574}\u{561}\u{580}\u{575}\u{561}", "\u{562}\u{578}\u{56C}\u{578}\u{580}", "\u{570}\u{578}\u{564}\u{57E}\u{561}\u{56E}\u{576}\u{565}\u{580}\u{568}", "\u{56F}\u{561}\u{580}\u{578}\u{572}", "\u{567}", "\u{56D}\u{574}\u{562}\u{561}\u{563}\u{580}\u{565}\u{56C}", "\u{581}\u{561}\u{576}\u{56F}\u{561}\u{581}", "\u{574}\u{561}\u{580}\u{564}", "\u{578}\u{57E}", "\u{56F}\u{561}\u{580}\u{578}\u{572}", "\u{567}", "\u{562}\u{561}\u{581}\u{565}\u{56C}", "\u{57E}\u{56B}\u{584}\u{56B}\u{57A}\u{565}\u{564}\u{56B}\u{561}\u{575}\u{56B}", "\u{56F}\u{561}\u{575}\u{584}\u{568}"])?;
    Ok(())
}

fn ea_test_amharic() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "\u{12CA}\u{12AA}\u{1354}\u{12F5}\u{12EB} \u{12E8}\u{1263}\u{1208} \u{1265}\u{12D9} \u{124B}\u{1295}\u{124B} \u{12E8}\u{1270}\u{121F}\u{120B} \u{1275}\u{12AD}\u{12AD}\u{1208}\u{129B}\u{1293} \u{1290}\u{133B} \u{1218}\u{12DD}\u{1308}\u{1260} \u{12D5}\u{12CD}\u{1240}\u{1275} (\u{12A2}\u{1295}\u{1233}\u{12ED}\u{12AD}\u{120E}\u{1352}\u{12F2}\u{12EB}) \u{1290}\u{12CD}\u{1362} \u{121B}\u{1295}\u{129B}\u{12CD}\u{121D}", &["\u{12CA}\u{12AA}\u{1354}\u{12F5}\u{12EB}", "\u{12E8}\u{1263}\u{1208}", "\u{1265}\u{12D9}", "\u{124B}\u{1295}\u{124B}", "\u{12E8}\u{1270}\u{121F}\u{120B}", "\u{1275}\u{12AD}\u{12AD}\u{1208}\u{129B}\u{1293}", "\u{1290}\u{133B}", "\u{1218}\u{12DD}\u{1308}\u{1260}", "\u{12D5}\u{12CD}\u{1240}\u{1275}", "\u{12A2}\u{1295}\u{1233}\u{12ED}\u{12AD}\u{120E}\u{1352}\u{12F2}\u{12EB}", "\u{1290}\u{12CD}", "\u{121B}\u{1295}\u{129B}\u{12CD}\u{121D}"])?;
    Ok(())
}

fn ea_test_arabic() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "\u{627}\u{644}\u{641}\u{64A}\u{644}\u{645} \u{627}\u{644}\u{648}\u{62B}\u{627}\u{626}\u{642}\u{64A} \u{627}\u{644}\u{623}\u{648}\u{644} \u{639}\u{646} \u{648}\u{64A}\u{643}\u{64A}\u{628}\u{64A}\u{62F}\u{64A}\u{627} \u{64A}\u{633}\u{645}\u{649} \"\u{627}\u{644}\u{62D}\u{642}\u{64A}\u{642}\u{629} \u{628}\u{627}\u{644}\u{623}\u{631}\u{642}\u{627}\u{645}: \u{642}\u{635}\u{629} \u{648}\u{64A}\u{643}\u{64A}\u{628}\u{64A}\u{62F}\u{64A}\u{627}\" (\u{628}\u{627}\u{644}\u{625}\u{646}\u{62C}\u{644}\u{64A}\u{632}\u{64A}\u{629}: Truth in Numbers: The Wikipedia Story)\u{60C} \u{633}\u{64A}\u{62A}\u{645} \u{625}\u{637}\u{644}\u{627}\u{642}\u{647} \u{641}\u{64A} 2008.", &["\u{627}\u{644}\u{641}\u{64A}\u{644}\u{645}", "\u{627}\u{644}\u{648}\u{62B}\u{627}\u{626}\u{642}\u{64A}", "\u{627}\u{644}\u{623}\u{648}\u{644}", "\u{639}\u{646}", "\u{648}\u{64A}\u{643}\u{64A}\u{628}\u{64A}\u{62F}\u{64A}\u{627}", "\u{64A}\u{633}\u{645}\u{649}", "\u{627}\u{644}\u{62D}\u{642}\u{64A}\u{642}\u{629}", "\u{628}\u{627}\u{644}\u{623}\u{631}\u{642}\u{627}\u{645}", "\u{642}\u{635}\u{629}", "\u{648}\u{64A}\u{643}\u{64A}\u{628}\u{64A}\u{62F}\u{64A}\u{627}", "\u{628}\u{627}\u{644}\u{625}\u{646}\u{62C}\u{644}\u{64A}\u{632}\u{64A}\u{629}", "truth", "numbers", "wikipedia", "story", "\u{633}\u{64A}\u{62A}\u{645}", "\u{625}\u{637}\u{644}\u{627}\u{642}\u{647}", "\u{641}\u{64A}", "2008"])?;
    Ok(())
}

fn ea_test_aramaic() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "\u{718}\u{71D}\u{729}\u{71D}\u{726}\u{715}\u{71D}\u{710} (\u{710}\u{722}\u{713}\u{720}\u{71D}\u{710}: Wikipedia) \u{717}\u{718} \u{710}\u{71D}\u{722}\u{723}\u{729}\u{720}\u{718}\u{726}\u{715}\u{71D}\u{710} \u{71A}\u{710}\u{72A}\u{72C}\u{710} \u{715}\u{710}\u{722}\u{71B}\u{72A}\u{722}\u{71B} \u{712}\u{720}\u{72B}\u{722}\u{308}\u{710} \u{723}\u{713}\u{71D}\u{710}\u{308}\u{710}\u{702} \u{72B}\u{721}\u{717} \u{710}\u{72C}\u{710} \u{721}\u{722} \u{721}\u{308}\u{720}\u{72C}\u{710} \u{715}\"\u{718}\u{71D}\u{729}\u{71D}\" \u{718}\"\u{710}\u{71D}\u{722}\u{723}\u{729}\u{720}\u{718}\u{726}\u{715}\u{71D}\u{710}\"\u{700}", &["\u{718}\u{71D}\u{729}\u{71D}\u{726}\u{715}\u{71D}\u{710}", "\u{710}\u{722}\u{713}\u{720}\u{71D}\u{710}", "wikipedia", "\u{717}\u{718}", "\u{710}\u{71D}\u{722}\u{723}\u{729}\u{720}\u{718}\u{726}\u{715}\u{71D}\u{710}", "\u{71A}\u{710}\u{72A}\u{72C}\u{710}", "\u{715}\u{710}\u{722}\u{71B}\u{72A}\u{722}\u{71B}", "\u{712}\u{720}\u{72B}\u{722}\u{308}\u{710}", "\u{723}\u{713}\u{71D}\u{710}\u{308}\u{710}", "\u{72B}\u{721}\u{717}", "\u{710}\u{72C}\u{710}", "\u{721}\u{722}", "\u{721}\u{308}\u{720}\u{72C}\u{710}", "\u{715}", "\u{718}\u{71D}\u{729}\u{71D}", "\u{718}", "\u{710}\u{71D}\u{722}\u{723}\u{729}\u{720}\u{718}\u{726}\u{715}\u{71D}\u{710}"])?;
    Ok(())
}

fn ea_test_bengali() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "\u{98F}\u{987} \u{9AC}\u{9BF}\u{9B6}\u{9CD}\u{9AC}\u{995}\u{9CB}\u{9B7} \u{9AA}\u{9B0}\u{9BF}\u{99A}\u{9BE}\u{9B2}\u{9A8}\u{9BE} \u{995}\u{9B0}\u{9C7} \u{989}\u{987}\u{995}\u{9BF}\u{9AE}\u{9BF}\u{9A1}\u{9BF}\u{9AF}\u{9BC}\u{9BE} \u{9AB}\u{9BE}\u{989}\u{9A8}\u{9CD}\u{9A1}\u{9C7}\u{9B6}\u{9A8} (\u{98F}\u{995}\u{99F}\u{9BF} \u{985}\u{9B2}\u{9BE}\u{9AD}\u{99C}\u{9A8}\u{995} \u{9B8}\u{982}\u{9B8}\u{9CD}\u{9A5}\u{9BE})\u{964} \u{989}\u{987}\u{995}\u{9BF}\u{9AA}\u{9BF}\u{9A1}\u{9BF}\u{9AF}\u{9BC}\u{9BE}\u{9B0} \u{9B6}\u{9C1}\u{9B0}\u{9C1} \u{9E7}\u{9EB} \u{99C}\u{9BE}\u{9A8}\u{9C1}\u{9AF}\u{9BC}\u{9BE}\u{9B0}\u{9BF}, \u{9E8}\u{9E6}\u{9E6}\u{9E7} \u{9B8}\u{9BE}\u{9B2}\u{9C7}\u{964} \u{98F}\u{996}\u{9A8} \u{9AA}\u{9B0}\u{9CD}\u{9AF}\u{9A8}\u{9CD}\u{9A4} \u{9E8}\u{9E6}\u{9E6}\u{99F}\u{9BF}\u{9B0}\u{993} \u{9AC}\u{9C7}\u{9B6}\u{9C0} \u{9AD}\u{9BE}\u{9B7}\u{9BE}\u{9AF}\u{9BC} \u{989}\u{987}\u{995}\u{9BF}\u{9AA}\u{9BF}\u{9A1}\u{9BF}\u{9AF}\u{9BC}\u{9BE} \u{9B0}\u{9AF}\u{9BC}\u{9C7}\u{99B}\u{9C7}\u{964}", &["\u{98F}\u{987}", "\u{9AC}\u{9BF}\u{9B6}\u{9CD}\u{9AC}\u{995}\u{9CB}\u{9B7}", "\u{9AA}\u{9B0}\u{9BF}\u{99A}\u{9BE}\u{9B2}\u{9A8}\u{9BE}", "\u{995}\u{9B0}\u{9C7}", "\u{989}\u{987}\u{995}\u{9BF}\u{9AE}\u{9BF}\u{9A1}\u{9BF}\u{9AF}\u{9BC}\u{9BE}", "\u{9AB}\u{9BE}\u{989}\u{9A8}\u{9CD}\u{9A1}\u{9C7}\u{9B6}\u{9A8}", "\u{98F}\u{995}\u{99F}\u{9BF}", "\u{985}\u{9B2}\u{9BE}\u{9AD}\u{99C}\u{9A8}\u{995}", "\u{9B8}\u{982}\u{9B8}\u{9CD}\u{9A5}\u{9BE}", "\u{989}\u{987}\u{995}\u{9BF}\u{9AA}\u{9BF}\u{9A1}\u{9BF}\u{9AF}\u{9BC}\u{9BE}\u{9B0}", "\u{9B6}\u{9C1}\u{9B0}\u{9C1}", "\u{9E7}\u{9EB}", "\u{99C}\u{9BE}\u{9A8}\u{9C1}\u{9AF}\u{9BC}\u{9BE}\u{9B0}\u{9BF}", "\u{9E8}\u{9E6}\u{9E6}\u{9E7}", "\u{9B8}\u{9BE}\u{9B2}\u{9C7}", "\u{98F}\u{996}\u{9A8}", "\u{9AA}\u{9B0}\u{9CD}\u{9AF}\u{9A8}\u{9CD}\u{9A4}", "\u{9E8}\u{9E6}\u{9E6}\u{99F}\u{9BF}\u{9B0}\u{993}", "\u{9AC}\u{9C7}\u{9B6}\u{9C0}", "\u{9AD}\u{9BE}\u{9B7}\u{9BE}\u{9AF}\u{9BC}", "\u{989}\u{987}\u{995}\u{9BF}\u{9AA}\u{9BF}\u{9A1}\u{9BF}\u{9AF}\u{9BC}\u{9BE}", "\u{9B0}\u{9AF}\u{9BC}\u{9C7}\u{99B}\u{9C7}"])?;
    Ok(())
}

fn ea_test_farsi() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "\u{648}\u{6CC}\u{6A9}\u{6CC} \u{67E}\u{62F}\u{6CC}\u{627}\u{6CC} \u{627}\u{646}\u{6AF}\u{644}\u{6CC}\u{633}\u{6CC} \u{62F}\u{631} \u{62A}\u{627}\u{631}\u{6CC}\u{62E} \u{6F2}\u{6F5} \u{62F}\u{6CC} \u{6F1}\u{6F3}\u{6F7}\u{6F9} \u{628}\u{647} \u{635}\u{648}\u{631}\u{62A} \u{645}\u{6A9}\u{645}\u{644}\u{6CC} \u{628}\u{631}\u{627}\u{6CC} \u{62F}\u{627}\u{646}\u{634}\u{646}\u{627}\u{645}\u{647}\u{654} \u{62A}\u{62E}\u{635}\u{635}\u{6CC} \u{646}\u{648}\u{67E}\u{62F}\u{6CC}\u{627} \u{646}\u{648}\u{634}\u{62A}\u{647} \u{634}\u{62F}.", &["\u{648}\u{6CC}\u{6A9}\u{6CC}", "\u{67E}\u{62F}\u{6CC}\u{627}\u{6CC}", "\u{627}\u{646}\u{6AF}\u{644}\u{6CC}\u{633}\u{6CC}", "\u{62F}\u{631}", "\u{62A}\u{627}\u{631}\u{6CC}\u{62E}", "\u{6F2}\u{6F5}", "\u{62F}\u{6CC}", "\u{6F1}\u{6F3}\u{6F7}\u{6F9}", "\u{628}\u{647}", "\u{635}\u{648}\u{631}\u{62A}", "\u{645}\u{6A9}\u{645}\u{644}\u{6CC}", "\u{628}\u{631}\u{627}\u{6CC}", "\u{62F}\u{627}\u{646}\u{634}\u{646}\u{627}\u{645}\u{647}\u{654}", "\u{62A}\u{62E}\u{635}\u{635}\u{6CC}", "\u{646}\u{648}\u{67E}\u{62F}\u{6CC}\u{627}", "\u{646}\u{648}\u{634}\u{62A}\u{647}", "\u{634}\u{62F}"])?;
    Ok(())
}

fn ea_test_greek() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "\u{393}\u{3C1}\u{3AC}\u{3C6}\u{3B5}\u{3C4}\u{3B1}\u{3B9} \u{3C3}\u{3B5} \u{3C3}\u{3C5}\u{3BD}\u{3B5}\u{3C1}\u{3B3}\u{3B1}\u{3C3}\u{3AF}\u{3B1} \u{3B1}\u{3C0}\u{3CC} \u{3B5}\u{3B8}\u{3B5}\u{3BB}\u{3BF}\u{3BD}\u{3C4}\u{3AD}\u{3C2} \u{3BC}\u{3B5} \u{3C4}\u{3BF} \u{3BB}\u{3BF}\u{3B3}\u{3B9}\u{3C3}\u{3BC}\u{3B9}\u{3BA}\u{3CC} wiki, \u{3BA}\u{3AC}\u{3C4}\u{3B9} \u{3C0}\u{3BF}\u{3C5} \u{3C3}\u{3B7}\u{3BC}\u{3B1}\u{3AF}\u{3BD}\u{3B5}\u{3B9} \u{3CC}\u{3C4}\u{3B9} \u{3AC}\u{3C1}\u{3B8}\u{3C1}\u{3B1} \u{3BC}\u{3C0}\u{3BF}\u{3C1}\u{3B5}\u{3AF} \u{3BD}\u{3B1} \u{3C0}\u{3C1}\u{3BF}\u{3C3}\u{3C4}\u{3B5}\u{3B8}\u{3BF}\u{3CD}\u{3BD} \u{3AE} \u{3BD}\u{3B1} \u{3B1}\u{3BB}\u{3BB}\u{3AC}\u{3BE}\u{3BF}\u{3C5}\u{3BD} \u{3B1}\u{3C0}\u{3CC} \u{3C4}\u{3BF}\u{3BD} \u{3BA}\u{3B1}\u{3B8}\u{3AD}\u{3BD}\u{3B1}.", &["\u{3B3}\u{3C1}\u{3AC}\u{3C6}\u{3B5}\u{3C4}\u{3B1}\u{3B9}", "\u{3C3}\u{3B5}", "\u{3C3}\u{3C5}\u{3BD}\u{3B5}\u{3C1}\u{3B3}\u{3B1}\u{3C3}\u{3AF}\u{3B1}", "\u{3B1}\u{3C0}\u{3CC}", "\u{3B5}\u{3B8}\u{3B5}\u{3BB}\u{3BF}\u{3BD}\u{3C4}\u{3AD}\u{3C2}", "\u{3BC}\u{3B5}", "\u{3C4}\u{3BF}", "\u{3BB}\u{3BF}\u{3B3}\u{3B9}\u{3C3}\u{3BC}\u{3B9}\u{3BA}\u{3CC}", "wiki", "\u{3BA}\u{3AC}\u{3C4}\u{3B9}", "\u{3C0}\u{3BF}\u{3C5}", "\u{3C3}\u{3B7}\u{3BC}\u{3B1}\u{3AF}\u{3BD}\u{3B5}\u{3B9}", "\u{3CC}\u{3C4}\u{3B9}", "\u{3AC}\u{3C1}\u{3B8}\u{3C1}\u{3B1}", "\u{3BC}\u{3C0}\u{3BF}\u{3C1}\u{3B5}\u{3AF}", "\u{3BD}\u{3B1}", "\u{3C0}\u{3C1}\u{3BF}\u{3C3}\u{3C4}\u{3B5}\u{3B8}\u{3BF}\u{3CD}\u{3BD}", "\u{3AE}", "\u{3BD}\u{3B1}", "\u{3B1}\u{3BB}\u{3BB}\u{3AC}\u{3BE}\u{3BF}\u{3C5}\u{3BD}", "\u{3B1}\u{3C0}\u{3CC}", "\u{3C4}\u{3BF}\u{3BD}", "\u{3BA}\u{3B1}\u{3B8}\u{3AD}\u{3BD}\u{3B1}"])?;
    Ok(())
}

fn ea_test_thai() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "\u{E01}\u{E32}\u{E23}\u{E17}\u{E35}\u{E48}\u{E44}\u{E14}\u{E49}\u{E15}\u{E49}\u{E2D}\u{E07}\u{E41}\u{E2A}\u{E14}\u{E07}\u{E27}\u{E48}\u{E32}\u{E07}\u{E32}\u{E19}\u{E14}\u{E35}. \u{E41}\u{E25}\u{E49}\u{E27}\u{E40}\u{E18}\u{E2D}\u{E08}\u{E30}\u{E44}\u{E1B}\u{E44}\u{E2B}\u{E19}? \u{E51}\u{E52}\u{E53}\u{E54}", &["\u{E01}\u{E32}\u{E23}\u{E17}\u{E35}\u{E48}\u{E44}\u{E14}\u{E49}\u{E15}\u{E49}\u{E2D}\u{E07}\u{E41}\u{E2A}\u{E14}\u{E07}\u{E27}\u{E48}\u{E32}\u{E07}\u{E32}\u{E19}\u{E14}\u{E35}", "\u{E41}\u{E25}\u{E49}\u{E27}\u{E40}\u{E18}\u{E2D}\u{E08}\u{E30}\u{E44}\u{E1B}\u{E44}\u{E2B}\u{E19}", "\u{E51}\u{E52}\u{E53}\u{E54}"])?;
    Ok(())
}

fn ea_test_lao() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "\u{EAA}\u{EB2}\u{E97}\u{EB2}\u{EA5}\u{EB0}\u{E99}\u{EB0}\u{EA5}\u{EB1}\u{E94} \u{E9B}\u{EB0}\u{E8A}\u{EB2}\u{E97}\u{EB4}\u{E9B}\u{EB0}\u{EC4}\u{E95} \u{E9B}\u{EB0}\u{E8A}\u{EB2}\u{E8A}\u{EBB}\u{E99}\u{EA5}\u{EB2}\u{EA7}", &["\u{EAA}\u{EB2}\u{E97}\u{EB2}\u{EA5}\u{EB0}\u{E99}\u{EB0}\u{EA5}\u{EB1}\u{E94}", "\u{E9B}\u{EB0}\u{E8A}\u{EB2}\u{E97}\u{EB4}\u{E9B}\u{EB0}\u{EC4}\u{E95}", "\u{E9B}\u{EB0}\u{E8A}\u{EB2}\u{E8A}\u{EBB}\u{E99}\u{EA5}\u{EB2}\u{EA7}"])?;
    Ok(())
}

fn ea_test_tibetan() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "\u{F66}\u{FA3}\u{F7C}\u{F53}\u{F0B}\u{F58}\u{F5B}\u{F7C}\u{F51}\u{F0B}\u{F51}\u{F44}\u{F0B}\u{F63}\u{F66}\u{F0B}\u{F60}\u{F51}\u{F72}\u{F66}\u{F0B}\u{F56}\u{F7C}\u{F51}\u{F0B}\u{F61}\u{F72}\u{F42}\u{F0B}\u{F58}\u{F72}\u{F0B}\u{F49}\u{F58}\u{F66}\u{F0B}\u{F42}\u{F7C}\u{F44}\u{F0B}\u{F60}\u{F55}\u{F7A}\u{F63}\u{F0B}\u{F51}\u{F74}\u{F0B}\u{F42}\u{F4F}\u{F7C}\u{F44}\u{F0B}\u{F56}\u{F62}\u{F0B}\u{F67}\u{F0B}\u{F45}\u{F44}\u{F0B}\u{F51}\u{F42}\u{F7A}\u{F0B}\u{F58}\u{F5A}\u{F53}\u{F0B}\u{F58}\u{F46}\u{F72}\u{F66}\u{F0B}\u{F66}\u{F7C}\u{F0D} \u{F0D}", &["\u{F66}\u{FA3}\u{F7C}\u{F53}", "\u{F58}\u{F5B}\u{F7C}\u{F51}", "\u{F51}\u{F44}", "\u{F63}\u{F66}", "\u{F60}\u{F51}\u{F72}\u{F66}", "\u{F56}\u{F7C}\u{F51}", "\u{F61}\u{F72}\u{F42}", "\u{F58}\u{F72}", "\u{F49}\u{F58}\u{F66}", "\u{F42}\u{F7C}\u{F44}", "\u{F60}\u{F55}\u{F7A}\u{F63}", "\u{F51}\u{F74}", "\u{F42}\u{F4F}\u{F7C}\u{F44}", "\u{F56}\u{F62}", "\u{F67}", "\u{F45}\u{F44}", "\u{F51}\u{F42}\u{F7A}", "\u{F58}\u{F5A}\u{F53}", "\u{F58}\u{F46}\u{F72}\u{F66}", "\u{F66}\u{F7C}"])?;
    Ok(())
}

fn ea_test_chinese() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "\u{6211}\u{662F}\u{4E2D}\u{56FD}\u{4EBA}\u{3002} \u{FF11}\u{FF12}\u{FF13}\u{FF14} \u{FF34}\u{FF45}\u{FF53}\u{FF54}\u{FF53} ", &["\u{6211}", "\u{662F}", "\u{4E2D}", "\u{56FD}", "\u{4EBA}", "\u{FF11}\u{FF12}\u{FF13}\u{FF14}", "\u{FF54}\u{FF45}\u{FF53}\u{FF54}\u{FF53}"])?;
    Ok(())
}

fn ea_test_empty() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "", &[])?;
    B::assert_analyzes_to(&*a, ".", &[])?;
    B::assert_analyzes_to(&*a, " ", &[])?;
    Ok(())
}

fn ea_test_lucene1545() -> Result<()> {
    let a = field(0);
    /* * Standard analyzer does not correctly tokenize combining character U+0364 COMBINING LATIN SMALL LETTER E. * The word "mo\u{364}chte" is incorrectly tokenized into "mo" "chte", the combining character is lost. * Expected result is only one token "mo\u{364}chte". */
    B::assert_analyzes_to(&*a, "mo\u{364}chte", &["mo\u{364}chte"])?;
    Ok(())
}

fn ea_test_alphanumeric_sa() -> Result<()> {
    let a = field(0);
    // alphanumeric tokens
    B::assert_analyzes_to(&*a, "B2B", &["b2b"])?;
    B::assert_analyzes_to(&*a, "2B", &["2b"])?;
    Ok(())
}

fn ea_test_delimiters_sa() -> Result<()> {
    let a = field(0);
    // other delimiters: "-", "/", ","
    B::assert_analyzes_to(&*a, "some-dashed-phrase", &["some", "dashed", "phrase"])?;
    B::assert_analyzes_to(&*a, "dogs,chase,cats", &["dogs", "chase", "cats"])?;
    B::assert_analyzes_to(&*a, "ac/dc", &["ac", "dc"])?;
    Ok(())
}

fn ea_test_apostrophes_sa() -> Result<()> {
    let a = field(0);
    // internal apostrophes: O'Reilly, you're, O'Reilly's
    B::assert_analyzes_to(&*a, "O'Reilly", &["o'reilly"])?;
    B::assert_analyzes_to(&*a, "you're", &["you're"])?;
    B::assert_analyzes_to(&*a, "she's", &["she's"])?;
    B::assert_analyzes_to(&*a, "Jim's", &["jim's"])?;
    B::assert_analyzes_to(&*a, "don't", &["don't"])?;
    B::assert_analyzes_to(&*a, "O'Reilly's", &["o'reilly's"])?;
    Ok(())
}

fn ea_test_numeric_sa() -> Result<()> {
    let a = field(0);
    // floating point, serial, model numbers, ip addresses, etc.
    B::assert_analyzes_to(&*a, "21.35", &["21.35"])?;
    B::assert_analyzes_to(&*a, "R2D2 C3PO", &["r2d2", "c3po"])?;
    B::assert_analyzes_to(&*a, "216.239.63.104", &["216.239.63.104"])?;
    B::assert_analyzes_to(&*a, "216.239.63.104", &["216.239.63.104"])?;
    Ok(())
}

fn ea_test_text_with_numbers_sa() -> Result<()> {
    let a = field(0);
    // numbers
    B::assert_analyzes_to(
        &*a,
        "David has 5000 bones",
        &["david", "has", "5000", "bones"],
    )?;
    Ok(())
}

fn ea_test_various_text_sa() -> Result<()> {
    let a = field(0);
    // various
    B::assert_analyzes_to(
        &*a,
        "C embedded developers wanted",
        &["c", "embedded", "developers", "wanted"],
    )?;
    B::assert_analyzes_to(&*a, "foo bar FOO BAR", &["foo", "bar", "foo", "bar"])?;
    B::assert_analyzes_to(
        &*a,
        "foo      bar .  FOO <> BAR",
        &["foo", "bar", "foo", "bar"],
    )?;
    B::assert_analyzes_to(&*a, "\"QUOTED\" word", &["quoted", "word"])?;
    Ok(())
}

fn ea_test_korean_sa() -> Result<()> {
    let a = field(0);
    // Korean words
    B::assert_analyzes_to(
        &*a,
        "\u{C548}\u{B155}\u{D558}\u{C138}\u{C694} \u{D55C}\u{AE00}\u{C785}\u{B2C8}\u{B2E4}",
        &[
            "\u{C548}\u{B155}\u{D558}\u{C138}\u{C694}",
            "\u{D55C}\u{AE00}\u{C785}\u{B2C8}\u{B2E4}",
        ],
    )?;
    Ok(())
}

fn ea_test_offsets() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to_offsets(
        &*a,
        "David has 5000 bones",
        &["david", "has", "5000", "bones"],
        &[0, 6, 10, 15],
        &[5, 9, 14, 20],
    )?;
    Ok(())
}

fn ea_test_types() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to_types_only(
        &*a,
        "david has 5000 bones",
        &["david", "has", "5000", "bones"],
        &["<ALPHANUM>", "<ALPHANUM>", "<NUM>", "<ALPHANUM>"],
    )?;
    Ok(())
}

fn ea_test_supplementary() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to_types_only(
        &*a,
        "\u{29B05}\u{8271}\u{935F}\u{41F9}\u{612F}\u{701B}",
        &[
            "\u{29B05}",
            "\u{8271}",
            "\u{935F}",
            "\u{41F9}",
            "\u{612F}",
            "\u{701B}",
        ],
        &[
            "<IDEOGRAPHIC>",
            "<IDEOGRAPHIC>",
            "<IDEOGRAPHIC>",
            "<IDEOGRAPHIC>",
            "<IDEOGRAPHIC>",
            "<IDEOGRAPHIC>",
        ],
    )?;
    Ok(())
}

fn ea_test_korean() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to_types_only(
        &*a,
        "\u{D6C8}\u{BBFC}\u{C815}\u{C74C}",
        &["\u{D6C8}\u{BBFC}\u{C815}\u{C74C}"],
        &["<HANGUL>"],
    )?;
    Ok(())
}

fn ea_test_japanese() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to_types_only(
        &*a,
        "\u{4EEE}\u{540D}\u{9063}\u{3044} \u{30AB}\u{30BF}\u{30AB}\u{30CA}",
        &[
            "\u{4EEE}",
            "\u{540D}",
            "\u{9063}",
            "\u{3044}",
            "\u{30AB}\u{30BF}\u{30AB}\u{30CA}",
        ],
        &[
            "<IDEOGRAPHIC>",
            "<IDEOGRAPHIC>",
            "<IDEOGRAPHIC>",
            "<HIRAGANA>",
            "<KATAKANA>",
        ],
    )?;
    Ok(())
}

fn ea_test_combining_marks() -> Result<()> {
    let a = field(0);
    B::check_one_term(&*a, "\u{3055}\u{3099}", "\u{3055}\u{3099}")?; // hiragana
    B::check_one_term(&*a, "\u{30B5}\u{3099}", "\u{30B5}\u{3099}")?; // katakana
    B::check_one_term(&*a, "\u{58F9}\u{3099}", "\u{58F9}\u{3099}")?; // ideographic
    B::check_one_term(&*a, "\u{C544}\u{3099}", "\u{C544}\u{3099}")?; // hangul
    Ok(())
}

fn ea_test_basic_emails() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to_types_only(&*a, "one test@example.com two three [A@example.CO.UK] \"ArakaBanassaMassanaBakarA\" <info@Info.info>", &["one", "test@example.com", "two", "three", "a@example.co.uk", "arakabanassamassanabakara", "info@info.info"], &["<ALPHANUM>", "<EMAIL>", "<ALPHANUM>", "<ALPHANUM>", "<EMAIL>", "<ALPHANUM>", "<EMAIL>"])?;
    Ok(())
}

fn ea_test_mailto_scheme_emails() -> Result<()> {
    let a = field(0);
    // See LUCENE-3880
    B::assert_analyzes_to_types_only(
        &*a,
        "MAILTO:Test@Example.ORG",
        &["mailto", "test@example.org"],
        &["<ALPHANUM>", "<EMAIL>"],
    )?;
    // TODO: Support full mailto: scheme URIs. See RFC 6068: http://tools.ietf.org/html/rfc6068
    // TODO: recognize ',' address delimiter. Also, see examples of ';' delimiter use at:
    // http://www.mailto.co.uk/
    // TODO: split field keys/values
    // TODO: Hex decoding + re-tokenization
    B::assert_analyzes_to_types_only(&*a, "mailto:personA@example.com,personB@example.com?cc=personC@example.com&subject=Subjectivity&body=Corpusivity%20or%20something%20like%20that", &["mailto", "persona@example.com", ",personb@example.com", "?cc=personc@example.com", "subject", "subjectivity", "body", "corpusivity", "20or", "20something", "20like", "20that"], &["<ALPHANUM>", "<EMAIL>", "<EMAIL>", "<EMAIL>", "<ALPHANUM>", "<ALPHANUM>", "<ALPHANUM>", "<ALPHANUM>", "<ALPHANUM>", "<ALPHANUM>", "<ALPHANUM>", "<ALPHANUM>"])?;
    Ok(())
}

fn ea_test_basic_urls() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to_types_only(&*a, "a <HTTPs://example.net/omg/isnt/that/NICE?no=its&n%30t#mntl-E>b-D ftp://www.example.com/ABC.txt file:///C:/path/to/a/FILE.txt C", &["https://example.net/omg/isnt/that/nice?no=its&n%30t#mntl-e", "b", "d", "ftp://www.example.com/abc.txt", "file:///c:/path/to/a/file.txt", "c"], &["<URL>", "<ALPHANUM>", "<ALPHANUM>", "<URL>", "<URL>", "<ALPHANUM>"])?;
    Ok(())
}

fn ea_test_no_scheme_urls() -> Result<()> {
    let a = field(0);
    // ".ph" is a Top Level Domain
    B::assert_analyzes_to_types_only(&*a, "<index.ph>", &["index.ph"], &["<URL>"])?;
    B::assert_analyzes_to_types_only(&*a, "index.ph", &["index.ph"], &["<URL>"])?;
    B::assert_analyzes_to_types_only(&*a, "index.php", &["index.php"], &["<ALPHANUM>"])?;
    B::assert_analyzes_to_types_only(
        &*a,
        "index.ph\u{3B1}",
        &["index.ph\u{3B1}"],
        &["<ALPHANUM>"],
    )?;
    B::assert_analyzes_to_types_only(
        &*a,
        "index-h.php",
        &["index", "h.php"],
        &["<ALPHANUM>", "<ALPHANUM>"],
    )?;
    B::assert_analyzes_to_types_only(
        &*a,
        "index2.php",
        &["index2", "php"],
        &["<ALPHANUM>", "<ALPHANUM>"],
    )?;
    B::assert_analyzes_to_types_only(
        &*a,
        "index2.ph\u{FF19},",
        &["index2", "ph\u{FF19}"],
        &["<ALPHANUM>", "<ALPHANUM>"],
    )?;
    B::assert_analyzes_to_types_only(
        &*a,
        "example.com,example.ph,index.php,index2.php,example2.ph",
        &[
            "example.com",
            "example.ph",
            "index.php",
            "index2",
            "php",
            "example2.ph",
        ],
        &[
            "<URL>",
            "<URL>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<URL>",
        ],
    )?;
    B::assert_analyzes_to_types_only(
        &*a,
        "example.com:8080 example.com/path/here example.com?query=something example.com#fragment",
        &[
            "example.com:8080",
            "example.com/path/here",
            "example.com?query=something",
            "example.com#fragment",
        ],
        &["<URL>", "<URL>", "<URL>", "<URL>"],
    )?;
    B::assert_analyzes_to_types_only(
        &*a,
        "example.com:8080/path/here?query=something#fragment",
        &["example.com:8080/path/here?query=something#fragment"],
        &["<URL>"],
    )?;
    B::assert_analyzes_to_types_only(
        &*a,
        "example.com:8080/path/here?query=something",
        &["example.com:8080/path/here?query=something"],
        &["<URL>"],
    )?;
    B::assert_analyzes_to_types_only(
        &*a,
        "example.com:8080/path/here#fragment",
        &["example.com:8080/path/here#fragment"],
        &["<URL>"],
    )?;
    B::assert_analyzes_to_types_only(
        &*a,
        "example.com:8080/path/here",
        &["example.com:8080/path/here"],
        &["<URL>"],
    )?;
    B::assert_analyzes_to_types_only(
        &*a,
        "example.com:8080?query=something#fragment",
        &["example.com:8080?query=something#fragment"],
        &["<URL>"],
    )?;
    B::assert_analyzes_to_types_only(
        &*a,
        "example.com:8080?query=something",
        &["example.com:8080?query=something"],
        &["<URL>"],
    )?;
    B::assert_analyzes_to_types_only(
        &*a,
        "example.com:8080#fragment",
        &["example.com:8080#fragment"],
        &["<URL>"],
    )?;
    B::assert_analyzes_to_types_only(
        &*a,
        "example.com/path/here?query=something#fragment",
        &["example.com/path/here?query=something#fragment"],
        &["<URL>"],
    )?;
    B::assert_analyzes_to_types_only(
        &*a,
        "example.com/path/here?query=something",
        &["example.com/path/here?query=something"],
        &["<URL>"],
    )?;
    B::assert_analyzes_to_types_only(
        &*a,
        "example.com/path/here#fragment",
        &["example.com/path/here#fragment"],
        &["<URL>"],
    )?;
    B::assert_analyzes_to_types_only(
        &*a,
        "example.com?query=something#fragment",
        &["example.com?query=something#fragment"],
        &["<URL>"],
    )?;
    Ok(())
}

fn ea_test_random_strings() -> Result<()> {
    B::check_random_data(
        &*LuceneTestCase::random(),
        &field(0),
        200 * *RANDOM_MULTIPLIER,
    )
}

fn ea_test_max_token_length_default() -> Result<()> {
    let a = field(0);
    // exact max length:
    let b_string = "b".repeat(DEFAULT_MAX_TOKEN_LENGTH as usize);
    // first bString is exact max default length; next one is 1 too long
    let input = format!("x {b_string} {b_string}b");
    B::assert_analyzes_to(&*a, &input, &["x", &b_string, &b_string, "b"])?;
    a.close();
    Ok(())
}

fn ea_test_max_token_length_non_default() -> Result<()> {
    let mut a = UAX29URLEmailAnalyzer::new();
    a.set_max_token_length(5);
    B::assert_analyzes_to(
        &a,
        "ab cd toolong xy z",
        &["ab", "cd", "toolo", "ng", "xy", "z"],
    )?;
    a.close();
    Ok(())
}
fn uax29_url_email_analyzer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.email.TestUAX29URLEmailAnalyzer")
        .method(hook("setUp", ea_set_up))
        .method(hook("tearDown", close_fields_tear_down))
        .method(t("testHugeDoc", ea_test_huge_doc))
        .method(t("testArmenian", ea_test_armenian))
        .method(t("testAmharic", ea_test_amharic))
        .method(t("testArabic", ea_test_arabic))
        .method(t("testAramaic", ea_test_aramaic))
        .method(t("testBengali", ea_test_bengali))
        .method(t("testFarsi", ea_test_farsi))
        .method(t("testGreek", ea_test_greek))
        .method(t("testThai", ea_test_thai))
        .method(t("testLao", ea_test_lao))
        .method(t("testTibetan", ea_test_tibetan))
        .method(t("testChinese", ea_test_chinese))
        .method(t("testEmpty", ea_test_empty))
        .method(t("testLUCENE1545", ea_test_lucene1545))
        .method(t("testAlphanumericSA", ea_test_alphanumeric_sa))
        .method(t("testDelimitersSA", ea_test_delimiters_sa))
        .method(t("testApostrophesSA", ea_test_apostrophes_sa))
        .method(t("testNumericSA", ea_test_numeric_sa))
        .method(t("testTextWithNumbersSA", ea_test_text_with_numbers_sa))
        .method(t("testVariousTextSA", ea_test_various_text_sa))
        .method(t("testKoreanSA", ea_test_korean_sa))
        .method(t("testOffsets", ea_test_offsets))
        .method(t("testTypes", ea_test_types))
        .method(t("testSupplementary", ea_test_supplementary))
        .method(t("testKorean", ea_test_korean))
        .method(t("testJapanese", ea_test_japanese))
        .method(t("testCombiningMarks", ea_test_combining_marks))
        .method(t("testBasicEmails", ea_test_basic_emails))
        .method(t("testMailtoSchemeEmails", ea_test_mailto_scheme_emails))
        .method(t("testBasicURLs", ea_test_basic_urls))
        .method(t("testNoSchemeURLs", ea_test_no_scheme_urls))
        .method(t("testRandomStrings", ea_test_random_strings))
        .method(t(
            "testMaxTokenLengthDefault",
            ea_test_max_token_length_default,
        ))
        .method(t(
            "testMaxTokenLengthNonDefault",
            ea_test_max_token_length_non_default,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestUAX29URLEmailTokenizer
// -----------------------------------------------------------------------------

/// `URLFilter` and `EmailFilter`: pass tokens of type `TOKEN_TYPES[kind]`
/// through and block all other types. Java compares the type with `==`
/// against the tokenizer's interned constant; the port compares the text.
#[derive(Debug)]
struct TypeOnlyFilterLogic {
    kind: usize,
}

impl TokenFilterLogic for TypeOnlyFilterLogic {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        let mut is_token_available = false;
        while input.increment_token()? {
            let ty = source
                .get_attribute_interface::<dyn TypeAttribute>()
                .expect("typeAtt")
                .type_value()
                .to_string();
            if ty == TOKEN_TYPES[self.kind] {
                is_token_available = true;
                break;
            }
        }
        Ok(is_token_available)
    }
}

fn type_only_filter(
    input: Box<dyn TokenStream>,
    kind: i32,
) -> Result<TokenFilter<TypeOnlyFilterLogic>> {
    let mut filter = TokenFilter::new(
        input,
        TypeOnlyFilterLogic {
            kind: kind as usize,
        },
    );
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn TypeAttribute>()?;
    Ok(filter)
}

/// The analyzer `a` of `setUp` (and of `testExampleURLs`).
fn tokenizer_analyzer(name: &'static str) -> Arc<dyn Analyzer> {
    FnAnalyzer::new(name, |_| {
        Ok(tokenizer_components(own(
            new_uax29_url_email_tokenizer_with_factory(B::new_attribute_factory())?,
        )))
    })
    .arc()
}

fn et_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    let a = tokenizer_analyzer("org.apache.lucene.analysis.email.TestUAX29URLEmailTokenizer$1");
    let url_analyzer = FnAnalyzer::new(
        "org.apache.lucene.analysis.email.TestUAX29URLEmailTokenizer$2",
        |_| {
            let mut tokenizer =
                new_uax29_url_email_tokenizer_with_factory(B::new_attribute_factory())?;
            // Tokenize arbitrary length URLs
            tokenizer
                .logic_mut()
                .set_max_token_length(MAX_TOKEN_LENGTH_LIMIT)?;
            let tokenizer = own(tokenizer);
            let filter = type_only_filter(shared(&tokenizer), URL)?;
            Ok(components(tokenizer, sink(filter)))
        },
    )
    .arc();
    let email_analyzer = FnAnalyzer::new(
        "org.apache.lucene.analysis.email.TestUAX29URLEmailTokenizer$3",
        |_| {
            let tokenizer = own(new_uax29_url_email_tokenizer_with_factory(
                B::new_attribute_factory(),
            )?);
            let filter = type_only_filter(shared(&tokenizer), EMAIL)?;
            Ok(components(tokenizer, sink(filter)))
        },
    )
    .arc();
    FIELDS.with(|f| *f.borrow_mut() = vec![a, url_analyzer, email_analyzer]);
    Ok(())
}

fn et_test_long_emailatom_text() -> Result<()> {
    // EMAILatomText = [A-Za-z0-9!#$%&'*+-/=?\^_`{|}~]
    let email_atom_chars: Vec<char> =
        "!#$%&'*+,-./0123456789=?ABCDEFGHIJKLMNOPQRSTUVWXYZ^_`abcdefghijklmnopqrstuvwxyz{|}~"
            .chars()
            .collect();
    let random = LuceneTestCase::random();
    let mut text = String::new();
    let num_chars = TestUtil::next_int(&*random, 100 * 1024, 3 * 1024 * 1024);
    for _ in 0..num_chars {
        text.push(email_atom_chars[random.next_int_bound(email_atom_chars.len() as i32) as usize]);
    }
    let mut token_count = 0;
    let mut ts = new_uax29_url_email_tokenizer();
    ts.set_reader(Box::new(StringCharReader::new(text.as_str())))?;
    ts.reset()?;
    while ts.increment_token()? {
        token_count += 1;
    }
    ts.end()?;
    ts.close()?;
    Assert::assert_true(token_count > 0);

    token_count = 0;
    let new_buffer_size = TestUtil::next_int(&*random, 200, 8192);
    ts.logic_mut().set_max_token_length(new_buffer_size)?;
    ts.set_reader(Box::new(StringCharReader::new(text.as_str())))?;
    ts.reset()?;
    while ts.increment_token()? {
        token_count += 1;
    }
    ts.end()?;
    ts.close()?;
    Assert::assert_true(token_count > 0);
    Ok(())
}

fn et_test_huge_doc() -> Result<()> {
    let input = huge_doc_input();
    let mut tokenizer = new_uax29_url_email_tokenizer_with_factory(B::new_attribute_factory())?;
    tokenizer.set_reader(Box::new(StringCharReader::new(input.as_str())))?;
    B::assert_token_stream_contents(&mut tokenizer, &["testing", "1234"])
}

fn et_test_armenian() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "\u{54E}\u{56B}\u{584}\u{56B}\u{57A}\u{565}\u{564}\u{56B}\u{561}\u{575}\u{56B} 13 \u{574}\u{56B}\u{56C}\u{56B}\u{578}\u{576} \u{570}\u{578}\u{564}\u{57E}\u{561}\u{56E}\u{576}\u{565}\u{580}\u{568} (4,600` \u{570}\u{561}\u{575}\u{565}\u{580}\u{565}\u{576} \u{57E}\u{56B}\u{584}\u{56B}\u{57A}\u{565}\u{564}\u{56B}\u{561}\u{575}\u{578}\u{582}\u{574}) \u{563}\u{580}\u{57E}\u{565}\u{56C} \u{565}\u{576} \u{56F}\u{561}\u{574}\u{561}\u{57E}\u{578}\u{580}\u{576}\u{565}\u{580}\u{56B} \u{56F}\u{578}\u{572}\u{574}\u{56B}\u{581} \u{578}\u{582} \u{570}\u{561}\u{574}\u{561}\u{580}\u{575}\u{561} \u{562}\u{578}\u{56C}\u{578}\u{580} \u{570}\u{578}\u{564}\u{57E}\u{561}\u{56E}\u{576}\u{565}\u{580}\u{568} \u{56F}\u{561}\u{580}\u{578}\u{572} \u{567} \u{56D}\u{574}\u{562}\u{561}\u{563}\u{580}\u{565}\u{56C} \u{581}\u{561}\u{576}\u{56F}\u{561}\u{581} \u{574}\u{561}\u{580}\u{564} \u{578}\u{57E} \u{56F}\u{561}\u{580}\u{578}\u{572} \u{567} \u{562}\u{561}\u{581}\u{565}\u{56C} \u{54E}\u{56B}\u{584}\u{56B}\u{57A}\u{565}\u{564}\u{56B}\u{561}\u{575}\u{56B} \u{56F}\u{561}\u{575}\u{584}\u{568}\u{589}", &["\u{54E}\u{56B}\u{584}\u{56B}\u{57A}\u{565}\u{564}\u{56B}\u{561}\u{575}\u{56B}", "13", "\u{574}\u{56B}\u{56C}\u{56B}\u{578}\u{576}", "\u{570}\u{578}\u{564}\u{57E}\u{561}\u{56E}\u{576}\u{565}\u{580}\u{568}", "4,600", "\u{570}\u{561}\u{575}\u{565}\u{580}\u{565}\u{576}", "\u{57E}\u{56B}\u{584}\u{56B}\u{57A}\u{565}\u{564}\u{56B}\u{561}\u{575}\u{578}\u{582}\u{574}", "\u{563}\u{580}\u{57E}\u{565}\u{56C}", "\u{565}\u{576}", "\u{56F}\u{561}\u{574}\u{561}\u{57E}\u{578}\u{580}\u{576}\u{565}\u{580}\u{56B}", "\u{56F}\u{578}\u{572}\u{574}\u{56B}\u{581}", "\u{578}\u{582}", "\u{570}\u{561}\u{574}\u{561}\u{580}\u{575}\u{561}", "\u{562}\u{578}\u{56C}\u{578}\u{580}", "\u{570}\u{578}\u{564}\u{57E}\u{561}\u{56E}\u{576}\u{565}\u{580}\u{568}", "\u{56F}\u{561}\u{580}\u{578}\u{572}", "\u{567}", "\u{56D}\u{574}\u{562}\u{561}\u{563}\u{580}\u{565}\u{56C}", "\u{581}\u{561}\u{576}\u{56F}\u{561}\u{581}", "\u{574}\u{561}\u{580}\u{564}", "\u{578}\u{57E}", "\u{56F}\u{561}\u{580}\u{578}\u{572}", "\u{567}", "\u{562}\u{561}\u{581}\u{565}\u{56C}", "\u{54E}\u{56B}\u{584}\u{56B}\u{57A}\u{565}\u{564}\u{56B}\u{561}\u{575}\u{56B}", "\u{56F}\u{561}\u{575}\u{584}\u{568}"])?;
    Ok(())
}

fn et_test_amharic() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "\u{12CA}\u{12AA}\u{1354}\u{12F5}\u{12EB} \u{12E8}\u{1263}\u{1208} \u{1265}\u{12D9} \u{124B}\u{1295}\u{124B} \u{12E8}\u{1270}\u{121F}\u{120B} \u{1275}\u{12AD}\u{12AD}\u{1208}\u{129B}\u{1293} \u{1290}\u{133B} \u{1218}\u{12DD}\u{1308}\u{1260} \u{12D5}\u{12CD}\u{1240}\u{1275} (\u{12A2}\u{1295}\u{1233}\u{12ED}\u{12AD}\u{120E}\u{1352}\u{12F2}\u{12EB}) \u{1290}\u{12CD}\u{1362} \u{121B}\u{1295}\u{129B}\u{12CD}\u{121D}", &["\u{12CA}\u{12AA}\u{1354}\u{12F5}\u{12EB}", "\u{12E8}\u{1263}\u{1208}", "\u{1265}\u{12D9}", "\u{124B}\u{1295}\u{124B}", "\u{12E8}\u{1270}\u{121F}\u{120B}", "\u{1275}\u{12AD}\u{12AD}\u{1208}\u{129B}\u{1293}", "\u{1290}\u{133B}", "\u{1218}\u{12DD}\u{1308}\u{1260}", "\u{12D5}\u{12CD}\u{1240}\u{1275}", "\u{12A2}\u{1295}\u{1233}\u{12ED}\u{12AD}\u{120E}\u{1352}\u{12F2}\u{12EB}", "\u{1290}\u{12CD}", "\u{121B}\u{1295}\u{129B}\u{12CD}\u{121D}"])?;
    Ok(())
}

fn et_test_arabic() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "\u{627}\u{644}\u{641}\u{64A}\u{644}\u{645} \u{627}\u{644}\u{648}\u{62B}\u{627}\u{626}\u{642}\u{64A} \u{627}\u{644}\u{623}\u{648}\u{644} \u{639}\u{646} \u{648}\u{64A}\u{643}\u{64A}\u{628}\u{64A}\u{62F}\u{64A}\u{627} \u{64A}\u{633}\u{645}\u{649} \"\u{627}\u{644}\u{62D}\u{642}\u{64A}\u{642}\u{629} \u{628}\u{627}\u{644}\u{623}\u{631}\u{642}\u{627}\u{645}: \u{642}\u{635}\u{629} \u{648}\u{64A}\u{643}\u{64A}\u{628}\u{64A}\u{62F}\u{64A}\u{627}\" (\u{628}\u{627}\u{644}\u{625}\u{646}\u{62C}\u{644}\u{64A}\u{632}\u{64A}\u{629}: Truth in Numbers: The Wikipedia Story)\u{60C} \u{633}\u{64A}\u{62A}\u{645} \u{625}\u{637}\u{644}\u{627}\u{642}\u{647} \u{641}\u{64A} 2008.", &["\u{627}\u{644}\u{641}\u{64A}\u{644}\u{645}", "\u{627}\u{644}\u{648}\u{62B}\u{627}\u{626}\u{642}\u{64A}", "\u{627}\u{644}\u{623}\u{648}\u{644}", "\u{639}\u{646}", "\u{648}\u{64A}\u{643}\u{64A}\u{628}\u{64A}\u{62F}\u{64A}\u{627}", "\u{64A}\u{633}\u{645}\u{649}", "\u{627}\u{644}\u{62D}\u{642}\u{64A}\u{642}\u{629}", "\u{628}\u{627}\u{644}\u{623}\u{631}\u{642}\u{627}\u{645}", "\u{642}\u{635}\u{629}", "\u{648}\u{64A}\u{643}\u{64A}\u{628}\u{64A}\u{62F}\u{64A}\u{627}", "\u{628}\u{627}\u{644}\u{625}\u{646}\u{62C}\u{644}\u{64A}\u{632}\u{64A}\u{629}", "Truth", "in", "Numbers", "The", "Wikipedia", "Story", "\u{633}\u{64A}\u{62A}\u{645}", "\u{625}\u{637}\u{644}\u{627}\u{642}\u{647}", "\u{641}\u{64A}", "2008"])?;
    Ok(())
}

fn et_test_aramaic() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "\u{718}\u{71D}\u{729}\u{71D}\u{726}\u{715}\u{71D}\u{710} (\u{710}\u{722}\u{713}\u{720}\u{71D}\u{710}: Wikipedia) \u{717}\u{718} \u{710}\u{71D}\u{722}\u{723}\u{729}\u{720}\u{718}\u{726}\u{715}\u{71D}\u{710} \u{71A}\u{710}\u{72A}\u{72C}\u{710} \u{715}\u{710}\u{722}\u{71B}\u{72A}\u{722}\u{71B} \u{712}\u{720}\u{72B}\u{722}\u{308}\u{710} \u{723}\u{713}\u{71D}\u{710}\u{308}\u{710}\u{702} \u{72B}\u{721}\u{717} \u{710}\u{72C}\u{710} \u{721}\u{722} \u{721}\u{308}\u{720}\u{72C}\u{710} \u{715}\"\u{718}\u{71D}\u{729}\u{71D}\" \u{718}\"\u{710}\u{71D}\u{722}\u{723}\u{729}\u{720}\u{718}\u{726}\u{715}\u{71D}\u{710}\"\u{700}", &["\u{718}\u{71D}\u{729}\u{71D}\u{726}\u{715}\u{71D}\u{710}", "\u{710}\u{722}\u{713}\u{720}\u{71D}\u{710}", "Wikipedia", "\u{717}\u{718}", "\u{710}\u{71D}\u{722}\u{723}\u{729}\u{720}\u{718}\u{726}\u{715}\u{71D}\u{710}", "\u{71A}\u{710}\u{72A}\u{72C}\u{710}", "\u{715}\u{710}\u{722}\u{71B}\u{72A}\u{722}\u{71B}", "\u{712}\u{720}\u{72B}\u{722}\u{308}\u{710}", "\u{723}\u{713}\u{71D}\u{710}\u{308}\u{710}", "\u{72B}\u{721}\u{717}", "\u{710}\u{72C}\u{710}", "\u{721}\u{722}", "\u{721}\u{308}\u{720}\u{72C}\u{710}", "\u{715}", "\u{718}\u{71D}\u{729}\u{71D}", "\u{718}", "\u{710}\u{71D}\u{722}\u{723}\u{729}\u{720}\u{718}\u{726}\u{715}\u{71D}\u{710}"])?;
    Ok(())
}

fn et_test_bengali() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "\u{98F}\u{987} \u{9AC}\u{9BF}\u{9B6}\u{9CD}\u{9AC}\u{995}\u{9CB}\u{9B7} \u{9AA}\u{9B0}\u{9BF}\u{99A}\u{9BE}\u{9B2}\u{9A8}\u{9BE} \u{995}\u{9B0}\u{9C7} \u{989}\u{987}\u{995}\u{9BF}\u{9AE}\u{9BF}\u{9A1}\u{9BF}\u{9AF}\u{9BC}\u{9BE} \u{9AB}\u{9BE}\u{989}\u{9A8}\u{9CD}\u{9A1}\u{9C7}\u{9B6}\u{9A8} (\u{98F}\u{995}\u{99F}\u{9BF} \u{985}\u{9B2}\u{9BE}\u{9AD}\u{99C}\u{9A8}\u{995} \u{9B8}\u{982}\u{9B8}\u{9CD}\u{9A5}\u{9BE})\u{964} \u{989}\u{987}\u{995}\u{9BF}\u{9AA}\u{9BF}\u{9A1}\u{9BF}\u{9AF}\u{9BC}\u{9BE}\u{9B0} \u{9B6}\u{9C1}\u{9B0}\u{9C1} \u{9E7}\u{9EB} \u{99C}\u{9BE}\u{9A8}\u{9C1}\u{9AF}\u{9BC}\u{9BE}\u{9B0}\u{9BF}, \u{9E8}\u{9E6}\u{9E6}\u{9E7} \u{9B8}\u{9BE}\u{9B2}\u{9C7}\u{964} \u{98F}\u{996}\u{9A8} \u{9AA}\u{9B0}\u{9CD}\u{9AF}\u{9A8}\u{9CD}\u{9A4} \u{9E8}\u{9E6}\u{9E6}\u{99F}\u{9BF}\u{9B0}\u{993} \u{9AC}\u{9C7}\u{9B6}\u{9C0} \u{9AD}\u{9BE}\u{9B7}\u{9BE}\u{9AF}\u{9BC} \u{989}\u{987}\u{995}\u{9BF}\u{9AA}\u{9BF}\u{9A1}\u{9BF}\u{9AF}\u{9BC}\u{9BE} \u{9B0}\u{9AF}\u{9BC}\u{9C7}\u{99B}\u{9C7}\u{964}", &["\u{98F}\u{987}", "\u{9AC}\u{9BF}\u{9B6}\u{9CD}\u{9AC}\u{995}\u{9CB}\u{9B7}", "\u{9AA}\u{9B0}\u{9BF}\u{99A}\u{9BE}\u{9B2}\u{9A8}\u{9BE}", "\u{995}\u{9B0}\u{9C7}", "\u{989}\u{987}\u{995}\u{9BF}\u{9AE}\u{9BF}\u{9A1}\u{9BF}\u{9AF}\u{9BC}\u{9BE}", "\u{9AB}\u{9BE}\u{989}\u{9A8}\u{9CD}\u{9A1}\u{9C7}\u{9B6}\u{9A8}", "\u{98F}\u{995}\u{99F}\u{9BF}", "\u{985}\u{9B2}\u{9BE}\u{9AD}\u{99C}\u{9A8}\u{995}", "\u{9B8}\u{982}\u{9B8}\u{9CD}\u{9A5}\u{9BE}", "\u{989}\u{987}\u{995}\u{9BF}\u{9AA}\u{9BF}\u{9A1}\u{9BF}\u{9AF}\u{9BC}\u{9BE}\u{9B0}", "\u{9B6}\u{9C1}\u{9B0}\u{9C1}", "\u{9E7}\u{9EB}", "\u{99C}\u{9BE}\u{9A8}\u{9C1}\u{9AF}\u{9BC}\u{9BE}\u{9B0}\u{9BF}", "\u{9E8}\u{9E6}\u{9E6}\u{9E7}", "\u{9B8}\u{9BE}\u{9B2}\u{9C7}", "\u{98F}\u{996}\u{9A8}", "\u{9AA}\u{9B0}\u{9CD}\u{9AF}\u{9A8}\u{9CD}\u{9A4}", "\u{9E8}\u{9E6}\u{9E6}\u{99F}\u{9BF}\u{9B0}\u{993}", "\u{9AC}\u{9C7}\u{9B6}\u{9C0}", "\u{9AD}\u{9BE}\u{9B7}\u{9BE}\u{9AF}\u{9BC}", "\u{989}\u{987}\u{995}\u{9BF}\u{9AA}\u{9BF}\u{9A1}\u{9BF}\u{9AF}\u{9BC}\u{9BE}", "\u{9B0}\u{9AF}\u{9BC}\u{9C7}\u{99B}\u{9C7}"])?;
    Ok(())
}

fn et_test_farsi() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "\u{648}\u{6CC}\u{6A9}\u{6CC} \u{67E}\u{62F}\u{6CC}\u{627}\u{6CC} \u{627}\u{646}\u{6AF}\u{644}\u{6CC}\u{633}\u{6CC} \u{62F}\u{631} \u{62A}\u{627}\u{631}\u{6CC}\u{62E} \u{6F2}\u{6F5} \u{62F}\u{6CC} \u{6F1}\u{6F3}\u{6F7}\u{6F9} \u{628}\u{647} \u{635}\u{648}\u{631}\u{62A} \u{645}\u{6A9}\u{645}\u{644}\u{6CC} \u{628}\u{631}\u{627}\u{6CC} \u{62F}\u{627}\u{646}\u{634}\u{646}\u{627}\u{645}\u{647}\u{654} \u{62A}\u{62E}\u{635}\u{635}\u{6CC} \u{646}\u{648}\u{67E}\u{62F}\u{6CC}\u{627} \u{646}\u{648}\u{634}\u{62A}\u{647} \u{634}\u{62F}.", &["\u{648}\u{6CC}\u{6A9}\u{6CC}", "\u{67E}\u{62F}\u{6CC}\u{627}\u{6CC}", "\u{627}\u{646}\u{6AF}\u{644}\u{6CC}\u{633}\u{6CC}", "\u{62F}\u{631}", "\u{62A}\u{627}\u{631}\u{6CC}\u{62E}", "\u{6F2}\u{6F5}", "\u{62F}\u{6CC}", "\u{6F1}\u{6F3}\u{6F7}\u{6F9}", "\u{628}\u{647}", "\u{635}\u{648}\u{631}\u{62A}", "\u{645}\u{6A9}\u{645}\u{644}\u{6CC}", "\u{628}\u{631}\u{627}\u{6CC}", "\u{62F}\u{627}\u{646}\u{634}\u{646}\u{627}\u{645}\u{647}\u{654}", "\u{62A}\u{62E}\u{635}\u{635}\u{6CC}", "\u{646}\u{648}\u{67E}\u{62F}\u{6CC}\u{627}", "\u{646}\u{648}\u{634}\u{62A}\u{647}", "\u{634}\u{62F}"])?;
    Ok(())
}

fn et_test_greek() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "\u{393}\u{3C1}\u{3AC}\u{3C6}\u{3B5}\u{3C4}\u{3B1}\u{3B9} \u{3C3}\u{3B5} \u{3C3}\u{3C5}\u{3BD}\u{3B5}\u{3C1}\u{3B3}\u{3B1}\u{3C3}\u{3AF}\u{3B1} \u{3B1}\u{3C0}\u{3CC} \u{3B5}\u{3B8}\u{3B5}\u{3BB}\u{3BF}\u{3BD}\u{3C4}\u{3AD}\u{3C2} \u{3BC}\u{3B5} \u{3C4}\u{3BF} \u{3BB}\u{3BF}\u{3B3}\u{3B9}\u{3C3}\u{3BC}\u{3B9}\u{3BA}\u{3CC} wiki, \u{3BA}\u{3AC}\u{3C4}\u{3B9} \u{3C0}\u{3BF}\u{3C5} \u{3C3}\u{3B7}\u{3BC}\u{3B1}\u{3AF}\u{3BD}\u{3B5}\u{3B9} \u{3CC}\u{3C4}\u{3B9} \u{3AC}\u{3C1}\u{3B8}\u{3C1}\u{3B1} \u{3BC}\u{3C0}\u{3BF}\u{3C1}\u{3B5}\u{3AF} \u{3BD}\u{3B1} \u{3C0}\u{3C1}\u{3BF}\u{3C3}\u{3C4}\u{3B5}\u{3B8}\u{3BF}\u{3CD}\u{3BD} \u{3AE} \u{3BD}\u{3B1} \u{3B1}\u{3BB}\u{3BB}\u{3AC}\u{3BE}\u{3BF}\u{3C5}\u{3BD} \u{3B1}\u{3C0}\u{3CC} \u{3C4}\u{3BF}\u{3BD} \u{3BA}\u{3B1}\u{3B8}\u{3AD}\u{3BD}\u{3B1}.", &["\u{393}\u{3C1}\u{3AC}\u{3C6}\u{3B5}\u{3C4}\u{3B1}\u{3B9}", "\u{3C3}\u{3B5}", "\u{3C3}\u{3C5}\u{3BD}\u{3B5}\u{3C1}\u{3B3}\u{3B1}\u{3C3}\u{3AF}\u{3B1}", "\u{3B1}\u{3C0}\u{3CC}", "\u{3B5}\u{3B8}\u{3B5}\u{3BB}\u{3BF}\u{3BD}\u{3C4}\u{3AD}\u{3C2}", "\u{3BC}\u{3B5}", "\u{3C4}\u{3BF}", "\u{3BB}\u{3BF}\u{3B3}\u{3B9}\u{3C3}\u{3BC}\u{3B9}\u{3BA}\u{3CC}", "wiki", "\u{3BA}\u{3AC}\u{3C4}\u{3B9}", "\u{3C0}\u{3BF}\u{3C5}", "\u{3C3}\u{3B7}\u{3BC}\u{3B1}\u{3AF}\u{3BD}\u{3B5}\u{3B9}", "\u{3CC}\u{3C4}\u{3B9}", "\u{3AC}\u{3C1}\u{3B8}\u{3C1}\u{3B1}", "\u{3BC}\u{3C0}\u{3BF}\u{3C1}\u{3B5}\u{3AF}", "\u{3BD}\u{3B1}", "\u{3C0}\u{3C1}\u{3BF}\u{3C3}\u{3C4}\u{3B5}\u{3B8}\u{3BF}\u{3CD}\u{3BD}", "\u{3AE}", "\u{3BD}\u{3B1}", "\u{3B1}\u{3BB}\u{3BB}\u{3AC}\u{3BE}\u{3BF}\u{3C5}\u{3BD}", "\u{3B1}\u{3C0}\u{3CC}", "\u{3C4}\u{3BF}\u{3BD}", "\u{3BA}\u{3B1}\u{3B8}\u{3AD}\u{3BD}\u{3B1}"])?;
    Ok(())
}

fn et_test_thai() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "\u{E01}\u{E32}\u{E23}\u{E17}\u{E35}\u{E48}\u{E44}\u{E14}\u{E49}\u{E15}\u{E49}\u{E2D}\u{E07}\u{E41}\u{E2A}\u{E14}\u{E07}\u{E27}\u{E48}\u{E32}\u{E07}\u{E32}\u{E19}\u{E14}\u{E35}. \u{E41}\u{E25}\u{E49}\u{E27}\u{E40}\u{E18}\u{E2D}\u{E08}\u{E30}\u{E44}\u{E1B}\u{E44}\u{E2B}\u{E19}? \u{E51}\u{E52}\u{E53}\u{E54}", &["\u{E01}\u{E32}\u{E23}\u{E17}\u{E35}\u{E48}\u{E44}\u{E14}\u{E49}\u{E15}\u{E49}\u{E2D}\u{E07}\u{E41}\u{E2A}\u{E14}\u{E07}\u{E27}\u{E48}\u{E32}\u{E07}\u{E32}\u{E19}\u{E14}\u{E35}", "\u{E41}\u{E25}\u{E49}\u{E27}\u{E40}\u{E18}\u{E2D}\u{E08}\u{E30}\u{E44}\u{E1B}\u{E44}\u{E2B}\u{E19}", "\u{E51}\u{E52}\u{E53}\u{E54}"])?;
    Ok(())
}

fn et_test_lao() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "\u{EAA}\u{EB2}\u{E97}\u{EB2}\u{EA5}\u{EB0}\u{E99}\u{EB0}\u{EA5}\u{EB1}\u{E94} \u{E9B}\u{EB0}\u{E8A}\u{EB2}\u{E97}\u{EB4}\u{E9B}\u{EB0}\u{EC4}\u{E95} \u{E9B}\u{EB0}\u{E8A}\u{EB2}\u{E8A}\u{EBB}\u{E99}\u{EA5}\u{EB2}\u{EA7}", &["\u{EAA}\u{EB2}\u{E97}\u{EB2}\u{EA5}\u{EB0}\u{E99}\u{EB0}\u{EA5}\u{EB1}\u{E94}", "\u{E9B}\u{EB0}\u{E8A}\u{EB2}\u{E97}\u{EB4}\u{E9B}\u{EB0}\u{EC4}\u{E95}", "\u{E9B}\u{EB0}\u{E8A}\u{EB2}\u{E8A}\u{EBB}\u{E99}\u{EA5}\u{EB2}\u{EA7}"])?;
    Ok(())
}

fn et_test_tibetan() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "\u{F66}\u{FA3}\u{F7C}\u{F53}\u{F0B}\u{F58}\u{F5B}\u{F7C}\u{F51}\u{F0B}\u{F51}\u{F44}\u{F0B}\u{F63}\u{F66}\u{F0B}\u{F60}\u{F51}\u{F72}\u{F66}\u{F0B}\u{F56}\u{F7C}\u{F51}\u{F0B}\u{F61}\u{F72}\u{F42}\u{F0B}\u{F58}\u{F72}\u{F0B}\u{F49}\u{F58}\u{F66}\u{F0B}\u{F42}\u{F7C}\u{F44}\u{F0B}\u{F60}\u{F55}\u{F7A}\u{F63}\u{F0B}\u{F51}\u{F74}\u{F0B}\u{F42}\u{F4F}\u{F7C}\u{F44}\u{F0B}\u{F56}\u{F62}\u{F0B}\u{F67}\u{F0B}\u{F45}\u{F44}\u{F0B}\u{F51}\u{F42}\u{F7A}\u{F0B}\u{F58}\u{F5A}\u{F53}\u{F0B}\u{F58}\u{F46}\u{F72}\u{F66}\u{F0B}\u{F66}\u{F7C}\u{F0D} \u{F0D}", &["\u{F66}\u{FA3}\u{F7C}\u{F53}", "\u{F58}\u{F5B}\u{F7C}\u{F51}", "\u{F51}\u{F44}", "\u{F63}\u{F66}", "\u{F60}\u{F51}\u{F72}\u{F66}", "\u{F56}\u{F7C}\u{F51}", "\u{F61}\u{F72}\u{F42}", "\u{F58}\u{F72}", "\u{F49}\u{F58}\u{F66}", "\u{F42}\u{F7C}\u{F44}", "\u{F60}\u{F55}\u{F7A}\u{F63}", "\u{F51}\u{F74}", "\u{F42}\u{F4F}\u{F7C}\u{F44}", "\u{F56}\u{F62}", "\u{F67}", "\u{F45}\u{F44}", "\u{F51}\u{F42}\u{F7A}", "\u{F58}\u{F5A}\u{F53}", "\u{F58}\u{F46}\u{F72}\u{F66}", "\u{F66}\u{F7C}"])?;
    Ok(())
}

fn et_test_chinese() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "\u{6211}\u{662F}\u{4E2D}\u{56FD}\u{4EBA}\u{3002} \u{FF11}\u{FF12}\u{FF13}\u{FF14} \u{FF34}\u{FF45}\u{FF53}\u{FF54}\u{FF53} ", &["\u{6211}", "\u{662F}", "\u{4E2D}", "\u{56FD}", "\u{4EBA}", "\u{FF11}\u{FF12}\u{FF13}\u{FF14}", "\u{FF34}\u{FF45}\u{FF53}\u{FF54}\u{FF53}"])?;
    Ok(())
}

fn et_test_empty() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to(&*a, "", &[])?;
    B::assert_analyzes_to(&*a, ".", &[])?;
    B::assert_analyzes_to(&*a, " ", &[])?;
    Ok(())
}

fn et_test_lucene1545() -> Result<()> {
    let a = field(0);
    /* * Standard analyzer does not correctly tokenize combining character U+0364 COMBINING LATIN SMALL LETTRE E. * The word "mo\u{364}chte" is incorrectly tokenized into "mo" "chte", the combining character is lost. * Expected result is only on token "mo\u{364}chte". */
    B::assert_analyzes_to(&*a, "mo\u{364}chte", &["mo\u{364}chte"])?;
    Ok(())
}

fn et_test_alphanumeric_sa() -> Result<()> {
    let a = field(0);
    // alphanumeric tokens
    B::assert_analyzes_to(&*a, "B2B", &["B2B"])?;
    B::assert_analyzes_to(&*a, "2B", &["2B"])?;
    Ok(())
}

fn et_test_delimiters_sa() -> Result<()> {
    let a = field(0);
    // other delimiters: "-", "/", ","
    B::assert_analyzes_to(&*a, "some-dashed-phrase", &["some", "dashed", "phrase"])?;
    B::assert_analyzes_to(&*a, "dogs,chase,cats", &["dogs", "chase", "cats"])?;
    B::assert_analyzes_to(&*a, "ac/dc", &["ac", "dc"])?;
    Ok(())
}

fn et_test_apostrophes_sa() -> Result<()> {
    let a = field(0);
    // internal apostrophes: O'Reilly, you're, O'Reilly's
    B::assert_analyzes_to(&*a, "O'Reilly", &["O'Reilly"])?;
    B::assert_analyzes_to(&*a, "you're", &["you're"])?;
    B::assert_analyzes_to(&*a, "she's", &["she's"])?;
    B::assert_analyzes_to(&*a, "Jim's", &["Jim's"])?;
    B::assert_analyzes_to(&*a, "don't", &["don't"])?;
    B::assert_analyzes_to(&*a, "O'Reilly's", &["O'Reilly's"])?;
    Ok(())
}

fn et_test_numeric_sa() -> Result<()> {
    let a = field(0);
    // floating point, serial, model numbers, ip addresses, etc.
    B::assert_analyzes_to(&*a, "21.35", &["21.35"])?;
    B::assert_analyzes_to(&*a, "R2D2 C3PO", &["R2D2", "C3PO"])?;
    B::assert_analyzes_to(&*a, "216.239.63.104", &["216.239.63.104"])?;
    Ok(())
}

fn et_test_text_with_numbers_sa() -> Result<()> {
    let a = field(0);
    // numbers
    B::assert_analyzes_to(
        &*a,
        "David has 5000 bones",
        &["David", "has", "5000", "bones"],
    )?;
    Ok(())
}

fn et_test_various_text_sa() -> Result<()> {
    let a = field(0);
    // various
    B::assert_analyzes_to(
        &*a,
        "C embedded developers wanted",
        &["C", "embedded", "developers", "wanted"],
    )?;
    B::assert_analyzes_to(&*a, "foo bar FOO BAR", &["foo", "bar", "FOO", "BAR"])?;
    B::assert_analyzes_to(
        &*a,
        "foo      bar .  FOO <> BAR",
        &["foo", "bar", "FOO", "BAR"],
    )?;
    B::assert_analyzes_to(&*a, "\"QUOTED\" word", &["QUOTED", "word"])?;
    Ok(())
}

fn et_test_korean_sa() -> Result<()> {
    let a = field(0);
    // Korean words
    B::assert_analyzes_to(
        &*a,
        "\u{C548}\u{B155}\u{D558}\u{C138}\u{C694} \u{D55C}\u{AE00}\u{C785}\u{B2C8}\u{B2E4}",
        &[
            "\u{C548}\u{B155}\u{D558}\u{C138}\u{C694}",
            "\u{D55C}\u{AE00}\u{C785}\u{B2C8}\u{B2E4}",
        ],
    )?;
    Ok(())
}

fn et_test_offsets() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to_offsets(
        &*a,
        "David has 5000 bones",
        &["David", "has", "5000", "bones"],
        &[0, 6, 10, 15],
        &[5, 9, 14, 20],
    )?;
    Ok(())
}

fn et_test_types() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to_types_only(
        &*a,
        "David has 5000 bones",
        &["David", "has", "5000", "bones"],
        &["<ALPHANUM>", "<ALPHANUM>", "<NUM>", "<ALPHANUM>"],
    )?;
    Ok(())
}

fn et_test_wiki_urls() -> Result<()> {
    let lucene_resources_wiki_page = read_resource("LuceneResourcesWikiPage.html")?;
    Assert::assert_true(!lucene_resources_wiki_page.is_empty());
    let urls = resource_lines("LuceneResourcesWikiPageURLs.txt")?;
    Assert::assert_true(!urls.is_empty());
    let urls: Vec<&str> = urls.iter().map(String::as_str).collect();
    B::assert_analyzes_to(&*field(1), &lucene_resources_wiki_page, &urls)
}

fn et_test_emails() -> Result<()> {
    let random_text_with_emails = read_resource("random.text.with.email.addresses.txt")?;
    Assert::assert_true(!random_text_with_emails.is_empty());
    let emails = resource_lines("email.addresses.from.random.text.with.email.addresses.txt")?;
    Assert::assert_true(!emails.is_empty());
    let emails: Vec<&str> = emails.iter().map(String::as_str).collect();
    B::assert_analyzes_to(&*field(2), &random_text_with_emails, &emails)
}

fn et_test_mailto_scheme_emails() -> Result<()> {
    let a = field(0);
    // See LUCENE-3880
    B::assert_analyzes_to_types_only(
        &*a,
        "mailto:test@example.org",
        &["mailto", "test@example.org"],
        &["<ALPHANUM>", "<EMAIL>"],
    )?;
    // TODO: Support full mailto: scheme URIs. See RFC 6068: http://tools.ietf.org/html/rfc6068
    // TODO: recognize ',' address delimiter. Also, see examples of ';' delimiter use at:
    // http://www.mailto.co.uk/
    // TODO: split field keys/values
    // TODO: Hex decoding + re-tokenization
    B::assert_analyzes_to_types_only(&*a, "mailto:personA@example.com,personB@example.com?cc=personC@example.com&subject=Subjectivity&body=Corpusivity%20or%20something%20like%20that", &["mailto", "personA@example.com", ",personB@example.com", "?cc=personC@example.com", "subject", "Subjectivity", "body", "Corpusivity", "20or", "20something", "20like", "20that"], &["<ALPHANUM>", "<EMAIL>", "<EMAIL>", "<EMAIL>", "<ALPHANUM>", "<ALPHANUM>", "<ALPHANUM>", "<ALPHANUM>", "<ALPHANUM>", "<ALPHANUM>", "<ALPHANUM>", "<ALPHANUM>"])?;
    Ok(())
}

fn et_test_urls() -> Result<()> {
    let random_text_with_urls = read_resource("random.text.with.urls.txt")?;
    Assert::assert_true(!random_text_with_urls.is_empty());
    let urls = resource_lines("urls.from.random.text.with.urls.txt")?;
    Assert::assert_true(!urls.is_empty());
    let urls: Vec<&str> = urls.iter().map(String::as_str).collect();
    B::assert_analyzes_to(&*field(1), &random_text_with_urls, &urls)
}

fn et_test_unicode_word_breaks() -> Result<()> {
    WordBreakTestUnicode_12_1_0::test(&*field(0))
}

fn et_test_supplementary() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to_types_only(
        &*a,
        "\u{29B05}\u{8271}\u{935F}\u{41F9}\u{612F}\u{701B}",
        &[
            "\u{29B05}",
            "\u{8271}",
            "\u{935F}",
            "\u{41F9}",
            "\u{612F}",
            "\u{701B}",
        ],
        &[
            "<IDEOGRAPHIC>",
            "<IDEOGRAPHIC>",
            "<IDEOGRAPHIC>",
            "<IDEOGRAPHIC>",
            "<IDEOGRAPHIC>",
            "<IDEOGRAPHIC>",
        ],
    )?;
    Ok(())
}

fn et_test_korean() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to_types_only(
        &*a,
        "\u{D6C8}\u{BBFC}\u{C815}\u{C74C}",
        &["\u{D6C8}\u{BBFC}\u{C815}\u{C74C}"],
        &["<HANGUL>"],
    )?;
    Ok(())
}

fn et_test_japanese() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to_types_only(
        &*a,
        "\u{4EEE}\u{540D}\u{9063}\u{3044} \u{30AB}\u{30BF}\u{30AB}\u{30CA}",
        &[
            "\u{4EEE}",
            "\u{540D}",
            "\u{9063}",
            "\u{3044}",
            "\u{30AB}\u{30BF}\u{30AB}\u{30CA}",
        ],
        &[
            "<IDEOGRAPHIC>",
            "<IDEOGRAPHIC>",
            "<IDEOGRAPHIC>",
            "<HIRAGANA>",
            "<KATAKANA>",
        ],
    )?;
    Ok(())
}

fn et_test_combining_marks() -> Result<()> {
    let a = field(0);
    B::check_one_term(&*a, "\u{3055}\u{3099}", "\u{3055}\u{3099}")?; // hiragana
    B::check_one_term(&*a, "\u{30B5}\u{3099}", "\u{30B5}\u{3099}")?; // katakana
    B::check_one_term(&*a, "\u{58F9}\u{3099}", "\u{58F9}\u{3099}")?; // ideographic
    B::check_one_term(&*a, "\u{C544}\u{3099}", "\u{C544}\u{3099}")?; // hangul
    Ok(())
}

fn et_test_mid() -> Result<()> {
    let a = field(0);
    // ':' is in \p{WB:MidLetter}, which should trigger a split unless there is a Letter char on
    // both sides
    B::assert_analyzes_to(&*a, "A:B", &["A:B"])?;
    B::assert_analyzes_to(&*a, "A::B", &["A", "B"])?;
    // '.' is in \p{WB:MidNumLet}, which should trigger a split unless there is a Letter or Numeric
    // char on both sides
    B::assert_analyzes_to(&*a, "1.2", &["1.2"])?;
    B::assert_analyzes_to(&*a, "A.B", &["A.B"])?;
    B::assert_analyzes_to(&*a, "1..2", &["1", "2"])?;
    B::assert_analyzes_to(&*a, "A..B", &["A", "B"])?;
    // ',' is in \p{WB:MidNum}, which should trigger a split unless there is a Numeric char on both
    // sides
    B::assert_analyzes_to(&*a, "1,2", &["1,2"])?;
    B::assert_analyzes_to(&*a, "1,,2", &["1", "2"])?;
    // Mixed consecutive \p{WB:MidLetter} and \p{WB:MidNumLet} should trigger a split
    B::assert_analyzes_to(&*a, "A.:B", &["A", "B"])?;
    B::assert_analyzes_to(&*a, "A:.B", &["A", "B"])?;
    // Mixed consecutive \p{WB:MidNum} and \p{WB:MidNumLet} should trigger a split
    B::assert_analyzes_to(&*a, "1,.2", &["1", "2"])?;
    B::assert_analyzes_to(&*a, "1.,2", &["1", "2"])?;
    // '_' is in \p{WB:ExtendNumLet}
    B::assert_analyzes_to(&*a, "A:B_A:B", &["A:B_A:B"])?;
    B::assert_analyzes_to(&*a, "A:B_A::B", &["A:B_A", "B"])?;
    B::assert_analyzes_to(&*a, "1.2_1.2", &["1.2_1.2"])?;
    B::assert_analyzes_to(&*a, "A.B_A.B", &["A.B_A.B"])?;
    B::assert_analyzes_to(&*a, "1.2_1..2", &["1.2_1", "2"])?;
    B::assert_analyzes_to(&*a, "A.B_A..B", &["A.B_A", "B"])?;
    B::assert_analyzes_to(&*a, "1,2_1,2", &["1,2_1,2"])?;
    B::assert_analyzes_to(&*a, "1,2_1,,2", &["1,2_1", "2"])?;
    B::assert_analyzes_to(&*a, "C_A.:B", &["C_A", "B"])?;
    B::assert_analyzes_to(&*a, "C_A:.B", &["C_A", "B"])?;
    B::assert_analyzes_to(&*a, "3_1,.2", &["3_1", "2"])?;
    B::assert_analyzes_to(&*a, "3_1.,2", &["3_1", "2"])?;
    Ok(())
}

fn et_test_emoji() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to_types_only(
        &*a,
        "\u{1F4A9} \u{1F4A9}\u{1F4A9}",
        &["\u{1F4A9}", "\u{1F4A9}", "\u{1F4A9}"],
        &["<EMOJI>", "<EMOJI>", "<EMOJI>"],
    )?;
    Ok(())
}

fn et_test_emoji_sequence() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to_types_only(
        &*a,
        "\u{1F469}\u{200D}\u{2764}\u{FE0F}\u{200D}\u{1F469}",
        &["\u{1F469}\u{200D}\u{2764}\u{FE0F}\u{200D}\u{1F469}"],
        &["<EMOJI>"],
    )?;
    Ok(())
}

fn et_test_emoji_sequence_with_modifier() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to_types_only(
        &*a,
        "\u{1F468}\u{1F3FC}\u{200D}\u{2695}\u{FE0F}",
        &["\u{1F468}\u{1F3FC}\u{200D}\u{2695}\u{FE0F}"],
        &["<EMOJI>"],
    )?;
    Ok(())
}

fn et_test_emoji_regional_indicator() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to_types_only(
        &*a,
        "\u{1F1FA}\u{1F1F8}\u{1F1FA}\u{1F1F8}",
        &["\u{1F1FA}\u{1F1F8}", "\u{1F1FA}\u{1F1F8}"],
        &["<EMOJI>", "<EMOJI>"],
    )?;
    Ok(())
}

fn et_test_emoji_variation_sequence() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to_types_only(
        &*a,
        "#\u{FE0F}\u{20E3}",
        &["#\u{FE0F}\u{20E3}"],
        &["<EMOJI>"],
    )?;
    B::assert_analyzes_to_types_only(
        &*a,
        "3\u{FE0F}\u{20E3}",
        &["3\u{FE0F}\u{20E3}"],
        &["<EMOJI>"],
    )?;
    // text presentation sequences
    B::assert_analyzes_to_types_only(&*a, "#\u{FE0E}", &[], &[])?;
    // \uFE0E is included in \p{WB:Extend}
    B::assert_analyzes_to_types_only(&*a, "3\u{FE0E}", &["3\u{FE0E}"], &["<NUM>"])?;
    // \u2B55 = HEAVY BLACK CIRCLE
    B::assert_analyzes_to_types_only(&*a, "\u{2B55}\u{FE0E}", &["\u{2B55}"], &["<EMOJI>"])?;
    B::assert_analyzes_to_types_only(
        &*a,
        "\u{2B55}\u{FE0E}\u{200D}\u{2B55}\u{FE0E}",
        &["\u{2B55}", "\u{200D}\u{2B55}"],
        &["<EMOJI>", "<EMOJI>"],
    )?;
    Ok(())
}

fn et_test_emoji_tag_sequence() -> Result<()> {
    let a = field(0);
    B::assert_analyzes_to_types_only(
        &*a,
        "\u{1F3F4}\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}",
        &["\u{1F3F4}\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}"],
        &["<EMOJI>"],
    )?;
    Ok(())
}

fn et_test_emoji_tokenization() -> Result<()> {
    let a = field(0);
    // simple emoji around latin
    B::assert_analyzes_to_types_only(
        &*a,
        "poo\u{1F4A9}poo",
        &["poo", "\u{1F4A9}", "poo"],
        &["<ALPHANUM>", "<EMOJI>", "<ALPHANUM>"],
    )?;
    // simple emoji around non-latin
    B::assert_analyzes_to_types_only(
        &*a,
        "\u{1F4A9}\u{4E2D}\u{570B}\u{1F4A9}",
        &["\u{1F4A9}", "\u{4E2D}", "\u{570B}", "\u{1F4A9}"],
        &["<EMOJI>", "<IDEOGRAPHIC>", "<IDEOGRAPHIC>", "<EMOJI>"],
    )?;
    Ok(())
}

fn et_test_unicode_emoji_tests() -> Result<()> {
    EmojiTokenizationTestUnicode_12_1::test(&*field(0))
}

fn et_test_random_strings() -> Result<()> {
    B::check_random_data(
        &*LuceneTestCase::random(),
        &field(0),
        200 * *RANDOM_MULTIPLIER,
    )
}

fn et_test_random_huge_strings() -> Result<()> {
    let random = LuceneTestCase::random();
    B::check_random_data_max_word_length(&*random, &field(0), 3 * *RANDOM_MULTIPLIER, 8192)
}

fn et_test_example_urls() -> Result<()> {
    let text = String::from_utf8(resource("TLDs.txt").to_vec()).expect("UTF-8");
    // `split("\n")` drops trailing empty strings; blank and comment lines are filtered out
    let tlds: Vec<&str> = text
        .split('\n')
        .filter(|line| {
            !line.trim_matches(|c: char| c <= ' ').starts_with('#') && !line.trim().is_empty()
        })
        .collect();
    let analyzer =
        tokenizer_analyzer("org.apache.lucene.analysis.email.TestUAX29URLEmailTokenizer$4");
    for tld in tlds {
        let url = format!("example.{tld}");
        B::assert_analyzes_to_types_only(&*analyzer, &url, &[&url], &["<URL>"])?;
    }
    Ok(())
}
fn uax29_url_email_tokenizer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.email.TestUAX29URLEmailTokenizer")
        .method(hook("setUp", et_set_up))
        .method(hook("tearDown", close_fields_tear_down))
        .method(t("testHugeDoc", et_test_huge_doc))
        .method(t("testArmenian", et_test_armenian))
        .method(t("testAmharic", et_test_amharic))
        .method(t("testArabic", et_test_arabic))
        .method(t("testAramaic", et_test_aramaic))
        .method(t("testBengali", et_test_bengali))
        .method(t("testFarsi", et_test_farsi))
        .method(t("testGreek", et_test_greek))
        .method(t("testThai", et_test_thai))
        .method(t("testLao", et_test_lao))
        .method(t("testTibetan", et_test_tibetan))
        .method(t("testChinese", et_test_chinese))
        .method(t("testEmpty", et_test_empty))
        .method(t("testLUCENE1545", et_test_lucene1545))
        .method(t("testAlphanumericSA", et_test_alphanumeric_sa))
        .method(t("testDelimitersSA", et_test_delimiters_sa))
        .method(t("testApostrophesSA", et_test_apostrophes_sa))
        .method(t("testNumericSA", et_test_numeric_sa))
        .method(t("testTextWithNumbersSA", et_test_text_with_numbers_sa))
        .method(t("testVariousTextSA", et_test_various_text_sa))
        .method(t("testKoreanSA", et_test_korean_sa))
        .method(t("testOffsets", et_test_offsets))
        .method(t("testTypes", et_test_types))
        .method(t("testWikiURLs", et_test_wiki_urls))
        .method(t("testEmails", et_test_emails))
        .method(t("testMailtoSchemeEmails", et_test_mailto_scheme_emails))
        .method(t("testURLs", et_test_urls))
        .method(t("testUnicodeWordBreaks", et_test_unicode_word_breaks))
        .method(t("testSupplementary", et_test_supplementary))
        .method(t("testKorean", et_test_korean))
        .method(t("testJapanese", et_test_japanese))
        .method(t("testCombiningMarks", et_test_combining_marks))
        .method(t("testMid", et_test_mid))
        .method(t("testEmoji", et_test_emoji))
        .method(t("testEmojiSequence", et_test_emoji_sequence))
        .method(t(
            "testEmojiSequenceWithModifier",
            et_test_emoji_sequence_with_modifier,
        ))
        .method(t(
            "testEmojiRegionalIndicator",
            et_test_emoji_regional_indicator,
        ))
        .method(t(
            "testEmojiVariationSequence",
            et_test_emoji_variation_sequence,
        ))
        .method(t("testEmojiTagSequence", et_test_emoji_tag_sequence))
        .method(t("testEmojiTokenization", et_test_emoji_tokenization))
        .method(t("testUnicodeEmojiTests", et_test_unicode_emoji_tests))
        .method(t("testRandomStrings", et_test_random_strings))
        .method(t("testRandomHugeStrings", et_test_random_huge_strings))
        .method(t("testExampleURLs", et_test_example_urls))
        .build()
}
/// The class with only its `@Nightly` method, `testLongEMAILatomText`
/// (unannotated: see the module docs).
fn uax29_url_email_tokenizer_nightly_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.email.TestUAX29URLEmailTokenizer")
        .method(hook("setUp", et_set_up))
        .method(hook("tearDown", close_fields_tear_down))
        .method(t("testLongEMAILatomText", et_test_long_emailatom_text))
        .build()
}

// -----------------------------------------------------------------------------
// TestUAX29URLEmailTokenizerFactory
// -----------------------------------------------------------------------------

/// `tokenizerFactory("UAX29URLEmail", params).create(newAttributeFactory())`
/// over `text`.
fn factory_stream(
    params: &[&str],
    text: &str,
) -> Result<rucene::analysis::factories::FactoryTokenizer> {
    let mut stream = F::tokenizer_factory("UAX29URLEmail", params)?
        .create_with_factory(B::new_attribute_factory())?;
    stream.set_reader(Box::new(StringCharReader::new(text)))?;
    Ok(stream)
}

fn ef_test_uax29_urlemail_tokenizer() -> Result<()> {
    let mut stream = factory_stream(&[], "Wha\u{301}t's this thing do?")?;
    B::assert_token_stream_contents(&mut stream, &["Wha\u{301}t's", "this", "thing", "do"])?;
    Ok(())
}

fn ef_test_arabic() -> Result<()> {
    let mut stream = factory_stream(&[], "\u{627}\u{644}\u{641}\u{64A}\u{644}\u{645} \u{627}\u{644}\u{648}\u{62B}\u{627}\u{626}\u{642}\u{64A} \u{627}\u{644}\u{623}\u{648}\u{644} \u{639}\u{646} \u{648}\u{64A}\u{643}\u{64A}\u{628}\u{64A}\u{62F}\u{64A}\u{627} \u{64A}\u{633}\u{645}\u{649} \"\u{627}\u{644}\u{62D}\u{642}\u{64A}\u{642}\u{629} \u{628}\u{627}\u{644}\u{623}\u{631}\u{642}\u{627}\u{645}: \u{642}\u{635}\u{629} \u{648}\u{64A}\u{643}\u{64A}\u{628}\u{64A}\u{62F}\u{64A}\u{627}\" (\u{628}\u{627}\u{644}\u{625}\u{646}\u{62C}\u{644}\u{64A}\u{632}\u{64A}\u{629}: Truth in Numbers: The Wikipedia Story)\u{60C} \u{633}\u{64A}\u{62A}\u{645} \u{625}\u{637}\u{644}\u{627}\u{642}\u{647} \u{641}\u{64A} 2008.")?;
    B::assert_token_stream_contents(
        &mut stream,
        &[
            "\u{627}\u{644}\u{641}\u{64A}\u{644}\u{645}",
            "\u{627}\u{644}\u{648}\u{62B}\u{627}\u{626}\u{642}\u{64A}",
            "\u{627}\u{644}\u{623}\u{648}\u{644}",
            "\u{639}\u{646}",
            "\u{648}\u{64A}\u{643}\u{64A}\u{628}\u{64A}\u{62F}\u{64A}\u{627}",
            "\u{64A}\u{633}\u{645}\u{649}",
            "\u{627}\u{644}\u{62D}\u{642}\u{64A}\u{642}\u{629}",
            "\u{628}\u{627}\u{644}\u{623}\u{631}\u{642}\u{627}\u{645}",
            "\u{642}\u{635}\u{629}",
            "\u{648}\u{64A}\u{643}\u{64A}\u{628}\u{64A}\u{62F}\u{64A}\u{627}",
            "\u{628}\u{627}\u{644}\u{625}\u{646}\u{62C}\u{644}\u{64A}\u{632}\u{64A}\u{629}",
            "Truth",
            "in",
            "Numbers",
            "The",
            "Wikipedia",
            "Story",
            "\u{633}\u{64A}\u{62A}\u{645}",
            "\u{625}\u{637}\u{644}\u{627}\u{642}\u{647}",
            "\u{641}\u{64A}",
            "2008",
        ],
    )?;
    Ok(())
}

fn ef_test_chinese() -> Result<()> {
    let mut stream = factory_stream(&[], "\u{6211}\u{662F}\u{4E2D}\u{56FD}\u{4EBA}\u{3002} \u{FF11}\u{FF12}\u{FF13}\u{FF14} \u{FF34}\u{FF45}\u{FF53}\u{FF54}\u{FF53} ")?;
    B::assert_token_stream_contents(
        &mut stream,
        &[
            "\u{6211}",
            "\u{662F}",
            "\u{4E2D}",
            "\u{56FD}",
            "\u{4EBA}",
            "\u{FF11}\u{FF12}\u{FF13}\u{FF14}",
            "\u{FF34}\u{FF45}\u{FF53}\u{FF54}\u{FF53}",
        ],
    )?;
    Ok(())
}

fn ef_test_korean() -> Result<()> {
    let mut stream = factory_stream(
        &[],
        "\u{C548}\u{B155}\u{D558}\u{C138}\u{C694} \u{D55C}\u{AE00}\u{C785}\u{B2C8}\u{B2E4}",
    )?;
    B::assert_token_stream_contents(
        &mut stream,
        &[
            "\u{C548}\u{B155}\u{D558}\u{C138}\u{C694}",
            "\u{D55C}\u{AE00}\u{C785}\u{B2C8}\u{B2E4}",
        ],
    )?;
    Ok(())
}

fn ef_test_hyphen() -> Result<()> {
    let mut stream = factory_stream(&[], "some-dashed-phrase")?;
    B::assert_token_stream_contents(&mut stream, &["some", "dashed", "phrase"])?;
    Ok(())
}

fn ef_test_urls() -> Result<()> {
    let mut stream = factory_stream(&[], "http://johno.jsmf.net/knowhow/ngrams/index.php?table=en-dickens-word-2gram&paragraphs=50&length=200&no-ads=on\n some extra\nWords thrown in here. http://c5-3486.bisynxu.FR/aI.YnNms/ samba Halta gamba ftp://119.220.152.185/JgJgdZ/31aW5c/viWlfQSTs5/1c8U5T/ih5rXx/YfUJ/xBW1uHrQo6.R\nM19nq.0URV4A.Me.CC/mj0kgt6hue/dRXv8YVLOw9v/CIOqb\nHttps://yu7v33rbt.vC6U3.XN--KPRW13D/y%4fMSzkGFlm/wbDF4m inter Locutio [c2d4::]/%471j5l/j3KFN%AAAn/Fip-NisKH/\nfile:///aXvSZS34is/eIgM8s~U5dU4Ifd%c7 blah Sirrah woof http://[a42:a7b6::]/qSmxSUU4z/%52qVl4\n")?;
    B::assert_token_stream_contents(&mut stream, &["http://johno.jsmf.net/knowhow/ngrams/index.php?table=en-dickens-word-2gram&paragraphs=50&length=200&no-ads=on", "some", "extra", "Words", "thrown", "in", "here", "http://c5-3486.bisynxu.FR/aI.YnNms/", "samba", "Halta", "gamba", "ftp://119.220.152.185/JgJgdZ/31aW5c/viWlfQSTs5/1c8U5T/ih5rXx/YfUJ/xBW1uHrQo6.R", "M19nq.0URV4A.Me.CC/mj0kgt6hue/dRXv8YVLOw9v/CIOqb", "Https://yu7v33rbt.vC6U3.XN--KPRW13D/y%4fMSzkGFlm/wbDF4m", "inter", "Locutio", "[c2d4::]/%471j5l/j3KFN%AAAn/Fip-NisKH/", "file:///aXvSZS34is/eIgM8s~U5dU4Ifd%c7", "blah", "Sirrah", "woof", "http://[a42:a7b6::]/qSmxSUU4z/%52qVl4"])?;
    Ok(())
}

fn ef_test_emails() -> Result<()> {
    let mut stream = factory_stream(&[], " some extra\nWords thrown in here. dJ8ngFi@avz13m.CC\nkU-l6DS@[082.015.228.189]\n\"%U\u{12}@?\\B\"@Fl2d.md samba Halta gamba Bvd#@tupjv.sn\nSBMm0Nm.oyk70.rMNdd8k.#ru3LI.gMMLBI.0dZRD4d.RVK2nY@au58t.B13albgy4u.mt\n~+Kdz@3mousnl.SE\n inter Locutio C'ts`@Vh4zk.uoafcft-dr753x4odt04q.UY\n}0tzWYDBuy@cSRQAABB9B.7c8xawf75-cyo.PM blah Sirrah woof lMahAA.j/5.RqUjS745.DtkcYdi@d2-4gb-l6.ae\nlv'p@tqk.vj5s0tgl.0dlu7su3iyiaz.dqso.494.3hb76.XN--MGBAAM7A8H\n")?;
    B::assert_token_stream_contents(
        &mut stream,
        &[
            "some",
            "extra",
            "Words",
            "thrown",
            "in",
            "here",
            "dJ8ngFi@avz13m.CC",
            "kU-l6DS@[082.015.228.189]",
            "\"%U\u{12}@?\\B\"@Fl2d.md",
            "samba",
            "Halta",
            "gamba",
            "Bvd#@tupjv.sn",
            "SBMm0Nm.oyk70.rMNdd8k.#ru3LI.gMMLBI.0dZRD4d.RVK2nY@au58t.B13albgy4u.mt",
            "~+Kdz@3mousnl.SE",
            "inter",
            "Locutio",
            "C'ts`@Vh4zk.uoafcft-dr753x4odt04q.UY",
            "}0tzWYDBuy@cSRQAABB9B.7c8xawf75-cyo.PM",
            "blah",
            "Sirrah",
            "woof",
            "lMahAA.j/5.RqUjS745.DtkcYdi@d2-4gb-l6.ae",
            "lv'p@tqk.vj5s0tgl.0dlu7su3iyiaz.dqso.494.3hb76.XN--MGBAAM7A8H",
        ],
    )?;
    Ok(())
}

fn ef_test_max_token_length() -> Result<()> {
    let long_word = "abcdefg".repeat(100);
    let content = format!("one two three {long_word} four five six");
    let mut stream = factory_stream(&["maxTokenLength", "1000"], &content)?;
    B::assert_token_stream_contents(
        &mut stream,
        &["one", "two", "three", &long_word, "four", "five", "six"],
    )
}

fn ef_test_bogus_arguments() -> Result<()> {
    let expected = expect_throws("java.lang.IllegalArgumentException", || {
        F::tokenizer_factory("UAX29URLEmail", &["bogusArg", "bogusValue"])
    });
    Assert::assert_true(
        expected
            .message()
            .unwrap_or("null")
            .contains("Unknown parameters"),
    );
    Ok(())
}

fn ef_test_illegal_arguments() -> Result<()> {
    let expected = expect_throws("java.lang.IllegalArgumentException", || {
        F::tokenizer_factory("UAX29URLEmail", &["maxTokenLength", "-1"])?.create()
    });
    Assert::assert_true(
        expected
            .message()
            .unwrap_or("null")
            .contains("maxTokenLength must be greater than zero"),
    );
    Ok(())
}
fn uax29_url_email_tokenizer_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.email.TestUAX29URLEmailTokenizerFactory")
        .method(t(
            "testUAX29URLEmailTokenizer",
            ef_test_uax29_urlemail_tokenizer,
        ))
        .method(t("testArabic", ef_test_arabic))
        .method(t("testChinese", ef_test_chinese))
        .method(t("testKorean", ef_test_korean))
        .method(t("testHyphen", ef_test_hyphen))
        .method(t("testURLs", ef_test_urls))
        .method(t("testEmails", ef_test_emails))
        .method(t("testMaxTokenLength", ef_test_max_token_length))
        .method(t("testBogusArguments", ef_test_bogus_arguments))
        .method(t("testIllegalArguments", ef_test_illegal_arguments))
        .build()
}

#[test]
fn test_uax29_url_email_analyzer() {
    run(uax29_url_email_analyzer_class);
}

#[test]
fn test_uax29_url_email_tokenizer() {
    run(uax29_url_email_tokenizer_class);
}

/// `testLongEMAILatomText` (`@Nightly` in Lucene; see the module docs).
#[test]
fn test_uax29_url_email_tokenizer_nightly() {
    run(uax29_url_email_tokenizer_nightly_class);
}

#[test]
fn test_uax29_url_email_tokenizer_factory() {
    run(uax29_url_email_tokenizer_factory_class);
}
