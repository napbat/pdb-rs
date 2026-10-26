//! A parser for the JSON values that the writer makes.
//!
//! The parser accepts `null`, unsigned integers, strings, arrays and objects. It rejects
//! booleans, negative numbers and fractions, because [`Value`] has no form for them.

use super::Value;

/// The maximum depth of nested arrays and objects.
const MAX_DEPTH: usize = 64;

/// Parse one JSON value. Whitespace can come before and after the value.
pub fn parse(text: &str) -> Result<Value, String> {
    let mut parser = Parser { text, position: 0 };
    parser.skip_whitespace();
    let value = parser.value(0)?;
    parser.skip_whitespace();
    if parser.position == text.len() {
        Ok(value)
    } else {
        Err(parser.error("unexpected data after the value"))
    }
}

struct Parser<'a> {
    text: &'a str,
    position: usize,
}

impl Parser<'_> {
    fn error(&self, message: &str) -> String {
        format!("{message} at byte {}", self.position)
    }

    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.position).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.position += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.peek() == Some(byte) {
            self.position += 1;
            Ok(())
        } else {
            Err(self.error(&format!("expected `{}`", char::from(byte))))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return Err(self.error("the values are nested too deeply"));
        }
        match self.peek() {
            Some(b'n') if self.text[self.position..].starts_with("null") => {
                self.position += 4;
                Ok(Value::Null)
            }
            Some(b'0'..=b'9') => self.number(),
            Some(b'"') => self.string().map(Value::String),
            Some(b'[') => self.array(depth),
            Some(b'{') => self.object(depth),
            Some(_) => Err(self.error("unsupported value")),
            None => Err(self.error("unexpected end of the text")),
        }
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.position;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.position += 1;
        }
        if matches!(self.peek(), Some(b'.' | b'e' | b'E')) {
            return Err(self.error("only unsigned integers are supported"));
        }
        let digits = &self.text[start..self.position];
        if digits.len() > 1 && digits.starts_with('0') {
            return Err(self.error("a number has a leading zero"));
        }
        digits
            .parse()
            .map(Value::Number)
            .map_err(|_| self.error("the number is too large"))
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut text = String::new();
        loop {
            // Bytes of multi-byte characters are never `"`, `\` or controls, so each slice
            // boundary below is a character boundary.
            let start = self.position;
            while matches!(self.peek(), Some(byte) if byte != b'"' && byte != b'\\' && byte >= 0x20)
            {
                self.position += 1;
            }
            text.push_str(&self.text[start..self.position]);
            match self.peek() {
                Some(b'"') => {
                    self.position += 1;
                    return Ok(text);
                }
                Some(b'\\') => {
                    self.position += 1;
                    let character = self.escape()?;
                    text.push(character);
                }
                Some(_) => return Err(self.error("a control character is not escaped")),
                None => return Err(self.error("a string has no end")),
            }
        }
    }

    fn escape(&mut self) -> Result<char, String> {
        let byte = self
            .peek()
            .ok_or_else(|| self.error("an escape has no end"))?;
        self.position += 1;
        Ok(match byte {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => return self.unicode_escape(),
            _ => return Err(self.error("unknown escape")),
        })
    }

    fn unicode_escape(&mut self) -> Result<char, String> {
        let first = self.hex4()?;
        let code = if (0xD800..0xDC00).contains(&first) {
            if !self.text[self.position..].starts_with("\\u") {
                return Err(self.error("a high surrogate has no low surrogate"));
            }
            self.position += 2;
            let second = self.hex4()?;
            if !(0xDC00..0xE000).contains(&second) {
                return Err(self.error("a high surrogate has no low surrogate"));
            }
            0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
        } else {
            first
        };
        char::from_u32(code).ok_or_else(|| self.error("an escape is not a Unicode scalar value"))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self
            .text
            .get(self.position..self.position + 4)
            .ok_or_else(|| self.error("a \\u escape needs four hex digits"))?;
        let code = u32::from_str_radix(digits, 16)
            .map_err(|_| self.error("a \\u escape needs four hex digits"))?;
        self.position += 4;
        Ok(code)
    }

    fn array(&mut self, depth: usize) -> Result<Value, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.position += 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.position += 1,
                Some(b']') => {
                    self.position += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(self.error("expected `,` or `]`")),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, String> {
        self.expect(b'{')?;
        let mut members = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.position += 1;
            return Ok(Value::Object(members));
        }
        loop {
            self.skip_whitespace();
            let name = self.string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            self.skip_whitespace();
            members.push((name, self.value(depth + 1)?));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.position += 1,
                Some(b'}') => {
                    self.position += 1;
                    return Ok(Value::Object(members));
                }
                _ => return Err(self.error("expected `,` or `}`")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse;
    use crate::json::Value;

    #[test]
    fn reads_what_the_writer_renders() {
        let value = Value::object([
            ("name", Value::string("a \"b\" \\ c\n\u{1}")),
            ("size", Value::Number(u64::MAX)),
            ("reason", Value::Null),
            ("empty", Value::Array(Vec::new())),
            ("text", Value::string("Größe ✓")),
            (
                "items",
                Value::Array(vec![Value::Number(0), Value::object([])]),
            ),
        ]);
        assert_eq!(parse(&value.render()), Ok(value));
    }

    #[test]
    fn decodes_unicode_escapes_and_surrogate_pairs() {
        assert_eq!(
            parse(r#""\u00e9\ud83d\ude00\/""#),
            Ok(Value::string("é😀/"))
        );
        assert!(parse(r#""\ud83d""#).is_err());
        assert!(parse(r#""\ud83d\u0041""#).is_err());
        assert!(parse(r#""\u12""#).is_err());
    }

    #[test]
    fn rejects_values_without_a_form() {
        assert!(parse("true").is_err());
        assert!(parse("-1").is_err());
        assert!(parse("1.5").is_err());
        assert!(parse("01").is_err());
        assert!(parse("18446744073709551616").is_err());
    }

    #[test]
    fn rejects_malformed_text() {
        assert_eq!(parse("[1,]"), Err("unsupported value at byte 3".to_owned()));
        assert!(parse("{\"a\" 1}").is_err());
        assert!(parse("[1 2]").is_err());
        assert!(parse("\"open").is_err());
        assert!(parse("\"tab\there\"").is_err());
        assert!(parse("{} {}").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn limits_the_depth() {
        let deep = format!("{}{}", "[".repeat(100), "]".repeat(100));
        assert!(parse(&deep).is_err());
        let shallow = format!("{}{}", "[".repeat(10), "]".repeat(10));
        assert!(parse(&shallow).is_ok());
    }
}
