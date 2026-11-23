/*
 * Copyright 2002-2009 Andy Clark, Marc Guillemot
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `HTMLElements.java`, of the Java package `org.cyberneko.html` (NekoHTML 1.9.17).

use std::sync::LazyLock;

use crate::java::{equals_ignore_case, string_hash_code};

/// Collection of HTML element information.
///
/// Equivalent to `org.cyberneko.html.HTMLElements`: its constants are the
/// constants of this module, its static methods the functions. The table
/// is built once, as Java's static initializer, and its elements are
/// shared `&'static` references (Java never mutates them after the
/// initializer).
#[derive(Debug)]
pub struct HTMLElements;

/// Element code of `A`.
pub const A: i16 = 0;
/// Element code of `ABBR`.
pub const ABBR: i16 = 1;
/// Element code of `ACRONYM`.
pub const ACRONYM: i16 = 2;
/// Element code of `ADDRESS`.
pub const ADDRESS: i16 = 3;
/// Element code of `APPLET`.
pub const APPLET: i16 = 4;
/// Element code of `AREA`.
pub const AREA: i16 = 5;
/// Element code of `B`.
pub const B: i16 = 6;
/// Element code of `BASE`.
pub const BASE: i16 = 7;
/// Element code of `BASEFONT`.
pub const BASEFONT: i16 = 8;
/// Element code of `BDO`.
pub const BDO: i16 = 9;
/// Element code of `BGSOUND`.
pub const BGSOUND: i16 = 10;
/// Element code of `BIG`.
pub const BIG: i16 = 11;
/// Element code of `BLINK`.
pub const BLINK: i16 = 12;
/// Element code of `BLOCKQUOTE`.
pub const BLOCKQUOTE: i16 = 13;
/// Element code of `BODY`.
pub const BODY: i16 = 14;
/// Element code of `BR`.
pub const BR: i16 = 15;
/// Element code of `BUTTON`.
pub const BUTTON: i16 = 16;
/// Element code of `CAPTION`.
pub const CAPTION: i16 = 17;
/// Element code of `CENTER`.
pub const CENTER: i16 = 18;
/// Element code of `CITE`.
pub const CITE: i16 = 19;
/// Element code of `CODE`.
pub const CODE: i16 = 20;
/// Element code of `COL`.
pub const COL: i16 = 21;
/// Element code of `COLGROUP`.
pub const COLGROUP: i16 = 22;
/// Element code of `COMMENT`.
pub const COMMENT: i16 = 23;
/// Element code of `DEL`.
pub const DEL: i16 = 24;
/// Element code of `DFN`.
pub const DFN: i16 = 25;
/// Element code of `DIR`.
pub const DIR: i16 = 26;
/// Element code of `DIV`.
pub const DIV: i16 = 27;
/// Element code of `DD`.
pub const DD: i16 = 28;
/// Element code of `DL`.
pub const DL: i16 = 29;
/// Element code of `DT`.
pub const DT: i16 = 30;
/// Element code of `EM`.
pub const EM: i16 = 31;
/// Element code of `EMBED`.
pub const EMBED: i16 = 32;
/// Element code of `FIELDSET`.
pub const FIELDSET: i16 = 33;
/// Element code of `FONT`.
pub const FONT: i16 = 34;
/// Element code of `FORM`.
pub const FORM: i16 = 35;
/// Element code of `FRAME`.
pub const FRAME: i16 = 36;
/// Element code of `FRAMESET`.
pub const FRAMESET: i16 = 37;
/// Element code of `H1`.
pub const H1: i16 = 38;
/// Element code of `H2`.
pub const H2: i16 = 39;
/// Element code of `H3`.
pub const H3: i16 = 40;
/// Element code of `H4`.
pub const H4: i16 = 41;
/// Element code of `H5`.
pub const H5: i16 = 42;
/// Element code of `H6`.
pub const H6: i16 = 43;
/// Element code of `HEAD`.
pub const HEAD: i16 = 44;
/// Element code of `HR`.
pub const HR: i16 = 45;
/// Element code of `HTML`.
pub const HTML: i16 = 46;
/// Element code of `I`.
pub const I: i16 = 47;
/// Element code of `IFRAME`.
pub const IFRAME: i16 = 48;
/// Element code of `ILAYER`.
pub const ILAYER: i16 = 49;
/// Element code of `IMG`.
pub const IMG: i16 = 50;
/// Element code of `INPUT`.
pub const INPUT: i16 = 51;
/// Element code of `INS`.
pub const INS: i16 = 52;
/// Element code of `ISINDEX`.
pub const ISINDEX: i16 = 53;
/// Element code of `KBD`.
pub const KBD: i16 = 54;
/// Element code of `KEYGEN`.
pub const KEYGEN: i16 = 55;
/// Element code of `LABEL`.
pub const LABEL: i16 = 56;
/// Element code of `LAYER`.
pub const LAYER: i16 = 57;
/// Element code of `LEGEND`.
pub const LEGEND: i16 = 58;
/// Element code of `LI`.
pub const LI: i16 = 59;
/// Element code of `LINK`.
pub const LINK: i16 = 60;
/// Element code of `LISTING`.
pub const LISTING: i16 = 61;
/// Element code of `MAP`.
pub const MAP: i16 = 62;
/// Element code of `MARQUEE`.
pub const MARQUEE: i16 = 63;
/// Element code of `MENU`.
pub const MENU: i16 = 64;
/// Element code of `META`.
pub const META: i16 = 65;
/// Element code of `MULTICOL`.
pub const MULTICOL: i16 = 66;
/// Element code of `NEXTID`.
pub const NEXTID: i16 = 67;
/// Element code of `NOBR`.
pub const NOBR: i16 = 68;
/// Element code of `NOEMBED`.
pub const NOEMBED: i16 = 69;
/// Element code of `NOFRAMES`.
pub const NOFRAMES: i16 = 70;
/// Element code of `NOLAYER`.
pub const NOLAYER: i16 = 71;
/// Element code of `NOSCRIPT`.
pub const NOSCRIPT: i16 = 72;
/// Element code of `OBJECT`.
pub const OBJECT: i16 = 73;
/// Element code of `OL`.
pub const OL: i16 = 74;
/// Element code of `OPTION`.
pub const OPTION: i16 = 75;
/// Element code of `OPTGROUP`.
pub const OPTGROUP: i16 = 76;
/// Element code of `P`.
pub const P: i16 = 77;
/// Element code of `PARAM`.
pub const PARAM: i16 = 78;
/// Element code of `PLAINTEXT`.
pub const PLAINTEXT: i16 = 79;
/// Element code of `PRE`.
pub const PRE: i16 = 80;
/// Element code of `Q`.
pub const Q: i16 = 81;
/// Element code of `RB`.
pub const RB: i16 = 82;
/// Element code of `RBC`.
pub const RBC: i16 = 83;
/// Element code of `RP`.
pub const RP: i16 = 84;
/// Element code of `RT`.
pub const RT: i16 = 85;
/// Element code of `RTC`.
pub const RTC: i16 = 86;
/// Element code of `RUBY`.
pub const RUBY: i16 = 87;
/// Element code of `S`.
pub const S: i16 = 88;
/// Element code of `SAMP`.
pub const SAMP: i16 = 89;
/// Element code of `SCRIPT`.
pub const SCRIPT: i16 = 90;
/// Element code of `SELECT`.
pub const SELECT: i16 = 91;
/// Element code of `SMALL`.
pub const SMALL: i16 = 92;
/// Element code of `SOUND`.
pub const SOUND: i16 = 93;
/// Element code of `SPACER`.
pub const SPACER: i16 = 94;
/// Element code of `SPAN`.
pub const SPAN: i16 = 95;
/// Element code of `STRIKE`.
pub const STRIKE: i16 = 96;
/// Element code of `STRONG`.
pub const STRONG: i16 = 97;
/// Element code of `STYLE`.
pub const STYLE: i16 = 98;
/// Element code of `SUB`.
pub const SUB: i16 = 99;
/// Element code of `SUP`.
pub const SUP: i16 = 100;
/// Element code of `TABLE`.
pub const TABLE: i16 = 101;
/// Element code of `TBODY`.
pub const TBODY: i16 = 102;
/// Element code of `TD`.
pub const TD: i16 = 103;
/// Element code of `TEXTAREA`.
pub const TEXTAREA: i16 = 104;
/// Element code of `TFOOT`.
pub const TFOOT: i16 = 105;
/// Element code of `TH`.
pub const TH: i16 = 106;
/// Element code of `THEAD`.
pub const THEAD: i16 = 107;
/// Element code of `TITLE`.
pub const TITLE: i16 = 108;
/// Element code of `TR`.
pub const TR: i16 = 109;
/// Element code of `TT`.
pub const TT: i16 = 110;
/// Element code of `U`.
pub const U: i16 = 111;
/// Element code of `UL`.
pub const UL: i16 = 112;
/// Element code of `VAR`.
pub const VAR: i16 = 113;
/// Element code of `WBR`.
pub const WBR: i16 = 114;
/// Element code of `XML`.
pub const XML: i16 = 115;
/// Element code of `XMP`.
pub const XMP: i16 = 116;
/// Element code of `UNKNOWN`.
pub const UNKNOWN: i16 = 117;

/// Element flag: inline element.
pub const INLINE: i32 = 0x01;
/// Element flag: block element.
pub const BLOCK: i32 = 0x02;
/// Element flag: empty element.
pub const EMPTY: i32 = 0x04;
/// Element flag: container element.
pub const CONTAINER: i32 = 0x08;
/// Element flag: special element.
pub const SPECIAL: i32 = 0x10;

/// `ELEMENTS_ARRAY`: the elements by the first letter of their name.
// `a['A' - 'A']` is kept as Java writes the index.
#[allow(clippy::eq_op, clippy::identity_op)]
fn elements_array() -> Vec<Vec<Element>> {
    let e = Element::with_parents_and_bounds;
    let mut a: Vec<Vec<Element>> = (0..26).map(|_| Vec::new()).collect();
    a[(b'A' - b'A') as usize] = vec![
        e(A, "A", INLINE, Some(&[BODY]), -1, Some(&[A])),
        e(ABBR, "ABBR", INLINE, Some(&[BODY]), -1, None),
        e(ACRONYM, "ACRONYM", INLINE, Some(&[BODY]), -1, None),
        e(ADDRESS, "ADDRESS", BLOCK, Some(&[BODY]), -1, None),
        e(APPLET, "APPLET", 0, Some(&[BODY]), -1, None),
        e(AREA, "AREA", EMPTY, Some(&[MAP]), -1, None),
    ];
    a[(b'B' - b'A') as usize] = vec![
        e(B, "B", INLINE, Some(&[BODY]), -1, None),
        e(BASE, "BASE", EMPTY, Some(&[HEAD]), -1, None),
        e(BASEFONT, "BASEFONT", 0, Some(&[HEAD]), -1, None),
        e(BDO, "BDO", INLINE, Some(&[BODY]), -1, None),
        e(BGSOUND, "BGSOUND", EMPTY, Some(&[HEAD]), -1, None),
        e(BIG, "BIG", INLINE, Some(&[BODY]), -1, None),
        e(BLINK, "BLINK", INLINE, Some(&[BODY]), -1, None),
        e(
            BLOCKQUOTE,
            "BLOCKQUOTE",
            BLOCK,
            Some(&[BODY]),
            -1,
            Some(&[P]),
        ),
        e(BODY, "BODY", CONTAINER, Some(&[HTML]), -1, Some(&[HEAD])),
        e(BR, "BR", EMPTY, Some(&[BODY]), -1, None),
        e(BUTTON, "BUTTON", INLINE | BLOCK, Some(&[BODY]), -1, None),
    ];
    a[(b'C' - b'A') as usize] = vec![
        e(CAPTION, "CAPTION", INLINE, Some(&[TABLE]), -1, None),
        e(CENTER, "CENTER", 0, Some(&[BODY]), -1, None),
        e(CITE, "CITE", INLINE, Some(&[BODY]), -1, None),
        e(CODE, "CODE", INLINE, Some(&[BODY]), -1, None),
        e(COL, "COL", EMPTY, Some(&[TABLE]), -1, None),
        e(
            COLGROUP,
            "COLGROUP",
            0,
            Some(&[TABLE]),
            -1,
            Some(&[COL, COLGROUP]),
        ),
        e(COMMENT, "COMMENT", SPECIAL, Some(&[HTML]), -1, None),
    ];
    a[(b'D' - b'A') as usize] = vec![
        e(DEL, "DEL", 0, Some(&[BODY]), -1, None),
        e(DFN, "DFN", INLINE, Some(&[BODY]), -1, None),
        e(DIR, "DIR", 0, Some(&[BODY]), -1, None),
        e(DIV, "DIV", BLOCK, Some(&[BODY]), -1, Some(&[P])),
        e(DD, "DD", 0, Some(&[DL]), -1, Some(&[DT, DD])),
        e(DL, "DL", BLOCK, Some(&[BODY]), -1, None),
        e(DT, "DT", 0, Some(&[DL]), -1, Some(&[DT, DD])),
    ];
    a[(b'E' - b'A') as usize] = vec![
        e(EM, "EM", INLINE, Some(&[BODY]), -1, None),
        e(EMBED, "EMBED", 0, Some(&[BODY]), -1, None),
    ];
    a[(b'F' - b'A') as usize] = vec![
        e(FIELDSET, "FIELDSET", 0, Some(&[BODY]), -1, None),
        e(FONT, "FONT", CONTAINER, Some(&[BODY]), -1, None),
        e(
            FORM,
            "FORM",
            CONTAINER,
            Some(&[BODY, TD, DIV]),
            -1,
            Some(&[BUTTON, P]),
        ),
        e(FRAME, "FRAME", EMPTY, Some(&[FRAMESET]), -1, None),
        e(FRAMESET, "FRAMESET", 0, Some(&[HTML]), -1, None),
    ];
    a[(b'H' - b'A') as usize] = vec![
        e(
            H1,
            "H1",
            BLOCK,
            Some(&[BODY, A]),
            -1,
            Some(&[H1, H2, H3, H4, H5, H6, P]),
        ),
        e(
            H2,
            "H2",
            BLOCK,
            Some(&[BODY, A]),
            -1,
            Some(&[H1, H2, H3, H4, H5, H6, P]),
        ),
        e(
            H3,
            "H3",
            BLOCK,
            Some(&[BODY, A]),
            -1,
            Some(&[H1, H2, H3, H4, H5, H6, P]),
        ),
        e(
            H4,
            "H4",
            BLOCK,
            Some(&[BODY, A]),
            -1,
            Some(&[H1, H2, H3, H4, H5, H6, P]),
        ),
        e(
            H5,
            "H5",
            BLOCK,
            Some(&[BODY, A]),
            -1,
            Some(&[H1, H2, H3, H4, H5, H6, P]),
        ),
        e(
            H6,
            "H6",
            BLOCK,
            Some(&[BODY, A]),
            -1,
            Some(&[H1, H2, H3, H4, H5, H6, P]),
        ),
        e(HEAD, "HEAD", 0, Some(&[HTML]), -1, None),
        e(HR, "HR", EMPTY, Some(&[BODY]), -1, Some(&[P])),
        e(HTML, "HTML", 0, None, -1, None),
    ];
    a[(b'I' - b'A') as usize] = vec![
        e(I, "I", INLINE, Some(&[BODY]), -1, None),
        e(IFRAME, "IFRAME", BLOCK, Some(&[BODY]), -1, None),
        e(ILAYER, "ILAYER", BLOCK, Some(&[BODY]), -1, None),
        e(IMG, "IMG", EMPTY, Some(&[BODY]), -1, None),
        e(INPUT, "INPUT", EMPTY, Some(&[BODY]), -1, None),
        e(INS, "INS", INLINE, Some(&[BODY]), -1, None),
        e(ISINDEX, "ISINDEX", 0, Some(&[HEAD]), -1, None),
    ];
    a[(b'K' - b'A') as usize] = vec![
        e(KBD, "KBD", INLINE, Some(&[BODY]), -1, None),
        e(KEYGEN, "KEYGEN", 0, Some(&[BODY]), -1, None),
    ];
    a[(b'L' - b'A') as usize] = vec![
        e(LABEL, "LABEL", 0, Some(&[BODY]), -1, None),
        e(LAYER, "LAYER", BLOCK, Some(&[BODY]), -1, None),
        e(LEGEND, "LEGEND", INLINE, Some(&[FIELDSET]), -1, None),
        e(LI, "LI", 0, Some(&[BODY, UL, OL]), -1, Some(&[LI])),
        e(LINK, "LINK", EMPTY, Some(&[HEAD]), -1, None),
        e(LISTING, "LISTING", 0, Some(&[BODY]), -1, None),
    ];
    a[(b'M' - b'A') as usize] = vec![
        e(MAP, "MAP", INLINE, Some(&[BODY]), -1, None),
        e(MARQUEE, "MARQUEE", 0, Some(&[BODY]), -1, None),
        e(MENU, "MENU", 0, Some(&[BODY]), -1, None),
        e(
            META,
            "META",
            EMPTY,
            Some(&[HEAD]),
            -1,
            Some(&[STYLE, TITLE]),
        ),
        e(MULTICOL, "MULTICOL", 0, Some(&[BODY]), -1, None),
    ];
    a[(b'N' - b'A') as usize] = vec![
        e(NEXTID, "NEXTID", EMPTY, Some(&[BODY]), -1, None),
        e(NOBR, "NOBR", INLINE, Some(&[BODY]), -1, None),
        e(NOEMBED, "NOEMBED", 0, Some(&[BODY]), -1, None),
        e(NOFRAMES, "NOFRAMES", 0, None, -1, None),
        e(NOLAYER, "NOLAYER", 0, Some(&[BODY]), -1, None),
        e(NOSCRIPT, "NOSCRIPT", 0, Some(&[BODY]), -1, None),
    ];
    a[(b'O' - b'A') as usize] = vec![
        e(OBJECT, "OBJECT", 0, Some(&[BODY]), -1, None),
        e(OL, "OL", BLOCK, Some(&[BODY]), -1, None),
        e(
            OPTGROUP,
            "OPTGROUP",
            0,
            Some(&[SELECT]),
            -1,
            Some(&[OPTION]),
        ),
        e(OPTION, "OPTION", 0, Some(&[SELECT]), -1, Some(&[OPTION])),
    ];
    a[(b'P' - b'A') as usize] = vec![
        e(P, "P", CONTAINER, Some(&[BODY]), -1, Some(&[P])),
        e(PARAM, "PARAM", EMPTY, Some(&[OBJECT, APPLET]), -1, None),
        e(PLAINTEXT, "PLAINTEXT", SPECIAL, Some(&[BODY]), -1, None),
        e(PRE, "PRE", 0, Some(&[BODY]), -1, None),
    ];
    a[(b'Q' - b'A') as usize] = vec![e(Q, "Q", INLINE, Some(&[BODY]), -1, None)];
    a[(b'R' - b'A') as usize] = vec![
        e(RB, "RB", INLINE, Some(&[RUBY]), -1, Some(&[RB])),
        e(RBC, "RBC", 0, Some(&[RUBY]), -1, None),
        e(RP, "RP", INLINE, Some(&[RUBY]), -1, Some(&[RB])),
        e(RT, "RT", INLINE, Some(&[RUBY]), -1, Some(&[RB, RP])),
        e(RTC, "RTC", 0, Some(&[RUBY]), -1, Some(&[RBC])),
        e(RUBY, "RUBY", 0, Some(&[BODY]), -1, Some(&[RUBY])),
    ];
    a[(b'S' - b'A') as usize] = vec![
        e(S, "S", 0, Some(&[BODY]), -1, None),
        e(SAMP, "SAMP", INLINE, Some(&[BODY]), -1, None),
        e(SCRIPT, "SCRIPT", SPECIAL, Some(&[HEAD, BODY]), -1, None),
        e(
            SELECT,
            "SELECT",
            CONTAINER,
            Some(&[BODY]),
            -1,
            Some(&[SELECT]),
        ),
        e(SMALL, "SMALL", INLINE, Some(&[BODY]), -1, None),
        e(SOUND, "SOUND", EMPTY, Some(&[HEAD]), -1, None),
        e(SPACER, "SPACER", EMPTY, Some(&[BODY]), -1, None),
        e(SPAN, "SPAN", CONTAINER, Some(&[BODY]), -1, None),
        e(STRIKE, "STRIKE", INLINE, Some(&[BODY]), -1, None),
        e(STRONG, "STRONG", INLINE, Some(&[BODY]), -1, None),
        e(
            STYLE,
            "STYLE",
            SPECIAL,
            Some(&[HEAD, BODY]),
            -1,
            Some(&[STYLE, TITLE, META]),
        ),
        e(SUB, "SUB", INLINE, Some(&[BODY]), -1, None),
        e(SUP, "SUP", INLINE, Some(&[BODY]), -1, None),
    ];
    a[(b'T' - b'A') as usize] = vec![
        e(TABLE, "TABLE", BLOCK | CONTAINER, Some(&[BODY]), -1, None),
        e(
            TBODY,
            "TBODY",
            0,
            Some(&[TABLE]),
            -1,
            Some(&[THEAD, TBODY, TFOOT, TD, TH, TR, COLGROUP]),
        ),
        e(TD, "TD", CONTAINER, Some(&[TR]), TABLE, Some(&[TD, TH])),
        e(TEXTAREA, "TEXTAREA", SPECIAL, Some(&[BODY]), -1, None),
        e(
            TFOOT,
            "TFOOT",
            0,
            Some(&[TABLE]),
            -1,
            Some(&[THEAD, TBODY, TFOOT, TD, TH, TR]),
        ),
        e(TH, "TH", CONTAINER, Some(&[TR]), TABLE, Some(&[TD, TH])),
        e(
            THEAD,
            "THEAD",
            0,
            Some(&[TABLE]),
            -1,
            Some(&[THEAD, TBODY, TFOOT, TD, TH, TR, COLGROUP]),
        ),
        e(TITLE, "TITLE", SPECIAL, Some(&[HEAD, BODY]), -1, None),
        e(
            TR,
            "TR",
            BLOCK,
            Some(&[TBODY, THEAD, TFOOT]),
            TABLE,
            Some(&[TD, TH, TR, COLGROUP]),
        ),
        e(TT, "TT", INLINE, Some(&[BODY]), -1, None),
    ];
    a[(b'U' - b'A') as usize] = vec![
        e(U, "U", INLINE, Some(&[BODY]), -1, None),
        e(UL, "UL", BLOCK, Some(&[BODY]), -1, None),
    ];
    a[(b'V' - b'A') as usize] = vec![e(VAR, "VAR", INLINE, Some(&[BODY]), -1, None)];
    a[(b'W' - b'A') as usize] = vec![e(WBR, "WBR", EMPTY, Some(&[BODY]), -1, None)];
    a[(b'X' - b'A') as usize] = vec![
        e(XML, "XML", 0, Some(&[BODY]), -1, None),
        e(XMP, "XMP", SPECIAL, Some(&[BODY]), -1, None),
    ];
    a
}

/// The static tables: `ELEMENTS_ARRAY`, `ELEMENTS` and `NO_SUCH_ELEMENT`.
struct Tables {
    /// `ELEMENTS_ARRAY`.
    elements_array: Vec<Vec<&'static Element>>,
    /// `ELEMENTS.data`: the elements in the order the static initializer
    /// adds them, `NO_SUCH_ELEMENT` last.
    elements: Vec<&'static Element>,
    /// `NO_SUCH_ELEMENT`.
    no_such_element: &'static Element,
}

static TABLES: LazyLock<Tables> = LazyLock::new(|| {
    let no_such_element: &'static Element = Box::leak(Box::new(Element::with_parents(
        UNKNOWN,
        "",
        CONTAINER,
        Some(&[BODY, HEAD]),
        None,
    )));
    let elements_array: Vec<Vec<&'static Element>> = elements_array()
        .into_iter()
        .map(|v| v.into_iter().map(|e| &*Box::leak(Box::new(e))).collect())
        .collect();
    let mut elements: Vec<&'static Element> = Vec::with_capacity(120);
    for array in &elements_array {
        elements.extend(array.iter().copied());
    }
    elements.push(no_such_element);
    Tables {
        elements_array,
        elements,
        no_such_element,
    }
});

/// No such element.
///
/// Equivalent to `HTMLElements.NO_SUCH_ELEMENT`.
pub fn no_such_element() -> &'static Element {
    TABLES.no_such_element
}

/// Returns the element information for the specified element code.
///
/// Equivalent to `getElement(short)`.
///
/// # Panics
///
/// For a code outside the table, where Java throws
/// `ArrayIndexOutOfBoundsException`.
pub fn get_element_by_code(code: i16) -> &'static Element {
    TABLES.elements[code as usize]
}

/// Returns the element information for the specified element name.
///
/// Equivalent to `getElement(String)`.
pub fn get_element(ename: &str) -> &'static Element {
    get_element_or(ename, no_such_element())
}

/// Returns the element information for the specified element name, or
/// `element` when there is none.
///
/// Equivalent to `getElement(String, Element)`.
pub fn get_element_or(ename: &str, element: &'static Element) -> &'static Element {
    let units: Vec<u16> = ename.encode_utf16().collect();
    if let Some(&first) = units.first() {
        let mut c = first as i32;
        if (b'a' as i32..=b'z' as i32).contains(&c) {
            c = b'A' as i32 + c - b'a' as i32;
        }
        if (b'A' as i32..=b'Z' as i32).contains(&c) {
            for &elem in &TABLES.elements_array[(c - b'A' as i32) as usize] {
                if equals_ignore_case(&elem.name, ename) {
                    return elem;
                }
            }
        }
    }
    element
}

/// Element information.
///
/// Equivalent to the nested class `HTMLElements.Element`; `parent` is the
/// codes Java's static initializer resolves to the elements, which
/// [`Element::parent`] returns.
#[derive(Debug)]
pub struct Element {
    /// The element code.
    pub code: i16,
    /// The element name.
    pub name: String,
    /// Informational flags.
    pub flags: i32,
    /// Parent elements (codes; `None` for Java's `null`).
    parent_codes: Option<&'static [i16]>,
    /// The bounding element code.
    pub bounds: i16,
    /// List of elements this element can close.
    pub closes: Option<&'static [i16]>,
}

impl Element {
    /// Constructs an element object.
    ///
    /// Equivalent to `new Element(short, String, int, short, short[])` and
    /// `new Element(short, String, int, short[], short[])`.
    pub fn with_parents(
        code: i16,
        name: &str,
        flags: i32,
        parents: Option<&'static [i16]>,
        closes: Option<&'static [i16]>,
    ) -> Self {
        Self::with_parents_and_bounds(code, name, flags, parents, -1, closes)
    }

    /// Constructs an element object.
    ///
    /// Equivalent to `new Element(short, String, int, short, short,
    /// short[])` and `new Element(short, String, int, short[], short,
    /// short[])`.
    pub fn with_parents_and_bounds(
        code: i16,
        name: &str,
        flags: i32,
        parents: Option<&'static [i16]>,
        bounds: i16,
        closes: Option<&'static [i16]>,
    ) -> Self {
        Self {
            code,
            name: name.to_string(),
            flags,
            parent_codes: parents,
            bounds,
            closes,
        }
    }

    /// Parent elements (Java's `parent` field).
    pub fn parent(&self) -> Option<Vec<&'static Element>> {
        self.parent_codes
            .map(|codes| codes.iter().map(|&c| get_element_by_code(c)).collect())
    }

    /// Returns true if this element is an inline element.
    ///
    /// Equivalent to `isInline()`.
    pub fn is_inline(&self) -> bool {
        (self.flags & INLINE) != 0
    }

    /// Returns true if this element is a block element.
    ///
    /// Equivalent to `isBlock()`.
    pub fn is_block(&self) -> bool {
        (self.flags & BLOCK) != 0
    }

    /// Returns true if this element is an empty element.
    ///
    /// Equivalent to `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        (self.flags & EMPTY) != 0
    }

    /// Returns true if this element is a container element.
    ///
    /// Equivalent to `isContainer()`.
    pub fn is_container(&self) -> bool {
        (self.flags & CONTAINER) != 0
    }

    /// Returns true if this element is special -- if its content should be
    /// parsed ignoring markup.
    ///
    /// Equivalent to `isSpecial()`.
    pub fn is_special(&self) -> bool {
        (self.flags & SPECIAL) != 0
    }

    /// Returns true if this element can close the specified Element.
    ///
    /// Equivalent to `closes(short)`.
    pub fn closes(&self, tag: i16) -> bool {
        self.closes.is_some_and(|c| c.contains(&tag))
    }

    /// Returns a hash code for this object.
    ///
    /// Equivalent to `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        string_hash_code(&self.name)
    }

    /// Returns true if the objects are equal: Java compares the name with
    /// the other object (`name.equals(o)`), so this is true only for a
    /// string equal to the name.
    ///
    /// Equivalent to `equals(Object)` with a `String` argument.
    pub fn equals_str(&self, o: &str) -> bool {
        self.name == o
    }

    /// Equivalent to `toString()`: `Object.toString()` (the class name and
    /// an identity hash, here the address) then `(name=...)`.
    pub fn to_java_string(&self) -> String {
        format!(
            "org.cyberneko.html.HTMLElements$Element@{:x}(name={})",
            self as *const Element as usize as u32, self.name
        )
    }

    /// Indicates if the provided element is an accepted parent of current
    /// element.
    ///
    /// Equivalent to `isParent(Element)`.
    pub fn is_parent(&self, element: &Element) -> bool {
        match self.parent_codes {
            None => false,
            Some(codes) => codes
                .iter()
                .any(|&c| element.code == get_element_by_code(c).code),
        }
    }
}

/// Unsynchronized list of elements.
///
/// Equivalent to the nested class `HTMLElements.ElementList`.
#[derive(Debug, Default)]
pub struct ElementList {
    /// The data.
    pub data: Vec<&'static Element>,
}

impl ElementList {
    /// The size of the list (`size`).
    pub fn size(&self) -> i32 {
        self.data.len() as i32
    }

    /// Adds an element to list, resizing if necessary.
    ///
    /// Equivalent to `addElement(Element)`.
    pub fn add_element(&mut self, element: &'static Element) {
        self.data.push(element);
    }
}
