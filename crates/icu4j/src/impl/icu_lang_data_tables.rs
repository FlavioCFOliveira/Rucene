// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 *******************************************************************************
 * Copyright (C) 2009-2016, International Business Machines Corporation and
 * others. All Rights Reserved.
 *******************************************************************************
 */

//! `com.ibm.icu.impl.ICULangDataTables` (icu4j `langdata` module, packaged
//! in `icu4j-77.1.jar`).

use crate::r#impl::icu_data::ICUData;
use crate::r#impl::locale_display_names_impl::DataTables;

/// The lang display-data tables of the ICU data.
///
/// Equivalent to `com.ibm.icu.impl.ICULangDataTables`, a subclass of
/// `LocaleDisplayNamesImpl.ICUDataTables` over
/// `ICUData.ICU_LANG_BASE_NAME`.
pub struct ICULangDataTables;

impl ICULangDataTables {
    /// `new ICULangDataTables()`.
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> DataTables {
        DataTables::Icu(ICUData::ICU_LANG_BASE_NAME)
    }
}
