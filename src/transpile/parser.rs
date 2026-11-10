//! Recursive-descent parser for the restricted Luau subset (see ast.rs).

use super::ast::*;
use super::lexer::{lex, Tok, Token};

#[derive(Debug)]
pub struct ParseError {
    pub msg: String,
    pub line: usize,
    pub col: usize,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.col, self.msg)
    }
}

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
}

pub fn parse(src: &str) -> Result<Vec<Stmt>, ParseError> {
    parse_with_comments(src).map(|(stmts, _, _)| stmts)
}

/// [v23] Parse and also hand back the source comments plus the starting
/// line of every top-level statement, so the caller can anchor each
/// comment to the statement that follows it.
pub fn parse_with_comments(
    src: &str,
) -> Result<(Vec<Stmt>, Vec<super::lexer::CapturedComment>, Vec<usize>), ParseError> {
    let (toks, comments) = super::lexer::lex_with_comments(src)
        .map_err(|e| ParseError { msg: e.msg, line: e.line, col: e.col })?;
    let mut p = Parser { toks, pos: 0 };
    let mut out = Vec::new();
    let mut starts: Vec<usize> = Vec::new();
    while !p.at(&Tok::Eof) {
        starts.push(p.cur().line);
        out.push(p.statement()?);
    }
    Ok((out, comments, starts))
}

impl Parser {
    fn cur(&self) -> &Token {
        &self.toks[self.pos.min(self.toks.len() - 1)]
    }
    fn at(&self, t: &Tok) -> bool {
        std::mem::discriminant(&self.cur().tok) == std::mem::discriminant(t)
    }
    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos.min(self.toks.len() - 1)].clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    fn eat(&mut self, t: &Tok) -> Result<Token, ParseError> {
        if self.at(t) {
            Ok(self.bump())
        } else {
            Err(self.err(format!(
                "expected {}, found {}",
                t.describe(),
                self.cur().tok.describe()
            )))
        }
    }
    fn err(&self, msg: String) -> ParseError {
        ParseError { msg, line: self.cur().line, col: self.cur().col }
    }

    fn block_end(&self) -> bool {
        matches!(self.cur().tok, Tok::Eof | Tok::End | Tok::Else | Tok::Elseif | Tok::Until)
    }

    fn statement(&mut self) -> Result<Stmt, ParseError> {
        // skip stray semicolons
        while self.at(&Tok::Semi) {
            self.bump();
        }
        match self.cur().tok.clone() {
            Tok::Local => {
                self.bump();
                if self.at(&Tok::Function) {
                    self.bump();
                    let name = match self.bump().tok {
                        Tok::Name(n) => n,
                        other => return Err(self.err(format!("bad function name: found {}", other.describe()))),
                    };
                    let params = self.params()?;
                    let body = self.body()?;
                    self.eat(&Tok::End)?;
                    return Ok(Stmt::LocalFunction(name, params, body));
                }
                let mut names = Vec::new();
                loop {
                    match self.bump().tok {
                        Tok::Name(n) => names.push(n),
                        other => return Err(self.err(format!("bad local name: found {}", other.describe()))),
                    }
                    if self.at(&Tok::Comma) {
                        self.bump();
                    } else {
                        break;
                    }
                }
                let mut exprs = Vec::new();
                if self.at(&Tok::Assign) {
                    self.bump();
                    loop {
                        exprs.push(self.expr()?);
                        if self.at(&Tok::Comma) {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                }
                Ok(Stmt::Local(names, exprs))
            }
            Tok::If => {
                self.bump();
                let mut arms = Vec::new();
                let cond = self.expr()?;
                self.eat(&Tok::Then)?;
                let mut body = self.body()?;
                // hoist a trailing `break` out of nested else{...break} is unnecessary;
                // Luau allows break as last statement of any block.
                arms.push((cond, std::mem::take(&mut body)));
                let mut els = None;
                loop {
                    if self.at(&Tok::Elseif) {
                        self.bump();
                        let c = self.expr()?;
                        self.eat(&Tok::Then)?;
                        let b = self.body()?;
                        arms.push((c, b));
                    } else if self.at(&Tok::Else) {
                        self.bump();
                        els = Some(self.body()?);
                        self.eat(&Tok::End)?;
                        break;
                    } else {
                        self.eat(&Tok::End)?;
                        break;
                    }
                }
                Ok(Stmt::If(arms, els))
            }
            Tok::While => {
                self.bump();
                let cond = self.expr()?;
                self.eat(&Tok::Do)?;
                let body = self.body()?;
                self.eat(&Tok::End)?;
                Ok(Stmt::While(cond, body))
            }
            Tok::Repeat => {
                self.bump();
                let mut body = self.body()?;
                self.eat(&Tok::Until)?;
                let cond = self.expr()?;
                // repeat B until C  ==  while true do B; if C then break end end
                // (block_end() already stops the body scan at Until)
                body.push(Stmt::If(vec![(cond, vec![Stmt::Break])], None));
                Ok(Stmt::While(Expr::Bool(true), body))
            }
            Tok::For => {
                self.bump();
                let first = match self.bump().tok {
                    Tok::Name(n) => n,
                    other => return Err(self.err(format!("bad for variable: found {}", other.describe()))),
                };
                if self.at(&Tok::Assign) {
                    self.bump();
                    let from = self.expr()?;
                    self.eat(&Tok::Comma)?;
                    let to = self.expr()?;
                    let step = if self.at(&Tok::Comma) {
                        self.bump();
                        Some(self.expr()?)
                    } else {
                        None
                    };
                    self.eat(&Tok::Do)?;
                    let body = self.body()?;
                    self.eat(&Tok::End)?;
                    Ok(Stmt::NumericFor(first, from, to, step, body))
                } else {
                    let mut names = vec![first];
                    while self.at(&Tok::Comma) {
                        self.bump();
                        match self.bump().tok {
                            Tok::Name(n) => names.push(n),
                            other => return Err(self.err(format!("bad for variable: found {}", other.describe()))),
                        }
                    }
                    self.eat(&Tok::In)?;
                    let mut exprs = Vec::new();
                    loop {
                        exprs.push(self.expr()?);
                        if self.at(&Tok::Comma) {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    self.eat(&Tok::Do)?;
                    let body = self.body()?;
                    self.eat(&Tok::End)?;
                    Ok(Stmt::GenericFor(names, exprs, body))
                }
            }
            Tok::Return => {
                self.bump();
                let mut exprs = Vec::new();
                if !self.block_end() && !self.at(&Tok::Semi) {
                    loop {
                        exprs.push(self.expr()?);
                        if self.at(&Tok::Comma) {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                }
                Ok(Stmt::Return(exprs))
            }
            Tok::Break => {
                self.bump();
                Ok(Stmt::Break)
            }
            Tok::Continue => {
                self.bump();
                Ok(Stmt::Continue)
            }
            Tok::Function => {
                // global function f(...) end / f.g = function -> treat as
                // LocalFunction on the first name (corpus only uses local functions)
                self.bump();
                let mut name = match self.bump().tok {
                    Tok::Name(n) => n,
                    other => return Err(self.err(format!("bad function name: found {}", other.describe()))),
                };
                while self.at(&Tok::Dot) {
                    self.bump();
                    if let Tok::Name(n) = self.bump().tok {
                        name = n; // take the LAST segment (OnInvoke-style assignment)
                    }
                }
                // method definition sugar: `function obj:m(a) end` - the
                // colon comes BEFORE the params; take the method name and
                // prepend the implicit `self` parameter
                let mut params = if self.at(&Tok::Colon) {
                    self.bump();
                    match self.bump().tok {
                        Tok::Name(n) => {
                            name = n;
                            let ps = self.params()?;
                            let mut ps2 = Vec::with_capacity(ps.len() + 1);
                            ps2.push("self".to_string());
                            ps2.extend(ps);
                            ps2
                        }
                        other => return Err(self.err(format!("bad method name: found {}", other.describe()))),
                    }
                } else {
                    self.params()?
                };
                let body = self.body()?;
                self.eat(&Tok::End)?;
                Ok(Stmt::LocalFunction(name, params, body))
            }
            _ => {
                // assignment or expression statement
                let mut targets = vec![self.suffixed_expr()?];
                while self.at(&Tok::Comma) {
                    self.bump();
                    targets.push(self.suffixed_expr()?);
                }
                if self.at(&Tok::Assign) {
                    self.bump();
                    let mut values = vec![self.expr()?];
                    while self.at(&Tok::Comma) {
                        self.bump();
                        values.push(self.expr()?);
                    }
                    if targets.len() == 1 && values.len() == 1 {
                        Ok(Stmt::Assign(targets.pop().unwrap(), values.pop().unwrap()))
                    } else {
                        Ok(Stmt::MultiAssign(targets, values))
                    }
                } else if targets.len() == 1 {
                    Ok(Stmt::ExprStmt(targets.pop().unwrap()))
                } else {
                    Err(self.err("expected = after target list".into()))
                }
            }
        }
    }

    fn params(&mut self) -> Result<Vec<String>, ParseError> {
        self.eat(&Tok::LParen)?;
        let mut out = Vec::new();
        if !self.at(&Tok::RParen) {
            loop {
                match self.bump().tok {
                    Tok::Name(nm) => out.push(nm),
                    other => return Err(self.err(format!("bad parameter: found {}", other.describe()))),
                }
                if self.at(&Tok::Comma) {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        self.eat(&Tok::RParen)?;
        Ok(out)
    }

    fn body(&mut self) -> Result<Vec<Stmt>, ParseError> {
        let mut out = Vec::new();
        while !self.block_end() {
            if self.at(&Tok::Eof) {
                return Err(self.err("unexpected end of file inside block".into()));
            }
            out.push(self.statement()?);
        }
        Ok(out)
    }

    // ----- expressions -----

    pub fn expr(&mut self) -> Result<Expr, ParseError> {
        self.expr_or()
    }

    fn expr_or(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.expr_and()?;
        while self.at(&Tok::Or) {
            self.bump();
            let rhs = self.expr_and()?;
            lhs = Expr::BinOp(BinOp::Or, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }
    fn expr_and(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.expr_cmp()?;
        while self.at(&Tok::And) {
            self.bump();
            let rhs = self.expr_cmp()?;
            lhs = Expr::BinOp(BinOp::And, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }
    fn expr_cmp(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.expr_concat()?;
        loop {
            let op = match self.cur().tok {
                Tok::Eq => BinOp::Eq,
                Tok::Ne => BinOp::Ne,
                Tok::Lt => BinOp::Lt,
                Tok::Le => BinOp::Le,
                Tok::Gt => BinOp::Gt,
                Tok::Ge => BinOp::Ge,
                _ => break,
            };
            self.bump();
            let rhs = self.expr_concat()?;
            lhs = Expr::BinOp(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }
    fn expr_concat(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.expr_add()?;
        while self.at(&Tok::DotDot) {
            self.bump();
            let rhs = self.expr_add()?;
            lhs = Expr::BinOp(BinOp::Concat, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }
    fn expr_add(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.expr_mul()?;
        loop {
            let op = match self.cur().tok {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => break,
            };
            self.bump();
            let rhs = self.expr_mul()?;
            lhs = Expr::BinOp(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }
    fn expr_mul(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.expr_unary()?;
        loop {
            let op = match self.cur().tok {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                Tok::Percent => BinOp::Mod,
                _ => break,
            };
            self.bump();
            let rhs = self.expr_unary()?;
            lhs = Expr::BinOp(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }
    fn expr_unary(&mut self) -> Result<Expr, ParseError> {
        let op = match self.cur().tok {
            Tok::Not => Some(UnOp::Not),
            Tok::Minus => Some(UnOp::Neg),
            Tok::Hash => Some(UnOp::Len),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let e = self.expr_unary()?;
            return Ok(Expr::UnOp(op, Box::new(e)));
        }
        self.expr_pow()
    }
    fn expr_pow(&mut self) -> Result<Expr, ParseError> {
        let base = self.suffixed_expr()?;
        if self.at(&Tok::Caret) {
            self.bump();
            let exp = self.expr_unary()?; // right-assoc
            return Ok(Expr::BinOp(BinOp::Pow, Box::new(base), Box::new(exp)));
        }
        Ok(base)
    }

    fn primary_expr(&mut self) -> Result<Expr, ParseError> {
        match self.cur().tok.clone() {
            Tok::Name(n) => {
                self.bump();
                Ok(Expr::Name(n))
            }
            Tok::LParen => {
                self.bump();
                let e = self.expr()?;
                self.eat(&Tok::RParen)?;
                Ok(Expr::Paren(Box::new(e)))
            }
            Tok::Str(s) => {
                self.bump();
                Ok(Expr::Str(s))
            }
            Tok::Number(v) => {
                self.bump();
                Ok(Expr::Number(v))
            }
            Tok::Nil => {
                self.bump();
                Ok(Expr::Nil)
            }
            Tok::True => {
                self.bump();
                Ok(Expr::Bool(true))
            }
            Tok::False => {
                self.bump();
                Ok(Expr::Bool(false))
            }
            Tok::Function => {
                self.bump();
                let params = self.params()?;
                let body = self.body()?;
                self.eat(&Tok::End)?;
                Ok(Expr::Function(params, body))
            }
            Tok::LBrace => {
                self.bump();
                let mut fields = Vec::new();
                while !self.at(&Tok::RBrace) {
                    if let Tok::LBracket = self.cur().tok {
                        // bracketed key: [expr] = value
                        self.bump();
                        let k = self.expr()?;
                        self.eat(&Tok::RBracket)?;
                        self.eat(&Tok::Assign)?;
                        let v = self.expr()?;
                        fields.push(TableField::Bracketed(k, v));
                    } else if let Tok::Name(nm) = self.cur().tok.clone() {
                        // lookahead for `name =`
                        if self.pos + 1 < self.toks.len()
                            && matches!(self.toks[self.pos + 1].tok, Tok::Assign)
                        {
                            self.bump();
                            self.bump();
                            let v = self.expr()?;
                            fields.push(TableField::Named(nm, v));
                        } else {
                            let v = self.expr()?;
                            fields.push(TableField::Pos(v));
                        }
                    } else {
                        let v = self.expr()?;
                        fields.push(TableField::Pos(v));
                    }
                    if self.at(&Tok::Comma) || self.at(&Tok::Semi) {
                        self.bump();
                    } else {
                        break;
                    }
                }
                self.eat(&Tok::RBrace)?;
                Ok(Expr::Table(fields))
            }
            other => Err(self.err(format!("unexpected {}", other.describe()))),
        }
    }

    fn suffixed_expr(&mut self) -> Result<Expr, ParseError> {
        let mut e = self.primary_expr()?;
        loop {
            match self.cur().tok.clone() {
                Tok::Dot => {
                    self.bump();
                    match self.bump().tok {
                        Tok::Name(n) => e = Expr::Index(Box::new(e), Box::new(Expr::Str(n))),
                        other => {
                            return Err(self.err(format!(
                                "expected a field name after '.', found {}",
                                other.describe()
                            )))
                        }
                    }
                }
                Tok::LBracket => {
                    self.bump();
                    let idx = self.expr()?;
                    self.eat(&Tok::RBracket)?;
                    e = Expr::Index(Box::new(e), Box::new(idx));
                }
                Tok::Colon => {
                    self.bump();
                    let m = match self.bump().tok {
                        Tok::Name(n) => n,
                        other => return Err(self.err(format!("bad method call: found {}", other.describe()))),
                    };
                    let args = self.args()?;
                    e = Expr::Method(Box::new(e), m, args);
                }
                Tok::LParen => {
                    let args = self.args()?;
                    e = Expr::Call(Box::new(e), args);
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn args(&mut self) -> Result<Vec<Expr>, ParseError> {
        if let Tok::Str(s) = self.cur().tok.clone() {
            self.bump();
            return Ok(vec![Expr::Str(s)]);
        }
        self.eat(&Tok::LParen)?;
        let mut out = Vec::new();
        if !self.at(&Tok::RParen) {
            loop {
                out.push(self.expr()?);
                if self.at(&Tok::Comma) {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        self.eat(&Tok::RParen)?;
        Ok(out)
    }
}