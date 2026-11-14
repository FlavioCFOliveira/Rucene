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
//! that rmp task #457 lists, each run as a suite by the ported runner:
//! `org.apache.lucene.analysis.charfilter.TestHTMLStripCharFilter`,
//! `TestHTMLStripCharFilterFactory`, `TestMappingCharFilter` and
//! `TestMappingCharFilterFactory`, with the resources `htmlStripReaderTest.html`
//! and `MS-Word 14 generated.htm` copied byte for byte.
//!
//! `TestHTMLStripCharFilter.doTestOffsets(String)`, `assertLegalOffsets`,
//! `assertHTMLStripsTo` and `appendChars` are helpers (they take arguments),
//! not test methods. Where the Java wraps a `StringReader` in a
//! `java.io.BufferedReader` the port reads the `StringCharReader` directly:
//! `BufferedReader.read(char[], int, int)` keeps reading while the
//! underlying `StringReader` is `ready()` (always), so it hands the filter
//! the same units per call as the `StringReader` does.

mod support;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{CharFilter, CharReader, StringCharReader, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::index::index_writer::MAX_TERM_LENGTH;
use rucene_analysis_common::analysis::charfilter::html_strip_char_filter::HTMLStripCharFilter;
use rucene_analysis_common::analysis::charfilter::mapping_char_filter::MappingCharFilter;
use rucene_analysis_common::analysis::charfilter::mapping_char_filter_factory::MappingCharFilterFactory;
use rucene_analysis_common::analysis::charfilter::normalize_char_map::{
    Builder as NormalizeCharMapBuilder, NormalizeCharMap,
};
use rucene_analysis_common::jdk::io::InputStreamReader;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, WHITESPACE};
use rucene_test_framework::analysis::{BaseTokenStreamFactoryTestCase, BaseTokenStreamTestCase};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::{RANDOM_MULTIPLIER, VERBOSE};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};
use support::{btstc_class, factory_class, hook, iae_message, own, run, t, FnAnalyzer};

type B = BaseTokenStreamTestCase;
type F = BaseTokenStreamFactoryTestCase;

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn reader(s: &str) -> Box<dyn CharReader> {
    Box::new(StringCharReader::new(s))
}

// -----------------------------------------------------------------------------
// TestHTMLStripCharFilter
// -----------------------------------------------------------------------------

/// `newTestAnalyzer()`.
fn new_test_analyzer() -> Arc<dyn rucene::analysis::Analyzer> {
    use rucene_test_framework::analysis::tokenizer_components;
    FnAnalyzer::new(
        "org.apache.lucene.analysis.charfilter.TestHTMLStripCharFilter$1",
        |_| {
            Ok(tokenizer_components(own(
                MockTokenizer::with_max_token_length(
                    Arc::clone(&WHITESPACE),
                    false,
                    (MAX_TERM_LENGTH / 2) as i32,
                )?,
            )))
        },
    )
    .init_reader(|_, r| Box::new(HTMLStripCharFilter::new(r)))
    .arc()
}

/// Every unit `reader.read()` returns, until `-1`.
fn read_all(reader: &mut HTMLStripCharFilter) -> Result<Vec<u16>> {
    let mut out = Vec::new();
    loop {
        let ch = reader.read_char()?;
        if ch == -1 {
            return Ok(out);
        }
        out.push(ch as u16);
    }
}

/// `assertHTMLStripsTo(Reader input, String gold, Set<String> escapedTags)`.
fn assert_html_strips_to_reader(
    input: Box<dyn CharReader>,
    gold: &str,
    escaped_tags: Option<&HashSet<String>>,
) -> Result<()> {
    let mut reader = match escaped_tags {
        None => HTMLStripCharFilter::new(input),
        Some(tags) => HTMLStripCharFilter::with_escaped_tags(input, Some(tags)),
    };
    let mut builder: Vec<u16> = Vec::new();
    loop {
        match reader.read_char() {
            Ok(-1) => break,
            Ok(ch) => builder.push(ch as u16),
            Err(e) => {
                if utf16(gold) == builder {
                    return Err(e);
                }
                return Err(LuceneError::unchecked(
                    "java.lang.Exception",
                    format!(
                        "('{}' is not equal to '{}').  {}",
                        String::from_utf16_lossy(&builder),
                        gold,
                        e
                    ),
                ));
            }
        }
    }
    let actual = String::from_utf16_lossy(&builder);
    Assert::assert_equals_msg(
        Some(&format!("'{actual}' is not equal to '{gold}'")),
        gold,
        actual.as_str(),
    );
    Ok(())
}

/// `assertHTMLStripsTo(String input, String gold, Set<String> escapedTags)`.
fn assert_html_strips_to(
    input: &str,
    gold: &str,
    escaped_tags: Option<&HashSet<String>>,
) -> Result<()> {
    assert_html_strips_to_reader(reader(input), gold, escaped_tags)
}

fn set(tags: &[&str]) -> HashSet<String> {
    tags.iter().map(|s| s.to_string()).collect()
}

/// The bytes of a test resource of the `charfilter` package.
fn resource(name: &str) -> &'static [u8] {
    let full = format!("org/apache/lucene/analysis/charfilter/{name}");
    support::TEST_RESOURCES
        .iter()
        .find(|(n, _)| *n == full)
        .unwrap_or_else(|| panic!("resource {full}"))
        .1
}

fn hs_test() -> Result<()> {
    let html =
        "<div class=\"foo\">this is some text</div> here is a <a href=\"#bar\">link</a> and \
                another <a href=\"http://lucene.apache.org/\">link</a>. \
                This is an entity: &amp; plus a &lt;.  Here is an &. <!-- is a comment -->";
    let gold = "\nthis is some text\n here is a link and \
                another link. \
                This is an entity: & plus a <.  Here is an &. ";
    assert_html_strips_to(html, gold, None)
}

/// Some sanity checks, but not a full-fledged check
fn hs_test_html() -> Result<()> {
    let mut reader = HTMLStripCharFilter::new(Box::new(InputStreamReader::new_utf8(resource(
        "htmlStripReaderTest.html",
    ))));
    let s = String::from_utf16_lossy(&read_all(&mut reader)?);
    Assert::assert_true_msg("Entity not properly escaped", !s.contains("&lt;"));
    Assert::assert_true_msg(
        "Forrest should have been stripped out",
        !s.contains("forrest") && !s.contains("Forrest"),
    );
    Assert::assert_true_msg(
        "File should start with 'Welcome to Solr' after trimming",
        java_trim(&s).starts_with("Welcome to Solr"),
    );
    Assert::assert_true_msg(
        "File should start with 'Foundation.' after trimming",
        java_trim(&s).ends_with("Foundation."),
    );
    Ok(())
}

/// `String.trim()`: strips the units `<= ' '` at both ends.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

fn hs_test_ms_word14_generated_html() -> Result<()> {
    let mut reader = HTMLStripCharFilter::new(Box::new(InputStreamReader::new_utf8(resource(
        "MS-Word 14 generated.htm",
    ))));
    let gold = "This is a test";
    let s = String::from_utf16_lossy(&read_all(&mut reader)?);
    Assert::assert_equals_msg(
        Some(&format!("'{}' is not equal to '{gold}'", java_trim(&s))),
        gold,
        java_trim(&s),
    );
    Ok(())
}

fn hs_test_gamma() -> Result<()> {
    assert_html_strips_to("&Gamma;", "\u{393}", Some(&set(&["reserved"])))
}

fn hs_test_entities() -> Result<()> {
    let test = "&nbsp; &lt;foo&gt; &Uuml;bermensch &#61; &Gamma; bar &#x393;";
    let gold = "  <foo> \u{DC}bermensch = \u{393} bar \u{393}";
    assert_html_strips_to(test, gold, Some(&set(&["reserved"])))
}

fn hs_test_more_entities() -> Result<()> {
    let test = "&nbsp; &lt;junk/&gt; &nbsp; &#33; &#64; and &#8217;";
    let gold = "  <junk/>   ! @ and \u{2019}";
    assert_html_strips_to(test, gold, Some(&set(&["reserved"])))
}

/// `String.indexOf(str, fromIndex)` over UTF-16 units.
fn index_of(s: &[u16], needle: &str, from: usize) -> i32 {
    let n = utf16(needle);
    if from > s.len() {
        return -1;
    }
    (from..=s.len().saturating_sub(n.len()))
        .find(|&i| s.len() >= n.len() && s[i..i + n.len()] == n[..])
        .map_or(-1, |i| i as i32)
}

fn hs_test_reserved() -> Result<()> {
    let test =
        "aaa bbb <reserved ccc=\"ddddd\"> eeee </reserved> ffff <reserved ggg=\"hhhh\"/> <other/>";
    let tags = set(&["reserved"]);
    let mut reader = HTMLStripCharFilter::with_escaped_tags(reader(test), Some(&tags));
    let result = read_all(&mut reader)?;
    Assert::assert_true_msg(
        &format!(
            "Escaped tag not preserved: {}",
            index_of(&result, "reserved", 0)
        ),
        index_of(&result, "reserved", 0) == 9,
    );
    Assert::assert_true_msg(
        &format!(
            "Escaped tag not preserved: {}",
            index_of(&result, "reserved", 15)
        ),
        index_of(&result, "reserved", 15) == 38,
    );
    Assert::assert_true_msg(
        &format!(
            "Escaped tag not preserved: {}",
            index_of(&result, "reserved", 41)
        ),
        index_of(&result, "reserved", 41) == 54,
    );
    Assert::assert_true_msg(
        "Other tag should be removed",
        index_of(&result, "other", 0) == -1,
    );
    Ok(())
}

fn hs_test_malformed_html() -> Result<()> {
    let test_gold: &[&str] = &[

      "a <a hr<ef=aa<a>> </close</a>",
      "a <a hr<ef=aa> </close",
      "<a href=http://dmoz.org/cgi-bin/add.cgi?where=/arts/\" class=lu style=\"font-size: 9px\" target=dmoz>Submit a Site</a>",
      "Submit a Site",
      "<a href=javascript:ioSwitch('p8','http://www.csmonitor.com/') title=expand id=e8 class=expanded rel=http://www.csmonitor.com/>Christian Science",
      "Christian Science",
      "<link rel=\"alternate\" type=\"application/rss+xml\" title=\"San Francisco \" 2008 RSS Feed\" href=\"http://2008.sf.wordcamp.org/feed/\" />",
      "\n",

      // "<" before ">" inhibits tag recognition
      "<a href=\" http://www.surgery4was.happyhost.org/video-of-arthroscopic-knee-surgery symptoms.html, heat congestive heart failure <a href=\" http://www.symptoms1bad.happyhost.org/canine",
      "<a href=\" http://www.surgery4was.happyhost.org/video-of-arthroscopic-knee-surgery symptoms.html, heat congestive heart failure <a href=\" http://www.symptoms1bad.happyhost.org/canine",
      "<a href=\"http://ucblibraries.colorado.edu/how/index.htm\"class=\"pageNavAreaText\">",
      "",
      "<link title=\"^\\\" 21Sta's Blog\" rel=\"search\"  type=\"application/opensearchdescription+xml\"  href=\"http://21sta.com/blog/inc/opensearch.php\" />",
      "\n",
      "<a href=\"#postcomment\" title=\"\"Leave a comment\";\">?",
      "?",
      "<a href='/modern-furniture'   ' id='21txt' class='offtab'   onMouseout=\"this.className='offtab';  return true;\" onMouseover=\"this.className='ontab';  return true;\">",
      "",
      "<a href='http://alievi.wordpress.com/category/01-todos-posts/' style='font-size: 275%; padding: 1px; margin: 1px;' title='01 - Todos Post's (83)'>",
      "",
      "The <a href=<a href=\"http://www.advancedmd.com>medical\">http://www.advancedmd.com>medical</a> practice software</a>",
      "The <a href=http://www.advancedmd.com>medical practice software",
      "<a href=\"node/21426\" class=\"clipTitle2\" title=\"Levi.com/BMX 2008 Clip of the Week 29 \"Morgan Wade Leftover Clips\"\">Levi.com/BMX 2008 Clip of the Week 29...",
      "Levi.com/BMX 2008 Clip of the Week 29...",
      "<a href=\"printer_friendly.php?branch=&year=&submit=go&screen=\";\">Printer Friendly",
      "Printer Friendly",
      "<a href=#\" ondragstart=\"return false\" onclick=\"window.external.AddFavorite('http://www.amazingtextures.com', 'Amazing Textures');return false\" onmouseover=\"window.status='Add to Favorites';return true\">Add to Favorites",
      "Add to Favorites",
      "<a href=\"../at_home/at_home_search.html\"../_home/at_home_search.html\">At",
      "At",
      "E-mail: <a href=\"\"mailto:XXXXXX@example.com\" \">XXXXXX@example.com </a>",
      "E-mail: XXXXXX@example.com ",
      "<li class=\"farsi\"><a title=\"A'13?\" alt=\"A'13?\" href=\"http://www.america.gov/persian\" alt=\"\" name=\"A'13?\"A'13? title=\"A'13?\">A'13?</a></li>",
      "\nA'13?\n",
      "<li><a href=\"#28\" title=\"Hubert \"Geese\" Ausby\">Hubert \"Geese\" Ausby</a></li>",
      "\nHubert \"Geese\" Ausby\n",
      "<href=\"http://anbportal.com/mms/login.asp\">",
      "\n",
      "<a href=\"",
      "<a href=\"",
      "<a href=\">",
      "",
      "<a rel=\"nofollow\" href=\"http://anissanina31.skyrock.com/1895039493-Hi-tout-le-monde.html\" title=\" Hi, tout le monde !>#</a>",
      "#",
      "<a href=\"http://annunciharleydavidsonusate.myblog.it/\" title=\"Annunci Moto e Accessori Harley Davidson\" target=\"_blank\"><img src=\"http://annunciharleydavidsonusate.myblog.it/images/Antipixel.gif\" /></a>",
      "",
      "<a href=\"video/addvideo&v=120838887181\" onClick=\"return confirm('Are you sure you want  add this video to your profile? If it exists some video in your profile will be overlapped by this video!!')\" \" onmouseover=\"this.className='border2'\" onmouseout=\"this.className=''\">",
      "",
      "<a href=#Services & Support>",
      "",

      // "<" and ">" chars are accepted in on[Event] attribute values
      "<input type=\"image\" src=\"http://apologyindex.com/ThemeFiles/83401-72905/images/btn_search.gif\"value=\"Search\" name=\"Search\" alt=\"Search\" class=\"searchimage\" onclick=\"incom ='&sc=' + document.getElementById('sel').value ; var dt ='&dt=' + document.getElementById('dt').value; var searchKeyword = document.getElementById('q').value ; searchKeyword = searchKeyword.replace(/\\s/g,''); if (searchKeyword.length < 3){alert('Nothing to search. Search keyword should contain atleast 3 chars.'); return false; } var al='&al=' +  document.getElementById('advancedlink').style.display ;  document.location.href='http://apologyindex.com/search.aspx?q=' + document.getElementById('q').value + incom + dt + al;\" />",
      "",
      "<input type=\"image\" src=\"images/afbe.gif\" width=\"22\" height=\"22\"  hspace=\"4\" title=\"Add to Favorite\" alt=\"Add to Favorite\"onClick=\" if(window.sidebar){ window.sidebar.addPanel(document.title,location.href,''); }else if(window.external){ window.external.AddFavorite(location.href,document.title); }else if(window.opera&&window.print) { return true; }\">",
      "",
      "<area shape=\"rect\" coords=\"12,153,115,305\" href=\"http://statenislandtalk.com/v-web/gallery/Osmundsen-family\"Art's Norwegian Roots in Rogaland\">",
      "\n",
      "<a rel=\"nofollow\" href=\"http://arth26.skyrock.com/660188240-bonzai.html\" title=\"bonza>#",
      "#",
      "<a href=  >",
      "",
      "<ahref=http:..",
      "<ahref=http:..",
      "<ahref=http:..>",
      "\n",
      "<ahref=\"http://aseigo.bddf.ca/cms/1025\">A",
      "\nA",
      "<a href=\"javascript:calendar_window=window.open('/calendar.aspx?formname=frmCalendar.txtDate','calendar_window','width=154,height=188');calendar_window.focus()\">",
      "",
      "<a href=\"/applications/defenseaerospace/19+rackmounts\" title=\"19\" Rackmounts\">",
      "",
      "<a href=http://www.azimprimerie.fr/flash/backup/lewes-zip-code/savage-model-110-manual.html title=savage model 110 manual rel=dofollow>",
      "",
      "<a class=\"at\" name=\"Lamborghini  href=\"http://lamborghini.coolbegin.com\">Lamborghini /a>",
      "Lamborghini /a>",
      "<A href='newslink.php?news_link=http%3A%2F%2Fwww.worldnetdaily.com%2Findex.php%3Ffa%3DPAGE.view%26pageId%3D85729&news_title=Florida QB makes 'John 3:16' hottest Google search Tebow inscribed Bible reference on eye black for championship game' TARGET=_blank>",
      "",
      "<a href=/myspace !style='color:#993333'>",
      "",
      "<meta name=3DProgId content=3DExcel.Sheet>",
      "\n",
      "<link id=3D\"shLink\" href=3D\"PSABrKelly-BADMINTONCupResults08FINAL2008_09_19=_files/sheet004.htm\">",
      "\n",
      "<td bgcolor=3D\"#FFFFFF\" nowrap>",
      "\n",
      "<a href=\"http://basnect.info/usersearch/\"predicciones-mundiales-2009\".html\">\"predicciones mundiales 2009\"</a>",
      "\"predicciones mundiales 2009\"",
      "<a class=\"comment-link\" href=\"https://www.blogger.com/comment.g?blogID=19402125&postID=114070605958684588\"location.href=https://www.blogger.com/comment.g?blogID=19402125&postID=114070605958684588;>",
      "",
      "<a href = \"/videos/Bishop\"/\" title = \"click to see more Bishop\" videos\">Bishop\"</a>",
      "Bishop\"",
      "<a href=\"http://bhaa.ie/calendar/event.php?eid=20081203150127531\"\">BHAA Eircom 2 &amp; 5 miles CC combined start</a>",
      "BHAA Eircom 2 & 5 miles CC combined start",
      "<a href=\"http://people.tribe.net/wolfmana\" onClick='setClick(\"Application[tribe].Person[bb7df210-9dc0-478c-917f-436b896bcb79]\")'\" title=\"Mana\">",
      "",
      "<a  href=\"http://blog.edu-cyberpg.com/ct.ashx?id=6143c528-080c-4bb2-b765-5ec56c8256d3&url=http%3a%2f%2fwww.gsa.ac.uk%2fmackintoshsketchbook%2f\"\" eudora=\"autourl\">",
      "",

      // LUCENE-10520: "<" and ">" in attribute values is valid per the HTML5 spec
      "<input type=\"text\" value=\"<search here>\">",
      "",
      "<input type=\"text\" value=\"<search here\">",
      "",
      "<input type=\"text\" value=\"search here>\">",
      "",

      // "<" and ">" chars are accepted in on[Event] attribute values
      "<input type=\"text\" value=\"&lt;search here&gt;\" onFocus=\"this.value='<search here>'\">",
      "",
      "<![if ! IE]>\n<link href=\"http://i.deviantart.com/icons/favicon.png\" rel=\"shortcut icon\"/>\n<![endif]>",
      "\n\n\n",
      "<![if supportMisalignedColumns]>\n<tr height=0 style='display:none'>\n<td width=64 style='width:48pt'></td>\n</tr>\n<![endif]>",
      "\n\n\n\n\n\n\n\n",
    ];
    for pair in test_gold.chunks(2) {
        assert_html_strips_to(pair[0], pair[1], None)?;
    }
    Ok(())
}

/// `appendChars(testBuilder, numChars)`.
fn append_chars(test_builder: &mut String, num_chars: usize) {
    let i1 = num_chars / 2;
    for _ in 0..i1 {
        // tack on enough to go beyond the mark readahead limit, since <?> makes
        // HTMLStripCharFilter think it is a processing instruction
        test_builder.push_str("a ");
    }
}

fn hs_test_buffer_overflow() -> Result<()> {
    let size = HTMLStripCharFilter::get_initial_buffer_size();
    let mut test_builder = String::with_capacity(size + 50);
    test_builder.push_str("ah<?> ??????");
    append_chars(&mut test_builder, size + 500);
    // force the use of BufferedReader (see the module documentation)
    assert_html_strips_to_reader(reader(&test_builder), &test_builder, None)?;

    test_builder.clear();
    test_builder.push_str("<!--"); // comments
    append_chars(&mut test_builder, 3 * size + 500); // comments have two lookaheads
    test_builder.push_str("-->foo");
    assert_html_strips_to(&test_builder, "foo", None)?;

    test_builder.clear();
    test_builder.push_str("<?");
    append_chars(&mut test_builder, size + 500);
    test_builder.push_str("?>");
    assert_html_strips_to(&test_builder, "", None)?;

    test_builder.clear();
    test_builder.push_str("<b ");
    append_chars(&mut test_builder, size + 500);
    test_builder.push_str("/>");
    assert_html_strips_to(&test_builder, "", None)
}

fn hs_test_comment() -> Result<()> {
    assert_html_strips_to("<!--- three dashes, still a valid comment ---> ", " ", None)?;
    assert_html_strips_to("<! -- blah > ", " ", None)?; // should not be recognized as a comment
    let mut test_builder = String::from("<!--");
    append_chars(
        &mut test_builder,
        TestUtil::next_int(&*LuceneTestCase::random(), 0, 1000) as usize,
    );
    assert_html_strips_to(&test_builder, "", None)
}

/// `doTestOffsets(in)`.
fn do_test_offsets(input: &str) -> Result<()> {
    let units = utf16(input);
    let mut reader = HTMLStripCharFilter::new(reader(input));
    let mut off = 0; // offset in the reader
    let mut str_off: i32 = -1; // offset in the original string
    loop {
        let ch = reader.read_char()?;
        if ch == -1 {
            break;
        }
        let corrected_off = reader.correct_offset(off);
        if ch == 'X' as i32 {
            str_off = (str_off + 1..units.len() as i32)
                .find(|&i| units[i as usize] == 'X' as u16)
                .unwrap_or(-1);
            Assert::assert_equals(str_off, corrected_off);
        }
        off += 1;
    }
    Ok(())
}

fn hs_test_offsets() -> Result<()> {
    do_test_offsets("hello <p> X<p> how <p>X are you")?;
    do_test_offsets("X &amp; X &#40; X &lt; &gt; X")?;
    // test backtracking
    do_test_offsets("X < &zz >X &# < X > < &l > &g < X")
}

/// `assertLegalOffsets(in)`.
fn assert_legal_offsets(input: &str) -> Result<()> {
    let length = input.encode_utf16().count() as i32;
    let mut reader = HTMLStripCharFilter::new(reader(input));
    let mut off = 0;
    while reader.read_char()? != -1 {
        let correction = reader.correct_offset(off);
        Assert::assert_true_msg(
            &format!("invalid offset correction: {off}->{correction} for doc of length: {length}"),
            correction <= length,
        );
        off += 1;
    }
    Ok(())
}

fn hs_test_legal_offsets() -> Result<()> {
    assert_legal_offsets("hello world")?;
    assert_legal_offsets("hello &#x world")
}

fn hs_test_random() -> Result<()> {
    let a = new_test_analyzer();
    B::check_random_data(&*LuceneTestCase::random(), &a, *RANDOM_MULTIPLIER * 1000)?;
    a.close();
    Ok(())
}

fn hs_test_random_huge_strings() -> Result<()> {
    let a = new_test_analyzer();
    B::check_random_data_max_word_length(
        &*LuceneTestCase::random(),
        &a,
        *RANDOM_MULTIPLIER * 100,
        8192,
    )?;
    a.close();
    Ok(())
}

fn hs_test_close_br() -> Result<()> {
    let a = new_test_analyzer();
    let random = LuceneTestCase::random();
    let use_char_filter = random.next_boolean();
    B::check_analysis_consistency(&random, &*a, use_char_filter, " Secretary)</br> [[M")?;
    a.close();
    Ok(())
}

fn hs_test_server_side_includes() -> Result<()> {
    let test = "one<img src=\"image.png\"\n \
                alt =  \"Alt: <!--#echo var='${IMAGE_CAPTION:<!--comment-->\\'Comment\\'}'  -->\"\n\n \
                title=\"Title: <!--#echo var=\"IMAGE_CAPTION\"-->\">two";
    assert_html_strips_to(test, "onetwo", None)?;
    let test = "one<script><!-- <!--#config comment=\"<!-- \\\"comment\\\"-->\"--> --></script>two";
    assert_html_strips_to(test, "one\ntwo", None)
}

fn hs_test_script_quotes() -> Result<()> {
    let test =
        "one<script attr= bare><!-- action('<!-- comment -->', \"\\\"-->\\\"\"); --></script>two";
    assert_html_strips_to(test, "one\ntwo", None)?;
    let test = "hello<script><!-- f('<!--internal--></script>'); --></script>";
    assert_html_strips_to(test, "hello\n", None)
}

fn hs_test_escape_script() -> Result<()> {
    let test = "one<script no-value-attr>callSomeMethod();</script>two";
    let gold = "one<script no-value-attr></script>two";
    assert_html_strips_to(test, gold, Some(&set(&["SCRIPT"])))
}

fn hs_test_style() -> Result<()> {
    let test = "one<style type=\"text/css\">\n\
                <!--\n\
                @import url('http://www.lasletrasdecanciones.com/css.css');\n\
                -->\n\
                </style>two";
    assert_html_strips_to(test, "one\ntwo", None)
}

fn hs_test_escape_style() -> Result<()> {
    let test = "one<style type=\"text/css\"> body,font,a { font-family:arial; } </style>two";
    let gold = "one<style type=\"text/css\"></style>two";
    assert_html_strips_to(test, gold, Some(&set(&["STYLE"])))
}

fn hs_test_br() -> Result<()> {
    let test_gold = [
        "one<BR />two<br>three",
        "one\ntwo\nthree",
        "one<BR some stuff here too>two</BR>",
        "one\ntwo\n",
    ];
    for pair in test_gold.chunks(2) {
        assert_html_strips_to(pair[0], pair[1], None)?;
    }
    Ok(())
}

fn hs_test_escape_br() -> Result<()> {
    let test = "one<BR class='whatever'>two</\nBR\n>";
    let gold = "one<BR class='whatever'>two</\nBR\n>";
    assert_html_strips_to(test, gold, Some(&set(&["BR"])))
}

fn hs_test_inline_tags_no_space() -> Result<()> {
    let test = "one<sPAn class=\"invisible\">two<sup>2<sup>e</sup></sup>.</SpaN>three";
    assert_html_strips_to(test, "onetwo2e.three", None)
}

/// `TestUtil.randomHtmlishString(random(), max).replace('>', ' ').replaceFirst("^--", "__")`.
fn htmlish_no_comment(max_num_elems: i32) -> String {
    let s = TestUtil::random_htmlish_string(&*LuceneTestCase::random(), max_num_elems)
        .replace('>', " ");
    match s.strip_prefix("--") {
        Some(rest) => format!("__{rest}"),
        None => s,
    }
}

fn hs_test_cdata() -> Result<()> {
    let max_num_elems = 100;
    // Don't create a comment (disallow "<!--") and don't include a closing ">"
    let random_htmlish_string1 = htmlish_no_comment(max_num_elems);
    let closed_angle_bang_non_cdata = format!("<!{random_htmlish_string1}-[CDATA[&]]>");
    // Don't create a comment (disallow "<!--") and don't include a closing ">"
    let random_htmlish_string2 = htmlish_no_comment(max_num_elems);
    let unclosed_angle_bang_non_cdata = format!("<!{random_htmlish_string2}-[CDATA[");
    let test_gold: Vec<&str> = vec![
        "one<![CDATA[<one><two>three<four></four></two></one>]]>two",
        "one<one><two>three<four></four></two></one>two",
        "one<![CDATA[two<![CDATA[three]]]]><![CDATA[>four]]>five",
        "onetwo<![CDATA[three]]>fourfive",
        "<! [CDATA[&]]>",
        "",
        "<! [CDATA[&] ] >",
        "",
        "<! [CDATA[&]]",
        "<! [CDATA[&]]", // unclosed angle bang - all input is output
        "<!\u{2009}[CDATA[&]]>",
        "",
        "<!\u{2009}[CDATA[&]\u{2009}]\u{2009}>",
        "",
        "<!\u{2009}[CDATA[&]\u{2009}]\u{2009}",
        "<!\u{2009}[CDATA[&]\u{2009}]\u{2009}", // unclosed angle bang - all input is output
        &closed_angle_bang_non_cdata,
        "",
        "<![CDATA[",
        "",
        "<![CDATA[<br>",
        "<br>",
        "<![CDATA[<br>]]",
        "<br>]]",
        "<![CDATA[<br>]]>",
        "<br>",
        "<![CDATA[<br>] ] >",
        "<br>] ] >",
        "<![CDATA[<br>]\u{2009}]\u{2009}>",
        "<br>]\u{2009}]\u{2009}>",
        "<!\u{2009}[CDATA[",
        "<!\u{2009}[CDATA[",
        &unclosed_angle_bang_non_cdata,
        &unclosed_angle_bang_non_cdata,
    ];
    for pair in test_gold.chunks(2) {
        assert_html_strips_to(pair[0], pair[1], None)?;
    }
    Ok(())
}

fn hs_test_unclosed_angle_bang() -> Result<()> {
    assert_html_strips_to("<![endif]", "<![endif]", None)
}

fn hs_test_uppercase_character_entity_variants() -> Result<()> {
    let test = " &QUOT;-&COPY;&GT;>&LT;<&REG;&AMP;";
    let gold = " \"-\u{A9}>><<\u{AE}&";
    assert_html_strips_to(test, gold, None)
}

fn hs_test_ms_word_malformed_processing_instruction() -> Result<()> {
    let test =
        "one<?xml:namespace prefix = o ns = \"urn:schemas-microsoft-com:office:office\" />two";
    assert_html_strips_to(test, "onetwo", None)
}

fn hs_test_supplementary_chars_in_tags() -> Result<()> {
    let test = "one<\u{29B05}\u{8271}\u{935F}\u{41F9}\u{612F}\u{701B}>two<\u{701B}\u{612F}\u{29B05}>three \u{701B}\u{612F}\u{29B05}</\u{701B}\u{612F}\u{29B05}>four</\u{29B05}\u{8271}\u{935F}\u{41F9}\u{612F}\u{701B}>five<\u{20000}\u{20000}>six<\u{20000}\u{20000}/>seven";
    let gold = "one\ntwo\nthree \u{701B}\u{612F}\u{29B05}\nfour\nfive\nsix\nseven";
    assert_html_strips_to(test, gold, None)
}

fn hs_test_random_broken_html() -> Result<()> {
    let max_num_elements = 10000;
    let random = LuceneTestCase::random();
    let text = TestUtil::random_htmlish_string(&*random, max_num_elements);
    let a = new_test_analyzer();
    let use_char_filter = random.next_boolean();
    B::check_analysis_consistency(&random, &*a, use_char_filter, &text)?;
    a.close();
    Ok(())
}

fn hs_test_random_text() -> Result<()> {
    let random = LuceneTestCase::random();
    let mut text = String::new();
    let min_num_words = 10;
    let max_num_words = 10000;
    let min_word_length = 3;
    let max_word_length = 20;
    let num_words = TestUtil::next_int(&*random, min_num_words, max_num_words);
    match TestUtil::next_int(&*random, 0, 4) {
        0 => {
            for _ in 0..num_words {
                text.push_str(&TestUtil::random_unicode_string_max(
                    &*random,
                    max_word_length,
                ));
                text.push(' ');
            }
        }
        1 => {
            for _ in 0..num_words {
                text.push_str(&TestUtil::random_realistic_unicode_string_between(
                    &*random,
                    min_word_length,
                    max_word_length,
                ));
                text.push(' ');
            }
        }
        _ => {
            // ASCII 50% of the time
            for _ in 0..num_words {
                text.push_str(&TestUtil::random_simple_string(&*random));
                text.push(' ');
            }
        }
    }
    let mut reader = HTMLStripCharFilter::new(reader(&text));
    while reader.read_char()? != -1 {}
    Ok(())
}

fn hs_test_utf16_surrogates() -> Result<()> {
    let analyzer = new_test_analyzer();
    let a = &*analyzer;
    // Paired surrogates
    B::assert_analyzes_to(
        a,
        " one two &#xD86C;&#XdC01;three",
        &["one", "two", "\u{2B001}three"],
    )?;
    B::assert_analyzes_to(a, " &#55404;&#XdC01;", &["\u{2B001}"])?;
    B::assert_analyzes_to(a, " &#xD86C;&#56321;", &["\u{2B001}"])?;
    B::assert_analyzes_to(a, " &#55404;&#56321;", &["\u{2B001}"])?;

    // Improperly paired surrogates
    B::assert_analyzes_to(a, " &#55404;&#57999;", &["\u{FFFD}\u{E28F}"])?;
    B::assert_analyzes_to(a, " &#xD86C;&#57999;", &["\u{FFFD}\u{E28F}"])?;
    B::assert_analyzes_to(a, " &#55002;&#XdC01;", &["\u{D6DA}\u{FFFD}"])?;
    B::assert_analyzes_to(a, " &#55002;&#56321;", &["\u{D6DA}\u{FFFD}"])?;

    // Unpaired high surrogates
    B::assert_analyzes_to(a, " &#Xd921;", &["\u{FFFD}"])?;
    B::assert_analyzes_to(a, " &#Xd921", &["\u{FFFD}"])?;
    B::assert_analyzes_to(a, " &#Xd921<br>", &["&#Xd921"])?;
    B::assert_analyzes_to(a, " &#55528;", &["\u{FFFD}"])?;
    B::assert_analyzes_to(a, " &#55528", &["\u{FFFD}"])?;
    B::assert_analyzes_to(a, " &#55528<br>", &["&#55528"])?;

    // Unpaired low surrogates
    B::assert_analyzes_to(a, " &#xdfdb;", &["\u{FFFD}"])?;
    B::assert_analyzes_to(a, " &#xdfdb", &["\u{FFFD}"])?;
    B::assert_analyzes_to(a, " &#xdfdb<br>", &["&#xdfdb"])?;
    B::assert_analyzes_to(a, " &#57209;", &["\u{FFFD}"])?;
    B::assert_analyzes_to(a, " &#57209", &["\u{FFFD}"])?;
    B::assert_analyzes_to(a, " &#57209<br>", &["&#57209"])?;
    analyzer.close();
    Ok(())
}

/// `filter.transferTo(writer)`: every unit `read(char[], int, int)` hands out.
fn transfer_to(filter: &mut dyn CharReader) -> Result<String> {
    let mut out: Vec<u16> = Vec::new();
    let mut buf = [0u16; 8192];
    loop {
        let n = filter.read(&mut buf)?;
        if n == 0 {
            return Ok(String::from_utf16_lossy(&out));
        }
        out.extend_from_slice(&buf[..n]);
    }
}

/// Test that attributes with `'>'` or `'<'` characters are parsed correctly
/// (GITHUB#11556).
fn hs_test_for_issue10520() -> Result<()> {
    let test = "<!DOCTYPE html><html lang=\"en\"><head><title>Test</title></head><body><p class=\"foo>bar\" id=\"baz\">Some text.</p></body></html>";
    let mut filter = HTMLStripCharFilter::new(reader(test));
    let result = transfer_to(&mut filter)?;
    Assert::assert_equals("Test\n\n\n\nSome text.", java_trim(&result));
    Ok(())
}

fn hs_test_for_issue10520_regression() -> Result<()> {
    let test = "<!DOCTYPE html><html lang=\"en\"><head><title>Test</title></head><a href=\"https://www.somewhere.com?data=\">a link</a> some text <a href=\"https://www.elsewhere.com\">another link</a></html>";
    let mut filter = HTMLStripCharFilter::new(reader(test));
    let result = transfer_to(&mut filter)?;
    Assert::assert_equals("Test\n\na link some text another link", java_trim(&result));
    Ok(())
}

/// `sb.toString().trim().replaceAll("\\s+", " ")` of all `read(buffer)` hands out.
fn read_normalized(input: &str) -> Result<String> {
    let mut filter = HTMLStripCharFilter::new(reader(input));
    let mut out: Vec<u16> = Vec::new();
    let mut buffer = [0u16; 1024];
    loop {
        let len = filter.read(&mut buffer)?;
        if len == 0 {
            break;
        }
        out.extend_from_slice(&buffer[..len]);
    }
    let s = String::from_utf16_lossy(&out);
    // `\s` of java.util.regex: [ \t\n\x0B\f\r]
    let is_ws = |c: char| matches!(c, ' ' | '\t' | '\n' | '\u{B}' | '\u{C}' | '\r');
    let mut result = String::new();
    let mut in_ws = false;
    for c in java_trim(&s).chars() {
        if is_ws(c) {
            if !in_ws {
                result.push(' ');
            }
            in_ws = true;
        } else {
            result.push(c);
            in_ws = false;
        }
    }
    Ok(result)
}

fn hs_test_equals_at_end_of_attribute_bug() -> Result<()> {
    // Input: The first tag's attribute ends with =" followed by >
    let input = "<a href=\"https://www.example.com/?test=\">example</a> this gets discarded <a href=\"next\">example2</a>";
    Assert::assert_equals(
        "example this gets discarded example2",
        read_normalized(input)?.as_str(),
    );
    Ok(())
}

fn hs_test_single_quote_equals_at_end_of_attribute_bug() -> Result<()> {
    // Input: The first tag's attribute ends with =' followed by >
    let input = "<a href='https://www.example.com/?test='>example</a> this gets discarded <a href='next'>example2</a>";
    Assert::assert_equals(
        "example this gets discarded example2",
        read_normalized(input)?.as_str(),
    );
    Ok(())
}

fn hs_test_space_after_equals() -> Result<()> {
    let input = "<a href=\"https://www.example.com/?test= \">example</a> this gets discarded <a href=\"next\">example2</a>";
    Assert::assert_equals(
        "example this gets discarded example2",
        read_normalized(input)?.as_str(),
    );
    Ok(())
}

fn html_strip_char_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.charfilter.TestHTMLStripCharFilter")
        .method(t("test", hs_test))
        .method(t("testHTML", hs_test_html))
        .method(t(
            "testMSWord14GeneratedHTML",
            hs_test_ms_word14_generated_html,
        ))
        .method(t("testGamma", hs_test_gamma))
        .method(t("testEntities", hs_test_entities))
        .method(t("testMoreEntities", hs_test_more_entities))
        .method(t("testReserved", hs_test_reserved))
        .method(t("testMalformedHTML", hs_test_malformed_html))
        .method(t("testBufferOverflow", hs_test_buffer_overflow))
        .method(t("testComment", hs_test_comment))
        .method(t("testOffsets", hs_test_offsets))
        .method(t("testLegalOffsets", hs_test_legal_offsets))
        .method(t("testRandom", hs_test_random))
        .method(t("testRandomHugeStrings", hs_test_random_huge_strings))
        .method(t("testCloseBR", hs_test_close_br))
        .method(t("testServerSideIncludes", hs_test_server_side_includes))
        .method(t("testScriptQuotes", hs_test_script_quotes))
        .method(t("testEscapeScript", hs_test_escape_script))
        .method(t("testStyle", hs_test_style))
        .method(t("testEscapeStyle", hs_test_escape_style))
        .method(t("testBR", hs_test_br))
        .method(t("testEscapeBR", hs_test_escape_br))
        .method(t("testInlineTagsNoSpace", hs_test_inline_tags_no_space))
        .method(t("testCDATA", hs_test_cdata))
        .method(t("testUnclosedAngleBang", hs_test_unclosed_angle_bang))
        .method(t(
            "testUppercaseCharacterEntityVariants",
            hs_test_uppercase_character_entity_variants,
        ))
        .method(t(
            "testMSWordMalformedProcessingInstruction",
            hs_test_ms_word_malformed_processing_instruction,
        ))
        .method(t(
            "testSupplementaryCharsInTags",
            hs_test_supplementary_chars_in_tags,
        ))
        .method(t("testRandomBrokenHTML", hs_test_random_broken_html))
        .method(t("testRandomText", hs_test_random_text))
        .method(t("testUTF16Surrogates", hs_test_utf16_surrogates))
        .method(t("testForIssue10520", hs_test_for_issue10520))
        .method(t(
            "testForIssue10520Regression",
            hs_test_for_issue10520_regression,
        ))
        .method(t(
            "testEqualsAtEndOfAttributeBug",
            hs_test_equals_at_end_of_attribute_bug,
        ))
        .method(t(
            "testSingleQuoteEqualsAtEndOfAttributeBug",
            hs_test_single_quote_equals_at_end_of_attribute_bug,
        ))
        .method(t("testSpaceAfterEquals", hs_test_space_after_equals))
        .build()
}

// -----------------------------------------------------------------------------
// TestHTMLStripCharFilterFactory
// -----------------------------------------------------------------------------

/// `charFilterFactory(name, keysAndValues...).create(new StringReader(text))`
/// under a whitespace `MockTokenizer`.
fn html_strip_stream(params: &[&str], text: &str) -> Result<Box<dyn TokenStream>> {
    let cs = F::char_filter_factory("HTMLStrip", params)?.create(reader(text))?;
    Ok(Box::new(B::whitespace_mock_tokenizer(cs)?))
}

fn hsf_test_nothing_changed() -> Result<()> {
    //                    11111111112
    //          012345678901234567890
    let text = "this is only a test.";
    let mut ts = html_strip_stream(&["escapedTags", "a, Title"], text)?;
    B::assert_token_stream_contents_offsets(
        &mut *ts,
        &["this", "is", "only", "a", "test."],
        &[0, 5, 8, 13, 15],
        &[4, 7, 12, 14, 20],
    )
}

const TAGGED: &str = "<u>this</u> is <b>only</b> a <I>test</I>.";

fn hsf_test_no_escaped_tags() -> Result<()> {
    let mut ts = html_strip_stream(&[], TAGGED)?;
    B::assert_token_stream_contents_offsets(
        &mut *ts,
        &["this", "is", "only", "a", "test."],
        &[3, 12, 18, 27, 32],
        &[11, 14, 26, 28, 41],
    )
}

fn hsf_test_escaped_tags() -> Result<()> {
    let mut ts = html_strip_stream(&["escapedTags", "U i"], TAGGED)?;
    B::assert_token_stream_contents_offsets(
        &mut *ts,
        &["<u>this</u>", "is", "only", "a", "<I>test</I>."],
        &[0, 12, 18, 27, 29],
        &[11, 14, 26, 28, 41],
    )
}

fn hsf_test_separator_only_escaped_tags() -> Result<()> {
    let mut ts = html_strip_stream(&["escapedTags", ",, , "], TAGGED)?;
    B::assert_token_stream_contents_offsets(
        &mut *ts,
        &["this", "is", "only", "a", "test."],
        &[3, 12, 18, 27, 32],
        &[11, 14, 26, 28, 41],
    )
}

fn hsf_test_empty_escaped_tags() -> Result<()> {
    let mut ts = html_strip_stream(&["escapedTags", ""], TAGGED)?;
    B::assert_token_stream_contents_offsets(
        &mut *ts,
        &["this", "is", "only", "a", "test."],
        &[3, 12, 18, 27, 32],
        &[11, 14, 26, 28, 41],
    )
}

fn hsf_test_single_escaped_tag() -> Result<()> {
    let mut ts = html_strip_stream(&["escapedTags", ", B\r\n\t"], TAGGED)?;
    B::assert_token_stream_contents_offsets(
        &mut *ts,
        &["this", "is", "<b>only</b>", "a", "test."],
        &[3, 12, 15, 27, 32],
        &[11, 14, 26, 28, 41],
    )
}

/// Test that bogus arguments result in exception
fn hsf_test_bogus_arguments() -> Result<()> {
    let expected = iae_message(|| F::char_filter_factory("HTMLStrip", &["bogusArg", "bogusValue"]));
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

fn html_strip_char_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.charfilter.TestHTMLStripCharFilterFactory")
        .method(t("testNothingChanged", hsf_test_nothing_changed))
        .method(t("testNoEscapedTags", hsf_test_no_escaped_tags))
        .method(t("testEscapedTags", hsf_test_escaped_tags))
        .method(t(
            "testSeparatorOnlyEscapedTags",
            hsf_test_separator_only_escaped_tags,
        ))
        .method(t("testEmptyEscapedTags", hsf_test_empty_escaped_tags))
        .method(t("testSingleEscapedTag", hsf_test_single_escaped_tag))
        .method(t("testBogusArguments", hsf_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestMappingCharFilter
// -----------------------------------------------------------------------------

thread_local! {
    /// The `normMap` field `setUp` sets.
    static NORM_MAP: std::cell::RefCell<Option<Arc<NormalizeCharMap>>> =
        const { std::cell::RefCell::new(None) };
}

fn norm_map() -> Arc<NormalizeCharMap> {
    NORM_MAP.with(|m| Arc::clone(m.borrow().as_ref().expect("setUp")))
}

fn mcf_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    let mut builder = NormalizeCharMapBuilder::new();
    builder.add("aa", "a")?;
    builder.add("bbb", "b")?;
    builder.add("cccc", "cc")?;

    builder.add("h", "i")?;
    builder.add("j", "jj")?;
    builder.add("k", "kkk")?;
    builder.add("ll", "llll")?;

    builder.add("empty", "")?;

    // BMP (surrogate pair):
    builder.add("\u{1D122}", "fclef")?;

    builder.add("\u{FF01}", "full-width-exclamation")?;

    let map = Arc::new(builder.build()?);
    NORM_MAP.with(|m| *m.borrow_mut() = Some(map));
    Ok(())
}

fn mcf_tear_down() -> Result<()> {
    NORM_MAP.with(|m| *m.borrow_mut() = None);
    LuceneTestCase::super_tear_down();
    Ok(())
}

fn mapping(s: &str) -> MappingCharFilter {
    MappingCharFilter::new(norm_map(), reader(s))
}

fn mcf_test_reader_reset() -> Result<()> {
    let mut cs = mapping("x");
    let mut buf = [0u16; 10];
    let len = cs.read(&mut buf)?;
    Assert::assert_equals(1, len);
    Assert::assert_equals('x' as u16, buf[0]);
    let len = cs.read(&mut buf)?;
    Assert::assert_equals(0, len); // Java's -1

    // rewind
    cs.reset()?;
    let len = cs.read(&mut buf)?;
    Assert::assert_equals(1, len);
    Assert::assert_equals('x' as u16, buf[0]);
    Ok(())
}

/// `assertTokenStreamContents(whitespaceMockTokenizer(cs), output, start, end, finalOffset)`.
fn assert_mapped(
    cs: Box<dyn CharReader>,
    output: &[&str],
    start: &[i32],
    end: &[i32],
    final_offset: i32,
) -> Result<()> {
    let mut ts = B::whitespace_mock_tokenizer(cs)?;
    B::assert_token_stream_contents_offsets_final_offset(
        &mut ts,
        output,
        start,
        end,
        Some(final_offset),
    )
}

fn mcf_test_nothing_change() -> Result<()> {
    assert_mapped(Box::new(mapping("x")), &["x"], &[0], &[1], 1)
}

fn mcf_test1to1() -> Result<()> {
    assert_mapped(Box::new(mapping("h")), &["i"], &[0], &[1], 1)
}

fn mcf_test1to2() -> Result<()> {
    assert_mapped(Box::new(mapping("j")), &["jj"], &[0], &[1], 1)
}

fn mcf_test1to3() -> Result<()> {
    assert_mapped(Box::new(mapping("k")), &["kkk"], &[0], &[1], 1)
}

fn mcf_test2to4() -> Result<()> {
    assert_mapped(Box::new(mapping("ll")), &["llll"], &[0], &[2], 2)
}

fn mcf_test2to1() -> Result<()> {
    assert_mapped(Box::new(mapping("aa")), &["a"], &[0], &[2], 2)
}

fn mcf_test3to1() -> Result<()> {
    assert_mapped(Box::new(mapping("bbb")), &["b"], &[0], &[3], 3)
}

fn mcf_test4to2() -> Result<()> {
    assert_mapped(Box::new(mapping("cccc")), &["cc"], &[0], &[4], 4)
}

fn mcf_test5to0() -> Result<()> {
    assert_mapped(Box::new(mapping("empty")), &[], &[], &[], 5)
}

fn mcf_test_non_bmp_char() -> Result<()> {
    assert_mapped(Box::new(mapping("\u{1D122}")), &["fclef"], &[0], &[2], 2)
}

fn mcf_test_full_width_char() -> Result<()> {
    assert_mapped(
        Box::new(mapping("\u{FF01}")),
        &["full-width-exclamation"],
        &[0],
        &[1],
        1,
    )
}

//
//                1111111111222
//      01234567890123456789012
// (in)  h i j k ll cccc bbb aa
//
//                1111111111222
//      01234567890123456789012
// (out) i i jj kkk llll cc b a
//
//    h, 0, 1 =>    i, 0, 1
//    i, 2, 3 =>    i, 2, 3
//    j, 4, 5 =>   jj, 4, 5
//    k, 6, 7 =>  kkk, 6, 7
//   ll, 8,10 => llll, 8,10
// cccc,11,15 =>   cc,11,15
//  bbb,16,19 =>    b,16,19
//   aa,20,22 =>    a,20,22
//
fn mcf_test_token_stream() -> Result<()> {
    let test_string = "h i j k ll cccc bbb aa";
    assert_mapped(
        Box::new(mapping(test_string)),
        &["i", "i", "jj", "kkk", "llll", "cc", "b", "a"],
        &[0, 2, 4, 6, 8, 11, 16, 20],
        &[1, 3, 5, 7, 10, 15, 19, 22],
        test_string.len() as i32,
    )
}

//
//
//        0123456789
// (in)    aaaa ll h
// (out-1) aa llll i
// (out-2) a llllllll i
//
// aaaa,0,4 => a,0,4
//   ll,5,7 => llllllll,5,7
//    h,8,9 => i,8,9
fn mcf_test_chained() -> Result<()> {
    let test_string = "aaaa ll h";
    let cs = MappingCharFilter::new(norm_map(), Box::new(mapping(test_string)));
    assert_mapped(
        Box::new(cs),
        &["a", "llllllll", "i"],
        &[0, 5, 8],
        &[4, 7, 9],
        test_string.len() as i32,
    )
}

/// The anonymous analyzer of the random tests: a whitespace `MockTokenizer`
/// over a `MappingCharFilter` of `map`.
fn mapping_analyzer(
    name: &'static str,
    map: Arc<NormalizeCharMap>,
) -> Arc<dyn rucene::analysis::Analyzer> {
    use rucene_test_framework::analysis::tokenizer_components;
    FnAnalyzer::new(name, |_| {
        Ok(tokenizer_components(own(MockTokenizer::new(
            Arc::clone(&WHITESPACE),
            false,
        )?)))
    })
    .init_reader(move |_, r| Box::new(MappingCharFilter::new(Arc::clone(&map), r)))
    .arc()
}

fn mcf_test_random() -> Result<()> {
    let analyzer = mapping_analyzer(
        "org.apache.lucene.analysis.charfilter.TestMappingCharFilter$1",
        norm_map(),
    );
    let num_rounds = *RANDOM_MULTIPLIER * 1000;
    B::check_random_data(&*LuceneTestCase::random(), &analyzer, num_rounds)?;
    analyzer.close();
    Ok(())
}

fn mcf_test_final_offset_special_case() -> Result<()> {
    let mut builder = NormalizeCharMapBuilder::new();
    builder.add("t", "")?;
    // even though this below rule has no effect, the test passes if you remove it!!
    builder.add("tmakdbl", "c")?;
    let map = Arc::new(builder.build()?);
    let analyzer = mapping_analyzer(
        "org.apache.lucene.analysis.charfilter.TestMappingCharFilter$2",
        map,
    );
    let text = "gzw f quaxot";
    B::check_analysis_consistency(&LuceneTestCase::random(), &*analyzer, false, text)?;
    analyzer.close();
    Ok(())
}

/// `randomMap()`.
fn random_map() -> Result<Arc<NormalizeCharMap>> {
    let random = LuceneTestCase::random();
    let mut builder = NormalizeCharMapBuilder::new();
    // we can't add duplicate keys, or NormalizeCharMap gets angry
    let mut keys: HashSet<String> = HashSet::new();
    let num = random.next_int_bound(5);
    for _ in 0..num {
        let key = TestUtil::random_simple_string(&*random);
        if !keys.contains(&key) && !key.is_empty() {
            let value = TestUtil::random_simple_string(&*random);
            builder.add(&key, &value)?;
            keys.insert(key);
        }
    }
    Ok(Arc::new(builder.build()?))
}

fn mcf_test_random_maps() -> Result<()> {
    let num_iterations = LuceneTestCase::at_least(3);
    for _ in 0..num_iterations {
        let map = random_map()?;
        let analyzer = mapping_analyzer(
            "org.apache.lucene.analysis.charfilter.TestMappingCharFilter$3",
            map,
        );
        B::check_random_data(&*LuceneTestCase::random(), &analyzer, 100)?;
        analyzer.close();
    }
    Ok(())
}

fn mcf_test_random_maps2() -> Result<()> {
    let random = LuceneTestCase::random();
    let num_iterations = LuceneTestCase::at_least(3);
    for iter in 0..num_iterations {
        if *VERBOSE {
            println!("\nTEST iter={iter}");
        }
        let end_letter =
            char::from_u32(TestUtil::next_int(&*random, 'b' as i32, 'z' as i32) as u32)
                .expect("a letter");
        // Java's `HashMap<String, String>`, iterated for the greedy match
        // only (the longest match wins, so its order is not observable).
        let mut map: HashMap<String, String> = HashMap::new();
        let mut builder = NormalizeCharMapBuilder::new();
        let num_mappings = LuceneTestCase::at_least(5);
        if *VERBOSE {
            println!("  mappings:");
        }
        while (map.len() as i32) < num_mappings {
            let key =
                TestUtil::random_simple_string_range(&*random, 'a' as u16, end_letter as u16, 7);
            if !key.is_empty() && !map.contains_key(&key) {
                let value = TestUtil::random_simple_string(&*random);
                builder.add(&key, &value)?;
                if *VERBOSE {
                    println!("    {key} -> {value}");
                }
                map.insert(key, value);
            }
        }
        let char_map = Arc::new(builder.build()?);
        if *VERBOSE {
            println!("  test random documents...");
        }
        for _iter2 in 0..100 {
            let content: Vec<u16> = utf16(&TestUtil::random_simple_string_range(
                &*random,
                'a' as u16,
                end_letter as u16,
                LuceneTestCase::at_least(1000),
            ));
            if *VERBOSE {
                println!("  content={}", String::from_utf16_lossy(&content));
            }
            // Do stupid dog-slow mapping:
            // Output string:
            let mut output: Vec<u16> = Vec::new();
            // Maps output offset to input offset:
            let mut input_offsets: Vec<i32> = Vec::new();
            let mut cum_diff: i32 = 0;
            let mut char_idx = 0usize;
            while char_idx < content.len() {
                let mut match_len: i32 = -1;
                let mut match_repl: Option<Vec<u16>> = None;
                for (k, v) in &map {
                    let m = utf16(k);
                    if char_idx + m.len() <= content.len()
                        && content[char_idx..char_idx + m.len()] == m[..]
                    {
                        // Greedy: longer match wins
                        if m.len() as i32 > match_len {
                            match_len = m.len() as i32;
                            match_repl = Some(utf16(v));
                        }
                    }
                }
                if match_len != -1 {
                    let repl = match_repl.expect("a replacement");
                    // We found a match here!
                    if *VERBOSE {
                        println!(
                            "    match={} @ off={char_idx} repl={}",
                            String::from_utf16_lossy(
                                &content[char_idx..char_idx + match_len as usize]
                            ),
                            String::from_utf16_lossy(&repl)
                        );
                    }
                    output.extend_from_slice(&repl);
                    let repl_len = repl.len() as i32;
                    let min_len = match_len.min(repl_len);
                    // Common part, directly maps back to input offset:
                    for out_idx in 0..min_len {
                        input_offsets.push(output.len() as i32 - repl_len + out_idx + cum_diff);
                    }
                    cum_diff += match_len - repl_len;
                    char_idx += match_len as usize;
                    if repl_len < match_len {
                        // Replacement string is shorter than matched input: nothing to do
                    } else if repl_len > match_len {
                        // Replacement string is longer than matched input: for all the "extra"
                        // chars we map back to a single input offset:
                        for _ in match_len..repl_len {
                            input_offsets.push(output.len() as i32 + cum_diff - 1);
                        }
                    } else {
                        // Same length: no change to offset
                    }
                    assert_eq!(input_offsets.len(), output.len());
                } else {
                    input_offsets.push(output.len() as i32 + cum_diff);
                    output.push(content[char_idx]);
                    char_idx += 1;
                }
            }
            let mut map_filter = MappingCharFilter::new(
                Arc::clone(&char_map),
                Box::new(StringCharReader::from_utf16(content.clone())),
            );
            let mut actual: Vec<u16> = Vec::new();
            let mut actual_input_offsets: Vec<i32> = Vec::new();
            // Now consume the actual mapFilter, somewhat randomly:
            loop {
                if random.next_boolean() {
                    let ch = map_filter.read_char()?;
                    if ch == -1 {
                        break;
                    }
                    actual.push(ch as u16);
                } else {
                    let len = TestUtil::next_int(&*random, 1, 100) as usize;
                    let mut buffer = vec![0u16; len];
                    let off = if len == 1 {
                        0
                    } else {
                        random.next_int_bound(len as i32 - 1) as usize
                    };
                    let count = map_filter.read(&mut buffer[off..])?;
                    if count == 0 {
                        break;
                    }
                    actual.extend_from_slice(&buffer[off..off + count]);
                }
                if random.next_int_bound(10) == 7 {
                    // Map offsets
                    while actual_input_offsets.len() < actual.len() {
                        actual_input_offsets
                            .push(map_filter.correct_offset(actual_input_offsets.len() as i32));
                    }
                }
            }
            // Finish mappping offsets
            while actual_input_offsets.len() < actual.len() {
                actual_input_offsets
                    .push(map_filter.correct_offset(actual_input_offsets.len() as i32));
            }
            // Verify:
            Assert::assert_equals(
                String::from_utf16_lossy(&output),
                String::from_utf16_lossy(&actual),
            );
            Assert::assert_true(input_offsets == actual_input_offsets);
        }
    }
    Ok(())
}

fn mapping_char_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.charfilter.TestMappingCharFilter")
        .method(hook("setUp", mcf_set_up))
        .method(hook("tearDown", mcf_tear_down))
        .method(t("testReaderReset", mcf_test_reader_reset))
        .method(t("testNothingChange", mcf_test_nothing_change))
        .method(t("test1to1", mcf_test1to1))
        .method(t("test1to2", mcf_test1to2))
        .method(t("test1to3", mcf_test1to3))
        .method(t("test2to4", mcf_test2to4))
        .method(t("test2to1", mcf_test2to1))
        .method(t("test3to1", mcf_test3to1))
        .method(t("test4to2", mcf_test4to2))
        .method(t("test5to0", mcf_test5to0))
        .method(t("testNonBMPChar", mcf_test_non_bmp_char))
        .method(t("testFullWidthChar", mcf_test_full_width_char))
        .method(t("testTokenStream", mcf_test_token_stream))
        .method(t("testChained", mcf_test_chained))
        .method(t("testRandom", mcf_test_random))
        .method(t(
            "testFinalOffsetSpecialCase",
            mcf_test_final_offset_special_case,
        ))
        .method(t("testRandomMaps", mcf_test_random_maps))
        .method(t("testRandomMaps2", mcf_test_random_maps2))
        .build()
}

// -----------------------------------------------------------------------------
// TestMappingCharFilterFactory
// -----------------------------------------------------------------------------

fn mcff_test_parse_string() -> Result<()> {
    // `(MappingCharFilterFactory) charFilterFactory("Mapping")`
    let mut f = support::concrete_factory(
        "org.apache.lucene.analysis.charfilter",
        MappingCharFilterFactory::new,
        &[],
    )?;
    expect_throws("java.lang.IllegalArgumentException", || {
        f.parse_string("\\")
    });
    Assert::assert_equals_msg(
        Some("unexpected escaped characters"),
        "\\\"\n\t\r\u{8}\u{C}".to_string(),
        String::from_utf16_lossy(&f.parse_string("\\\\\\\"\\n\\t\\r\\b\\f")?),
    );
    Assert::assert_equals_msg(
        Some("unexpected escaped characters"),
        "A".to_string(),
        String::from_utf16_lossy(&f.parse_string("\\u0041")?),
    );
    Assert::assert_equals_msg(
        Some("unexpected escaped characters"),
        "AB".to_string(),
        String::from_utf16_lossy(&f.parse_string("\\u0041\\u0042")?),
    );
    expect_throws("java.lang.IllegalArgumentException", || {
        f.parse_string("\\u000")
    });
    // invalid hex number
    expect_throws("java.lang.NumberFormatException", || {
        f.parse_string("\\u123x")
    });
    Ok(())
}

/// Test that bogus arguments result in exception
fn mcff_test_bogus_arguments() -> Result<()> {
    let expected = iae_message(|| F::char_filter_factory("Mapping", &["bogusArg", "bogusValue"]));
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

fn mapping_char_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.charfilter.TestMappingCharFilterFactory")
        .method(t("testParseString", mcff_test_parse_string))
        .method(t("testBogusArguments", mcff_test_bogus_arguments))
        .build()
}

#[test]
fn test_html_strip_char_filter() {
    run(html_strip_char_filter_class);
}

#[test]
fn test_html_strip_char_filter_factory() {
    run(html_strip_char_filter_factory_class);
}

#[test]
fn test_mapping_char_filter() {
    run(mapping_char_filter_class);
}

#[test]
fn test_mapping_char_filter_factory() {
    run(mapping_char_filter_factory_class);
}
