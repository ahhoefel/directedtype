use logos::Logos;
use std::fmt;

#[derive(Logos, Debug, Clone, PartialEq)]
#[logos(skip r"[ \t\r\n]+")]
#[logos(skip r"//[^\n]*")]
#[logos(skip r"/\*([^*]|\*[^/])*\*/")]
pub enum Token {
    // Backslash for commands / nodes: \Component, \Rect, etc.
    #[token("\\")]
    Backslash,

    // Keywords
    #[token("Component")]
    Component,

    #[token("Span")]
    Span,

    #[token("Enum")]
    Enum,

    #[token("Children")]
    Children,

    #[token("let")]
    Let,

    #[token("env")]
    Env,

    #[token("alias")]
    Alias,

    #[token("state")]
    State,

    #[token("use")]
    Use,

    #[token("assert")]
    Assert,

    #[token("as")]
    As,

    #[token("true")]
    True,

    #[token("false")]
    False,

    // Delimiters
    #[token("(")]
    LParen,

    #[token(")")]
    RParen,

    #[token("{")]
    LBrace,

    #[token("}")]
    RBrace,

    // Punctuation
    #[token(":")]
    Colon,

    #[token(",")]
    Comma,

    #[token(".")]
    Dot,

    #[token(";")]
    Semicolon,

    #[token("=")]
    Eq,

    #[token("?")]
    Question,

    // Arithmetic & Bitwise / Logic operators
    #[token("+")]
    Plus,

    #[token("-")]
    Minus,

    #[token("*")]
    Star,

    #[token("/")]
    Slash,

    #[token("%")]
    Percent,

    #[token("==")]
    EqEq,

    #[token("!=")]
    BangEq,

    #[token("<=")]
    LtEq,

    #[token(">=")]
    GtEq,

    #[token("<")]
    Lt,

    #[token(">")]
    Gt,

    #[token("&&")]
    AmpAmp,

    #[token("||")]
    PipePipe,

    #[token("!")]
    Bang,

    // Literals
    #[regex(r"[0-9]+", priority = 3, callback = |lex| lex.slice().parse::<i64>().ok())]
    Integer(i64),

    #[regex(r"[0-9]+\.[0-9]+([eE][+-]?[0-9]+)?|[0-9]+[eE][+-]?[0-9]+", priority = 3, callback = |lex| lex.slice().parse::<f64>().ok())]
    Float(f64),

    #[regex(r"[0-9]+(\.[0-9]+)?([eE][+-]?[0-9]+)?[a-zA-Z_][a-zA-Z0-9_]*", priority = 2, callback = parse_dimension_token)]
    Dimension((f64, String)),

    #[regex(r#""([^"\\]|\\.)*""#, |lex| {
        let s = lex.slice();
        // Strip enclosing quotes and unescape basic escapes
        let inner = &s[1..s.len() - 1];
        let mut unescaped = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => unescaped.push('\n'),
                    Some('t') => unescaped.push('\t'),
                    Some('r') => unescaped.push('\r'),
                    Some('\\') => unescaped.push('\\'),
                    Some('"') => unescaped.push('"'),
                    Some(other) => unescaped.push(other),
                    None => unescaped.push('\\'),
                }
            } else {
                unescaped.push(c);
            }
        }
        unescaped
    })]
    String(String),

    #[regex(r"#[0-9a-fA-F]{3,8}", |lex| lex.slice().to_string())]
    Color(String),

    // Identifiers
    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*", |lex| lex.slice().to_string())]
    Ident(String),
}

fn parse_dimension_token(lex: &mut logos::Lexer<'_, Token>) -> Option<(f64, String)> {
    let s = lex.slice();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        let mut exp_i = i + 1;
        if exp_i < bytes.len() && (bytes[exp_i] == b'+' || bytes[exp_i] == b'-') {
            exp_i += 1;
        }
        if exp_i < bytes.len() && bytes[exp_i].is_ascii_digit() {
            while exp_i < bytes.len() && bytes[exp_i].is_ascii_digit() {
                exp_i += 1;
            }
            i = exp_i;
        }
    }
    let num_str = &s[..i];
    let unit_str = &s[i..];
    let val = num_str.parse::<f64>().ok()?;
    Some((val, unit_str.to_string()))
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Backslash => write!(f, "\\"),
            Token::Component => write!(f, "Component"),
            Token::Span => write!(f, "Span"),
            Token::Enum => write!(f, "Enum"),
            Token::Children => write!(f, "Children"),
            Token::Let => write!(f, "let"),
            Token::Env => write!(f, "env"),
            Token::Alias => write!(f, "alias"),
            Token::State => write!(f, "state"),
            Token::Use => write!(f, "use"),
            Token::Assert => write!(f, "assert"),
            Token::As => write!(f, "as"),
            Token::True => write!(f, "true"),
            Token::False => write!(f, "false"),
            Token::LParen => write!(f, "("),
            Token::RParen => write!(f, ")"),
            Token::LBrace => write!(f, "{{"),
            Token::RBrace => write!(f, "}}"),
            Token::Colon => write!(f, ":"),
            Token::Comma => write!(f, ","),
            Token::Dot => write!(f, "."),
            Token::Semicolon => write!(f, ";"),
            Token::Eq => write!(f, "="),
            Token::Question => write!(f, "?"),
            Token::Plus => write!(f, "+"),
            Token::Minus => write!(f, "-"),
            Token::Star => write!(f, "*"),
            Token::Slash => write!(f, "/"),
            Token::Percent => write!(f, "%"),
            Token::EqEq => write!(f, "=="),
            Token::BangEq => write!(f, "!="),
            Token::LtEq => write!(f, "<="),
            Token::GtEq => write!(f, ">="),
            Token::Lt => write!(f, "<"),
            Token::Gt => write!(f, ">"),
            Token::AmpAmp => write!(f, "&&"),
            Token::PipePipe => write!(f, "||"),
            Token::Bang => write!(f, "!"),
            Token::Integer(n) => write!(f, "{}", n),
            Token::Float(n) => write!(f, "{}", n),
            Token::Dimension((n, u)) => write!(f, "{}{}", n, u),
            Token::String(s) => write!(f, "\"{}\"", s),
            Token::Color(c) => write!(f, "{}", c),
            Token::Ident(id) => write!(f, "{}", id),
        }
    }
}
