//! Parser for the Datadog search syntax, shared by logs and spans.
//!
//! Supported: implicit `AND` between terms, `AND` / `OR` / `NOT`, `-` negation,
//! parentheses, `"quoted phrases"`, `key:value`, `@attribute:value`,
//! `key:val*` wildcards, `key:*` existence, `@n:>=10` comparisons,
//! `@n:[10 TO 20]` ranges, `key:(a OR b)` groups and `\` escapes.

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    All,
    And(Vec<Expr>),
    Or(Vec<Expr>),
    Not(Box<Expr>),
    Text(Value),
    Field { key: String, cond: Cond },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Value {
    pub text: String,
    pub wildcard: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CmpOp {
    Gt,
    Ge,
    Lt,
    Le,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Cond {
    Eq(Value),
    Cmp(CmpOp, String),
    Range {
        lo: String,
        hi: String,
        lo_inc: bool,
        hi_inc: bool,
    },
    Exists,
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    LParen,
    RParen,
    And,
    Or,
    Not,
    Minus,
    Word {
        text: String,
        quoted: bool,
        wildcard: bool,
    },
    Key(String),
    Range {
        lo: String,
        hi: String,
        lo_inc: bool,
        hi_inc: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn lex(input: &str) -> Result<Vec<Tok>, ParseError> {
    let chars: Vec<char> = input.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    let mut after_key = false;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            after_key = false;
            continue;
        }
        let value_mode = after_key;
        after_key = false;
        match c {
            '(' => {
                toks.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                toks.push(Tok::RParen);
                i += 1;
            }
            '-' if !value_mode && i + 1 < chars.len() && !chars[i + 1].is_whitespace() => {
                toks.push(Tok::Minus);
                i += 1;
            }
            '"' => {
                i += 1;
                let mut text = String::new();
                let mut closed = false;
                while i < chars.len() {
                    match chars[i] {
                        '\\' if i + 1 < chars.len() => {
                            text.push(chars[i + 1]);
                            i += 2;
                        }
                        '"' => {
                            closed = true;
                            i += 1;
                            break;
                        }
                        ch => {
                            text.push(ch);
                            i += 1;
                        }
                    }
                }
                if !closed {
                    return Err(ParseError("unterminated quoted string".into()));
                }
                toks.push(Tok::Word {
                    text,
                    quoted: true,
                    wildcard: false,
                });
            }
            '[' | '{' if value_mode => {
                let lo_inc = c == '[';
                let start = i + 1;
                let mut j = start;
                while j < chars.len() && chars[j] != ']' && chars[j] != '}' {
                    j += 1;
                }
                if j >= chars.len() {
                    return Err(ParseError("unterminated range".into()));
                }
                let hi_inc = chars[j] == ']';
                let inner: String = chars[start..j].iter().collect();
                let parts: Vec<&str> = inner.split(" TO ").collect();
                if parts.len() != 2 {
                    return Err(ParseError(format!(
                        "invalid range [{inner}], expected [a TO b]"
                    )));
                }
                toks.push(Tok::Range {
                    lo: parts[0].trim().to_string(),
                    hi: parts[1].trim().to_string(),
                    lo_inc,
                    hi_inc,
                });
                i = j + 1;
            }
            _ => {
                let mut text = String::new();
                let mut wildcard = false;
                let mut escaped_any = false;
                while i < chars.len() {
                    let ch = chars[i];
                    if ch.is_whitespace() || ch == '(' || ch == ')' {
                        break;
                    }
                    if ch == '\\' && i + 1 < chars.len() {
                        text.push(chars[i + 1]);
                        escaped_any = true;
                        i += 2;
                        continue;
                    }
                    if ch == ':' && !value_mode && !text.is_empty() {
                        break;
                    }
                    if ch == '*' {
                        wildcard = true;
                    }
                    text.push(ch);
                    i += 1;
                }
                if i < chars.len() && chars[i] == ':' && !value_mode && !text.is_empty() {
                    i += 1;
                    toks.push(Tok::Key(text));
                    after_key = true;
                    continue;
                }
                if !escaped_any && !value_mode {
                    match text.as_str() {
                        "AND" => {
                            toks.push(Tok::And);
                            continue;
                        }
                        "OR" => {
                            toks.push(Tok::Or);
                            continue;
                        }
                        "NOT" => {
                            toks.push(Tok::Not);
                            continue;
                        }
                        _ => {}
                    }
                }
                toks.push(Tok::Word {
                    text,
                    quoted: false,
                    wildcard,
                });
            }
        }
    }
    Ok(toks)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn parse_or(&mut self, key: Option<&str>) -> Result<Expr, ParseError> {
        let mut items = vec![self.parse_and(key)?];
        while matches!(self.peek(), Some(Tok::Or)) {
            self.pos += 1;
            items.push(self.parse_and(key)?);
        }
        Ok(if items.len() == 1 {
            items.pop().unwrap()
        } else {
            Expr::Or(items)
        })
    }

    fn parse_and(&mut self, key: Option<&str>) -> Result<Expr, ParseError> {
        let mut items = vec![self.parse_unary(key)?];
        loop {
            match self.peek() {
                None | Some(Tok::Or) | Some(Tok::RParen) => break,
                Some(Tok::And) => {
                    self.pos += 1;
                }
                _ => {}
            }
            items.push(self.parse_unary(key)?);
        }
        Ok(if items.len() == 1 {
            items.pop().unwrap()
        } else {
            Expr::And(items)
        })
    }

    fn parse_unary(&mut self, key: Option<&str>) -> Result<Expr, ParseError> {
        match self.peek() {
            Some(Tok::Not) | Some(Tok::Minus) => {
                self.pos += 1;
                Ok(Expr::Not(Box::new(self.parse_unary(key)?)))
            }
            _ => self.parse_primary(key),
        }
    }

    fn parse_primary(&mut self, key: Option<&str>) -> Result<Expr, ParseError> {
        match self.next() {
            Some(Tok::LParen) => {
                let e = self.parse_or(key)?;
                match self.next() {
                    Some(Tok::RParen) => Ok(e),
                    _ => Err(ParseError("missing closing parenthesis".into())),
                }
            }
            Some(Tok::Key(k)) => match self.next() {
                Some(Tok::LParen) => {
                    let e = self.parse_or(Some(&k))?;
                    match self.next() {
                        Some(Tok::RParen) => Ok(e),
                        _ => Err(ParseError("missing closing parenthesis".into())),
                    }
                }
                Some(Tok::Word {
                    text,
                    quoted,
                    wildcard,
                }) => Ok(Expr::Field {
                    cond: cond_from_word(text, quoted, wildcard),
                    key: k,
                }),
                Some(Tok::Range {
                    lo,
                    hi,
                    lo_inc,
                    hi_inc,
                }) => Ok(Expr::Field {
                    key: k,
                    cond: Cond::Range {
                        lo,
                        hi,
                        lo_inc,
                        hi_inc,
                    },
                }),
                _ => Err(ParseError(format!("missing value after `{k}:`"))),
            },
            Some(Tok::Word {
                text,
                quoted,
                wildcard,
            }) => Ok(match key {
                Some(k) => Expr::Field {
                    key: k.to_string(),
                    cond: cond_from_word(text, quoted, wildcard),
                },
                None => Expr::Text(Value { text, wildcard }),
            }),
            Some(Tok::Range { .. }) => Err(ParseError("unexpected range".into())),
            Some(t) => Err(ParseError(format!("unexpected {t:?}"))),
            None => Err(ParseError("unexpected end of query".into())),
        }
    }
}

fn cond_from_word(text: String, quoted: bool, wildcard: bool) -> Cond {
    if !quoted {
        if text == "*" {
            return Cond::Exists;
        }
        for (prefix, op) in [
            (">=", CmpOp::Ge),
            ("<=", CmpOp::Le),
            (">", CmpOp::Gt),
            ("<", CmpOp::Lt),
        ] {
            if let Some(rest) = text.strip_prefix(prefix) {
                return Cond::Cmp(op, rest.to_string());
            }
        }
    }
    Cond::Eq(Value {
        text,
        wildcard: wildcard && !quoted,
    })
}

pub fn parse(input: &str) -> Result<Expr, ParseError> {
    let toks = lex(input)?;
    if toks.is_empty() {
        return Ok(Expr::All);
    }
    let mut p = Parser { toks, pos: 0 };
    let e = p.parse_or(None)?;
    if p.pos < p.toks.len() {
        return Err(ParseError("unexpected closing parenthesis".into()));
    }
    Ok(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eq(key: &str, v: &str) -> Expr {
        Expr::Field {
            key: key.into(),
            cond: Cond::Eq(Value {
                text: v.into(),
                wildcard: v.contains('*'),
            }),
        }
    }

    #[test]
    fn empty_is_all() {
        assert_eq!(parse("   ").unwrap(), Expr::All);
    }

    #[test]
    fn implicit_and() {
        assert_eq!(
            parse("service:web env:prod").unwrap(),
            Expr::And(vec![eq("service", "web"), eq("env", "prod")])
        );
    }

    #[test]
    fn or_and_precedence() {
        assert_eq!(
            parse("a b OR c").unwrap(),
            Expr::Or(vec![
                Expr::And(vec![
                    Expr::Text(Value {
                        text: "a".into(),
                        wildcard: false
                    }),
                    Expr::Text(Value {
                        text: "b".into(),
                        wildcard: false
                    })
                ]),
                Expr::Text(Value {
                    text: "c".into(),
                    wildcard: false
                })
            ])
        );
    }

    #[test]
    fn negation_and_groups() {
        assert_eq!(
            parse("-status:info service:(web OR api)").unwrap(),
            Expr::And(vec![
                Expr::Not(Box::new(eq("status", "info"))),
                Expr::Or(vec![eq("service", "web"), eq("service", "api")])
            ])
        );
    }

    #[test]
    fn values_may_contain_colons() {
        assert_eq!(
            parse("@http.url:http://x:80/a").unwrap(),
            eq("@http.url", "http://x:80/a")
        );
    }

    #[test]
    fn comparisons_and_ranges() {
        assert_eq!(
            parse("@http.status_code:>=500").unwrap(),
            Expr::Field {
                key: "@http.status_code".into(),
                cond: Cond::Cmp(CmpOp::Ge, "500".into())
            }
        );
        assert_eq!(
            parse("@duration:[1ms TO 2s}").unwrap(),
            Expr::Field {
                key: "@duration".into(),
                cond: Cond::Range {
                    lo: "1ms".into(),
                    hi: "2s".into(),
                    lo_inc: true,
                    hi_inc: false
                }
            }
        );
    }

    #[test]
    fn quoted_and_wildcards() {
        assert_eq!(
            parse("\"connection refused\"").unwrap(),
            Expr::Text(Value {
                text: "connection refused".into(),
                wildcard: false
            })
        );
        assert_eq!(parse("service:web-*").unwrap(), eq("service", "web-*"));
        assert_eq!(
            parse("@user.id:*").unwrap(),
            Expr::Field {
                key: "@user.id".into(),
                cond: Cond::Exists
            }
        );
    }

    #[test]
    fn hyphen_inside_word_is_not_negation() {
        assert_eq!(
            parse("service:shop-api").unwrap(),
            eq("service", "shop-api")
        );
    }

    #[test]
    fn errors() {
        assert!(parse("(a").is_err());
        assert!(parse("a)").is_err());
        assert!(parse("\"a").is_err());
        assert!(parse("service:").is_err());
    }
}
