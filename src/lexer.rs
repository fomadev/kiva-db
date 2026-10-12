// Copyright (c) 2026 Fordi / FomaDev.
// Licensed under FomaDev Public License.
// See LICENSE file in the project root for full license information.

//! Shell tokenizer.
//!
//! * `"..."` / `'...'` are string literals (no escape sequences: use the
//!   other quote character to embed a quote).
//! * `` `...` `` quotes a key name.
//! * `[...]` and `{...}` at the start of a token are read as one JSON literal.
//! * `,` `(` `)` are standalone separator tokens.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Bare,
    DQuote,
    SQuote,
    Backtick,
    Json,
    Comma,
    LParen,
    RParen,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub text: String,
    pub kind: Kind,
    /// Byte offset of the token in the input line.
    pub start: usize,
    pub end: usize,
}

impl Token {
    pub fn is_quoted(&self) -> bool {
        matches!(self.kind, Kind::DQuote | Kind::SQuote)
    }

    pub fn is_bare(&self) -> bool {
        self.kind == Kind::Bare
    }

    /// Case-insensitive keyword test; quoted text is never a keyword.
    pub fn is_kw(&self, kw: &str) -> bool {
        self.kind == Kind::Bare && self.text.eq_ignore_ascii_case(kw)
    }
}

pub fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut cur_start = 0usize;
    let chars: Vec<(usize, char)> = input.char_indices().collect();
    let mut i = 0usize;

    let flush = |current: &mut String, start: usize, end: usize, tokens: &mut Vec<Token>| {
        if !current.is_empty() {
            tokens.push(Token { text: std::mem::take(current), kind: Kind::Bare, start, end });
        }
    };

    while i < chars.len() {
        let (pos, c) = chars[i];
        match c {
            '"' | '\'' | '`' => {
                flush(&mut current, cur_start, pos, &mut tokens);
                let mut j = i + 1;
                let mut text = String::new();
                let mut closed = false;
                while j < chars.len() {
                    if chars[j].1 == c {
                        closed = true;
                        break;
                    }
                    text.push(chars[j].1);
                    j += 1;
                }
                if !closed {
                    return Err(format!("Syntax Error: Unclosed quote detected ({c})."));
                }
                let kind = match c {
                    '"' => Kind::DQuote,
                    '\'' => Kind::SQuote,
                    _ => Kind::Backtick,
                };
                tokens.push(Token { text, kind, start: pos, end: chars[j].0 + 1 });
                i = j + 1;
            }
            '[' | '{' if current.is_empty() => {
                // One JSON literal: scan to the matching bracket, honouring JSON strings.
                let mut depth = 0i32;
                let mut in_str = false;
                let mut escaped = false;
                let mut j = i;
                let mut end = None;
                while j < chars.len() {
                    let ch = chars[j].1;
                    if in_str {
                        if escaped {
                            escaped = false;
                        } else if ch == '\\' {
                            escaped = true;
                        } else if ch == '"' {
                            in_str = false;
                        }
                    } else {
                        match ch {
                            '"' => in_str = true,
                            '[' | '{' => depth += 1,
                            ']' | '}' => {
                                depth -= 1;
                                if depth == 0 {
                                    end = Some(j);
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    j += 1;
                }
                let Some(e) = end else {
                    return Err(format!("Syntax Error: Unclosed bracket detected ({c})."));
                };
                let end_pos = chars[e].0 + chars[e].1.len_utf8();
                tokens.push(Token {
                    text: input[pos..end_pos].to_string(),
                    kind: Kind::Json,
                    start: pos,
                    end: end_pos,
                });
                i = e + 1;
            }
            ',' | '(' | ')' => {
                flush(&mut current, cur_start, pos, &mut tokens);
                let kind = match c {
                    ',' => Kind::Comma,
                    '(' => Kind::LParen,
                    _ => Kind::RParen,
                };
                tokens.push(Token { text: c.to_string(), kind, start: pos, end: pos + 1 });
                i += 1;
            }
            c if c.is_whitespace() => {
                flush(&mut current, cur_start, pos, &mut tokens);
                i += 1;
            }
            c => {
                if current.is_empty() {
                    cur_start = pos;
                }
                current.push(c);
                i += 1;
            }
        }
    }
    flush(&mut current, cur_start, input.len(), &mut tokens);
    Ok(tokens)
}

/// Splits a token list on the bare keyword `and` (command chaining).
pub fn split_by_and(tokens: &[Token]) -> Vec<Vec<Token>> {
    let mut groups = Vec::new();
    let mut cur = Vec::new();
    for t in tokens {
        if t.is_kw("and") {
            if !cur.is_empty() {
                groups.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(t.clone());
        }
    }
    if !cur.is_empty() {
        groups.push(cur);
    }
    groups
}
