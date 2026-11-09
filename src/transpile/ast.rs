//! Restricted Luau AST for the transpiler.

#[derive(Debug, Clone)]
pub enum Expr {
    Nil,
    Bool(bool),
    Number(f64),
    Str(String),
    Vararg,
    Name(String),
    Index(Box<Expr>, Box<Expr>),      // a.b -> Index(Var a, Str b); a[k] -> Index(Var a, k)
    Call(Box<Expr>, Vec<Expr>),       // f(args)
    Method(Box<Expr>, String, Vec<Expr>), // obj:method(args)
    Function(Vec<String>, Vec<Stmt>), // function(params) body end (params without names = Vararg)
    BinOp(BinOp, Box<Expr>, Box<Expr>),
    UnOp(UnOp, Box<Expr>),
    Table(Vec<TableField>),
    Paren(Box<Expr>),
}

#[derive(Debug, Clone)]
pub enum TableField {
    Pos(Expr),               // { v }
    Named(String, Expr),     // { k = v }
    Bracketed(Expr, Expr),   // { [k] = v }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add, Sub, Mul, Div, Mod, Pow,
    Concat,
    Eq, Ne, Lt, Le, Gt, Ge,
    And, Or,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnOp {
    Neg,
    Not,
    Len,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Local(Vec<String>, Vec<Expr>),          // local a, b = e1, e2 (exprs 0 or len==names)
    LocalFunction(String, Vec<String>, Vec<Stmt>),
    Assign(Expr, Expr),                     // single target = value
    MultiAssign(Vec<Expr>, Vec<Expr>),      // t1, t2, ... = v1, v2, ...
    If(Vec<(Expr, Vec<Stmt>)>, Option<Vec<Stmt>>), // elseif chain
    While(Expr, Vec<Stmt>),
    NumericFor(String, Expr, Expr, Option<Expr>, Vec<Stmt>), // var, from, to, step
    GenericFor(Vec<String>, Vec<Expr>, Vec<Stmt>),           // names, exprs (ipairs call), body
    Return(Vec<Expr>),
    Break,
    Continue,
    ExprStmt(Expr),
}