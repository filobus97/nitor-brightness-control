//! A minimal JSON reader (no serde): enough for GitHub releases and Open-Meteo.

/// Only what the release and geocoding lookups need: strings, numbers, arrays and
/// objects; every other value (booleans, null) is parsed and kept as `Other`.
pub enum Json {
    Str(String),
    Num(f64),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
    Other,
}

pub fn field<'a>(obj: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    obj.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

pub fn field_str(obj: &[(String, Json)], key: &str) -> Option<String> {
    match field(obj, key)? {
        Json::Str(s) => Some(s.clone()),
        _ => None,
    }
}

pub struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    pub fn new(text: &'a str) -> Self {
        Parser {
            s: text.as_bytes(),
            i: 0,
        }
    }

    fn skip_ws(&mut self) {
        while self.s.get(self.i).is_some_and(u8::is_ascii_whitespace) {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.skip_ws();
        let hit = self.s.get(self.i) == Some(&c);
        self.i += usize::from(hit);
        hit
    }

    pub fn value(&mut self) -> Option<Json> {
        self.skip_ws();
        match *self.s.get(self.i)? {
            b'"' => self.string().map(Json::Str),
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                if !self.eat(b']') {
                    loop {
                        items.push(self.value()?);
                        if !self.eat(b',') {
                            break;
                        }
                    }
                    if !self.eat(b']') {
                        return None;
                    }
                }
                Some(Json::Arr(items))
            }
            b'{' => {
                self.i += 1;
                let mut fields = Vec::new();
                if !self.eat(b'}') {
                    loop {
                        self.skip_ws();
                        let key = self.string()?;
                        if !self.eat(b':') {
                            return None;
                        }
                        fields.push((key, self.value()?));
                        if !self.eat(b',') {
                            break;
                        }
                    }
                    if !self.eat(b'}') {
                        return None;
                    }
                }
                Some(Json::Obj(fields))
            }
            _ => {
                // number, true, false, null: skip to the next delimiter
                let start = self.i;
                while self
                    .s
                    .get(self.i)
                    .is_some_and(|c| !matches!(c, b',' | b']' | b'}') && !c.is_ascii_whitespace())
                {
                    self.i += 1;
                }
                let text = std::str::from_utf8(&self.s[start..self.i]).ok()?;
                (self.i > start).then(|| text.parse().map_or(Json::Other, Json::Num))
            }
        }
    }

    /// A string literal starting at the opening quote.
    fn string(&mut self) -> Option<String> {
        if self.s.get(self.i) != Some(&b'"') {
            return None;
        }
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = *self.s.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).ok(),
                b'\\' => {
                    let e = *self.s.get(self.i)?;
                    self.i += 1;
                    let ch = match e {
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'u' => self.unicode()?,
                        // `"`, `\`, `/`
                        other => char::from(other),
                    };
                    out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                }
                _ => out.push(c),
            }
        }
    }

    /// The four hex digits after `\u` (with a following low surrogate if needed).
    fn unicode(&mut self) -> Option<char> {
        let hi = self.hex4()?;
        if (0xD800..0xDC00).contains(&hi) && self.s.get(self.i..self.i + 2) == Some(b"\\u") {
            self.i += 2;
            let lo = self.hex4()?;
            let code = 0x10000 + ((hi - 0xD800) << 10) + lo.checked_sub(0xDC00)?;
            return char::from_u32(code);
        }
        Some(char::from_u32(hi).unwrap_or('\u{fffd}'))
    }

    fn hex4(&mut self) -> Option<u32> {
        let digits = self.s.get(self.i..self.i + 4)?;
        self.i += 4;
        u32::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_strings() {
        let mut p = Parser::new(r#""a\"b\\c\/d è 😀""#);
        assert_eq!(p.string().unwrap(), "a\"b\\c/d è 😀");
    }

    #[test]
    fn numbers() {
        let Some(Json::Arr(v)) = Parser::new("[1.5, -2, true, null]").value() else {
            panic!("not an array")
        };
        assert!(matches!(v[0], Json::Num(x) if x == 1.5));
        assert!(matches!(v[1], Json::Num(x) if x == -2.0));
        assert!(matches!(v[2], Json::Other) && matches!(v[3], Json::Other));
    }
}
