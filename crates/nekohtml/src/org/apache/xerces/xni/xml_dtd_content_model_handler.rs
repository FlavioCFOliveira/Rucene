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

//! Port of `XMLDTDContentModelHandler.java`, of the Java package `org.apache.xerces.xni` (Xerces-J 2.12.2).

use std::rc::Rc;

use crate::java::JResult;
use crate::org::apache::xerces::xni::augmentations::Augmentations;
use crate::org::apache::xerces::xni::parser::xml_dtd_content_model_source::XMLDTDContentModelSource;

/// The DTD content model handler interface defines callback methods to
/// report information items in DTD content models of an element
/// declaration.
///
/// Equivalent to `org.apache.xerces.xni.XMLDTDContentModelHandler`.
pub trait XMLDTDContentModelHandler {
    /// Equivalent to `startContentModel(String, Augmentations)`.
    fn start_content_model(
        &self,
        element_name: &str,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()>;
    /// Equivalent to `any(Augmentations)`.
    fn any(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `empty(Augmentations)`.
    fn empty(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `startGroup(Augmentations)`.
    fn start_group(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `pcdata(Augmentations)`.
    fn pcdata(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `element(String, Augmentations)`.
    fn element(&self, element_name: &str, augmentations: Option<&dyn Augmentations>)
        -> JResult<()>;
    /// Equivalent to `separator(short, Augmentations)`.
    fn separator(&self, separator: i16, augmentations: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `occurrence(short, Augmentations)`.
    fn occurrence(&self, occurrence: i16, augmentations: Option<&dyn Augmentations>)
        -> JResult<()>;
    /// Equivalent to `endGroup(Augmentations)`.
    fn end_group(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `endContentModel(Augmentations)`.
    fn end_content_model(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `setDTDContentModelSource(XMLDTDContentModelSource)`.
    fn set_dtd_content_model_source(&self, source: Option<Rc<dyn XMLDTDContentModelSource>>);
    /// Equivalent to `getDTDContentModelSource()`.
    fn get_dtd_content_model_source(&self) -> Option<Rc<dyn XMLDTDContentModelSource>>;
}

/// A choice separator for children and mixed content models.
pub const SEPARATOR_CHOICE: i16 = 0;
/// A sequence separator for children content models.
pub const SEPARATOR_SEQUENCE: i16 = 1;
/// This occurrence count limits the element, choice, or sequence in a
/// children content model to zero or one.
pub const OCCURS_ZERO_OR_ONE: i16 = 2;
/// This occurrence count limits the element, choice, or sequence in a
/// children content model to zero or more.
pub const OCCURS_ZERO_OR_MORE: i16 = 3;
/// This occurrence count limits the element, choice, or sequence in a
/// children content model to one or more.
pub const OCCURS_ONE_OR_MORE: i16 = 4;
