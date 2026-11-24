# Module Specification: `nekohtml`

**Spec ID:** NEKOHTML  
**Upstream:** NekoHTML 1.9.17 (`net.sourceforge.nekohtml:nekohtml:1.9.17`), with the Xerces-J 2.12.2 and SAX 2.0.2 types it uses  
**Crate:** `crates/nekohtml`

## 1. Purpose

Provide the SAX HTML parser `org.cyberneko.html.parsers.SAXParser` that Lucene 10.5.0 `lucene/benchmark` (`byTask.feeds.DemoHTMLParser`) uses, with the same events for the same input (tag balancing, entity decoding, element and attribute name case).

## 2. Key classes / concepts ported

- `org.cyberneko.html`: `HTMLScanner`, `HTMLTagBalancer`, `HTMLConfiguration`, `HTMLElements`, `HTMLEntities`, the filters (`NamespaceBinder`, ...), `parsers.SAXParser`.
- `org.apache.xerces`: the parser and XNI types of their transitive closure (`AbstractSAXParser`, `XMLAttributesImpl`, `URI`, ...).
- `org.xml.sax`: the SAX 2 interfaces and exceptions.

## 3. Design notes

- Re-entrant Java object graphs are shared as `Rc<T>` with `Cell`/`RefCell` state (declared divergence in the crate docs).
- Names and attribute values are Rust `String`s (an unpaired surrogate becomes U+FFFD); character data stays UTF-16 (declared divergence in the crate docs).
- A Java exception travels as `java::Exception` by value (`JResult<T>`).

## 4. Acceptance criteria

- `DemoHTMLParser` over this crate passes Lucene's `TestHtmlParser` and equals Lucene 10.5.0 on the 49-document HTML corpus of rmp #611 (benchmark oracle), except the open lone-surrogate question of `DemoHTMLParser`'s title/body strings.
