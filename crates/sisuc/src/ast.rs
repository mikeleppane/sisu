//! The syntax tree, and its S-expression printer used by `--emit ast` and the parser tests.

use std::fmt;

use crate::diagnostic::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Program {
    pub(crate) items: Vec<Item>,
    /// The named items that hold a syntax error, which the checker treats as declared.
    pub(crate) broken: Vec<Broken>,
    /// Where each comment starts, so that a fix does not delete one as if it were certain.
    pub(crate) comments: Vec<usize>,
}

/// A function, class or constant that holds a syntax error, so only its name survives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Broken {
    pub(crate) name: Ident,
    pub(crate) kind: BrokenKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrokenKind {
    Function,
    Class,
    Constant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Item {
    Function(Function),
    Class(Class),
    Const(Const),
}

/// `const name: ty = value`, at the top level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Const {
    pub(crate) name: Ident,
    pub(crate) ty: Option<TypeExpr>,
    pub(crate) value: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Class {
    pub(crate) name: Ident,
    pub(crate) members: Vec<Member>,
    /// The names of the methods that hold a syntax error; the class itself is intact.
    pub(crate) broken_methods: Vec<Ident>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Member {
    Field(FieldDecl),
    Method(Function),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FieldDecl {
    pub(crate) mutable: bool,
    pub(crate) name: Ident,
    pub(crate) ty: TypeExpr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Function {
    pub(crate) name: Ident,
    /// The `self` token of a method's first parameter.
    pub(crate) self_param: Option<Span>,
    pub(crate) params: Vec<Param>,
    pub(crate) ret: Option<TypeExpr>,
    pub(crate) body: Block,
    /// `fn` through `}`.
    pub(crate) span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ident {
    pub(crate) name: String,
    pub(crate) span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Param {
    pub(crate) name: Ident,
    pub(crate) ty: TypeExpr,
}

/// A type as written, and whether a `?` follows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TypeExpr {
    pub(crate) kind: TypeKind,
    pub(crate) optional: bool,
    /// The name, or `[` through `]`, then through the `?`, if any.
    pub(crate) span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TypeKind {
    Name(String),
    /// `[T]`; the element type may itself be optional or an array.
    Array(Box<TypeExpr>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Block {
    pub(crate) stmts: Vec<Stmt>,
    /// `{` through `}`.
    pub(crate) span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stmt {
    pub(crate) kind: StmtKind,
    pub(crate) span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StmtKind {
    Let {
        /// The `mut` keyword of a `let mut`.
        mutable: Option<Span>,
        name: Ident,
        ty: Option<TypeExpr>,
        init: Expr,
    },
    While {
        cond: Expr,
        body: Block,
    },
    /// `while let name = value { body }`.
    WhileLet {
        name: Ident,
        value: Expr,
        body: Block,
    },
    /// `for name in source { body }`.
    For {
        name: Ident,
        source: ForSource,
        body: Block,
    },
    Return(Option<Expr>),
    Break,
    Continue,
    /// `target` is a `Name` or a `Field`.
    Assign {
        target: Expr,
        value: Expr,
    },
    /// `target op= value`; the checker lowers a name to `target = target op value`, and a
    /// field `a.f` to `Block { let t = a; Assign { t.f, t.f op value } }`.
    CompoundAssign {
        op: BinaryOp,
        target: Expr,
        value: Expr,
    },
    Expr(Expr),
}

/// What a `for` walks; a range belongs to the `for` and is not an expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ForSource {
    Array(Expr),
    /// `start..end`.
    Range {
        start: Expr,
        end: Expr,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Expr {
    pub(crate) kind: ExprKind,
    pub(crate) span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExprKind {
    /// An integer literal: its value and its `text` as written, which a diagnostic quotes.
    Int {
        value: i64,
        text: String,
    },
    Bool(bool),
    /// A `char` literal, decoded.
    Char(char),
    Name(String),
    Call {
        callee: Ident,
        args: Vec<Arg>,
    },
    SelfValue,
    NoneLit,
    /// `safe` is `base?.name`.
    Field {
        base: Box<Expr>,
        name: Ident,
        safe: bool,
    },
    /// `safe` is `receiver?.method(args)`.
    MethodCall {
        receiver: Box<Expr>,
        method: Ident,
        args: Vec<Arg>,
        safe: bool,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    /// A chain `a < b <= c`; `operands.len() == ops.len() + 1`.
    Compare {
        operands: Vec<Expr>,
        ops: Vec<CompareOp>,
    },
    /// `lhs ?? rhs`.
    Coalesce {
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    /// `[a, b]`.
    ArrayLit(Vec<Expr>),
    /// `[value; len]`.
    ArrayFill {
        value: Box<Expr>,
        len: Box<Expr>,
    },
    /// `base[index]`.
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    /// `lhs is rhs`.
    Is {
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    /// `else if` is an `else_block` holding one `Expr` statement with the inner `if`.
    If {
        cond: Box<Expr>,
        then_block: Block,
        else_block: Option<Block>,
    },
    /// `if let name = value { then } else { else }`; `else if` and `else if let` fold into
    /// `else_block` as for `If`.
    IfLet {
        name: Ident,
        value: Box<Expr>,
        then_block: Block,
        else_block: Option<Block>,
    },
    /// `{ ... }` where an expression belongs: its value is its last expression.
    Block(Block),
}

/// A call argument; the checker decides where a label belongs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Arg {
    pub(crate) label: Option<Ident>,
    pub(crate) value: Expr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    And,
    Or,
    Eq,
    Ne,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompareOp {
    Lt,
    Le,
    Gt,
    Ge,
}

impl CompareOp {
    /// `<` and `<=` run one way, `>` and `>=` the other.
    pub(crate) fn is_less(self) -> bool {
        matches!(self, CompareOp::Lt | CompareOp::Le)
    }
}

impl fmt::Display for UnaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            UnaryOp::Neg => "-",
            UnaryOp::Not => "!",
        })
    }
}

impl fmt::Display for BinaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Rem => "%",
            BinaryOp::And => "&&",
            BinaryOp::Or => "||",
            BinaryOp::Eq => "==",
            BinaryOp::Ne => "!=",
        })
    }
}

impl fmt::Display for CompareOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CompareOp::Lt => "<",
            CompareOp::Le => "<=",
            CompareOp::Gt => ">",
            CompareOp::Ge => ">=",
        })
    }
}

impl fmt::Display for TypeExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            TypeKind::Name(name) => f.write_str(name)?,
            TypeKind::Array(element) => write!(f, "[{element}]")?,
        }
        if self.optional {
            f.write_str("?")?;
        }
        Ok(())
    }
}

/// Writes each item after a space.
fn spaced<T: fmt::Display>(f: &mut fmt::Formatter<'_>, items: &[T]) -> fmt::Result {
    items.iter().try_for_each(|item| write!(f, " {item}"))
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, item) in self.items.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            match item {
                Item::Function(function) => write!(f, "{function}")?,
                Item::Class(class) => write!(f, "{class}")?,
                Item::Const(c) => {
                    write!(f, "(const {}", c.name.name)?;
                    if let Some(ty) = &c.ty {
                        write!(f, " {ty}")?;
                    }
                    write!(f, " {})", c.value)?;
                }
            }
        }
        Ok(())
    }
}

impl fmt::Display for Class {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(class {}", self.name.name)?;
        for member in &self.members {
            match member {
                Member::Field(field) => {
                    let keyword = if field.mutable { "mut " } else { "" };
                    write!(f, " ({keyword}{} {})", field.name.name, field.ty)?;
                }
                Member::Method(method) => write!(f, " {method}")?,
            }
        }
        f.write_str(")")
    }
}

impl fmt::Display for Arg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.label {
            Some(label) => write!(f, "(: {} {})", label.name, self.value),
            None => write!(f, "{}", self.value),
        }
    }
}

impl fmt::Display for Function {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(fn {} (", self.name.name)?;
        let params = self
            .params
            .iter()
            .map(|p| format!("({} {})", p.name.name, p.ty));
        let all: Vec<String> = self
            .self_param
            .map(|_| "self".to_string())
            .into_iter()
            .chain(params)
            .collect();
        f.write_str(&all.join(" "))?;
        let ret = self
            .ret
            .as_ref()
            .map_or("unit".to_string(), ToString::to_string);
        write!(f, ") {ret} {})", self.body)
    }
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(block")?;
        spaced(f, &self.stmts)?;
        f.write_str(")")
    }
}

impl fmt::Display for Stmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            StmtKind::Let {
                mutable,
                name,
                ty,
                init,
            } => {
                let keyword = if mutable.is_some() { "let mut" } else { "let" };
                write!(f, "({keyword} {}", name.name)?;
                if let Some(ty) = ty {
                    write!(f, " {ty}")?;
                }
                write!(f, " {init})")
            }
            StmtKind::While { cond, body } => write!(f, "(while {cond} {body})"),
            StmtKind::WhileLet { name, value, body } => {
                write!(f, "(while-let {} {value} {body})", name.name)
            }
            StmtKind::For { name, source, body } => match source {
                ForSource::Array(e) => write!(f, "(for {} {e} {body})", name.name),
                ForSource::Range { start, end } => {
                    write!(f, "(for {} (.. {start} {end}) {body})", name.name)
                }
            },
            StmtKind::Return(None) => f.write_str("(return)"),
            StmtKind::Return(Some(e)) => write!(f, "(return {e})"),
            StmtKind::Break => f.write_str("(break)"),
            StmtKind::Continue => f.write_str("(continue)"),
            StmtKind::Assign { target, value } => write!(f, "(= {target} {value})"),
            StmtKind::CompoundAssign { op, target, value } => {
                write!(f, "({op}= {target} {value})")
            }
            StmtKind::Expr(e) => write!(f, "{e}"),
        }
    }
}

/// The access operator as the tree prints it.
pub(crate) fn dot(safe: bool) -> &'static str {
    if safe { "?." } else { "." }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ExprKind::Int { value, .. } => write!(f, "{value}"),
            ExprKind::Bool(b) => write!(f, "{b}"),
            ExprKind::Char(c) => write!(f, "{c:?}"),
            ExprKind::Name(name) => f.write_str(name),
            ExprKind::Call { callee, args } => {
                write!(f, "(call {}", callee.name)?;
                spaced(f, args)?;
                f.write_str(")")
            }
            ExprKind::SelfValue => f.write_str("self"),
            ExprKind::NoneLit => f.write_str("None"),
            ExprKind::Field { base, name, safe } => {
                write!(f, "({} {base} {})", dot(*safe), name.name)
            }
            ExprKind::MethodCall {
                receiver,
                method,
                args,
                safe,
            } => {
                write!(f, "(call ({} {receiver} {})", dot(*safe), method.name)?;
                spaced(f, args)?;
                f.write_str(")")
            }
            ExprKind::Unary { op, operand } => write!(f, "({op} {operand})"),
            ExprKind::Binary { op, lhs, rhs } => write!(f, "({op} {lhs} {rhs})"),
            ExprKind::Coalesce { lhs, rhs } => write!(f, "(?? {lhs} {rhs})"),
            ExprKind::Is { lhs, rhs } => write!(f, "(is {lhs} {rhs})"),
            ExprKind::Compare { operands, ops } => {
                // `(< a b <= c)`: the first operator leads, the rest sit between operands.
                write!(f, "({} {} {}", ops[0], operands[0], operands[1])?;
                for (op, operand) in ops[1..].iter().zip(&operands[2..]) {
                    write!(f, " {op} {operand}")?;
                }
                f.write_str(")")
            }
            ExprKind::ArrayLit(elements) => {
                f.write_str("(array")?;
                spaced(f, elements)?;
                f.write_str(")")
            }
            ExprKind::ArrayFill { value, len } => write!(f, "(array-fill {value} {len})"),
            ExprKind::Index { base, index } => write!(f, "(index {base} {index})"),
            ExprKind::If {
                cond,
                then_block,
                else_block,
            } => {
                write!(f, "(if {cond} {then_block}")?;
                if let Some(else_block) = else_block {
                    write!(f, " {else_block}")?;
                }
                f.write_str(")")
            }
            ExprKind::IfLet {
                name,
                value,
                then_block,
                else_block,
            } => {
                write!(f, "(if-let {} {value} {then_block}", name.name)?;
                if let Some(else_block) = else_block {
                    write!(f, " {else_block}")?;
                }
                f.write_str(")")
            }
            ExprKind::Block(block) => write!(f, "{block}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp() -> Span {
        Span::new(0, 0)
    }

    fn ident(name: &str) -> Ident {
        Ident {
            name: name.to_string(),
            span: sp(),
        }
    }

    fn ty(name: &str) -> TypeExpr {
        TypeExpr {
            kind: TypeKind::Name(name.to_string()),
            optional: false,
            span: sp(),
        }
    }

    fn expr(kind: ExprKind) -> Expr {
        Expr { kind, span: sp() }
    }

    fn name(n: &str) -> Expr {
        expr(ExprKind::Name(n.to_string()))
    }

    fn int(n: i64) -> Expr {
        expr(ExprKind::Int {
            value: n,
            text: n.to_string(),
        })
    }

    fn stmt(kind: StmtKind) -> Stmt {
        Stmt { kind, span: sp() }
    }

    fn block(stmts: Vec<Stmt>) -> Block {
        Block { stmts, span: sp() }
    }

    fn func(name: &str, params: Vec<Param>, ret: Option<TypeExpr>, body: Block) -> Function {
        Function {
            name: ident(name),
            self_param: None,
            params,
            ret,
            body,
            span: sp(),
        }
    }

    #[test]
    fn display_formats() {
        let main = func(
            "main",
            vec![],
            None,
            block(vec![
                stmt(StmtKind::Let {
                    mutable: None,
                    name: ident("x"),
                    ty: None,
                    init: int(1),
                }),
                stmt(StmtKind::Expr(expr(ExprKind::Call {
                    callee: ident("print"),
                    args: vec![Arg {
                        label: None,
                        value: name("x"),
                    }],
                }))),
            ]),
        );
        assert_eq!(
            main.to_string(),
            "(fn main () unit (block (let x 1) (call print x)))"
        );
    }

    #[test]
    fn display_every_node() {
        let cmp = expr(ExprKind::Compare {
            operands: vec![name("a"), name("b"), name("c")],
            ops: vec![CompareOp::Lt, CompareOp::Le],
        });
        let is_prime = func(
            "is_prime",
            vec![Param {
                name: ident("n"),
                ty: ty("i64"),
            }],
            Some(ty("bool")),
            block(vec![
                stmt(StmtKind::Let {
                    mutable: Some(sp()),
                    name: ident("d"),
                    ty: Some(ty("i64")),
                    init: expr(ExprKind::Unary {
                        op: UnaryOp::Neg,
                        operand: Box::new(int(2)),
                    }),
                }),
                stmt(StmtKind::Assign {
                    target: name("d"),
                    value: expr(ExprKind::Binary {
                        op: BinaryOp::Add,
                        lhs: Box::new(name("d")),
                        rhs: Box::new(int(1)),
                    }),
                }),
                stmt(StmtKind::While {
                    cond: cmp.clone(),
                    body: block(vec![]),
                }),
                stmt(StmtKind::Expr(expr(ExprKind::If {
                    cond: Box::new(expr(ExprKind::Unary {
                        op: UnaryOp::Not,
                        operand: Box::new(expr(ExprKind::Bool(true))),
                    })),
                    then_block: block(vec![stmt(StmtKind::Return(None))]),
                    else_block: Some(block(vec![stmt(StmtKind::Return(Some(cmp)))])),
                }))),
            ]),
        );
        let second = func("g", vec![], None, block(vec![]));
        let program = Program {
            items: vec![Item::Function(is_prime), Item::Function(second)],
            broken: Vec::new(),
            comments: Vec::new(),
        };
        assert_eq!(
            program.to_string(),
            "(fn is_prime ((n i64)) bool (block (let mut d i64 (- 2)) (= d (+ d 1)) \
             (while (< a b <= c) (block)) \
             (if (! true) (block (return)) (block (return (< a b <= c))))))\n\
             (fn g () unit (block))"
        );
    }
}
