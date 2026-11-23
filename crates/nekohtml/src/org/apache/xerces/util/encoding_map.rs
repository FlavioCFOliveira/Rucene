/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 * 
 *      http://www.apache.org/licenses/LICENSE-2.0
 * 
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `EncodingMap.java`, of the Java package `org.apache.xerces.util` (Xerces-J 2.12.2).

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

/// EncodingMap is a convenience class which handles conversions between
/// IANA encoding names and Java encoding names, and vice versa.
///
/// Equivalent to `org.apache.xerces.util.EncodingMap`: the two static
/// `Hashtable`s are process-wide maps, initialized on first use with the
/// static initializer's entries (in its order, so a later `put` of a key
/// replaces an earlier one, as in Java).
#[derive(Debug, Default)]
pub struct EncodingMap;

/// The static initializer's `fIANA2JavaMap.put` entries, in order.
const IANA2JAVA: &[(&str, &str)] = &[
    ("BIG5", "Big5"),
    ("CSBIG5", "Big5"),
    ("CP037", "CP037"),
    ("IBM037", "CP037"),
    ("CSIBM037", "CP037"),
    ("EBCDIC-CP-US", "CP037"),
    ("EBCDIC-CP-CA", "CP037"),
    ("EBCDIC-CP-NL", "CP037"),
    ("EBCDIC-CP-WT", "CP037"),
    ("IBM273", "CP273"),
    ("CP273", "CP273"),
    ("CSIBM273", "CP273"),
    ("IBM277", "CP277"),
    ("CP277", "CP277"),
    ("CSIBM277", "CP277"),
    ("EBCDIC-CP-DK", "CP277"),
    ("EBCDIC-CP-NO", "CP277"),
    ("IBM278", "CP278"),
    ("CP278", "CP278"),
    ("CSIBM278", "CP278"),
    ("EBCDIC-CP-FI", "CP278"),
    ("EBCDIC-CP-SE", "CP278"),
    ("IBM280", "CP280"),
    ("CP280", "CP280"),
    ("CSIBM280", "CP280"),
    ("EBCDIC-CP-IT", "CP280"),
    ("IBM284", "CP284"),
    ("CP284", "CP284"),
    ("CSIBM284", "CP284"),
    ("EBCDIC-CP-ES", "CP284"),
    ("EBCDIC-CP-GB", "CP285"),
    ("IBM285", "CP285"),
    ("CP285", "CP285"),
    ("CSIBM285", "CP285"),
    ("EBCDIC-JP-KANA", "CP290"),
    ("IBM290", "CP290"),
    ("CP290", "CP290"),
    ("CSIBM290", "CP290"),
    ("EBCDIC-CP-FR", "CP297"),
    ("IBM297", "CP297"),
    ("CP297", "CP297"),
    ("CSIBM297", "CP297"),
    ("EBCDIC-CP-AR1", "CP420"),
    ("IBM420", "CP420"),
    ("CP420", "CP420"),
    ("CSIBM420", "CP420"),
    ("EBCDIC-CP-HE", "CP424"),
    ("IBM424", "CP424"),
    ("CP424", "CP424"),
    ("CSIBM424", "CP424"),
    ("IBM437", "CP437"),
    ("437", "CP437"),
    ("CP437", "CP437"),
    ("CSPC8CODEPAGE437", "CP437"),
    ("EBCDIC-CP-CH", "CP500"),
    ("IBM500", "CP500"),
    ("CP500", "CP500"),
    ("CSIBM500", "CP500"),
    ("EBCDIC-CP-CH", "CP500"),
    ("EBCDIC-CP-BE", "CP500"),
    ("IBM775", "CP775"),
    ("CP775", "CP775"),
    ("CSPC775BALTIC", "CP775"),
    ("IBM850", "CP850"),
    ("850", "CP850"),
    ("CP850", "CP850"),
    ("CSPC850MULTILINGUAL", "CP850"),
    ("IBM852", "CP852"),
    ("852", "CP852"),
    ("CP852", "CP852"),
    ("CSPCP852", "CP852"),
    ("IBM855", "CP855"),
    ("855", "CP855"),
    ("CP855", "CP855"),
    ("CSIBM855", "CP855"),
    ("IBM857", "CP857"),
    ("857", "CP857"),
    ("CP857", "CP857"),
    ("CSIBM857", "CP857"),
    ("IBM00858", "CP858"),
    ("CP00858", "CP858"),
    ("CCSID00858", "CP858"),
    ("IBM860", "CP860"),
    ("860", "CP860"),
    ("CP860", "CP860"),
    ("CSIBM860", "CP860"),
    ("IBM861", "CP861"),
    ("861", "CP861"),
    ("CP861", "CP861"),
    ("CP-IS", "CP861"),
    ("CSIBM861", "CP861"),
    ("IBM862", "CP862"),
    ("862", "CP862"),
    ("CP862", "CP862"),
    ("CSPC862LATINHEBREW", "CP862"),
    ("IBM863", "CP863"),
    ("863", "CP863"),
    ("CP863", "CP863"),
    ("CSIBM863", "CP863"),
    ("IBM864", "CP864"),
    ("CP864", "CP864"),
    ("CSIBM864", "CP864"),
    ("IBM865", "CP865"),
    ("865", "CP865"),
    ("CP865", "CP865"),
    ("CSIBM865", "CP865"),
    ("IBM866", "CP866"),
    ("866", "CP866"),
    ("CP866", "CP866"),
    ("CSIBM866", "CP866"),
    ("IBM868", "CP868"),
    ("CP868", "CP868"),
    ("CSIBM868", "CP868"),
    ("CP-AR", "CP868"),
    ("IBM869", "CP869"),
    ("CP869", "CP869"),
    ("CSIBM869", "CP869"),
    ("CP-GR", "CP869"),
    ("IBM870", "CP870"),
    ("CP870", "CP870"),
    ("CSIBM870", "CP870"),
    ("EBCDIC-CP-ROECE", "CP870"),
    ("EBCDIC-CP-YU", "CP870"),
    ("IBM871", "CP871"),
    ("CP871", "CP871"),
    ("CSIBM871", "CP871"),
    ("EBCDIC-CP-IS", "CP871"),
    ("IBM918", "CP918"),
    ("CP918", "CP918"),
    ("CSIBM918", "CP918"),
    ("EBCDIC-CP-AR2", "CP918"),
    ("IBM00924", "CP924"),
    ("CP00924", "CP924"),
    ("CCSID00924", "CP924"),
    ("EBCDIC-LATIN9--EURO", "CP924"),
    ("IBM1026", "CP1026"),
    ("CP1026", "CP1026"),
    ("CSIBM1026", "CP1026"),
    ("IBM01140", "Cp1140"),
    ("CP01140", "Cp1140"),
    ("CCSID01140", "Cp1140"),
    ("IBM01141", "Cp1141"),
    ("CP01141", "Cp1141"),
    ("CCSID01141", "Cp1141"),
    ("IBM01142", "Cp1142"),
    ("CP01142", "Cp1142"),
    ("CCSID01142", "Cp1142"),
    ("IBM01143", "Cp1143"),
    ("CP01143", "Cp1143"),
    ("CCSID01143", "Cp1143"),
    ("IBM01144", "Cp1144"),
    ("CP01144", "Cp1144"),
    ("CCSID01144", "Cp1144"),
    ("IBM01145", "Cp1145"),
    ("CP01145", "Cp1145"),
    ("CCSID01145", "Cp1145"),
    ("IBM01146", "Cp1146"),
    ("CP01146", "Cp1146"),
    ("CCSID01146", "Cp1146"),
    ("IBM01147", "Cp1147"),
    ("CP01147", "Cp1147"),
    ("CCSID01147", "Cp1147"),
    ("IBM01148", "Cp1148"),
    ("CP01148", "Cp1148"),
    ("CCSID01148", "Cp1148"),
    ("IBM01149", "Cp1149"),
    ("CP01149", "Cp1149"),
    ("CCSID01149", "Cp1149"),
    ("EUC-JP", "EUCJIS"),
    ("CSEUCPKDFMTJAPANESE", "EUCJIS"),
    ("EXTENDED_UNIX_CODE_PACKED_FORMAT_FOR_JAPANESE", "EUCJIS"),
    ("EUC-KR", "KSC5601"),
    ("CSEUCKR", "KSC5601"),
    ("KS_C_5601-1987", "KS_C_5601-1987"),
    ("ISO-IR-149", "KS_C_5601-1987"),
    ("KS_C_5601-1989", "KS_C_5601-1987"),
    ("KSC_5601", "KS_C_5601-1987"),
    ("KOREAN", "KS_C_5601-1987"),
    ("CSKSC56011987", "KS_C_5601-1987"),
    ("GB2312", "GB2312"),
    ("CSGB2312", "GB2312"),
    ("ISO-2022-JP", "JIS"),
    ("CSISO2022JP", "JIS"),
    ("ISO-2022-KR", "ISO2022KR"),
    ("CSISO2022KR", "ISO2022KR"),
    ("ISO-2022-CN", "ISO2022CN"),
    ("X0201", "JIS0201"),
    ("CSISO13JISC6220JP", "JIS0201"),
    ("X0208", "JIS0208"),
    ("ISO-IR-87", "JIS0208"),
    ("X0208dbiJIS_X0208-1983", "JIS0208"),
    ("CSISO87JISX0208", "JIS0208"),
    ("X0212", "JIS0212"),
    ("ISO-IR-159", "JIS0212"),
    ("CSISO159JISX02121990", "JIS0212"),
    ("GB18030", "GB18030"),
    ("GBK", "GBK"),
    ("CP936", "GBK"),
    ("MS936", "GBK"),
    ("WINDOWS-936", "GBK"),
    ("SHIFT_JIS", "SJIS"),
    ("CSSHIFTJIS", "SJIS"),
    ("MS_KANJI", "SJIS"),
    ("WINDOWS-31J", "MS932"),
    ("CSWINDOWS31J", "MS932"),
    ("WINDOWS-1250", "Cp1250"),
    ("WINDOWS-1251", "Cp1251"),
    ("WINDOWS-1252", "Cp1252"),
    ("WINDOWS-1253", "Cp1253"),
    ("WINDOWS-1254", "Cp1254"),
    ("WINDOWS-1255", "Cp1255"),
    ("WINDOWS-1256", "Cp1256"),
    ("WINDOWS-1257", "Cp1257"),
    ("WINDOWS-1258", "Cp1258"),
    ("TIS-620", "TIS620"),
    ("ISO-8859-1", "ISO8859_1"),
    ("ISO-IR-100", "ISO8859_1"),
    ("ISO_8859-1", "ISO8859_1"),
    ("LATIN1", "ISO8859_1"),
    ("CSISOLATIN1", "ISO8859_1"),
    ("L1", "ISO8859_1"),
    ("IBM819", "ISO8859_1"),
    ("CP819", "ISO8859_1"),
    ("ISO-8859-2", "ISO8859_2"),
    ("ISO-IR-101", "ISO8859_2"),
    ("ISO_8859-2", "ISO8859_2"),
    ("LATIN2", "ISO8859_2"),
    ("CSISOLATIN2", "ISO8859_2"),
    ("L2", "ISO8859_2"),
    ("ISO-8859-3", "ISO8859_3"),
    ("ISO-IR-109", "ISO8859_3"),
    ("ISO_8859-3", "ISO8859_3"),
    ("LATIN3", "ISO8859_3"),
    ("CSISOLATIN3", "ISO8859_3"),
    ("L3", "ISO8859_3"),
    ("ISO-8859-4", "ISO8859_4"),
    ("ISO-IR-110", "ISO8859_4"),
    ("ISO_8859-4", "ISO8859_4"),
    ("LATIN4", "ISO8859_4"),
    ("CSISOLATIN4", "ISO8859_4"),
    ("L4", "ISO8859_4"),
    ("ISO-8859-5", "ISO8859_5"),
    ("ISO-IR-144", "ISO8859_5"),
    ("ISO_8859-5", "ISO8859_5"),
    ("CYRILLIC", "ISO8859_5"),
    ("CSISOLATINCYRILLIC", "ISO8859_5"),
    ("ISO-8859-6", "ISO8859_6"),
    ("ISO-IR-127", "ISO8859_6"),
    ("ISO_8859-6", "ISO8859_6"),
    ("ECMA-114", "ISO8859_6"),
    ("ASMO-708", "ISO8859_6"),
    ("ARABIC", "ISO8859_6"),
    ("CSISOLATINARABIC", "ISO8859_6"),
    ("ISO-8859-7", "ISO8859_7"),
    ("ISO-IR-126", "ISO8859_7"),
    ("ISO_8859-7", "ISO8859_7"),
    ("ELOT_928", "ISO8859_7"),
    ("ECMA-118", "ISO8859_7"),
    ("GREEK", "ISO8859_7"),
    ("CSISOLATINGREEK", "ISO8859_7"),
    ("GREEK8", "ISO8859_7"),
    ("ISO-8859-8", "ISO8859_8"),
    ("ISO-8859-8-I", "ISO8859_8"),
    ("ISO-IR-138", "ISO8859_8"),
    ("ISO_8859-8", "ISO8859_8"),
    ("HEBREW", "ISO8859_8"),
    ("CSISOLATINHEBREW", "ISO8859_8"),
    ("ISO-8859-9", "ISO8859_9"),
    ("ISO-IR-148", "ISO8859_9"),
    ("ISO_8859-9", "ISO8859_9"),
    ("LATIN5", "ISO8859_9"),
    ("CSISOLATIN5", "ISO8859_9"),
    ("L5", "ISO8859_9"),
    ("ISO-8859-13", "ISO8859_13"),
    ("ISO-8859-15", "ISO8859_15_FDIS"),
    ("ISO_8859-15", "ISO8859_15_FDIS"),
    ("LATIN-9", "ISO8859_15_FDIS"),
    ("KOI8-R", "KOI8_R"),
    ("CSKOI8R", "KOI8_R"),
    ("US-ASCII", "ASCII"),
    ("ISO-IR-6", "ASCII"),
    ("ANSI_X3.4-1968", "ASCII"),
    ("ANSI_X3.4-1986", "ASCII"),
    ("ISO_646.IRV:1991", "ASCII"),
    ("ASCII", "ASCII"),
    ("CSASCII", "ASCII"),
    ("ISO646-US", "ASCII"),
    ("US", "ASCII"),
    ("IBM367", "ASCII"),
    ("CP367", "ASCII"),
    ("UTF-8", "UTF8"),
    ("UTF-16", "UTF-16"),
    ("UTF-16BE", "UnicodeBig"),
    ("UTF-16LE", "UnicodeLittle"),
    ("IBM-1047", "Cp1047"),
    ("IBM1047", "Cp1047"),
    ("CP1047", "Cp1047"),
    ("IBM-37", "CP037"),
    ("IBM-273", "CP273"),
    ("IBM-277", "CP277"),
    ("IBM-278", "CP278"),
    ("IBM-280", "CP280"),
    ("IBM-284", "CP284"),
    ("IBM-285", "CP285"),
    ("IBM-290", "CP290"),
    ("IBM-297", "CP297"),
    ("IBM-420", "CP420"),
    ("IBM-424", "CP424"),
    ("IBM-437", "CP437"),
    ("IBM-500", "CP500"),
    ("IBM-775", "CP775"),
    ("IBM-850", "CP850"),
    ("IBM-852", "CP852"),
    ("IBM-855", "CP855"),
    ("IBM-857", "CP857"),
    ("IBM-858", "CP858"),
    ("IBM-860", "CP860"),
    ("IBM-861", "CP861"),
    ("IBM-862", "CP862"),
    ("IBM-863", "CP863"),
    ("IBM-864", "CP864"),
    ("IBM-865", "CP865"),
    ("IBM-866", "CP866"),
    ("IBM-868", "CP868"),
    ("IBM-869", "CP869"),
    ("IBM-870", "CP870"),
    ("IBM-871", "CP871"),
    ("IBM-918", "CP918"),
    ("IBM-924", "CP924"),
    ("IBM-1026", "CP1026"),
    ("IBM-1140", "Cp1140"),
    ("IBM-1141", "Cp1141"),
    ("IBM-1142", "Cp1142"),
    ("IBM-1143", "Cp1143"),
    ("IBM-1144", "Cp1144"),
    ("IBM-1145", "Cp1145"),
    ("IBM-1146", "Cp1146"),
    ("IBM-1147", "Cp1147"),
    ("IBM-1148", "Cp1148"),
    ("IBM-1149", "Cp1149"),
    ("IBM-819", "ISO8859_1"),
    ("IBM-367", "ASCII"),
];

/// The static initializer's `fJava2IANAMap.put` entries, in order.
const JAVA2IANA: &[(&str, &str)] = &[
    ("ISO8859_1", "ISO-8859-1"),
    ("ISO8859_2", "ISO-8859-2"),
    ("ISO8859_3", "ISO-8859-3"),
    ("ISO8859_4", "ISO-8859-4"),
    ("ISO8859_5", "ISO-8859-5"),
    ("ISO8859_6", "ISO-8859-6"),
    ("ISO8859_7", "ISO-8859-7"),
    ("ISO8859_8", "ISO-8859-8"),
    ("ISO8859_9", "ISO-8859-9"),
    ("ISO8859_13", "ISO-8859-13"),
    ("ISO8859_15", "ISO-8859-15"),
    ("ISO8859_15_FDIS", "ISO-8859-15"),
    ("Big5", "BIG5"),
    ("CP037", "EBCDIC-CP-US"),
    ("CP273", "IBM273"),
    ("CP277", "EBCDIC-CP-DK"),
    ("CP278", "EBCDIC-CP-FI"),
    ("CP280", "EBCDIC-CP-IT"),
    ("CP284", "EBCDIC-CP-ES"),
    ("CP285", "EBCDIC-CP-GB"),
    ("CP290", "EBCDIC-JP-KANA"),
    ("CP297", "EBCDIC-CP-FR"),
    ("CP420", "EBCDIC-CP-AR1"),
    ("CP424", "EBCDIC-CP-HE"),
    ("CP437", "IBM437"),
    ("CP500", "EBCDIC-CP-CH"),
    ("CP775", "IBM775"),
    ("CP850", "IBM850"),
    ("CP852", "IBM852"),
    ("CP855", "IBM855"),
    ("CP857", "IBM857"),
    ("CP858", "IBM00858"),
    ("CP860", "IBM860"),
    ("CP861", "IBM861"),
    ("CP862", "IBM862"),
    ("CP863", "IBM863"),
    ("CP864", "IBM864"),
    ("CP865", "IBM865"),
    ("CP866", "IBM866"),
    ("CP868", "IBM868"),
    ("CP869", "IBM869"),
    ("CP870", "EBCDIC-CP-ROECE"),
    ("CP871", "EBCDIC-CP-IS"),
    ("CP918", "EBCDIC-CP-AR2"),
    ("CP924", "IBM00924"),
    ("CP1026", "IBM1026"),
    ("CP1140", "IBM01140"),
    ("CP1141", "IBM01141"),
    ("CP1142", "IBM01142"),
    ("CP1143", "IBM01143"),
    ("CP1144", "IBM01144"),
    ("CP1145", "IBM01145"),
    ("CP1146", "IBM01146"),
    ("CP1147", "IBM01147"),
    ("CP1148", "IBM01148"),
    ("CP1149", "IBM01149"),
    ("EUCJIS", "EUC-JP"),
    ("KS_C_5601-1987", "KS_C_5601-1987"),
    ("GB2312", "GB2312"),
    ("ISO2022KR", "ISO-2022-KR"),
    ("ISO2022CN", "ISO-2022-CN"),
    ("JIS", "ISO-2022-JP"),
    ("KOI8_R", "KOI8-R"),
    ("KSC5601", "EUC-KR"),
    ("GB18030", "GB18030"),
    ("GBK", "GBK"),
    ("SJIS", "SHIFT_JIS"),
    ("MS932", "WINDOWS-31J"),
    ("UTF8", "UTF-8"),
    ("Unicode", "UTF-16"),
    ("UnicodeBig", "UTF-16BE"),
    ("UnicodeLittle", "UTF-16LE"),
    ("JIS0201", "X0201"),
    ("JIS0208", "X0208"),
    ("JIS0212", "ISO-IR-159"),
    ("CP1047", "IBM1047"),
];

fn map(cell: &'static OnceLock<Mutex<HashMap<String, String>>>, init: &[(&str, &str)]) -> MutexGuard<'static, HashMap<String, String>> {
    cell.get_or_init(|| {
        let mut m = HashMap::new();
        for (k, v) in init {
            m.insert((*k).to_string(), (*v).to_string());
        }
        Mutex::new(m)
    })
    .lock()
    .unwrap_or_else(PoisonError::into_inner)
}

/// fIANA2JavaMap
fn iana2java() -> MutexGuard<'static, HashMap<String, String>> {
    static CELL: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    map(&CELL, IANA2JAVA)
}

/// fJava2IANAMap
fn java2iana() -> MutexGuard<'static, HashMap<String, String>> {
    static CELL: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    map(&CELL, JAVA2IANA)
}

impl EncodingMap {
    /// Default constructor.
    ///
    /// Equivalent to `new EncodingMap()`.
    pub fn new() -> Self {
        Self
    }

    /// Adds an IANA to Java encoding name mapping.
    ///
    /// Equivalent to `putIANA2JavaMapping(String, String)`.
    pub fn put_iana2java_mapping(iana_encoding: &str, java_encoding: &str) {
        iana2java().insert(iana_encoding.to_string(), java_encoding.to_string());
    }

    /// Returns the Java encoding name for the specified IANA encoding name.
    ///
    /// Equivalent to `getIANA2JavaMapping(String)`.
    pub fn get_iana2java_mapping(iana_encoding: &str) -> Option<String> {
        iana2java().get(iana_encoding).cloned()
    }

    /// Removes an IANA to Java encoding name mapping.
    ///
    /// Equivalent to `removeIANA2JavaMapping(String)`.
    pub fn remove_iana2java_mapping(iana_encoding: &str) -> Option<String> {
        iana2java().remove(iana_encoding)
    }

    /// Adds a Java to IANA encoding name mapping.
    ///
    /// Equivalent to `putJava2IANAMapping(String, String)`.
    pub fn put_java2iana_mapping(java_encoding: &str, iana_encoding: &str) {
        java2iana().insert(java_encoding.to_string(), iana_encoding.to_string());
    }

    /// Returns the IANA encoding name for the specified Java encoding name.
    ///
    /// Equivalent to `getJava2IANAMapping(String)`.
    pub fn get_java2iana_mapping(java_encoding: &str) -> Option<String> {
        java2iana().get(java_encoding).cloned()
    }

    /// Removes a Java to IANA encoding name mapping.
    ///
    /// Equivalent to `removeJava2IANAMapping(String)`.
    pub fn remove_java2iana_mapping(java_encoding: &str) -> Option<String> {
        java2iana().remove(java_encoding)
    }
}
