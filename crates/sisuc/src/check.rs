//! The checker: signatures, scopes, names and types.

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
    params: Vec<Type>,
    ret: Type,
    name_span: Span,
}

#[derive(PartialEq)]
enum BindingKind {
    Let,
    Var,
    Param,
}

struct Binding {
    ty: Type,
    kind: BindingKind,
    span: Span,
    reassigned: bool,
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
        let sig = &self.functions[&f.name.name];
        let (params, ret) = (sig.params.clone(), sig.ret);
        self.ret = ret;
        self.scopes.push(HashMap::new());
        for (p, ty) in f.params.iter().zip(params) {
            self.declare(&p.name, ty, BindingKind::Param)?;
        }
        let body = self.block(&f.body)?;
        self.pop_scope();
        match body {
            _ if ret == Type::Unit || body == Type::Never || body == ret => Ok(()),
            Type::Unit => {
                let end = f.body.span.end;
                Err(Diagnostic::error(
                    Span::new(end - 1, end),
                    format!(
                        "function `{}` must return {ret}, but its body has no value",
                        f.name.name
                    ),
                ))
            }
            _ => Err(mismatch(value_span(&f.body), ret, body)),
        }
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
                reassigned: false,
            },
        );
        Ok(())
    }

    /// Closes the innermost scope, warning about each `var` that was never reassigned.
    fn pop_scope(&mut self) {
        let scope = self.scopes.pop().expect("a scope is open");
        for (name, b) in scope {
            if b.kind == BindingKind::Var && !b.reassigned {
                self.warnings.push(
                    Diagnostic::warning(b.span, format!("`{name}` is never reassigned"))
                        .help("declare it with `let`"),
                );
            }
        }
    }

    fn lookup(&mut self, name: &str, span: Span) -> Result<&mut Binding, Diagnostic> {
        self.scopes
            .iter_mut()
            .rev()
            .find_map(|scope| scope.get_mut(name))
            .ok_or_else(|| Diagnostic::error(span, format!("cannot find `{name}` in this scope")))
    }

    /// `never` once a statement is `never`, else the type of the last statement (`unit` when
    /// there is none). Warns "unreachable code" on the first statement after a `never` one.
    fn block(&mut self, block: &Block) -> Result<Type, Diagnostic> {
        self.scopes.push(HashMap::new());
        let mut ty = Type::Unit;
        let mut warned = false;
        for stmt in &block.stmts {
            if ty == Type::Never && !warned {
                self.warnings
                    .push(Diagnostic::warning(stmt.span, "unreachable code"));
                warned = true;
            }
            let stmt_ty = self.stmt(stmt)?;
            if ty != Type::Never {
                ty = stmt_ty;
            }
        }
        self.pop_scope();
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
                let ty = match ty {
                    Some(ty) => {
                        let declared = resolve(ty)?;
                        self.expect(init, declared)?;
                        declared
                    }
                    None => self.value(init)?,
                };
                let kind = if *mutable {
                    BindingKind::Var
                } else {
                    BindingKind::Let
                };
                self.declare(name, ty, kind)?;
                Ok(Type::Unit)
            }
            StmtKind::While { cond, body } => {
                self.expect(cond, Type::Bool)?;
                self.block(body)?;
                Ok(Type::Unit)
            }
            StmtKind::Return(None) if self.ret != Type::Unit => Err(Diagnostic::error(
                stmt.span,
                format!("`return` needs a value of type {}", self.ret),
            )),
            StmtKind::Return(None) => Ok(Type::Never),
            StmtKind::Return(Some(value)) => {
                let ty = self.value(value)?;
                if self.ret == Type::Unit {
                    return Err(Diagnostic::error(
                        value.span,
                        "this function returns no value",
                    ));
                }
                if ty != self.ret {
                    return Err(mismatch(value.span, self.ret, ty));
                }
                Ok(Type::Never)
            }
            StmtKind::Assign { target, value } => {
                let binding = self.lookup(&target.name, target.span)?;
                let want = binding.ty;
                let declared = binding.span;
                match binding.kind {
                    BindingKind::Var => binding.reassigned = true,
                    BindingKind::Let => {
                        return Err(cannot_assign(stmt.span, target)
                            .label("cannot assign twice")
                            .secondary(declared, "declared with `let` here")
                            .help("declare it with `var`"));
                    }
                    BindingKind::Param => {
                        return Err(cannot_assign(stmt.span, target)
                            .secondary(declared, "declared as a parameter here")
                            .help(format!(
                                "copy it into a `var`: `var {0} = {0}`",
                                target.name
                            )));
                    }
                }
                self.expect(value, want)?;
                Ok(Type::Unit)
            }
            StmtKind::Expr(e) => self.expr(e),
        }
    }

    /// The type of an expression whose value is used: `i64` or `bool`.
    fn value(&mut self, expr: &Expr) -> Result<Type, Diagnostic> {
        match self.expr(expr)? {
            Type::Unit => Err(Diagnostic::error(expr.span, "expression has no value")),
            Type::Never => Err(Diagnostic::error(expr.span, "unreachable code")),
            ty => Ok(ty),
        }
    }

    fn expect(&mut self, expr: &Expr, want: Type) -> Result<Type, Diagnostic> {
        let ty = self.value(expr)?;
        if ty == want {
            Ok(ty)
        } else {
            Err(mismatch(expr.span, want, ty))
        }
    }

    fn expr(&mut self, expr: &Expr) -> Result<Type, Diagnostic> {
        match &expr.kind {
            ExprKind::Int(_) => Ok(Type::I64),
            ExprKind::Bool(_) => Ok(Type::Bool),
            ExprKind::Name(name) => Ok(self.lookup(name, expr.span)?.ty),
            ExprKind::Call { callee, args } => self.call(expr, callee, args),
            ExprKind::Unary { op, operand } => self.expect(
                operand,
                match op {
                    UnaryOp::Neg => Type::I64,
                    UnaryOp::Not => Type::Bool,
                },
            ),
            ExprKind::Binary { op, lhs, rhs } => {
                let operand = match op {
                    BinaryOp::Eq | BinaryOp::Ne => {
                        let (l, r) = (self.value(lhs)?, self.value(rhs)?);
                        if l != r {
                            return Err(Diagnostic::error(
                                expr.span,
                                format!("cannot compare {l} with {r}"),
                            ));
                        }
                        return Ok(Type::Bool);
                    }
                    BinaryOp::And | BinaryOp::Or => Type::Bool,
                    BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Rem => Type::I64,
                };
                self.expect(lhs, operand)?;
                self.expect(rhs, operand)
            }
            ExprKind::Compare { operands, .. } => {
                for operand in operands {
                    self.expect(operand, Type::I64)?;
                }
                Ok(Type::Bool)
            }
            ExprKind::If {
                cond,
                then_block,
                else_block,
            } => {
                self.expect(cond, Type::Bool)?;
                let then_ty = self.block(then_block)?;
                let Some(else_block) = else_block else {
                    return Ok(Type::Unit);
                };
                match (then_ty, self.block(else_block)?) {
                    (Type::Never, ty) => Ok(ty),
                    (ty, else_ty) if else_ty == ty || else_ty == Type::Never => Ok(ty),
                    (ty, else_ty) => Err(mismatch(value_span(else_block), ty, else_ty)),
                }
            }
        }
    }

    fn call(&mut self, expr: &Expr, callee: &Ident, args: &[Expr]) -> Result<Type, Diagnostic> {
        if callee.name == "print" {
            // Not `value()`: a `unit` argument needs this message, not "expression has no value".
            let wrong = || Diagnostic::error(expr.span, "`print` takes one `i64` or `bool`");
            let [arg] = args else {
                return Err(wrong());
            };
            return match self.expr(arg)? {
                Type::Unit => Err(wrong()),
                Type::Never => Err(Diagnostic::error(arg.span, "unreachable code")),
                _ => Ok(Type::Unit),
            };
        }
        let sig = self.functions.get(&callee.name).ok_or_else(|| {
            Diagnostic::error(
                callee.span,
                format!("cannot find function `{}`", callee.name),
            )
        })?;
        let (params, ret) = (sig.params.clone(), sig.ret);
        if params.len() != args.len() {
            let plural = if params.len() == 1 { "" } else { "s" };
            return Err(Diagnostic::error(
                expr.span,
                format!(
                    "`{}` takes {} argument{plural}, found {}",
                    callee.name,
                    params.len(),
                    args.len()
                ),
            ));
        }
        for (arg, param) in args.iter().zip(params) {
            self.expect(arg, param)?;
        }
        Ok(ret)
    }
}

fn cannot_assign(span: Span, target: &Ident) -> Diagnostic {
    Diagnostic::error(span, format!("cannot assign to `{}`", target.name))
}

fn mismatch(span: Span, want: Type, got: Type) -> Diagnostic {
    Diagnostic::error(span, format!("expected {want}, found {got}"))
}

/// Where a block's value comes from: its last statement, or the block itself when empty.
fn value_span(block: &Block) -> Span {
    block.stmts.last().map_or(block.span, |s| s.span)
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

    /// Asserts exactly one error with this message and position, and no label or help.
    fn error(src: &str, message: &str, pos: (usize, usize)) {
        expect_error(src, message, pos, None, &[], None);
    }

    /// Asserts exactly one diagnostic: a warning with this message and position.
    fn warning(src: &str, message: &str, pos: (usize, usize)) {
        let ds = diags(src);
        assert_eq!(ds.len(), 1, "{ds:?}");
        assert_eq!(ds[0].severity, Severity::Warning);
        assert_eq!(ds[0].message, message);
        assert_eq!(at(src, ds[0].span), pos);
    }

    fn clean(src: &str) {
        let ds = diags(src);
        assert!(ds.is_empty(), "{ds:?}");
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
        let src = "fn f(n: i64) {\n    var n = n\n    n = 1\n}\nfn main() {}";
        clean(src);
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

    #[test]
    fn unknown_type_in_signature() {
        expect_error(
            "fn f(a: str) {}\nfn main() {}",
            "unknown type `str`",
            (1, 9),
            None,
            &[],
            Some("the types are `i64` and `bool`"),
        );
    }

    #[test]
    fn condition_must_be_bool() {
        error(
            "fn main() { if 1 { } }",
            "expected `bool`, found `i64`",
            (1, 16),
        );
    }

    #[test]
    fn while_condition_must_be_bool() {
        error(
            "fn main() { while 1 { } }",
            "expected `bool`, found `i64`",
            (1, 19),
        );
    }

    #[test]
    fn arithmetic_needs_i64() {
        error(
            "fn main() { print(1 + true) }",
            "expected `i64`, found `bool`",
            (1, 23),
        );
    }

    #[test]
    fn not_needs_bool() {
        error(
            "fn main() { print(!1) }",
            "expected `bool`, found `i64`",
            (1, 20),
        );
    }

    #[test]
    fn equality_needs_same_type() {
        error(
            "fn main() { print(1 == true) }",
            "cannot compare `i64` with `bool`",
            (1, 19),
        );
    }

    #[test]
    fn arity() {
        error(
            "fn f(a: i64) {}\nfn main() { f() }",
            "`f` takes 1 argument, found 0",
            (2, 13),
        );
        error(
            "fn f(a: i64, b: i64) {}\nfn main() { f(1) }",
            "`f` takes 2 arguments, found 1",
            (2, 13),
        );
    }

    #[test]
    fn argument_type() {
        error(
            "fn f(a: i64) {}\nfn main() { f(true) }",
            "expected `i64`, found `bool`",
            (2, 15),
        );
    }

    #[test]
    fn print_no_argument() {
        error(
            "fn main() { print() }",
            "`print` takes one `i64` or `bool`",
            (1, 13),
        );
    }

    #[test]
    fn print_two_arguments() {
        error(
            "fn main() { print(1, 2) }",
            "`print` takes one `i64` or `bool`",
            (1, 13),
        );
    }

    #[test]
    fn print_unit_argument() {
        error(
            "fn g() {}\nfn main() { print(g()) }",
            "`print` takes one `i64` or `bool`",
            (2, 13),
        );
    }

    #[test]
    fn bind_unit() {
        error(
            "fn g() {}\nfn main() { let x = g() }",
            "expression has no value",
            (2, 21),
        );
    }

    #[test]
    fn if_without_else_has_no_value() {
        error(
            "fn main() {\n    let x = if true { 1 }\n}",
            "expression has no value",
            (2, 13),
        );
    }

    #[test]
    fn if_branches_differ() {
        error(
            "fn main() {\n    let x = if true { 1 } else { false }\n}",
            "expected `i64`, found `bool`",
            (2, 34),
        );
    }

    #[test]
    fn body_value_type() {
        error(
            "fn f() -> i64 {\n    true\n}\nfn main() {}",
            "expected `i64`, found `bool`",
            (2, 5),
        );
    }

    #[test]
    fn body_without_value() {
        let src = "fn f() -> i64 {\n    let x = 1\n}\nfn main() {}";
        error(
            src,
            "function `f` must return `i64`, but its body has no value",
            (3, 1),
        );
        assert_eq!(diags(src)[0].span, Span::new(30, 31));
    }

    #[test]
    fn return_value_type() {
        error(
            "fn f() -> i64 {\n    return true\n}\nfn main() {}",
            "expected `i64`, found `bool`",
            (2, 12),
        );
    }

    #[test]
    fn return_needs_value() {
        error(
            "fn f() -> i64 {\n    return\n}\nfn main() {}",
            "`return` needs a value of type `i64`",
            (2, 5),
        );
    }

    #[test]
    fn return_value_in_unit_fn() {
        error(
            "fn main() {\n    return 1\n}",
            "this function returns no value",
            (2, 12),
        );
    }

    #[test]
    fn all_paths_return_is_fine() {
        clean(
            "fn sign(n: i64) -> i64 {\n    if n < 0 { return -1 } else { return 1 }\n}\nfn main() {}",
        );
    }

    #[test]
    fn never_as_return_value() {
        error(
            "fn f(c: bool) -> i64 {\n    return if c { return 1 } else { return 2 }\n}\nfn main() {}",
            "unreachable code",
            (2, 12),
        );
    }

    #[test]
    fn annotated_let_type() {
        error(
            "fn main() { let x: bool = 1 }",
            "expected `bool`, found `i64`",
            (1, 27),
        );
    }

    #[test]
    fn annotated_let_matches() {
        clean("fn main() {\n    let x: bool = true\n    print(x)\n}");
    }

    #[test]
    fn never_as_operand() {
        error(
            "fn f(c: bool) -> i64 {\n    1 + if c { return 1 } else { return 2 }\n}\nfn main() {}",
            "unreachable code",
            (2, 9),
        );
    }

    #[test]
    fn statement_after_return() {
        warning(
            "fn main() {\n    return\n    print(1)\n}",
            "unreachable code",
            (3, 5),
        );
    }

    #[test]
    fn block_after_return_is_never() {
        warning(
            "fn f() -> i64 {\n    return 1\n    print(2)\n}\nfn main() {}",
            "unreachable code",
            (3, 5),
        );
    }

    #[test]
    fn neg_needs_i64() {
        error(
            "fn main() { print(-true) }",
            "expected `i64`, found `bool`",
            (1, 20),
        );
    }

    #[test]
    fn less_needs_i64() {
        error(
            "fn main() { print(true < false) }",
            "expected `i64`, found `bool`",
            (1, 19),
        );
    }

    #[test]
    fn and_needs_bool() {
        error(
            "fn main() { print(1 && true) }",
            "expected `bool`, found `i64`",
            (1, 19),
        );
    }

    #[test]
    fn unit_operand() {
        error(
            "fn g() {}\nfn main() { print(1 + g()) }",
            "expression has no value",
            (2, 23),
        );
    }

    #[test]
    fn unit_argument() {
        error(
            "fn g() {}\nfn f(a: i64) {}\nfn main() { f(g()) }",
            "expression has no value",
            (3, 15),
        );
    }

    #[test]
    fn separate_namespaces() {
        clean("fn f() -> i64 { 1 }\nfn main() {\n    let f = 2\n    print(f)\n    print(f())\n}");
    }

    #[test]
    fn never_branch_takes_other_type() {
        clean(
            "fn f(c: bool) -> i64 {\n    let x = if c { return 1 } else { 2 }\n    let y = if c { 3 } else { return 4 }\n    x + y\n}\nfn main() {}",
        );
    }

    #[test]
    fn unreachable_warned_once_per_block() {
        warning(
            "fn main() {\n    return\n    print(1)\n    print(2)\n}",
            "unreachable code",
            (3, 5),
        );
    }

    #[test]
    fn statements_after_return_are_checked() {
        let src = "fn main() {\n    return\n    print(true + 1)\n}";
        let ds = diags(src);
        let found: Vec<_> = ds
            .iter()
            .map(|d| (d.severity, d.message.as_str(), at(src, d.span)))
            .collect();
        assert_eq!(
            found,
            [
                (Severity::Warning, "unreachable code", (3, 5)),
                (Severity::Error, "expected `i64`, found `bool`", (3, 11)),
            ]
        );
    }

    #[test]
    fn assign_to_let() {
        let src = "fn main() {\n    let n = 0\n    n = 1\n}";
        expect_error(
            src,
            "cannot assign to `n`",
            (3, 5),
            Some("cannot assign twice"),
            &[(2, 9, "declared with `let` here")],
            Some("declare it with `var`"),
        );
        let span = diags(src)[0].span;
        assert_eq!(&src[span.start..span.end], "n = 1");
    }

    #[test]
    fn never_assigned_value() {
        error(
            "fn f(c: bool) -> i64 {\n    var x = 0\n    x = if c { return 1 } else { return 2 }\n    x\n}\nfn main() {}",
            "unreachable code",
            (3, 9),
        );
    }

    #[test]
    fn compound_assign_to_let() {
        expect_error(
            "fn main() {\n    let n = 0\n    n += 1\n}",
            "cannot assign to `n`",
            (3, 5),
            Some("cannot assign twice"),
            &[(2, 9, "declared with `let` here")],
            Some("declare it with `var`"),
        );
    }

    #[test]
    fn assign_to_param() {
        expect_error(
            "fn f(n: i64) {\n    n = 1\n}\nfn main() {}",
            "cannot assign to `n`",
            (2, 5),
            None,
            &[(1, 6, "declared as a parameter here")],
            Some("copy it into a `var`: `var n = n`"),
        );
    }

    #[test]
    fn assign_type() {
        error(
            "fn main() {\n    var x = 1\n    x = true\n}",
            "expected `i64`, found `bool`",
            (3, 9),
        );
    }

    #[test]
    fn unused_var() {
        let src = "fn main() {\n    var count = 0\n    print(count)\n}";
        warning(src, "`count` is never reassigned", (2, 9));
        assert_eq!(diags(src)[0].help.as_deref(), Some("declare it with `let`"));
    }

    #[test]
    fn unused_vars_in_source_order() {
        // The inner scope pops first, so this fails unless warnings are sorted.
        let src = "fn main() {\n    var a = 0\n    if true {\n        var b = 0\n        print(b)\n    }\n    print(a)\n}";
        let found: Vec<_> = diags(src)
            .iter()
            .map(|d| (d.severity, d.message.clone(), at(src, d.span)))
            .collect();
        assert_eq!(
            found,
            [
                (
                    Severity::Warning,
                    "`a` is never reassigned".to_string(),
                    (2, 9)
                ),
                (
                    Severity::Warning,
                    "`b` is never reassigned".to_string(),
                    (4, 13)
                ),
            ]
        );
    }

    #[test]
    fn var_reassigned_in_inner_block() {
        clean("fn main() {\n    var n = 0\n    if true { n = 1 }\n    print(n)\n}");
    }

    #[test]
    fn empty_file_has_no_main() {
        let ds = diags("");
        assert_eq!(ds.len(), 1, "{ds:?}");
        assert_eq!(ds[0].message, "no `main` function");
        assert_eq!(at("", ds[0].span), (1, 1));
        assert!(
            ds[0]
                .render("empty.sisu", "")
                .starts_with("error: no `main` function")
        );
    }

    #[test]
    fn never_as_print_argument() {
        error(
            "fn f(c: bool) -> i64 {\n    print(if c { return 1 } else { return 2 })\n    1\n}\nfn main() {}",
            "unreachable code",
            (2, 11),
        );
    }
}
