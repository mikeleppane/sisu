//! The checker: signatures, scopes and names. Typing rules follow in Task 10.

use std::collections::HashMap;
use std::fmt;

use crate::ast::{
    BinaryOp, Block, Expr, ExprKind, Function, Ident, Program, Stmt, StmtKind, TypeExpr, UnaryOp,
};
use crate::diagnostic::{Diagnostic, Span};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Type {
    I64,
    Bool,
    Unit,
    Never,
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Type::I64 => "`i64`",
            Type::Bool => "`bool`",
            Type::Unit => "`unit`",
            Type::Never => "`never`",
        })
    }
}

struct Signature {
    #[cfg_attr(
        test,
        expect(dead_code, reason = "Task 10 checks call arguments against it")
    )]
    params: Vec<Type>,
    ret: Type,
    name_span: Span,
}

enum BindingKind {
    Let,
    Var,
    Param,
}

struct Binding {
    ty: Type,
    #[cfg_attr(
        test,
        expect(dead_code, reason = "Task 11 checks assignments against it")
    )]
    kind: BindingKind,
    span: Span,
}

struct Checker {
    functions: HashMap<String, Signature>,
    scopes: Vec<HashMap<String, Binding>>,
    warnings: Vec<Diagnostic>,
    ret: Type,
}

/// Warnings sorted by span start, then at most one error.
pub(crate) fn check(program: &Program) -> Vec<Diagnostic> {
    let mut checker = Checker {
        functions: HashMap::new(),
        scopes: Vec::new(),
        warnings: Vec::new(),
        ret: Type::Unit,
    };
    let result = checker.collect_signatures(program).and_then(|()| {
        program
            .functions
            .iter()
            .try_for_each(|f| checker.function(f))
    });
    let mut diagnostics = checker.warnings;
    diagnostics.sort_by_key(|w| w.span.start);
    diagnostics.extend(result.err());
    diagnostics
}

fn resolve(ty: &TypeExpr) -> Result<Type, Diagnostic> {
    match ty.name.as_str() {
        "i64" => Ok(Type::I64),
        "bool" => Ok(Type::Bool),
        name => Err(Diagnostic::error(ty.span, format!("unknown type `{name}`"))
            .help("the types are `i64` and `bool`")),
    }
}

impl Checker {
    fn collect_signatures(&mut self, program: &Program) -> Result<(), Diagnostic> {
        for f in &program.functions {
            let name = &f.name;
            if name.name == "print" {
                return Err(duplicate(name).label("`print` is built in"));
            }
            if let Some(first) = self.functions.get(&name.name) {
                return Err(duplicate(name).secondary(first.name_span, "first defined here"));
            }
            let params = f
                .params
                .iter()
                .map(|p| resolve(&p.ty))
                .collect::<Result<_, _>>()?;
            let ret = f.ret.as_ref().map_or(Ok(Type::Unit), resolve)?;
            self.functions.insert(
                name.name.clone(),
                Signature {
                    params,
                    ret,
                    name_span: name.span,
                },
            );
        }
        let Some(main) = program.functions.iter().find(|f| f.name.name == "main") else {
            return Err(Diagnostic::error(Span::new(0, 0), "no `main` function"));
        };
        if !main.params.is_empty() || main.ret.is_some() {
            return Err(Diagnostic::error(
                main.name.span,
                "`main` takes no parameters and returns no value",
            ));
        }
        Ok(())
    }

    fn function(&mut self, f: &Function) -> Result<(), Diagnostic> {
        self.ret = self.functions[&f.name.name].ret;
        self.scopes.push(HashMap::new());
        for p in &f.params {
            self.declare(&p.name, resolve(&p.ty)?, BindingKind::Param)?;
        }
        self.block(&f.body)?;
        self.scopes.pop();
        Ok(())
    }

    fn declare(&mut self, name: &Ident, ty: Type, kind: BindingKind) -> Result<(), Diagnostic> {
        let scope = self.scopes.last_mut().expect("a scope is open");
        if let Some(first) = scope.get(&name.name) {
            return Err(Diagnostic::error(
                name.span,
                format!("`{}` is already declared in this scope", name.name),
            )
            .secondary(first.span, "first declared here"));
        }
        scope.insert(
            name.name.clone(),
            Binding {
                ty,
                kind,
                span: name.span,
            },
        );
        Ok(())
    }

    fn lookup(&self, name: &str, span: Span) -> Result<&Binding, Diagnostic> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name))
            .ok_or_else(|| Diagnostic::error(span, format!("cannot find `{name}` in this scope")))
    }

    /// The type of the block's last statement, or `unit` when it has none.
    fn block(&mut self, block: &Block) -> Result<Type, Diagnostic> {
        self.scopes.push(HashMap::new());
        let mut ty = Type::Unit;
        for stmt in &block.stmts {
            ty = self.stmt(stmt)?;
        }
        self.scopes.pop();
        Ok(ty)
    }

    fn stmt(&mut self, stmt: &Stmt) -> Result<Type, Diagnostic> {
        match &stmt.kind {
            StmtKind::Let {
                mutable,
                name,
                ty,
                init,
            } => {
                let declared = ty.as_ref().map(resolve).transpose()?;
                let init_ty = self.expr(init)?;
                let kind = if *mutable {
                    BindingKind::Var
                } else {
                    BindingKind::Let
                };
                self.declare(name, declared.unwrap_or(init_ty), kind)?;
                Ok(Type::Unit)
            }
            StmtKind::While { cond, body } => {
                self.expr(cond)?;
                self.block(body)?;
                Ok(Type::Unit)
            }
            StmtKind::Return(value) => {
                value.as_ref().map(|v| self.expr(v)).transpose()?;
                Ok(Type::Never)
            }
            StmtKind::Assign { target, value } => {
                self.lookup(&target.name, target.span)?;
                self.expr(value)?;
                Ok(Type::Unit)
            }
            StmtKind::Expr(e) => self.expr(e),
        }
    }

    fn expr(&mut self, expr: &Expr) -> Result<Type, Diagnostic> {
        match &expr.kind {
            ExprKind::Int(_) => Ok(Type::I64),
            ExprKind::Bool(_) => Ok(Type::Bool),
            ExprKind::Name(name) => Ok(self.lookup(name, expr.span)?.ty),
            ExprKind::Call { callee, args } => {
                let ret = if callee.name == "print" {
                    Type::Unit
                } else {
                    self.functions
                        .get(&callee.name)
                        .map(|s| s.ret)
                        .ok_or_else(|| {
                            Diagnostic::error(
                                callee.span,
                                format!("cannot find function `{}`", callee.name),
                            )
                        })?
                };
                for arg in args {
                    self.expr(arg)?;
                }
                Ok(ret)
            }
            ExprKind::Unary { op, operand } => {
                self.expr(operand)?;
                Ok(match op {
                    UnaryOp::Neg => Type::I64,
                    UnaryOp::Not => Type::Bool,
                })
            }
            ExprKind::Binary { op, lhs, rhs } => {
                self.expr(lhs)?;
                self.expr(rhs)?;
                Ok(match op {
                    BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Rem => Type::I64,
                    _ => Type::Bool,
                })
            }
            ExprKind::Compare { operands, .. } => {
                for operand in operands {
                    self.expr(operand)?;
                }
                Ok(Type::Bool)
            }
            ExprKind::If {
                cond,
                then_block,
                else_block,
            } => {
                self.expr(cond)?;
                let ty = self.block(then_block)?;
                match else_block {
                    Some(block) => self.block(block).map(|_| ty),
                    None => Ok(Type::Unit),
                }
            }
        }
    }
}

fn duplicate(name: &Ident) -> Diagnostic {
    Diagnostic::error(
        name.span,
        format!("function `{}` is defined twice", name.name),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::{Severity, Span, line_col};
    use crate::lexer::lex;
    use crate::parser::parse;

    fn diags(src: &str) -> Vec<Diagnostic> {
        check(&parse(&lex(src).expect("source lexes")).expect("source parses"))
    }

    fn at(src: &str, span: Span) -> (usize, usize) {
        line_col(src, span.start)
    }

    /// Asserts exactly one error with these parts; `secondary` is `(line, col, text)` rows.
    fn expect_error(
        src: &str,
        message: &str,
        pos: (usize, usize),
        label: Option<&str>,
        secondary: &[(usize, usize, &str)],
        help: Option<&str>,
    ) {
        let ds = diags(src);
        assert_eq!(ds.len(), 1, "{ds:?}");
        let d = &ds[0];
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.message, message);
        assert_eq!(at(src, d.span), pos);
        assert_eq!(d.label.as_deref(), label);
        let found: Vec<_> = d
            .secondary
            .iter()
            .map(|(span, text)| {
                let (line, col) = at(src, *span);
                (line, col, text.as_str())
            })
            .collect();
        assert_eq!(found, secondary);
        assert_eq!(d.help.as_deref(), help);
    }

    #[test]
    fn duplicate_function() {
        expect_error(
            "fn main() {}\nfn main() {}",
            "function `main` is defined twice",
            (2, 4),
            None,
            &[(1, 4, "first defined here")],
            None,
        );
    }

    #[test]
    fn print_is_built_in() {
        expect_error(
            "fn print() {}\nfn main() {}",
            "function `print` is defined twice",
            (1, 4),
            Some("`print` is built in"),
            &[],
            None,
        );
    }

    #[test]
    fn missing_main() {
        let src = "fn f() {}";
        expect_error(src, "no `main` function", (1, 1), None, &[], None);
        assert_eq!(diags(src)[0].span, Span::new(0, 0));
    }

    #[test]
    fn main_with_params() {
        expect_error(
            "fn main(x: i64) {}",
            "`main` takes no parameters and returns no value",
            (1, 4),
            None,
            &[],
            None,
        );
    }

    #[test]
    fn main_with_return_type() {
        expect_error(
            "fn main() -> i64 { 0 }",
            "`main` takes no parameters and returns no value",
            (1, 4),
            None,
            &[],
            None,
        );
    }

    #[test]
    fn unknown_type() {
        expect_error(
            "fn main() { let x: str = 1 }",
            "unknown type `str`",
            (1, 20),
            None,
            &[],
            Some("the types are `i64` and `bool`"),
        );
    }

    #[test]
    fn unknown_name() {
        expect_error(
            "fn main() { print(y) }",
            "cannot find `y` in this scope",
            (1, 19),
            None,
            &[],
            None,
        );
    }

    #[test]
    fn unknown_function() {
        expect_error(
            "fn main() { g() }",
            "cannot find function `g`",
            (1, 13),
            None,
            &[],
            None,
        );
    }

    #[test]
    fn redeclare_in_block() {
        expect_error(
            "fn main() {\n    let x = 1\n    let x = 2\n}",
            "`x` is already declared in this scope",
            (3, 9),
            None,
            &[(2, 9, "first declared here")],
            None,
        );
    }

    #[test]
    fn repeated_param() {
        expect_error(
            "fn f(a: i64, a: i64) {}\nfn main() {}",
            "`a` is already declared in this scope",
            (1, 14),
            None,
            &[(1, 6, "first declared here")],
            None,
        );
    }

    #[test]
    fn shadowing_in_inner_block() {
        let src = "fn main() {\n    let x = 1\n    if true {\n        let x = true\n        print(x)\n    }\n    print(x)\n}";
        assert!(diags(src).is_empty());
    }

    #[test]
    fn functions_in_any_order() {
        assert!(diags("fn main() { print(f(1)) }\nfn f(n: i64) -> i64 { n }").is_empty());
    }

    #[test]
    fn var_may_shadow_a_param() {
        assert!(diags("fn f(n: i64) { var n = n }\nfn main() {}").is_empty());
    }

    #[test]
    fn one_error_per_run() {
        expect_error(
            "fn main() {\n    print(a)\n    print(b)\n}",
            "cannot find `a` in this scope",
            (2, 11),
            None,
            &[],
            None,
        );
    }
}
