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

//! Port of `Constants.java`, of the Java package `org.apache.xerces.impl` (Xerces-J 2.12.2).

#![allow(missing_docs)]

/// Commonly used constants.
///
/// Equivalent to `org.apache.xerces.impl.Constants`: its constants are the
/// constants of this module, its static methods the functions.
#[derive(Debug)]
pub struct Constants;

/// Equivalent to `Constants.NS_XMLSCHEMA`.
pub const NS_XMLSCHEMA: &str = "http://www.w3.org/2001/XMLSchema";

/// Equivalent to `Constants.NS_DTD`.
pub const NS_DTD: &str = "http://www.w3.org/TR/REC-xml";

/// Equivalent to `Constants.W3C_XML_SCHEMA10_NS_URI`.
pub const W3C_XML_SCHEMA10_NS_URI: &str = "http://www.w3.org/XML/XMLSchema/v1.0";

/// SAX feature prefix ("http://xml.org/sax/features/").
pub const SAX_FEATURE_PREFIX: &str = "http://xml.org/sax/features/";

/// Namespaces feature ("namespaces").
pub const NAMESPACES_FEATURE: &str = "namespaces";

/// Namespace prefixes feature ("namespace-prefixes").
pub const NAMESPACE_PREFIXES_FEATURE: &str = "namespace-prefixes";

/// String interning feature ("string-interning").
pub const STRING_INTERNING_FEATURE: &str = "string-interning";

/// Validation feature ("validation").
pub const VALIDATION_FEATURE: &str = "validation";

/// External general entities feature ("external-general-entities ").
pub const EXTERNAL_GENERAL_ENTITIES_FEATURE: &str = "external-general-entities";

/// External parameter entities feature ("external-parameter-entities ").
pub const EXTERNAL_PARAMETER_ENTITIES_FEATURE: &str = "external-parameter-entities";

/// Lexical handler parameter entities feature ("lexical-handler/parameter-entities").
pub const LEXICAL_HANDLER_PARAMETER_ENTITIES_FEATURE: &str = "lexical-handler/parameter-entities";

/// Is standalone feature ("is-standalone").
pub const IS_STANDALONE_FEATURE: &str = "is-standalone";

/// Resolve DTD URIs feature ("resolve-dtd-uris").
pub const RESOLVE_DTD_URIS_FEATURE: &str = "resolve-dtd-uris";

/// Use Attributes2 feature ("use-attributes2").
pub const USE_ATTRIBUTES2_FEATURE: &str = "use-attributes2";

/// Use Locator2 feature ("use-locator2").
pub const USE_LOCATOR2_FEATURE: &str = "use-locator2";

/// Use EntityResolver2 feature ("use-entity-resolver2").
pub const USE_ENTITY_RESOLVER2_FEATURE: &str = "use-entity-resolver2";

/// Unicode normalization checking feature ("unicode-normalization-checking").
pub const UNICODE_NORMALIZATION_CHECKING_FEATURE: &str = "unicode-normalization-checking";

/// xmlns URIs feature ("xmlns-uris").
pub const XMLNS_URIS_FEATURE: &str = "xmlns-uris";

/// XML 1.1 feature ("xml-1.1").
pub const XML_11_FEATURE: &str = "xml-1.1";

/// Allow unparsed entity and notation declaration events to be sent after the end DTD event ("allow-dtd-events-after-endDTD")
pub const ALLOW_DTD_EVENTS_AFTER_ENDDTD_FEATURE: &str = "allow-dtd-events-after-endDTD";

/// SAX property prefix ("http://xml.org/sax/properties/").
pub const SAX_PROPERTY_PREFIX: &str = "http://xml.org/sax/properties/";

/// Declaration handler property ("declaration-handler").
pub const DECLARATION_HANDLER_PROPERTY: &str = "declaration-handler";

/// Lexical handler property ("lexical-handler").
pub const LEXICAL_HANDLER_PROPERTY: &str = "lexical-handler";

/// DOM node property ("dom-node").
pub const DOM_NODE_PROPERTY: &str = "dom-node";

/// XML string property ("xml-string").
pub const XML_STRING_PROPERTY: &str = "xml-string";

/// Document XML version property ("document-xml-version").
pub const DOCUMENT_XML_VERSION_PROPERTY: &str = "document-xml-version";

/// JAXP property prefix ("http://java.sun.com/xml/jaxp/properties/").
pub const JAXP_PROPERTY_PREFIX: &str = "http://java.sun.com/xml/jaxp/properties/";

/// JAXP schemaSource property: when used internally may include DTD sources (DOM)
pub const SCHEMA_SOURCE: &str = "schemaSource";

/// JAXP schemaSource language: when used internally may include DTD namespace (DOM)
pub const SCHEMA_LANGUAGE: &str = "schemaLanguage";

/// Comments feature ("include-comments").
pub const INCLUDE_COMMENTS_FEATURE: &str = "include-comments";

/// Create cdata nodes feature ("create-cdata-nodes").
pub const CREATE_CDATA_NODES_FEATURE: &str = "create-cdata-nodes";

/// Feature id: load as infoset.
pub const LOAD_AS_INFOSET: &str = "load-as-infoset";

/// Equivalent to `Constants.DOM_CANONICAL_FORM`.
pub const DOM_CANONICAL_FORM: &str = "canonical-form";

/// Equivalent to `Constants.DOM_CDATA_SECTIONS`.
pub const DOM_CDATA_SECTIONS: &str = "cdata-sections";

/// Equivalent to `Constants.DOM_COMMENTS`.
pub const DOM_COMMENTS: &str = "comments";

/// Equivalent to `Constants.DOM_CHARSET_OVERRIDES_XML_ENCODING`.
pub const DOM_CHARSET_OVERRIDES_XML_ENCODING: &str = "charset-overrides-xml-encoding";

/// Equivalent to `Constants.DOM_DATATYPE_NORMALIZATION`.
pub const DOM_DATATYPE_NORMALIZATION: &str = "datatype-normalization";

/// Equivalent to `Constants.DOM_ENTITIES`.
pub const DOM_ENTITIES: &str = "entities";

/// Equivalent to `Constants.DOM_INFOSET`.
pub const DOM_INFOSET: &str = "infoset";

/// Equivalent to `Constants.DOM_NAMESPACES`.
pub const DOM_NAMESPACES: &str = "namespaces";

/// Equivalent to `Constants.DOM_NAMESPACE_DECLARATIONS`.
pub const DOM_NAMESPACE_DECLARATIONS: &str = "namespace-declarations";

/// Equivalent to `Constants.DOM_SUPPORTED_MEDIATYPES_ONLY`.
pub const DOM_SUPPORTED_MEDIATYPES_ONLY: &str = "supported-media-types-only";

/// Equivalent to `Constants.DOM_VALIDATE_IF_SCHEMA`.
pub const DOM_VALIDATE_IF_SCHEMA: &str = "validate-if-schema";

/// Equivalent to `Constants.DOM_VALIDATE`.
pub const DOM_VALIDATE: &str = "validate";

/// Equivalent to `Constants.DOM_ELEMENT_CONTENT_WHITESPACE`.
pub const DOM_ELEMENT_CONTENT_WHITESPACE: &str = "element-content-whitespace";

/// Equivalent to `Constants.DOM_DISCARD_DEFAULT_CONTENT`.
pub const DOM_DISCARD_DEFAULT_CONTENT: &str = "discard-default-content";

/// Equivalent to `Constants.DOM_NORMALIZE_CHARACTERS`.
pub const DOM_NORMALIZE_CHARACTERS: &str = "normalize-characters";

/// Equivalent to `Constants.DOM_CHECK_CHAR_NORMALIZATION`.
pub const DOM_CHECK_CHAR_NORMALIZATION: &str = "check-character-normalization";

/// Equivalent to `Constants.DOM_WELLFORMED`.
pub const DOM_WELLFORMED: &str = "well-formed";

/// Equivalent to `Constants.DOM_SPLIT_CDATA`.
pub const DOM_SPLIT_CDATA: &str = "split-cdata-sections";

/// Equivalent to `Constants.DOM_FORMAT_PRETTY_PRINT`.
pub const DOM_FORMAT_PRETTY_PRINT: &str = "format-pretty-print";

/// Equivalent to `Constants.DOM_XMLDECL`.
pub const DOM_XMLDECL: &str = "xml-declaration";

/// Equivalent to `Constants.DOM_UNKNOWNCHARS`.
pub const DOM_UNKNOWNCHARS: &str = "unknown-characters";

/// Equivalent to `Constants.DOM_CERTIFIED`.
pub const DOM_CERTIFIED: &str = "certified";

/// Equivalent to `Constants.DOM_DISALLOW_DOCTYPE`.
pub const DOM_DISALLOW_DOCTYPE: &str = "disallow-doctype";

/// Equivalent to `Constants.DOM_IGNORE_UNKNOWN_CHARACTER_DENORMALIZATIONS`.
pub const DOM_IGNORE_UNKNOWN_CHARACTER_DENORMALIZATIONS: &str =
    "ignore-unknown-character-denormalizations";

/// Equivalent to `Constants.DOM_RESOURCE_RESOLVER`.
pub const DOM_RESOURCE_RESOLVER: &str = "resource-resolver";

/// Equivalent to `Constants.DOM_ERROR_HANDLER`.
pub const DOM_ERROR_HANDLER: &str = "error-handler";

/// Equivalent to `Constants.DOM_SCHEMA_TYPE`.
pub const DOM_SCHEMA_TYPE: &str = "schema-type";

/// Equivalent to `Constants.DOM_SCHEMA_LOCATION`.
pub const DOM_SCHEMA_LOCATION: &str = "schema-location";

/// Equivalent to `Constants.DOM_PSVI`.
pub const DOM_PSVI: &str = "psvi";

/// Xerces features prefix ("http://apache.org/xml/features/").
pub const XERCES_FEATURE_PREFIX: &str = "http://apache.org/xml/features/";

/// Schema validation feature ("validation/schema").
pub const SCHEMA_VALIDATION_FEATURE: &str = "validation/schema";

/// Expose schema normalized values
pub const SCHEMA_NORMALIZED_VALUE: &str = "validation/schema/normalized-value";

/// Send schema default value via characters()
pub const SCHEMA_ELEMENT_DEFAULT: &str = "validation/schema/element-default";

/// Schema full constraint checking ("validation/schema-full-checking").
pub const SCHEMA_FULL_CHECKING: &str = "validation/schema-full-checking";

/// Augment Post-Schema-Validation-Infoset
pub const SCHEMA_AUGMENT_PSVI: &str = "validation/schema/augment-psvi";

/// Dynamic validation feature ("validation/dynamic").
pub const DYNAMIC_VALIDATION_FEATURE: &str = "validation/dynamic";

/// Warn on duplicate attribute declaration feature ("validation/warn-on-duplicate-attdef").
pub const WARN_ON_DUPLICATE_ATTDEF_FEATURE: &str = "validation/warn-on-duplicate-attdef";

/// Warn on undeclared element feature ("validation/warn-on-undeclared-elemdef").
pub const WARN_ON_UNDECLARED_ELEMDEF_FEATURE: &str = "validation/warn-on-undeclared-elemdef";

/// Warn on duplicate entity declaration feature ("warn-on-duplicate-entitydef").
pub const WARN_ON_DUPLICATE_ENTITYDEF_FEATURE: &str = "warn-on-duplicate-entitydef";

/// Allow Java encoding names feature ("allow-java-encodings").
pub const ALLOW_JAVA_ENCODINGS_FEATURE: &str = "allow-java-encodings";

/// Disallow DOCTYPE declaration feature ("disallow-doctype-decl").
pub const DISALLOW_DOCTYPE_DECL_FEATURE: &str = "disallow-doctype-decl";

/// Continue after fatal error feature ("continue-after-fatal-error").
pub const CONTINUE_AFTER_FATAL_ERROR_FEATURE: &str = "continue-after-fatal-error";

/// Load dtd grammar when nonvalidating feature ("nonvalidating/load-dtd-grammar").
pub const LOAD_DTD_GRAMMAR_FEATURE: &str = "nonvalidating/load-dtd-grammar";

/// Load external dtd when nonvalidating feature ("nonvalidating/load-external-dtd").
pub const LOAD_EXTERNAL_DTD_FEATURE: &str = "nonvalidating/load-external-dtd";

/// Defer node expansion feature ("dom/defer-node-expansion").
pub const DEFER_NODE_EXPANSION_FEATURE: &str = "dom/defer-node-expansion";

/// Create entity reference nodes feature ("dom/create-entity-ref-nodes").
pub const CREATE_ENTITY_REF_NODES_FEATURE: &str = "dom/create-entity-ref-nodes";

/// Include ignorable whitespace feature ("dom/include-ignorable-whitespace").
pub const INCLUDE_IGNORABLE_WHITESPACE: &str = "dom/include-ignorable-whitespace";

/// Default attribute values feature ("validation/default-attribute-values").
pub const DEFAULT_ATTRIBUTE_VALUES_FEATURE: &str = "validation/default-attribute-values";

/// Validate content models feature ("validation/validate-content-models").
pub const VALIDATE_CONTENT_MODELS_FEATURE: &str = "validation/validate-content-models";

/// Validate datatypes feature ("validation/validate-datatypes").
pub const VALIDATE_DATATYPES_FEATURE: &str = "validation/validate-datatypes";

/// Balance syntax trees feature ("validation/balance-syntax-trees").
pub const BALANCE_SYNTAX_TREES: &str = "validation/balance-syntax-trees";

/// Notify character references feature (scanner/notify-char-refs").
pub const NOTIFY_CHAR_REFS_FEATURE: &str = "scanner/notify-char-refs";

/// Notify built-in (&amp;amp;, etc.) references feature (scanner/notify-builtin-refs").
pub const NOTIFY_BUILTIN_REFS_FEATURE: &str = "scanner/notify-builtin-refs";

/// Standard URI conformant feature ("standard-uri-conformant").
pub const STANDARD_URI_CONFORMANT_FEATURE: &str = "standard-uri-conformant";

/// Generate synthetic annotations feature ("generate-synthetic-annotations").
pub const GENERATE_SYNTHETIC_ANNOTATIONS_FEATURE: &str = "generate-synthetic-annotations";

/// Validate annotations feature ("validate-annotations").
pub const VALIDATE_ANNOTATIONS_FEATURE: &str = "validate-annotations";

/// Honour all schemaLocations feature ("honour-all-schemaLocations").
pub const HONOUR_ALL_SCHEMALOCATIONS_FEATURE: &str = "honour-all-schemaLocations";

/// Namespace growth feature ("namespace-growth").
pub const NAMESPACE_GROWTH_FEATURE: &str = "namespace-growth";

/// Tolerate duplicates feature ("internal/tolerate-duplicates").
pub const TOLERATE_DUPLICATES_FEATURE: &str = "internal/tolerate-duplicates";

/// String interned feature ("internal/strings-interned").
pub const STRINGS_INTERNED_FEATURE: &str = "internal/strings-interned";

/// XInclude processing feature ("xinclude").
pub const XINCLUDE_FEATURE: &str = "xinclude";

/// XInclude fixup base URIs feature ("xinclude/fixup-base-uris").
pub const XINCLUDE_FIXUP_BASE_URIS_FEATURE: &str = "xinclude/fixup-base-uris";

/// XInclude fixup language feature ("xinclude/fixup-language").
pub const XINCLUDE_FIXUP_LANGUAGE_FEATURE: &str = "xinclude/fixup-language";

/// Feature to ignore xsi:type attributes on elements during validation,
/// until a global element declaration is found. ("validation/schema/ignore-xsi-type-until-elemdecl")
/// If this feature is on when validating a document, then beginning at the validation root
pub const IGNORE_XSI_TYPE_FEATURE: &str = "validation/schema/ignore-xsi-type-until-elemdecl";

/// Perform checking of ID/IDREFs ("validation/id-idref-checking")
pub const ID_IDREF_CHECKING_FEATURE: &str = "validation/id-idref-checking";

/// Feature to ignore errors caused by identity constraints ("validation/identity-constraint-checking")
pub const IDC_CHECKING_FEATURE: &str = "validation/identity-constraint-checking";

/// Feature to ignore errors caused by unparsed entities ("validation/unparsed-entity-checking")
pub const UNPARSED_ENTITY_CHECKING_FEATURE: &str = "validation/unparsed-entity-checking";

/// Internal feature. When set to true the schema validator will only use
/// schema components from the grammar pool provided.
pub const USE_GRAMMAR_POOL_ONLY_FEATURE: &str = "internal/validation/schema/use-grammar-pool-only";

/// Internal performance related feature:
/// false - the parser settings (features/properties) have not changed between 2 parses
/// true - the parser settings have changed between 2 parses
pub const PARSER_SETTINGS: &str = "internal/parser-settings";

/// Xerces properties prefix ("http://apache.org/xml/properties/").
pub const XERCES_PROPERTY_PREFIX: &str = "http://apache.org/xml/properties/";

/// Current element node property ("dom/current-element-node").
pub const CURRENT_ELEMENT_NODE_PROPERTY: &str = "dom/current-element-node";

/// Document class name property ("dom/document-class-name").
pub const DOCUMENT_CLASS_NAME_PROPERTY: &str = "dom/document-class-name";

/// Symbol table property ("internal/symbol-table").
pub const SYMBOL_TABLE_PROPERTY: &str = "internal/symbol-table";

/// Error reporter property ("internal/error-reporter").
pub const ERROR_REPORTER_PROPERTY: &str = "internal/error-reporter";

/// Error handler property ("internal/error-handler").
pub const ERROR_HANDLER_PROPERTY: &str = "internal/error-handler";

/// XInclude handler property ("internal/xinclude-handler").
pub const XINCLUDE_HANDLER_PROPERTY: &str = "internal/xinclude-handler";

/// XPointer handler property ("internal/xpointer-handler").
pub const XPOINTER_HANDLER_PROPERTY: &str = "internal/xpointer-handler";

/// Entity manager property ("internal/entity-manager").
pub const ENTITY_MANAGER_PROPERTY: &str = "internal/entity-manager";

/// Input buffer size property ("input-buffer-size").
pub const BUFFER_SIZE_PROPERTY: &str = "input-buffer-size";

/// Security manager property ("security-manager").
pub const SECURITY_MANAGER_PROPERTY: &str = "security-manager";

/// Locale property ("locale").
pub const LOCALE_PROPERTY: &str = "locale";

/// Entity resolver property ("internal/entity-resolver").
pub const ENTITY_RESOLVER_PROPERTY: &str = "internal/entity-resolver";

/// Grammar pool property ("internal/grammar-pool").
pub const XMLGRAMMAR_POOL_PROPERTY: &str = "internal/grammar-pool";

/// Datatype validator factory ("internal/datatype-validator-factory").
pub const DATATYPE_VALIDATOR_FACTORY_PROPERTY: &str = "internal/datatype-validator-factory";

/// Document scanner property ("internal/document-scanner").
pub const DOCUMENT_SCANNER_PROPERTY: &str = "internal/document-scanner";

/// DTD scanner property ("internal/dtd-scanner").
pub const DTD_SCANNER_PROPERTY: &str = "internal/dtd-scanner";

/// DTD processor property ("internal/dtd-processor").
pub const DTD_PROCESSOR_PROPERTY: &str = "internal/dtd-processor";

/// Validator property ("internal/validator").
pub const VALIDATOR_PROPERTY: &str = "internal/validator";

/// Validator property ("internal/validator/dtd").
pub const DTD_VALIDATOR_PROPERTY: &str = "internal/validator/dtd";

/// Validator property ("internal/validator/schema").
pub const SCHEMA_VALIDATOR_PROPERTY: &str = "internal/validator/schema";

/// No namespace schema location property ("schema/external-schemaLocation").
pub const SCHEMA_LOCATION: &str = "schema/external-schemaLocation";

/// Schema location property ("schema/external-noNamespaceSchemaLocation").
pub const SCHEMA_NONS_LOCATION: &str = "schema/external-noNamespaceSchemaLocation";

/// Namespace binder property ("internal/namespace-binder").
pub const NAMESPACE_BINDER_PROPERTY: &str = "internal/namespace-binder";

/// Namespace context property ("internal/namespace-context").
pub const NAMESPACE_CONTEXT_PROPERTY: &str = "internal/namespace-context";

/// Validation manager property ("internal/validation-manager").
pub const VALIDATION_MANAGER_PROPERTY: &str = "internal/validation-manager";

/// Schema type for the root element in a document ("validation/schema/root-type-definition").
pub const ROOT_TYPE_DEFINITION_PROPERTY: &str = "validation/schema/root-type-definition";

/// Schema element declaration for the root element in a document ("validation/schema/root-element-declaration").
pub const ROOT_ELEMENT_DECLARATION_PROPERTY: &str = "validation/schema/root-element-declaration";

/// Schema element declaration for the root element in a document ("internal/validation/schema/dv-factory").
pub const SCHEMA_DV_FACTORY_PROPERTY: &str = "internal/validation/schema/dv-factory";

/// Element PSVI is stored in augmentations using string "ELEMENT_PSVI"
pub const ELEMENT_PSVI: &str = "ELEMENT_PSVI";

/// Attribute PSVI is stored in augmentations using string "ATTRIBUTE_PSVI"
pub const ATTRIBUTE_PSVI: &str = "ATTRIBUTE_PSVI";

/// Boolean indicating whether an attribute is declared in the DTD is stored
/// in augmentations using the string "ATTRIBUTE_DECLARED". The absence of this
/// augmentation indicates that the attribute was not declared in the DTD.
pub const ATTRIBUTE_DECLARED: &str = "ATTRIBUTE_DECLARED";

/// Boolean indicating whether an entity referenced in the document has
/// not been read is stored in augmentations using the string "ENTITY_SKIPPED".
/// The absence of this augmentation indicates that the entity had a
pub const ENTITY_SKIPPED: &str = "ENTITY_SKIPPED";

/// Boolean indicating whether a character is a probable white space
/// character (ch <= 0x20) that was the replacement text of a character
/// reference is stored in augmentations using the string "CHAR_REF_PROBABLE_WS".
pub const CHAR_REF_PROBABLE_WS: &str = "CHAR_REF_PROBABLE_WS";

/// Equivalent to `Constants.XML_VERSION_ERROR`.
pub const XML_VERSION_ERROR: i16 = -1;

/// Equivalent to `Constants.XML_VERSION_1_0`.
pub const XML_VERSION_1_0: i16 = 1;

/// Equivalent to `Constants.XML_VERSION_1_1`.
pub const XML_VERSION_1_1: i16 = 2;

/// Equivalent to `Constants.SCHEMA_1_1_SUPPORT`.
pub const SCHEMA_1_1_SUPPORT: bool = false;

/// Equivalent to `Constants.SCHEMA_VERSION_1_0`.
pub const SCHEMA_VERSION_1_0: i16 = 1;

/// Equivalent to `Constants.SCHEMA_VERSION_1_0_EXTENDED`.
pub const SCHEMA_VERSION_1_0_EXTENDED: i16 = 2;

/// Equivalent to `Constants.fgSAXFeatures`.
const FGSAXFEATURES: &[&str] = &[
    NAMESPACES_FEATURE,
    NAMESPACE_PREFIXES_FEATURE,
    STRING_INTERNING_FEATURE,
    VALIDATION_FEATURE,
    EXTERNAL_GENERAL_ENTITIES_FEATURE,
    EXTERNAL_PARAMETER_ENTITIES_FEATURE,
];

/// Equivalent to `Constants.fgSAXProperties`.
const FGSAXPROPERTIES: &[&str] = &[
    DECLARATION_HANDLER_PROPERTY,
    LEXICAL_HANDLER_PROPERTY,
    DOM_NODE_PROPERTY,
    XML_STRING_PROPERTY,
];

/// Equivalent to `Constants.fgXercesFeatures`.
const FGXERCESFEATURES: &[&str] = &[
    SCHEMA_VALIDATION_FEATURE,
    SCHEMA_FULL_CHECKING,
    DYNAMIC_VALIDATION_FEATURE,
    WARN_ON_DUPLICATE_ATTDEF_FEATURE,
    WARN_ON_UNDECLARED_ELEMDEF_FEATURE,
    ALLOW_JAVA_ENCODINGS_FEATURE,
    CONTINUE_AFTER_FATAL_ERROR_FEATURE,
    LOAD_DTD_GRAMMAR_FEATURE,
    LOAD_EXTERNAL_DTD_FEATURE,
    CREATE_ENTITY_REF_NODES_FEATURE,
    INCLUDE_IGNORABLE_WHITESPACE,
    DEFAULT_ATTRIBUTE_VALUES_FEATURE,
    VALIDATE_CONTENT_MODELS_FEATURE,
    VALIDATE_DATATYPES_FEATURE,
    BALANCE_SYNTAX_TREES,
    NOTIFY_CHAR_REFS_FEATURE,
    NOTIFY_BUILTIN_REFS_FEATURE,
    DISALLOW_DOCTYPE_DECL_FEATURE,
    STANDARD_URI_CONFORMANT_FEATURE,
    GENERATE_SYNTHETIC_ANNOTATIONS_FEATURE,
    VALIDATE_ANNOTATIONS_FEATURE,
    HONOUR_ALL_SCHEMALOCATIONS_FEATURE,
    XINCLUDE_FEATURE,
    XINCLUDE_FIXUP_BASE_URIS_FEATURE,
    XINCLUDE_FIXUP_LANGUAGE_FEATURE,
    IGNORE_XSI_TYPE_FEATURE,
    ID_IDREF_CHECKING_FEATURE,
    IDC_CHECKING_FEATURE,
    UNPARSED_ENTITY_CHECKING_FEATURE,
    NAMESPACE_GROWTH_FEATURE,
    TOLERATE_DUPLICATES_FEATURE,
    STRINGS_INTERNED_FEATURE,
];

/// Equivalent to `Constants.fgXercesProperties`.
const FGXERCESPROPERTIES: &[&str] = &[
    CURRENT_ELEMENT_NODE_PROPERTY,
    DOCUMENT_CLASS_NAME_PROPERTY,
    SYMBOL_TABLE_PROPERTY,
    ERROR_HANDLER_PROPERTY,
    ERROR_REPORTER_PROPERTY,
    ENTITY_MANAGER_PROPERTY,
    ENTITY_RESOLVER_PROPERTY,
    XMLGRAMMAR_POOL_PROPERTY,
    DATATYPE_VALIDATOR_FACTORY_PROPERTY,
    DOCUMENT_SCANNER_PROPERTY,
    DTD_SCANNER_PROPERTY,
    VALIDATOR_PROPERTY,
    SCHEMA_LOCATION,
    SCHEMA_NONS_LOCATION,
    VALIDATION_MANAGER_PROPERTY,
    BUFFER_SIZE_PROPERTY,
    SECURITY_MANAGER_PROPERTY,
    LOCALE_PROPERTY,
    ROOT_TYPE_DEFINITION_PROPERTY,
    ROOT_ELEMENT_DECLARATION_PROPERTY,
    SCHEMA_DV_FACTORY_PROPERTY,
];

/// Returns an enumeration of the SAX features.
///
/// Equivalent to `getSAXFeatures()`.
pub fn get_sax_features() -> Vec<&'static str> {
    FGSAXFEATURES.to_vec()
}

/// Returns an enumeration of the SAX properties.
///
/// Equivalent to `getSAXProperties()`.
pub fn get_sax_properties() -> Vec<&'static str> {
    FGSAXPROPERTIES.to_vec()
}

/// Returns an enumeration of the Xerces features.
///
/// Equivalent to `getXercesFeatures()`.
pub fn get_xerces_features() -> Vec<&'static str> {
    FGXERCESFEATURES.to_vec()
}

/// Returns an enumeration of the Xerces properties.
///
/// Equivalent to `getXercesProperties()`.
pub fn get_xerces_properties() -> Vec<&'static str> {
    FGXERCESPROPERTIES.to_vec()
}

/// Prints all features and properties.
///
/// Equivalent to `main(String[])`.
pub fn main() {
    print("SAX features:", SAX_FEATURE_PREFIX, FGSAXFEATURES);
    print("SAX properties:", SAX_PROPERTY_PREFIX, FGSAXPROPERTIES);
    print("Xerces features:", XERCES_FEATURE_PREFIX, FGXERCESFEATURES);
    print(
        "Xerces properties:",
        XERCES_PROPERTY_PREFIX,
        FGXERCESPROPERTIES,
    );
}

/// Prints a list of features/properties.
fn print(header: &str, prefix: &str, array: &[&str]) {
    print!("{header}");
    if !array.is_empty() {
        println!();
        for item in array {
            print!("  ");
            print!("{prefix}");
            println!("{item}");
        }
    } else {
        println!(" none.");
    }
}
