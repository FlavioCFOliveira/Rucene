// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.RBBIRuleParseTable`: the state table of the RBBI rule
//! parser, generated in ICU from `rbbirpt.txt`.

/// The state table of the RBBI rule source parser.
///
/// Equivalent to the class `com.ibm.icu.text.RBBIRuleParseTable` (package
/// private): the action codes, the character-class codes and
/// `gRuleParseStateTable`.
pub(crate) struct RBBIRuleParseTable;

#[allow(non_upper_case_globals, dead_code)]
impl RBBIRuleParseTable {
    /// `doCheckVarDef`.
    pub(crate) const doCheckVarDef: i16 = 1;
    /// `doDotAny`.
    pub(crate) const doDotAny: i16 = 2;
    /// `doEndAssign`.
    pub(crate) const doEndAssign: i16 = 3;
    /// `doEndOfRule`.
    pub(crate) const doEndOfRule: i16 = 4;
    /// `doEndVariableName`.
    pub(crate) const doEndVariableName: i16 = 5;
    /// `doExit`.
    pub(crate) const doExit: i16 = 6;
    /// `doExprCatOperator`.
    pub(crate) const doExprCatOperator: i16 = 7;
    /// `doExprFinished`.
    pub(crate) const doExprFinished: i16 = 8;
    /// `doExprOrOperator`.
    pub(crate) const doExprOrOperator: i16 = 9;
    /// `doExprRParen`.
    pub(crate) const doExprRParen: i16 = 10;
    /// `doExprStart`.
    pub(crate) const doExprStart: i16 = 11;
    /// `doLParen`.
    pub(crate) const doLParen: i16 = 12;
    /// `doNOP`.
    pub(crate) const doNOP: i16 = 13;
    /// `doNoChain`.
    pub(crate) const doNoChain: i16 = 14;
    /// `doOptionEnd`.
    pub(crate) const doOptionEnd: i16 = 15;
    /// `doOptionStart`.
    pub(crate) const doOptionStart: i16 = 16;
    /// `doReverseDir`.
    pub(crate) const doReverseDir: i16 = 17;
    /// `doRuleChar`.
    pub(crate) const doRuleChar: i16 = 18;
    /// `doRuleError`.
    pub(crate) const doRuleError: i16 = 19;
    /// `doRuleErrorAssignExpr`.
    pub(crate) const doRuleErrorAssignExpr: i16 = 20;
    /// `doScanUnicodeSet`.
    pub(crate) const doScanUnicodeSet: i16 = 21;
    /// `doSlash`.
    pub(crate) const doSlash: i16 = 22;
    /// `doStartAssign`.
    pub(crate) const doStartAssign: i16 = 23;
    /// `doStartTagValue`.
    pub(crate) const doStartTagValue: i16 = 24;
    /// `doStartVariableName`.
    pub(crate) const doStartVariableName: i16 = 25;
    /// `doTagDigit`.
    pub(crate) const doTagDigit: i16 = 26;
    /// `doTagExpectedError`.
    pub(crate) const doTagExpectedError: i16 = 27;
    /// `doTagValue`.
    pub(crate) const doTagValue: i16 = 28;
    /// `doUnaryOpPlus`.
    pub(crate) const doUnaryOpPlus: i16 = 29;
    /// `doUnaryOpQuestion`.
    pub(crate) const doUnaryOpQuestion: i16 = 30;
    /// `doUnaryOpStar`.
    pub(crate) const doUnaryOpStar: i16 = 31;
    /// `doVariableNameExpectedErr`.
    pub(crate) const doVariableNameExpectedErr: i16 = 32;
    /// `kRuleSet_default`.
    pub(crate) const kRuleSet_default: i16 = 255;
    /// `kRuleSet_digit_char`.
    pub(crate) const kRuleSet_digit_char: i16 = 128;
    /// `kRuleSet_eof`.
    pub(crate) const kRuleSet_eof: i16 = 252;
    /// `kRuleSet_escaped`.
    pub(crate) const kRuleSet_escaped: i16 = 254;
    /// `kRuleSet_name_char`.
    pub(crate) const kRuleSet_name_char: i16 = 129;
    /// `kRuleSet_name_start_char`.
    pub(crate) const kRuleSet_name_start_char: i16 = 130;
    /// `kRuleSet_rule_char`.
    pub(crate) const kRuleSet_rule_char: i16 = 131;
    /// `kRuleSet_white_space`.
    pub(crate) const kRuleSet_white_space: i16 = 132;
}

/// One row of the parser's state table.
///
/// Equivalent to the nested class `RBBIRuleParseTable.RBBIRuleTableElement`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RBBIRuleTableElement {
    /// `fAction`.
    pub(crate) f_action: i16,
    /// `fCharClass`: 0-127 a literal character, above 127 a character class.
    pub(crate) f_char_class: i16,
    /// `fNextState`.
    pub(crate) f_next_state: i16,
    /// `fPushState`.
    pub(crate) f_push_state: i16,
    /// `fNextChar`.
    pub(crate) f_next_char: bool,
    /// `fStateName`, `null` for most rows.
    pub(crate) f_state_name: Option<&'static str>,
}

/// `RBBIRuleTableElement(short a, int cc, int ns, int ps, boolean nc, String sn)`.
const fn e(
    a: i16,
    cc: i32,
    ns: i32,
    ps: i32,
    nc: bool,
    sn: Option<&'static str>,
) -> RBBIRuleTableElement {
    RBBIRuleTableElement {
        f_action: a,
        f_char_class: cc as i16,
        f_next_state: ns as i16,
        f_push_state: ps as i16,
        f_next_char: nc,
        f_state_name: sn,
    }
}

type T = RBBIRuleParseTable;

/// `gRuleParseStateTable`.
pub(crate) static G_RULE_PARSE_STATE_TABLE: [RBBIRuleTableElement; 104] = [
    e(T::doNOP, 0, 0, 0, true, None),
    e(T::doExprStart, 254, 29, 9, false, Some("start")),
    e(T::doNOP, 132, 1, 0, true, None),
    e(T::doNoChain, b'^' as i32, 12, 9, true, None),
    e(T::doExprStart, b'$' as i32, 88, 98, false, None),
    e(T::doNOP, b'!' as i32, 19, 0, true, None),
    e(T::doNOP, b';' as i32, 1, 0, true, None),
    e(T::doNOP, 252, 0, 0, false, None),
    e(T::doExprStart, 255, 29, 9, false, None),
    e(
        T::doEndOfRule,
        b';' as i32,
        1,
        0,
        true,
        Some("break-rule-end"),
    ),
    e(T::doNOP, 132, 9, 0, true, None),
    e(T::doRuleError, 255, 103, 0, false, None),
    e(T::doExprStart, 254, 29, 0, false, Some("start-after-caret")),
    e(T::doNOP, 132, 12, 0, true, None),
    e(T::doRuleError, b'^' as i32, 103, 0, false, None),
    e(T::doExprStart, b'$' as i32, 88, 37, false, None),
    e(T::doRuleError, b';' as i32, 103, 0, false, None),
    e(T::doRuleError, 252, 103, 0, false, None),
    e(T::doExprStart, 255, 29, 0, false, None),
    e(T::doNOP, b'!' as i32, 21, 0, true, Some("rev-option")),
    e(T::doReverseDir, 255, 28, 9, false, None),
    e(T::doOptionStart, 130, 23, 0, true, Some("option-scan1")),
    e(T::doRuleError, 255, 103, 0, false, None),
    e(T::doNOP, 129, 23, 0, true, Some("option-scan2")),
    e(T::doOptionEnd, 255, 25, 0, false, None),
    e(T::doNOP, b';' as i32, 1, 0, true, Some("option-scan3")),
    e(T::doNOP, 132, 25, 0, true, None),
    e(T::doRuleError, 255, 103, 0, false, None),
    e(T::doExprStart, 255, 29, 9, false, Some("reverse-rule")),
    e(T::doRuleChar, 254, 38, 0, true, Some("term")),
    e(T::doNOP, 132, 29, 0, true, None),
    e(T::doRuleChar, 131, 38, 0, true, None),
    e(T::doNOP, b'[' as i32, 94, 38, false, None),
    e(T::doLParen, b'(' as i32, 29, 38, true, None),
    e(T::doNOP, b'$' as i32, 88, 37, false, None),
    e(T::doDotAny, b'.' as i32, 38, 0, true, None),
    e(T::doRuleError, 255, 103, 0, false, None),
    e(T::doCheckVarDef, 255, 38, 0, false, Some("term-var-ref")),
    e(T::doNOP, 132, 38, 0, true, Some("expr-mod")),
    e(T::doUnaryOpStar, b'*' as i32, 43, 0, true, None),
    e(T::doUnaryOpPlus, b'+' as i32, 43, 0, true, None),
    e(T::doUnaryOpQuestion, b'?' as i32, 43, 0, true, None),
    e(T::doNOP, 255, 43, 0, false, None),
    e(T::doExprCatOperator, 254, 29, 0, false, Some("expr-cont")),
    e(T::doNOP, 132, 43, 0, true, None),
    e(T::doExprCatOperator, 131, 29, 0, false, None),
    e(T::doExprCatOperator, b'[' as i32, 29, 0, false, None),
    e(T::doExprCatOperator, b'(' as i32, 29, 0, false, None),
    e(T::doExprCatOperator, b'$' as i32, 29, 0, false, None),
    e(T::doExprCatOperator, b'.' as i32, 29, 0, false, None),
    e(T::doExprCatOperator, b'/' as i32, 55, 0, false, None),
    e(T::doExprCatOperator, b'{' as i32, 67, 0, true, None),
    e(T::doExprOrOperator, b'|' as i32, 29, 0, true, None),
    e(T::doExprRParen, b')' as i32, 255, 0, true, None),
    e(T::doExprFinished, 255, 255, 0, false, None),
    e(T::doSlash, b'/' as i32, 57, 0, true, Some("look-ahead")),
    e(T::doNOP, 255, 103, 0, false, None),
    e(
        T::doExprCatOperator,
        254,
        29,
        0,
        false,
        Some("expr-cont-no-slash"),
    ),
    e(T::doNOP, 132, 43, 0, true, None),
    e(T::doExprCatOperator, 131, 29, 0, false, None),
    e(T::doExprCatOperator, b'[' as i32, 29, 0, false, None),
    e(T::doExprCatOperator, b'(' as i32, 29, 0, false, None),
    e(T::doExprCatOperator, b'$' as i32, 29, 0, false, None),
    e(T::doExprCatOperator, b'.' as i32, 29, 0, false, None),
    e(T::doExprOrOperator, b'|' as i32, 29, 0, true, None),
    e(T::doExprRParen, b')' as i32, 255, 0, true, None),
    e(T::doExprFinished, 255, 255, 0, false, None),
    e(T::doNOP, 132, 67, 0, true, Some("tag-open")),
    e(T::doStartTagValue, 128, 70, 0, false, None),
    e(T::doTagExpectedError, 255, 103, 0, false, None),
    e(T::doNOP, 132, 74, 0, true, Some("tag-value")),
    e(T::doNOP, b'}' as i32, 74, 0, false, None),
    e(T::doTagDigit, 128, 70, 0, true, None),
    e(T::doTagExpectedError, 255, 103, 0, false, None),
    e(T::doNOP, 132, 74, 0, true, Some("tag-close")),
    e(T::doTagValue, b'}' as i32, 77, 0, true, None),
    e(T::doTagExpectedError, 255, 103, 0, false, None),
    e(
        T::doExprCatOperator,
        254,
        29,
        0,
        false,
        Some("expr-cont-no-tag"),
    ),
    e(T::doNOP, 132, 77, 0, true, None),
    e(T::doExprCatOperator, 131, 29, 0, false, None),
    e(T::doExprCatOperator, b'[' as i32, 29, 0, false, None),
    e(T::doExprCatOperator, b'(' as i32, 29, 0, false, None),
    e(T::doExprCatOperator, b'$' as i32, 29, 0, false, None),
    e(T::doExprCatOperator, b'.' as i32, 29, 0, false, None),
    e(T::doExprCatOperator, b'/' as i32, 55, 0, false, None),
    e(T::doExprOrOperator, b'|' as i32, 29, 0, true, None),
    e(T::doExprRParen, b')' as i32, 255, 0, true, None),
    e(T::doExprFinished, 255, 255, 0, false, None),
    e(
        T::doStartVariableName,
        b'$' as i32,
        90,
        0,
        true,
        Some("scan-var-name"),
    ),
    e(T::doNOP, 255, 103, 0, false, None),
    e(T::doNOP, 130, 92, 0, true, Some("scan-var-start")),
    e(T::doVariableNameExpectedErr, 255, 103, 0, false, None),
    e(T::doNOP, 129, 92, 0, true, Some("scan-var-body")),
    e(T::doEndVariableName, 255, 255, 0, false, None),
    e(
        T::doScanUnicodeSet,
        b'[' as i32,
        255,
        0,
        true,
        Some("scan-unicode-set"),
    ),
    e(T::doScanUnicodeSet, b'p' as i32, 255, 0, true, None),
    e(T::doScanUnicodeSet, b'P' as i32, 255, 0, true, None),
    e(T::doNOP, 255, 103, 0, false, None),
    e(T::doNOP, 132, 98, 0, true, Some("assign-or-rule")),
    e(T::doStartAssign, b'=' as i32, 29, 101, true, None),
    e(T::doNOP, 255, 37, 9, false, None),
    e(T::doEndAssign, b';' as i32, 1, 0, true, Some("assign-end")),
    e(T::doRuleErrorAssignExpr, 255, 103, 0, false, None),
    e(T::doExit, 255, 103, 0, true, Some("errorDeath")),
];
