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

//! The context of a word root, ported from
//! `org.apache.lucene.analysis.hunspell.WordContext`.

use super::dictionary::{Dictionary, FLAG_UNSET};

/// Where a root stands: a simple word, or a part of a compound.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.WordContext`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum WordContext {
    /// Java constant `SIMPLE_WORD`: non-compound.
    SimpleWord,
    /// Java constant `COMPOUND_BEGIN`: the first root in a word with
    /// COMPOUNDFLAG/BEGIN/MIDDLE/END compounding.
    CompoundBegin,
    /// Java constant `COMPOUND_MIDDLE`: a middle root in a word with
    /// COMPOUNDFLAG/BEGIN/MIDDLE/END compounding.
    CompoundMiddle,
    /// Java constant `COMPOUND_END`: the final root in a word with
    /// COMPOUNDFLAG/BEGIN/MIDDLE/END compounding.
    CompoundEnd,
    /// Java constant `COMPOUND_RULE_END`: the final root in a word with
    /// COMPOUNDRULE compounding. The difference to
    /// [`CompoundEnd`](Self::CompoundEnd) is that this context doesn't
    /// require COMPOUNDFLAG/COMPOUNDEND flags, but allows ONLYINCOMPOUND.
    CompoundRuleEnd,
}

impl WordContext {
    /// Equivalent to `isCompound()`.
    pub(crate) fn is_compound(self) -> bool {
        self != Self::SimpleWord
    }

    /// Equivalent to `isAffixAllowedWithoutSpecialPermit(boolean)`.
    pub(crate) fn is_affix_allowed_without_special_permit(self, is_prefix: bool) -> bool {
        if is_prefix {
            return self == Self::CompoundBegin;
        }
        self == Self::CompoundEnd || self == Self::CompoundRuleEnd
    }

    /// Equivalent to `requiredFlag(Dictionary)`.
    pub(crate) fn required_flag(self, dictionary: &Dictionary) -> u16 {
        match self {
            Self::CompoundBegin => dictionary.compound_begin,
            Self::CompoundMiddle => dictionary.compound_middle,
            Self::CompoundEnd => dictionary.compound_end,
            Self::CompoundRuleEnd | Self::SimpleWord => FLAG_UNSET,
        }
    }
}
