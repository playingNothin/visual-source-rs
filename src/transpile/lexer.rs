//! Luau tokenizer for the transpiler's restricted subset.
//! Handles: comments, strings, numbers, names/keywords, all operators used
//! by the corpus, and line/column tracking for error messages.

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Name(String),
    Number(f64),
    Str(String),
    // keywords
    Local, Function, End, If, Then, Elseif, Else, While, Do, For, In,
    Return, Break, Continue, And, Or, Not, Nil, True, False, Repeat, Until,
    // symbols
    Plus, Minus, Star, Slash, Percent, Caret, Hash, DotDot, Dot, Comma,
    Semi, Assign, Eq, Ne, Lt, Le, Gt, Ge, LParen, RParen, LBrace, RBrace,
    LBracket, RBracket, Colon, DColon,
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
    pub col: usize,
}

#[derive(Debug)]
pub struct LexError {
    pub msg: String,
    pub line: usize,
    pub col: usize,
}

/// [v23] A comment captured from the source. Line comments keep their `--`,
/// long comments their `--[[ ]]` body; `long` marks which shape it was so
/// the emitter can join runs of line comments with newlines.
#[derive(Debug, Clone)]
pub struct CapturedComment {
    pub line_start: usize,
    pub line_end: usize,
    pub text: String,
    pub long: bool,
}

impl std::fmt::Display for LexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.col, self.msg)
    }
}

pub fn lex(src: &str) -> Result<Vec<Token>, LexError> {
    lex_with_comments(src).map(|(toks, _)| toks)
}

/// [v23] Same token stream as [`lex`], plus every comment in source order
/// (tokens themselves are unchanged - comments never reach the parser).
pub fn lex_with_comments(src: &str) -> Result<(Vec<Token>, Vec<CapturedComment>), LexError> {
    let b: Vec<char> = src.chars().collect();
    let n = b.len();
    let mut i = 0usize;
    let mut line = 1usize;
    let mut col = 1usize;
    let mut out = Vec::new();
    let mut comments: Vec<CapturedComment> = Vec::new();

    macro_rules! push {
        ($t:expr) => {
            out.push(Token { tok: $t, line, col })
        };
    }

    while i < n {
        let c = b[i];
        // whitespace
        if c == '\n' {
            i += 1; line += 1; col = 1; continue;
        }
        if c.is_whitespace() {
            i += 1; col += 1; continue;
        }
        // comments
        if c == '-' && i + 1 < n && b[i + 1] == '-' {
            // long comment --[[ ... ]]
            if i + 3 < n && b[i + 2] == '[' && b[i + 3] == '[' {
                let mut j = i + 4;
                let mut l = line;
                let mut cl = col + 4;
                let text_start = j;
                while j + 1 < n && !(b[j] == ']' && b[j + 1] == ']') {
                    if b[j] == '\n' { l += 1; cl = 1; } else { cl += 1; }
                    j += 1;
                }
                if j + 1 >= n {
                    return Err(LexError { msg: "unterminated long comment".into(), line, col });
                }
                let text: String = b[text_start..j].iter().collect();
                comments.push(CapturedComment {
                    line_start: line,
                    line_end: l,
                    text,
                    long: true,
                });
                i = j + 2; line = l; col = cl + 2; continue;
            }
            // line comment
            let text_start = i + 2;
            while i < n && b[i] != '\n' {
                i += 1;
            }
            let text: String = b[text_start..i].iter().collect();
            comments.push(CapturedComment {
                line_start: line,
                line_end: line,
                text,
                long: false,
            });
            continue;
        }
        // names / keywords
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < n && (b[i].is_alphanumeric() || b[i] == '_') {
                i += 1;
            }
            let word: String = b[start..i].iter().collect();
            col += i - start;
            let t = match word.as_str() {
                "local" => Tok::Local, "function" => Tok::Function, "end" => Tok::End,
                "if" => Tok::If, "then" => Tok::Then, "elseif" => Tok::Elseif,
                "else" => Tok::Else, "while" => Tok::While, "do" => Tok::Do,
                "for" => Tok::For, "in" => Tok::In, "return" => Tok::Return,
                "break" => Tok::Break, "continue" => Tok::Continue,
                "and" => Tok::And, "or" => Tok::Or, "not" => Tok::Not,
                "nil" => Tok::Nil, "true" => Tok::True, "false" => Tok::False,
                "repeat" => Tok::Repeat, "until" => Tok::Until,
                _ => Tok::Name(word),
            };
            push!(t);
            continue;
        }
        // numbers
        if c.is_ascii_digit() || (c == '.' && i + 1 < n && b[i + 1].is_ascii_digit()) {
            let start = i;
            while i < n && (b[i].is_ascii_digit() || b[i] == '.') {
                i += 1;
            }
            // hex literals 0x...
            let s: String = b[start..i].iter().collect();
            let v: f64 = s.parse().map_err(|_| LexError {
                msg: format!("bad number {:?}", s),
                line,
                col,
            })?;
            col += i - start;
            push!(Tok::Number(v));
            continue;
        }
        // strings
        if c == '"' || c == '\'' {
            let quote = c;
            i += 1; col += 1;
            let mut s = String::new();
            while i < n && b[i] != quote {
                if b[i] == '\\' && i + 1 < n {
                    i += 1; col += 1;
                    let e = b[i];
                    s.push(match e {
                        'n' => '\n', 't' => '\t', 'r' => '\r', '\\' => '\\',
                        '"' => '"', '\'' => '\'',
                        other => other,
                    });
                    i += 1; col += 1;
                } else {
                    if b[i] == '\n' { line += 1; col = 1; } else { col += 1; }
                    s.push(b[i]);
                    i += 1;
                }
            }
            if i >= n {
                return Err(LexError { msg: "unterminated string".into(), line, col });
            }
            i += 1; col += 1;
            push!(Tok::Str(s));
            continue;
        }
        // operators
        let two = |a: char, d: char| i + 1 < n && b[i] == a && b[i + 1] == d;
        if two('.', '.') {
            i += 2; col += 2; push!(Tok::DotDot); continue;
        }
        if two(':', ':') {
            i += 2; col += 2; push!(Tok::DColon); continue;
        }
        let single = match c {
            '+' => Some(Tok::Plus), '-' => Some(Tok::Minus), '*' => Some(Tok::Star),
            '/' => Some(Tok::Slash), '%' => Some(Tok::Percent), '^' => Some(Tok::Caret),
            '#' => Some(Tok::Hash), '.' => Some(Tok::Dot), ',' => Some(Tok::Comma),
            ';' => Some(Tok::Semi), '(' => Some(Tok::LParen), ')' => Some(Tok::RParen),
            '{' => Some(Tok::LBrace), '}' => Some(Tok::RBrace),
            '[' => Some(Tok::LBracket), ']' => Some(Tok::RBracket),
            ':' => Some(Tok::Colon),
            _ => None,
        };
        if let Some(t) = single {
            i += 1; col += 1; push!(t); continue;
        }
        let eqlike = match c {
            '=' => if i + 1 < n && b[i + 1] == '=' { Some(Tok::Eq) } else { Some(Tok::Assign) },
            '~' => if i + 1 < n && b[i + 1] == '=' { Some(Tok::Ne) } else { None },
            '<' => if i + 1 < n && b[i + 1] == '=' { Some(Tok::Le) } else { Some(Tok::Lt) },
            '>' => if i + 1 < n && b[i + 1] == '=' { Some(Tok::Ge) } else { Some(Tok::Gt) },
            _ => None,
        };
        if let Some(t) = eqlike {
            let len = if matches!(t, Tok::Assign | Tok::Lt | Tok::Gt) { 1 } else { 2 };
            i += len; col += len; push!(t); continue;
        }
        return Err(LexError { msg: format!("unexpected character {:?}", c), line, col });
    }
    out.push(Token { tok: Tok::Eof, line, col });
    Ok((out, comments))
}
