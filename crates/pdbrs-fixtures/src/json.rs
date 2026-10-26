//! A minimal JSON value, an indented writer and a parser for the written values.

mod parse;

use std::fmt::Write as _;

pub use parse::parse;

/// A JSON value. Objects keep the insertion order of their members.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// The `null` literal.
    Null,
    /// An unsigned integer.
    Number(u64),
    /// A string.
    String(String),
    /// An ordered list of values.
    Array(Vec<Value>),
    /// An ordered list of members.
    Object(Vec<(String, Value)>),
}

impl Value {
    /// Make a string value.
    pub fn string(text: impl Into<String>) -> Self {
        Self::String(text.into())
    }

    /// Make a string value, or `null` if `text` is `None`.
    pub fn optional(text: Option<&str>) -> Self {
        text.map_or(Self::Null, Self::string)
    }

    /// Make an object from `(name, value)` pairs.
    pub fn object<const N: usize>(members: [(&str, Value); N]) -> Self {
        Self::Object(
            members
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        )
    }

    /// The value of the member `name`, if the value is an object with that member.
    pub fn get(&self, name: &str) -> Option<&Value> {
        match self {
            Self::Object(members) => members
                .iter()
                .find_map(|(member, value)| (member == name).then_some(value)),
            _ => None,
        }
    }

    /// Replace the member `name` of an object, or append it if the object has no such member.
    ///
    /// The call has no effect on a value that is not an object.
    pub fn set(&mut self, name: &str, value: Value) {
        if let Self::Object(members) = self {
            match members.iter_mut().find(|(member, _)| member == name) {
                Some((_, old)) => *old = value,
                None => members.push((name.to_owned(), value)),
            }
        }
    }

    /// The text of a string value.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            _ => None,
        }
    }

    /// Render the value with two-space indentation and a final newline.
    pub fn render(&self) -> String {
        let mut text = String::new();
        self.write(&mut text, 0);
        text.push('\n');
        text
    }

    fn write(&self, text: &mut String, depth: usize) {
        match self {
            Self::Null => text.push_str("null"),
            Self::Number(number) => {
                let _ = write!(text, "{number}");
            }
            Self::String(value) => escape_into(text, value),
            Self::Array(items) if items.is_empty() => text.push_str("[]"),
            Self::Array(items) => {
                text.push('[');
                for (index, item) in items.iter().enumerate() {
                    separator(text, index, depth + 1);
                    item.write(text, depth + 1);
                }
                newline(text, depth);
                text.push(']');
            }
            Self::Object(members) if members.is_empty() => text.push_str("{}"),
            Self::Object(members) => {
                text.push('{');
                for (index, (name, value)) in members.iter().enumerate() {
                    separator(text, index, depth + 1);
                    escape_into(text, name);
                    text.push_str(": ");
                    value.write(text, depth + 1);
                }
                newline(text, depth);
                text.push('}');
            }
        }
    }
}

fn separator(text: &mut String, index: usize, depth: usize) {
    if index > 0 {
        text.push(',');
    }
    newline(text, depth);
}

fn newline(text: &mut String, depth: usize) {
    text.push('\n');
    for _ in 0..depth {
        text.push_str("  ");
    }
}

/// Append `value` to `text` as a quoted JSON string.
pub fn escape_into(text: &mut String, value: &str) {
    text.push('"');
    for character in value.chars() {
        match character {
            '"' => text.push_str("\\\""),
            '\\' => text.push_str("\\\\"),
            '\n' => text.push_str("\\n"),
            '\r' => text.push_str("\\r"),
            '\t' => text.push_str("\\t"),
            '\u{8}' => text.push_str("\\b"),
            '\u{c}' => text.push_str("\\f"),
            control if u32::from(control) < 0x20 || control == '\u{7f}' => {
                let _ = write!(text, "\\u{:04x}", u32::from(control));
            }
            other => text.push(other),
        }
    }
    text.push('"');
}

#[cfg(test)]
mod tests {
    use super::{Value, escape_into};

    fn escaped(value: &str) -> String {
        let mut text = String::new();
        escape_into(&mut text, value);
        text
    }

    #[test]
    fn escapes_quotes_backslashes_and_controls() {
        assert_eq!(escaped(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(escaped("line\nfeed\r\ttab"), r#""line\nfeed\r\ttab""#);
        assert_eq!(
            escaped("\u{0}\u{1f}\u{7f}\u{8}\u{c}"),
            r#""\u0000\u001f\u007f\b\f""#
        );
    }

    #[test]
    fn keeps_non_ascii_text() {
        assert_eq!(escaped("Größe ✓"), "\"Größe ✓\"");
    }

    #[test]
    fn escapes_windows_paths() {
        assert_eq!(escaped(r"C:\Program Files\x"), r#""C:\\Program Files\\x""#);
    }

    #[test]
    fn renders_nested_values_with_indentation() {
        let value = Value::object([
            ("name", Value::string("a")),
            ("size", Value::Number(12)),
            ("reason", Value::optional(None)),
            ("empty", Value::Array(Vec::new())),
            (
                "items",
                Value::Array(vec![Value::Number(1), Value::object([])]),
            ),
        ]);
        let expected = "{\n  \"name\": \"a\",\n  \"size\": 12,\n  \"reason\": null,\n  \
                        \"empty\": [],\n  \"items\": [\n    1,\n    {}\n  ]\n}\n";
        assert_eq!(value.render(), expected);
    }

    #[test]
    fn gets_and_sets_object_members() {
        let mut value = Value::object([("status", Value::string("built"))]);
        assert_eq!(value.get("status").and_then(Value::as_str), Some("built"));
        assert_eq!(value.get("reason"), None);
        value.set("status", Value::string("unavailable"));
        value.set("reason", Value::string("gone"));
        assert_eq!(
            value,
            Value::object([
                ("status", Value::string("unavailable")),
                ("reason", Value::string("gone")),
            ])
        );
        let mut number = Value::Number(1);
        number.set("status", Value::Null);
        assert_eq!(number, Value::Number(1));
        assert_eq!(number.get("status"), None);
        assert_eq!(number.as_str(), None);
    }
}
