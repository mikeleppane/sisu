//! The typed tree: what the checker produces and codegen reads. Its S-expression printer backs
//! `--emit tir` and the checker tests.

use std::fmt;

use crate::diagnostic::Span;

pub(crate) use crate::ast::{CompareOp, UnaryOp};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct LocalId(pub(crate) usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FuncId(pub(crate) usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ClassId(pub(crate) usize);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Type {
    I64,
    Bool,
    Unit,
    Never,
    Class(ClassId),
}

/// `==` and `!=` are `ExprKind::Equal`, comparisons are `ExprKind::Compare`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    And,
    Or,
}

#[derive(Debug)]
pub(crate) struct Program {
    pub(crate) classes: Vec<Class>,
    pub(crate) functions: Vec<Function>,
}

#[derive(Debug)]
pub(crate) struct Class {
    pub(crate) name: String,
    pub(crate) fields: Vec<Field>,
}

#[derive(Debug)]
pub(crate) struct Field {
    pub(crate) name: String,
    pub(crate) ty: Type,
    pub(crate) mutable: bool,
}

#[derive(Debug)]
pub(crate) struct Function {
    pub(crate) name: String,
    pub(crate) params: Vec<LocalId>,
    pub(crate) ret: Type,
    pub(crate) locals: Vec<Local>,
    pub(crate) body: Block,
}

#[derive(Debug)]
pub(crate) struct Local {
    /// `None` for a local the checker made up.
    pub(crate) name: Option<String>,
    pub(crate) ty: Type,
    pub(crate) mutable: bool,
}

/// `value` is the last statement when it was an expression statement in `ast`.
#[derive(Debug)]
pub(crate) struct Block {
    pub(crate) stmts: Vec<Stmt>,
    pub(crate) value: Option<Box<Expr>>,
    pub(crate) ty: Type,
}

#[derive(Debug)]
pub(crate) enum Stmt {
    Let { local: LocalId, init: Expr },
    Assign { place: Place, value: Expr },
    Expr(Expr),
}

#[derive(Debug)]
pub(crate) enum Place {
    Local(LocalId),
    /// Field `index` of the class `base` has.
    Field {
        base: Expr,
        index: usize,
    },
}

#[derive(Debug)]
pub(crate) struct Expr {
    pub(crate) kind: ExprKind,
    pub(crate) ty: Type,
    pub(crate) span: Span,
}

#[derive(Debug)]
pub(crate) enum ExprKind {
    Int(i64),
    Bool(bool),
    Local(LocalId),
    Call {
        func: FuncId,
        args: Vec<Expr>,
    },
    New {
        class: ClassId,
        args: Vec<Expr>,
    },
    /// Field `index` of the class `base` has.
    Field {
        base: Box<Expr>,
        index: usize,
    },
    Print(Box<Expr>),
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Compare {
        operands: Vec<Expr>,
        ops: Vec<CompareOp>,
    },
    Equal {
        negated: bool,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    If {
        cond: Box<Expr>,
        then_block: Block,
        else_block: Option<Block>,
    },
    Loop(Block),
    Break,
    Continue,
    Return(Option<Box<Expr>>),
    Block(Block),
}

impl Type {
    /// Whether a value of this type is an owned reference, which codegen counts.
    pub(crate) fn is_counted(&self) -> bool {
        self.counted_class().is_some()
    }

    /// The class whose count a value of this type holds a reference to, if any.
    pub(crate) fn counted_class(&self) -> Option<ClassId> {
        match self {
            Type::Class(id) => Some(*id),
            _ => None,
        }
    }
}

impl Expr {
    /// Whether a `Return`, `Break` or `Continue` sits anywhere inside, a nested loop included.
    pub(crate) fn exits(&self) -> bool {
        match &self.kind {
            ExprKind::Int(_) | ExprKind::Bool(_) | ExprKind::Local(_) => false,
            ExprKind::Break | ExprKind::Continue | ExprKind::Return(_) => true,
            ExprKind::Call { args, .. } | ExprKind::New { args, .. } => {
                args.iter().any(Expr::exits)
            }
            ExprKind::Compare { operands, .. } => operands.iter().any(Expr::exits),
            ExprKind::Field { base: operand, .. }
            | ExprKind::Print(operand)
            | ExprKind::Unary { operand, .. } => operand.exits(),
            ExprKind::Binary { lhs, rhs, .. } | ExprKind::Equal { lhs, rhs, .. } => {
                lhs.exits() || rhs.exits()
            }
            ExprKind::If {
                cond,
                then_block,
                else_block,
            } => {
                cond.exits() || then_block.exits() || else_block.as_ref().is_some_and(Block::exits)
            }
            ExprKind::Loop(block) | ExprKind::Block(block) => block.exits(),
        }
    }
}

impl Block {
    fn exits(&self) -> bool {
        self.stmts.iter().any(Stmt::exits) || self.value.as_ref().is_some_and(|v| v.exits())
    }
}

impl Stmt {
    fn exits(&self) -> bool {
        match self {
            Stmt::Let { init: e, .. } | Stmt::Expr(e) => e.exits(),
            Stmt::Assign { place, value } => {
                matches!(place, Place::Field { base, .. } if base.exits()) || value.exits()
            }
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Type::I64 => "i64",
            Type::Bool => "bool",
            Type::Unit => "unit",
            Type::Never => "never",
            // A class's name lives in the program: `Program::type_name` prints it.
            Type::Class(id) => return write!(f, "class#{}", id.0),
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
        })
    }
}

/// What printing a node needs: callee names come from the program, local names from the function.
struct Printer<'a> {
    program: &'a Program,
    function: &'a Function,
}

impl Printer<'_> {
    fn local(&self, f: &mut fmt::Formatter<'_>, id: LocalId) -> fmt::Result {
        match &self.function.locals[id.0].name {
            Some(name) => write!(f, "{name}#{}", id.0),
            None => write!(f, "#{}", id.0),
        }
    }

    fn block(&self, f: &mut fmt::Formatter<'_>, block: &Block) -> fmt::Result {
        f.write_str("(block")?;
        for stmt in &block.stmts {
            f.write_str(" ")?;
            self.stmt(f, stmt)?;
        }
        if let Some(value) = &block.value {
            f.write_str(" ")?;
            self.expr(f, value)?;
        }
        f.write_str(")")
    }

    fn stmt(&self, f: &mut fmt::Formatter<'_>, stmt: &Stmt) -> fmt::Result {
        match stmt {
            Stmt::Let { local, init } => {
                let keyword = if self.function.locals[local.0].mutable {
                    "var"
                } else {
                    "let"
                };
                write!(f, "({keyword} ")?;
                self.local(f, *local)?;
                f.write_str(" ")?;
                self.expr(f, init)?;
                f.write_str(")")
            }
            Stmt::Assign { place, value } => {
                f.write_str("(= ")?;
                match place {
                    Place::Local(local) => self.local(f, *local)?,
                    Place::Field { base, index } => self.field(f, base, *index)?,
                }
                f.write_str(" ")?;
                self.expr(f, value)?;
                f.write_str(")")
            }
            Stmt::Expr(expr) => self.expr(f, expr),
        }
    }

    fn expr(&self, f: &mut fmt::Formatter<'_>, expr: &Expr) -> fmt::Result {
        match &expr.kind {
            ExprKind::Int(n) => write!(f, "{n}"),
            ExprKind::Bool(b) => write!(f, "{b}"),
            ExprKind::Local(id) => self.local(f, *id),
            ExprKind::Call { func, args } => {
                write!(f, "(call {}", self.program.functions[func.0].name)?;
                self.each(f, args)?;
                f.write_str(")")
            }
            ExprKind::New { class, args } => {
                write!(f, "(new {}", self.program.classes[class.0].name)?;
                self.each(f, args)?;
                f.write_str(")")
            }
            ExprKind::Field { base, index } => self.field(f, base, *index),
            ExprKind::Print(operand) => self.unary(f, "print", operand),
            ExprKind::Unary { op, operand } => self.unary(f, &op.to_string(), operand),
            ExprKind::Binary { op, lhs, rhs } => self.binary(f, &op.to_string(), lhs, rhs),
            ExprKind::Compare { operands, ops } => {
                // `(< a b <= c)`: the first operator leads, the rest sit between operands.
                write!(f, "({} ", ops[0])?;
                self.expr(f, &operands[0])?;
                f.write_str(" ")?;
                self.expr(f, &operands[1])?;
                for (op, operand) in ops[1..].iter().zip(&operands[2..]) {
                    write!(f, " {op} ")?;
                    self.expr(f, operand)?;
                }
                f.write_str(")")
            }
            ExprKind::Equal { negated, lhs, rhs } => {
                self.binary(f, if *negated { "!=" } else { "==" }, lhs, rhs)
            }
            ExprKind::If {
                cond,
                then_block,
                else_block,
            } => {
                f.write_str("(if ")?;
                self.expr(f, cond)?;
                f.write_str(" ")?;
                self.block(f, then_block)?;
                if let Some(else_block) = else_block {
                    f.write_str(" ")?;
                    self.block(f, else_block)?;
                }
                f.write_str(")")
            }
            ExprKind::Loop(body) => {
                f.write_str("(loop ")?;
                self.block(f, body)?;
                f.write_str(")")
            }
            ExprKind::Break => f.write_str("(break)"),
            ExprKind::Continue => f.write_str("(continue)"),
            ExprKind::Return(None) => f.write_str("(return)"),
            ExprKind::Return(Some(value)) => self.unary(f, "return", value),
            ExprKind::Block(block) => self.block(f, block),
        }
    }

    /// `(. base name)`.
    fn field(&self, f: &mut fmt::Formatter<'_>, base: &Expr, index: usize) -> fmt::Result {
        let Type::Class(class) = base.ty else {
            panic!("checked: a field's base is a class, not `{}`", base.ty)
        };
        f.write_str("(. ")?;
        self.expr(f, base)?;
        write!(f, " {})", self.program.classes[class.0].fields[index].name)
    }

    fn unary(&self, f: &mut fmt::Formatter<'_>, head: &str, operand: &Expr) -> fmt::Result {
        write!(f, "({head} ")?;
        self.expr(f, operand)?;
        f.write_str(")")
    }

    fn binary(
        &self,
        f: &mut fmt::Formatter<'_>,
        head: &str,
        lhs: &Expr,
        rhs: &Expr,
    ) -> fmt::Result {
        write!(f, "({head} ")?;
        self.expr(f, lhs)?;
        f.write_str(" ")?;
        self.expr(f, rhs)?;
        f.write_str(")")
    }

    /// Writes each expression after a space.
    fn each(&self, f: &mut fmt::Formatter<'_>, exprs: &[Expr]) -> fmt::Result {
        exprs.iter().try_for_each(|e| {
            f.write_str(" ")?;
            self.expr(f, e)
        })
    }
}

impl Program {
    /// A type as `--emit tir` prints it: a class by its name.
    fn type_name(&self, ty: &Type) -> String {
        match ty {
            Type::Class(id) => self.classes[id.0].name.clone(),
            ty => ty.to_string(),
        }
    }
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Classes first, then functions, one per line.
        let mut separator = "";
        for class in &self.classes {
            write!(f, "{separator}(class {}", class.name)?;
            for field in &class.fields {
                let keyword = if field.mutable { "var" } else { "let" };
                let ty = self.type_name(&field.ty);
                write!(f, " ({keyword} {} {ty})", field.name)?;
            }
            f.write_str(")")?;
            separator = "\n";
        }
        for function in &self.functions {
            f.write_str(separator)?;
            separator = "\n";
            let printer = Printer {
                program: self,
                function,
            };
            write!(f, "(fn {} (", function.name)?;
            for (j, param) in function.params.iter().enumerate() {
                if j > 0 {
                    f.write_str(" ")?;
                }
                f.write_str("(")?;
                printer.local(f, *param)?;
                write!(f, " {})", self.type_name(&function.locals[param.0].ty))?;
            }
            write!(f, ") {} ", self.type_name(&function.ret))?;
            printer.block(f, &function.body)?;
            f.write_str(")")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expr(kind: ExprKind, ty: Type) -> Expr {
        Expr {
            kind,
            ty,
            span: Span::new(0, 0),
        }
    }

    fn int(n: i64) -> Expr {
        expr(ExprKind::Int(n), Type::I64)
    }

    fn local(id: usize) -> Expr {
        expr(ExprKind::Local(LocalId(id)), Type::I64)
    }

    fn block(stmts: Vec<Stmt>) -> Block {
        Block {
            stmts,
            value: None,
            ty: Type::Unit,
        }
    }

    /// `(if (!= x#1 2) (block (print (< 1 n#0 <= 9))))` and `(loop (block (if true (block (break)) (block))))`.
    fn if_and_loop() -> (Expr, Expr) {
        let x_ne_2 = expr(
            ExprKind::Equal {
                negated: true,
                lhs: Box::new(local(1)),
                rhs: Box::new(int(2)),
            },
            Type::Bool,
        );
        let chain = expr(
            ExprKind::Compare {
                operands: vec![int(1), local(0), int(9)],
                ops: vec![CompareOp::Lt, CompareOp::Le],
            },
            Type::Bool,
        );
        let print = expr(ExprKind::Print(Box::new(chain)), Type::Unit);
        let if_stmt = expr(
            ExprKind::If {
                cond: Box::new(x_ne_2),
                then_block: block(vec![Stmt::Expr(print)]),
                else_block: None,
            },
            Type::Unit,
        );
        let brk = expr(ExprKind::Break, Type::Never);
        let inner_if = expr(
            ExprKind::If {
                cond: Box::new(expr(ExprKind::Bool(true), Type::Bool)),
                then_block: block(vec![Stmt::Expr(brk)]),
                else_block: Some(block(vec![])),
            },
            Type::Unit,
        );
        let looped = expr(
            ExprKind::Loop(block(vec![Stmt::Expr(inner_if)])),
            Type::Unit,
        );
        (if_stmt, looped)
    }

    #[test]
    fn prints_every_node() {
        let locals = vec![
            Local {
                name: Some("n".into()),
                ty: Type::I64,
                mutable: false,
            },
            Local {
                name: Some("x".into()),
                ty: Type::I64,
                mutable: true,
            },
            Local {
                name: None,
                ty: Type::I64,
                mutable: false,
            },
        ];
        let neg3 = expr(
            ExprKind::Unary {
                op: UnaryOp::Neg,
                operand: Box::new(int(3)),
            },
            Type::I64,
        );
        let call = expr(
            ExprKind::Call {
                func: FuncId(0),
                args: vec![neg3],
            },
            Type::I64,
        );
        let x_plus_1 = expr(
            ExprKind::Binary {
                op: BinaryOp::Add,
                lhs: Box::new(local(1)),
                rhs: Box::new(int(1)),
            },
            Type::I64,
        );
        let (if_stmt, looped) = if_and_loop();
        let ret = expr(ExprKind::Return(Some(Box::new(local(1)))), Type::Never);
        let program = Program {
            classes: vec![],
            functions: vec![Function {
                name: "f".into(),
                params: vec![LocalId(0)],
                ret: Type::I64,
                locals,
                body: Block {
                    stmts: vec![
                        Stmt::Let {
                            local: LocalId(1),
                            init: local(0),
                        },
                        Stmt::Assign {
                            place: Place::Local(LocalId(1)),
                            value: x_plus_1,
                        },
                        Stmt::Let {
                            local: LocalId(2),
                            init: call,
                        },
                        Stmt::Expr(if_stmt),
                        Stmt::Expr(looped),
                        Stmt::Expr(ret),
                    ],
                    value: None,
                    ty: Type::Never,
                },
            }],
        };
        assert_eq!(
            program.to_string(),
            "(fn f ((n#0 i64)) i64 (block (var x#1 n#0) (= x#1 (+ x#1 1)) (let #2 (call f (- 3))) (if (!= x#1 2) (block (print (< 1 n#0 <= 9)))) (loop (block (if true (block (break)) (block)))) (return x#1)))"
        );
    }

    fn binary(op: BinaryOp, lhs: Expr, rhs: Expr) -> Expr {
        expr(
            ExprKind::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            },
            Type::Bool,
        )
    }

    fn param(name: &str, ty: Type) -> Local {
        Local {
            name: Some(name.into()),
            ty,
            mutable: false,
        }
    }

    #[test]
    fn prints_functions_joined_by_newline() {
        // fn first() { return }
        // fn second(a: i64, ok: bool) -> bool { first(); ok && (a == 3 && a < 4) }
        let first = Function {
            name: "first".into(),
            params: vec![],
            ret: Type::Unit,
            locals: vec![],
            body: block(vec![Stmt::Expr(expr(ExprKind::Return(None), Type::Never))]),
        };
        let call = expr(
            ExprKind::Call {
                func: FuncId(0),
                args: vec![],
            },
            Type::Unit,
        );
        let a_eq_3 = expr(
            ExprKind::Equal {
                negated: false,
                lhs: Box::new(local(0)),
                rhs: Box::new(int(3)),
            },
            Type::Bool,
        );
        let a_lt_4 = expr(
            ExprKind::Compare {
                operands: vec![local(0), int(4)],
                ops: vec![CompareOp::Lt],
            },
            Type::Bool,
        );
        let ok = expr(ExprKind::Local(LocalId(1)), Type::Bool);
        let value = binary(BinaryOp::And, ok, binary(BinaryOp::And, a_eq_3, a_lt_4));
        let second = Function {
            name: "second".into(),
            params: vec![LocalId(0), LocalId(1)],
            ret: Type::Bool,
            locals: vec![param("a", Type::I64), param("ok", Type::Bool)],
            body: Block {
                stmts: vec![Stmt::Expr(call)],
                value: Some(Box::new(value)),
                ty: Type::Bool,
            },
        };
        let program = Program {
            classes: vec![],
            functions: vec![first, second],
        };
        assert_eq!(
            program.to_string(),
            "(fn first () unit (block (return)))\n\
             (fn second ((a#0 i64) (ok#1 bool)) bool (block (call first) (&& ok#1 (&& (== a#0 3) (< a#0 4)))))"
        );
    }

    #[test]
    fn prints_every_binary_operator() {
        let ops = [
            (BinaryOp::Add, "+"),
            (BinaryOp::Sub, "-"),
            (BinaryOp::Mul, "*"),
            (BinaryOp::Div, "/"),
            (BinaryOp::Rem, "%"),
            (BinaryOp::And, "&&"),
            (BinaryOp::Or, "||"),
        ];
        for (op, symbol) in ops {
            assert_eq!(op.to_string(), symbol);
        }
    }

    fn block_expr(stmts: Vec<Stmt>, value: Option<Expr>) -> Expr {
        let block = Block {
            stmts,
            value: value.map(Box::new),
            ty: Type::I64,
        };
        expr(ExprKind::Block(block), Type::I64)
    }

    #[test]
    fn exits_finds_a_jump_at_any_depth() {
        let (if_stmt, looped) = if_and_loop();
        // A `break` in a loop in a block's statement, and in a block's value.
        assert!(block_expr(vec![Stmt::Expr(looped)], None).exits());
        let (_, looped) = if_and_loop();
        assert!(block_expr(vec![], Some(looped)).exits());
        // A `continue` in a `let` initializer, and a `return` in an argument of a call.
        let cont = expr(ExprKind::Continue, Type::Never);
        let let_cont = Stmt::Let {
            local: LocalId(0),
            init: block_expr(vec![Stmt::Expr(cont)], None),
        };
        assert!(block_expr(vec![let_cont], Some(int(1))).exits());
        let ret = expr(ExprKind::Return(None), Type::Never);
        let call = ExprKind::Call {
            func: FuncId(0),
            args: vec![int(1), ret],
        };
        assert!(expr(call, Type::Unit).exits());
        // An `if`, a comparison chain and a print with no jump in them.
        assert!(!if_stmt.exits());
        assert!(!block_expr(vec![Stmt::Expr(if_stmt)], Some(int(1))).exits());
    }
}
