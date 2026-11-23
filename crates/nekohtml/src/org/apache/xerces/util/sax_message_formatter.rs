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

//! Port of `SAXMessageFormatter.java`, of the Java package `org.apache.xerces.util` (Xerces-J 2.12.2).

use std::sync::Once;

use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::message_format::MessageFormat;
use rucene_queryparser::jdk::object::JavaObject;
use rucene_queryparser::jdk::resource_bundle::{MissingResourceException, ResourceBundle};

use crate::java::{Exception, JResult};

/// Used to format SAX error messages using a specified locale.
///
/// Equivalent to `org.apache.xerces.util.SAXMessageFormatter`. The bundle
/// `org.apache.xerces.impl.msg.SAXMessages` is the Xerces 2.12.2 resource,
/// registered on first use.
#[derive(Debug)]
pub struct SAXMessageFormatter;

const BUNDLE: &str = "org.apache.xerces.impl.msg.SAXMessages";

fn register() {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        ResourceBundle::register_properties(BUNDLE, include_bytes!("../impl/msg/SAXMessages.properties"));
    });
}

impl SAXMessageFormatter {
    /// Formats a message with the specified arguments using the given locale
    /// information.
    ///
    /// Equivalent to `formatMessage(Locale, String, Object[])`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` for an unknown key.
    pub fn format_message(locale: Option<&Locale>, key: &str, arguments: Option<&[JavaObject]>) -> JResult<String> {
        register();
        let default_locale;
        let locale = match locale {
            Some(l) => l,
            None => {
                default_locale = Locale::get_default();
                &default_locale
            }
        };
        let resource_bundle = ResourceBundle::get_bundle(BUNDLE, locale).map_err(Exception::from)?;
        let msg = match resource_bundle.get_string(Some(key)) {
            Ok(mut msg) => {
                if let Some(arguments) = arguments {
                    let formatted = MessageFormat::new(&msg, &Locale::get_default())
                        .and_then(|f| f.format(Some(arguments)));
                    match formatted {
                        Ok(m) => msg = m,
                        Err(_) => {
                            msg = resource_bundle.get_string(Some("FormatFailed")).map_err(Exception::from)?;
                            msg.push(' ');
                            msg.push_str(&resource_bundle.get_string(Some(key)).map_err(Exception::from)?);
                        }
                    }
                }
                msg
            }
            Err(_) => {
                let msg = resource_bundle.get_string(Some("BadMessageKey")).map_err(Exception::from)?;
                return Err(Exception::runtime(MissingResourceException::new(msg, key, key).error()));
            }
        };
        Ok(msg)
    }
}
