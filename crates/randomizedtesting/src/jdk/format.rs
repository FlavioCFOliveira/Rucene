//! `String.format(Locale.ROOT, format, args)` for the conversions the
//! runner's argument templates use.
//!
//! **Divergence.** `java.util.Formatter` supports every conversion; the
//! port supports `%s`, `%S`, `%d`, `%b`, `%n` and `%%`, with an explicit
//! argument index (`%2$s`), the `-` flag and a width. Any other conversion
//! throws `UnknownFormatConversionException`, as an unknown one does in
//! Java.

use super::exception::Throwable;
use super::reflect::Value;

/// `String.format(Locale.ROOT, format, args)`.
///
/// # Panics
///
/// `java.util.MissingFormatArgumentException` (`Format specifier '%s'`)
/// when an argument is missing; `java.util.UnknownFormatConversionException`
/// for an unsupported conversion.
pub fn format(template: &str, args: &[Value]) -> String {
    let chars: Vec<char> = template.chars().collect();
    let mut out = String::new();
    let mut ordinary = 0usize;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c != '%' {
            out.push(c);
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        // argument_index$
        let mut j = i;
        while j < chars.len() && chars[j].is_ascii_digit() {
            j += 1;
        }
        let mut index: Option<usize> = None;
        if j < chars.len() && chars[j] == '$' && j > i {
            let n: String = chars[i..j].iter().collect();
            index = n.parse::<usize>().ok();
            i = j + 1;
        }
        // flags
        let mut left = false;
        while i < chars.len() && matches!(chars[i], '-' | '#' | '+' | ' ' | '0' | ',' | '(') {
            if chars[i] == '-' {
                left = true;
            }
            i += 1;
        }
        // width
        let mut w = i;
        while w < chars.len() && chars[w].is_ascii_digit() {
            w += 1;
        }
        let width: usize = chars[i..w].iter().collect::<String>().parse().unwrap_or(0);
        i = w;
        let Some(&conv) = chars.get(i) else {
            Throwable::new(
                "java.util.UnknownFormatConversionException",
                format!(
                    "Conversion = '{}'",
                    chars[start..].iter().collect::<String>()
                ),
            )
            .throw()
        };
        i += 1;
        let spec: String = chars[start..i].iter().collect();
        let text = match conv {
            '%' => "%".to_string(),
            'n' => "\n".to_string(),
            's' | 'S' | 'd' | 'b' | 'B' => {
                let arg_index = match index {
                    Some(n) => n.checked_sub(1),
                    None => {
                        ordinary += 1;
                        Some(ordinary - 1)
                    }
                };
                let Some(arg) = arg_index.and_then(|k| args.get(k)) else {
                    Throwable::new(
                        "java.util.MissingFormatArgumentException",
                        format!("Format specifier '{spec}'"),
                    )
                    .throw()
                };
                match conv {
                    's' => arg.to_string(),
                    'S' => arg.to_string().to_uppercase(),
                    'b' | 'B' => {
                        let b = match arg {
                            Value::Null => "false".to_string(),
                            Value::Bool(b) => b.to_string(),
                            _ => "true".to_string(),
                        };
                        if conv == 'B' {
                            b.to_uppercase()
                        } else {
                            b
                        }
                    }
                    _ => match arg {
                        Value::Int(_) | Value::Long(_) => arg.to_string(),
                        _ => Throwable::new(
                            "java.util.IllegalFormatConversionException",
                            format!("d != {}", value_class(arg)),
                        )
                        .throw(),
                    },
                }
            }
            other => Throwable::new(
                "java.util.UnknownFormatConversionException",
                format!("Conversion = '{other}'"),
            )
            .throw(),
        };
        let len = text.chars().count();
        if len < width {
            let pad = " ".repeat(width - len);
            if left {
                out.push_str(&text);
                out.push_str(&pad);
            } else {
                out.push_str(&pad);
                out.push_str(&text);
            }
        } else {
            out.push_str(&text);
        }
    }
    out
}

fn value_class(v: &Value) -> &'static str {
    match v {
        Value::Str(_) => "java.lang.String",
        Value::Bool(_) => "java.lang.Boolean",
        Value::Double(_) => "java.lang.Double",
        _ => "java.lang.Object",
    }
}
