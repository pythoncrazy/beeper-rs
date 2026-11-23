//! The `SyntaxError` message of Desktop's JSON body parser (Express's `body-parser` on V8's `JSON.parse`) for a
//! malformed body, which Desktop answers with `{message, code: "UNKNOWN_ERROR"}`.

/// What V8's `ReportUnexpectedToken` reports.
enum Report {
    /// A message of its own, which the position follows.
    Expected(&'static str),
    /// The end of the input.
    End,
    /// A number token where none can be.
    Number,
    /// A string token where none can be.
    String,
    /// Any other character, which the message quotes with the input around it.
    Token,
}

type Fail = (usize, Report);

/// V8's `JsonParser` over UTF-16 units, keeping only what it reports.
struct Parser<'a> {
    s: &'a [u16],
    at: usize,
}

fn is_digit(c: Option<u16>) -> bool {
    c.is_some_and(|c| (b'0' as u16..=b'9' as u16).contains(&c))
}

/// V8's `NumberPart` scan flag.
fn number_part(c: Option<u16>) -> bool {
    is_digit(c) || c.is_some_and(|c| b"+-.eE".iter().any(|&b| u16::from(b) == c))
}

impl Parser<'_> {
    fn cur(&self) -> Option<u16> {
        self.s.get(self.at).copied()
    }

    /// V8's `NextCharacter`: advance, then the current character.
    fn next(&mut self) -> Option<u16> {
        self.at += 1;
        self.cur()
    }

    fn is(&self, b: u8) -> bool {
        self.cur() == Some(u16::from(b))
    }

    fn skip_whitespace(&mut self) {
        while self.cur().is_some_and(|c| [0x20, 0x09, 0x0A, 0x0D].contains(&c)) {
            self.at += 1;
        }
    }

    /// V8's `ReportUnexpectedCharacter` at the cursor.
    fn unexpected(&self) -> Fail {
        let report = match self.cur() {
            None => Report::End,
            c if is_digit(c) || c == Some(u16::from(b'-')) => Report::Number,
            Some(0x22) => Report::String,
            _ => Report::Token,
        };
        (self.at, report)
    }

    /// V8's `Check`: skips whitespace, then takes `b` if it is next.
    fn check(&mut self, b: u8) -> bool {
        self.skip_whitespace();
        let found = self.is(b);
        self.at += usize::from(found);
        found
    }

    /// V8's `Expect` with its message.
    fn expect(&mut self, b: u8, message: &'static str) -> Result<(), Fail> {
        if !self.is(b) {
            return Err((self.at, Report::Expected(message)));
        }
        self.at += 1;
        Ok(())
    }

    /// V8's `ExpectNext`: skips whitespace, then [`Self::expect`].
    fn expect_next(&mut self, b: u8, message: &'static str) -> Result<(), Fail> {
        self.skip_whitespace();
        self.expect(b, message)
    }

    /// V8's `ScanJsonString`, after the opening quote.
    fn string(&mut self) -> Result<(), Fail> {
        loop {
            let ends = |&c: &u16| c == u16::from(b'"') || c == u16::from(b'\\') || c < 0x20;
            let Some(i) = self.s[self.at..].iter().position(ends) else {
                return Err((self.s.len(), Report::Expected("Unterminated string in JSON")));
            };
            self.at += i;
            match self.cur().expect("a stop character") {
                0x22 => {
                    self.at += 1;
                    return Ok(());
                }
                0x5C => match self.next() {
                    None => return Err(self.unexpected()),
                    Some(c) if c > 0xFF => return Err(self.unexpected()),
                    Some(c) if b"\"\\/bfnrt".iter().any(|&b| u16::from(b) == c) => self.at += 1,
                    Some(0x75) => {
                        for _ in 0..4 {
                            if !matches!(self.next(), Some(0x30..=0x39 | 0x41..=0x46 | 0x61..=0x66)) {
                                return Err((self.at, Report::Expected("Bad Unicode escape in JSON")));
                            }
                        }
                        self.at += 1;
                    }
                    Some(_) => return Err((self.at, Report::Expected("Bad escaped character in JSON"))),
                },
                _ => return Err((self.at, Report::Expected("Bad control character in string literal in JSON"))),
            }
        }
    }

    /// V8's `ParseJsonNumber`, at its first character.
    fn number(&mut self) -> Result<(), Fail> {
        let mut c = self.cur();
        let negative = c == Some(u16::from(b'-'));
        if negative {
            c = self.next();
        }
        if c == Some(u16::from(b'0')) {
            c = self.next();
            if number_part(c) {
                if is_digit(c) {
                    return Err((self.at, Report::Number));
                }
            } else if !negative {
                return Ok(());
            }
        } else {
            let start = self.at;
            while self.at < start + 9 && is_digit(self.cur()) {
                self.at += 1;
            }
            if self.at == start {
                return Err((self.at, Report::Expected("No number after minus sign in JSON")));
            }
            if !number_part(self.cur()) {
                return Ok(());
            }
            self.digits();
        }
        if self.is(b'.') {
            if !is_digit(self.next()) {
                return Err((self.at, Report::Expected("Unterminated fractional number in JSON")));
            }
            self.digits();
        }
        if self.is(b'e') || self.is(b'E') {
            let mut c = self.next();
            if c == Some(u16::from(b'-')) || c == Some(u16::from(b'+')) {
                c = self.next();
            }
            if !is_digit(c) {
                return Err((self.at, Report::Expected("Exponent part is missing a number in JSON")));
            }
            self.digits();
        }
        Ok(())
    }

    fn digits(&mut self) {
        while is_digit(self.cur()) {
            self.at += 1;
        }
    }

    /// V8's `ScanLiteral`, at the first character of `word`.
    fn literal(&mut self, word: &str) -> Result<(), Fail> {
        for b in word.bytes().skip(1) {
            match self.next() {
                Some(c) if c == u16::from(b) => {}
                _ => return Err(self.unexpected()),
            }
        }
        self.at += 1;
        Ok(())
    }

    /// V8's `ParseJson`: the values in a loop, with a stack of the open objects (`true`) and arrays.
    fn parse(&mut self) -> Result<(), Fail> {
        let mut open: Vec<bool> = Vec::new();
        loop {
            self.skip_whitespace();
            match self.cur().and_then(|c| u8::try_from(c).ok()) {
                Some(b'"') => {
                    self.at += 1;
                    self.string()?;
                }
                Some(b'-' | b'0'..=b'9') => self.number()?,
                Some(b'{') => {
                    self.at += 1;
                    if !self.check(b'}') {
                        open.push(true);
                        self.expect_next(b'"', "Expected property name or '}' in JSON")?;
                        self.string()?;
                        self.expect_next(b':', "Expected ':' after property name in JSON")?;
                        continue;
                    }
                }
                Some(b'[') => {
                    self.at += 1;
                    if !self.check(b']') {
                        open.push(false);
                        continue;
                    }
                }
                Some(b't') => self.literal("true")?,
                Some(b'f') => self.literal("false")?,
                Some(b'n') => self.literal("null")?,
                _ => return Err(self.unexpected()),
            }
            // The value ends its containers until one takes another value.
            loop {
                match open.last() {
                    None => {
                        self.skip_whitespace();
                        return match self.cur() {
                            None => Ok(()),
                            Some(_) => Err((self.at, Report::Expected("Unexpected non-whitespace character after JSON"))),
                        };
                    }
                    Some(true) if self.check(b',') => {
                        self.expect_next(b'"', "Expected double-quoted property name in JSON")?;
                        self.string()?;
                        self.expect_next(b':', "Expected ':' after property name in JSON")?;
                    }
                    Some(true) => {
                        self.expect(b'}', "Expected ',' or '}' after property value in JSON")?;
                        open.pop();
                        continue;
                    }
                    Some(false) if self.check(b',') => {}
                    Some(false) => {
                        self.expect(b']', "Expected ',' or ']' after array element in JSON")?;
                        open.pop();
                        continue;
                    }
                }
                break;
            }
        }
    }
}

/// V8's `CalculateFileLocation`: the line and the column of a position, where `\r\n` is one line break.
fn line_column(s: &[u16], pos: usize) -> (usize, usize) {
    let (mut line, mut start, mut i) = (1, 0, 0);
    while i < pos {
        if s[i] == 0x0D && i + 1 < pos && s[i + 1] == 0x0A {
            i += 1;
        }
        if s[i] == 0x0D || s[i] == 0x0A {
            line += 1;
            start = i + 1;
        }
        i += 1;
    }
    (line, 1 + i - start)
}

/// V8's `JSON.parse` message for the UTF-16 units of a malformed input, or `None` for valid JSON.
fn v8_message(s: &[u16]) -> Option<String> {
    let (pos, report) = Parser { s, at: 0 }.parse().err()?;
    let text = |r: std::ops::Range<usize>| String::from_utf16_lossy(&s[r]);
    let (line, column) = line_column(s, pos);
    let at = format!("at position {pos} (line {line} column {column})");
    Some(match report {
        Report::Expected(message) => format!("{message} {at}"),
        Report::End => "Unexpected end of JSON input".into(),
        Report::Number => format!("Unexpected number in JSON {at}"),
        Report::String => format!("Unexpected string in JSON {at}"),
        Report::Token => {
            const CONTEXT: usize = 10;
            let all = text(0..s.len());
            if ["[object Object]", "undefined", "Infinity", "NaN"].contains(&all.as_str()) {
                return Some(format!("\"{all}\" is not valid JSON"));
            }
            let token = text(pos..pos + 1);
            let len = s.len();
            match () {
                _ if len <= 2 * CONTEXT => format!("Unexpected token '{token}', \"{all}\" is not valid JSON"),
                _ if pos < CONTEXT => {
                    format!("Unexpected token '{token}', \"{}\"... is not valid JSON", text(0..pos + CONTEXT))
                }
                _ if pos < len - CONTEXT => {
                    format!("Unexpected token '{token}', ...\"{}\"... is not valid JSON", text(pos - CONTEXT..pos + CONTEXT))
                }
                _ => format!("Unexpected token '{token}', ...\"{}\" is not valid JSON", text(pos - CONTEXT..len)),
            }
        }
    })
}

/// The message of `body-parser`'s `json` parser for a body, or `None` when it parses. In strict mode a body must
/// start with `{` or `[`, else the parser parses the body with `#` from that character on and puts the characters
/// back in the message.
pub(super) fn message(body: &str) -> Option<String> {
    let s: Vec<u16> = body.encode_utf16().collect();
    if s.is_empty() {
        return None;
    }
    let first = s.iter().position(|c| ![0x20, 0x09, 0x0A, 0x0D].contains(c));
    if first.is_some_and(|i| s[i] == u16::from(b'{') || s[i] == u16::from(b'[')) {
        return v8_message(&s);
    }
    let partial: Vec<u16> = match first {
        Some(i) => s[..i].iter().copied().chain(std::iter::repeat_n(u16::from(b'#'), s.len() - i)).collect(),
        None => Vec::new(),
    };
    let message = v8_message(&partial).expect("a body with # does not parse");
    // `message.replace(/#+/g, run => body.substring(index, index + run.length))`.
    let index = first.unwrap_or(0);
    let (mut out, mut run) = (String::new(), 0);
    for c in message.chars().chain(['\0']) {
        if c == '#' {
            run += 1;
            continue;
        }
        if run > 0 {
            out.push_str(&String::from_utf16_lossy(&s[index..(index + run).min(s.len())]));
            run = 0;
        }
        out.push(c);
    }
    out.pop();
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::message;

    /// The messages come from Node 22's `JSON.parse` behind `body-parser`'s strict check.
    #[test]
    fn malformed_bodies_get_the_messages_of_v8() {
        let cases = [
            ("{x", "Expected property name or '}' in JSON at position 1 (line 1 column 2)"),
            ("{", "Expected property name or '}' in JSON at position 1 (line 1 column 2)"),
            ("[", "Unexpected end of JSON input"),
            ("x", "Unexpected token 'x', \"x\" is not valid JSON"),
            ("\"str\"", "Unexpected token '\"', \"\"str\"\" is not valid JSON"),
            ("123", "Unexpected token '1', \"123\" is not valid JSON"),
            ("  ", "Unexpected end of JSON input"),
            ("{\"a\":1", "Expected ',' or '}' after property value in JSON at position 6 (line 1 column 7)"),
            ("{\"a\" 1}", "Expected ':' after property name in JSON at position 5 (line 1 column 6)"),
            ("[1 2]", "Expected ',' or ']' after array element in JSON at position 3 (line 1 column 4)"),
            ("[1,]", "Unexpected token ']', \"[1,]\" is not valid JSON"),
            ("{\"a\":1,}", "Expected double-quoted property name in JSON at position 7 (line 1 column 8)"),
            ("{} x", "Unexpected non-whitespace character after JSON at position 3 (line 1 column 4)"),
            ("{\"a\":tru}", "Unexpected token '}', \"{\"a\":tru}\" is not valid JSON"),
            ("{\"a\":tr", "Unexpected end of JSON input"),
            ("{\"a\":-x}", "No number after minus sign in JSON at position 6 (line 1 column 7)"),
            ("{\"a\":01}", "Unexpected number in JSON at position 6 (line 1 column 7)"),
            ("{\"a\":1.x}", "Unterminated fractional number in JSON at position 7 (line 1 column 8)"),
            ("{\"a\":1e+}", "Exponent part is missing a number in JSON at position 8 (line 1 column 9)"),
            ("{\"a\":1E5x}", "Expected ',' or '}' after property value in JSON at position 8 (line 1 column 9)"),
            ("{\"a\":\"abc", "Unterminated string in JSON at position 9 (line 1 column 10)"),
            ("{\"a\":\"a\\x\"}", "Bad escaped character in JSON at position 8 (line 1 column 9)"),
            ("{\"a\":\"a\\u12g4\"}", "Bad Unicode escape in JSON at position 11 (line 1 column 12)"),
            ("{\"a\":\"a\\u12", "Bad Unicode escape in JSON at position 11 (line 1 column 12)"),
            ("{\"a\":\"a\u{1}\"}", "Bad control character in string literal in JSON at position 7 (line 1 column 8)"),
            ("{\"a\":\"a\\", "Unexpected end of JSON input"),
            ("[object Object]", "\"[object Object]\" is not valid JSON"),
            ("{\"abcdefghijklmnopqrstuvwxyz\": tru}", "Unexpected token '}', ...\"wxyz\": tru}\" is not valid JSON"),
            ("{\"a\":\n1,\r\n\"b\" 2}", "Expected ':' after property name in JSON at position 14 (line 3 column 5)"),
            ("{\"a\":\r1 x}", "Expected ',' or '}' after property value in JSON at position 8 (line 2 column 3)"),
            ("[\"😀\", x]", "Unexpected token 'x', \"[\"😀\", x]\" is not valid JSON"),
            ("{'a':1}", "Expected property name or '}' in JSON at position 1 (line 1 column 2)"),
            ("[1,,2]", "Unexpected token ',', \"[1,,2]\" is not valid JSON"),
            ("{\"a\":1}\n\n x", "Unexpected non-whitespace character after JSON at position 10 (line 3 column 2)"),
            ("[-0, -01]", "Unexpected number in JSON at position 7 (line 1 column 8)"),
            ("[0x10]", "Expected ',' or ']' after array element in JSON at position 2 (line 1 column 3)"),
            ("[.5]", "Unexpected token '.', \"[.5]\" is not valid JSON"),
            ("abcdefghijklmnopqrstuvwxyz", "Unexpected token 'a', \"abcdefghij\"... is not valid JSON"),
            (
                "{\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\": 1, \"b\": 2 x",
                "Expected ',' or '}' after property value in JSON at position 55 (line 1 column 56)",
            ),
            ("[1, 2, 3, 4, 5, 6, 7, 8, x, 9, 10]", "Unexpected token 'x', ...\" 6, 7, 8, x, 9, 10]\" is not valid JSON"),
            ("[1, 2, 3, x, 4, 5, 6, 7, 8, 9, 10]", "Unexpected token 'x', ...\"[1, 2, 3, x, 4, 5, 6\"... is not valid JSON"),
            ("  x  ", "Unexpected token 'x', \"  x  \" is not valid JSON"),
            ("[nul\"null", "Unexpected string in JSON at position 4 (line 1 column 5)"),
        ];
        for (body, expected) in cases {
            assert_eq!(message(body).as_deref(), Some(expected), "{body:?}");
        }
        for body in ["", "  {\"a\":1} ", "[1e5, 1.5e-3, 123456789012345]", "{\"a\":\"\\u00e9\\n\"}", "[-0, 0.5, true]"] {
            assert_eq!(message(body), None, "{body:?}");
        }
    }
}
