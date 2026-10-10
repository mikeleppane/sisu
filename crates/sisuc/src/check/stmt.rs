//! Statements, blocks and scopes.

use std::collections::HashMap;

use super::{Binding, BindingKind, CheckedBlock, Checker, Poisoned, not_found, type_of};
use crate::ast::{self, StmtKind};
use crate::diagnostic::{Diagnostic, Span};
use crate::tir::{self, ExprKind, Type};

impl Checker {
    /// `never` once a statement is `never`, else the type of the last statement (`unit` when
    /// there is none). Warns "unreachable code" on the first statement after a `never` one.
    /// Every statement is checked, also after one fails.
    pub(super) fn block(&mut self, block: &ast::Block) -> CheckedBlock {
        self.scopes.push(HashMap::new());
        let mut stmts = Vec::new();
        let mut ty = Some(Type::Unit);
        let (mut failed, mut warned) = (false, false);
        for stmt in &block.stmts {
            if ty == Some(Type::Never) && !warned {
                self.diagnostics
                    .push(Diagnostic::warning(stmt.span, "unreachable code"));
                warned = true;
            }
            let stmt_ty = match self.stmt(stmt) {
                Ok(stmt) => {
                    let ty = stmt_type(&stmt);
                    stmts.push(stmt);
                    Some(ty)
                }
                Err(poisoned) => {
                    failed = true;
                    poisoned.ty
                }
            };
            if ty != Some(Type::Never) {
                ty = stmt_ty;
            }
        }
        self.pop_scope();
        let ty = match ty {
            Some(ty) if !failed => ty,
            ty => return Err(Poisoned { ty }),
        };
        let ends_in_expr = matches!(
            block.stmts.last(),
            Some(ast::Stmt {
                kind: StmtKind::Expr(_),
                ..
            })
        );
        let value = match stmts.pop() {
            Some(tir::Stmt::Expr(e)) if ends_in_expr => Some(Box::new(e)),
            last => {
                stmts.extend(last);
                None
            }
        };
        Ok(tir::Block { stmts, value, ty })
    }

    /// The statement, or its type when it failed.
    fn stmt(&mut self, stmt: &ast::Stmt) -> Result<tir::Stmt, Poisoned> {
        let unit = || Poisoned {
            ty: Some(Type::Unit),
        };
        match &stmt.kind {
            StmtKind::Let {
                mutable,
                name,
                ty,
                init,
            } => {
                let declared = ty.as_ref().map(|ty| self.resolve(ty));
                let init = match &declared {
                    Some(declared) => self.expect(init, declared.as_ref()),
                    None => self.value(init),
                };
                // The initializer's type, which a failed one keeps only while it is still
                // known (`if` with one failed branch); an unknown declared type is `Error`.
                let ty = match declared {
                    Some(None) => None,
                    _ => type_of(&init).cloned(),
                };
                let kind = if *mutable {
                    BindingKind::Var
                } else {
                    BindingKind::Let
                };
                let local = self.declare(name, ty, kind);
                Ok(tir::Stmt::Let {
                    local,
                    init: init.map_err(|_| unit())?,
                })
            }
            StmtKind::While { cond, body } => {
                self.loop_depth += 1;
                let cond = self.expect(cond, Some(&Type::Bool));
                let body = self.block(body);
                self.loop_depth -= 1;
                let (Ok(cond), Ok(body)) = (cond, body) else {
                    return Err(unit());
                };
                Ok(tir::Stmt::Expr(lower_while(cond, body, stmt.span)))
            }
            StmtKind::Return(value) => self.return_stmt(stmt.span, value.as_ref()),
            StmtKind::Break => self.jump(stmt.span, "break", ExprKind::Break),
            StmtKind::Continue => self.jump(stmt.span, "continue", ExprKind::Continue),
            StmtKind::Assign { target, value } => {
                self.assign(stmt.span, &place_name(target), value)
            }
            StmtKind::CompoundAssign { op, target, value } => {
                let target = &place_name(target);
                // `assign` already reports an unknown target; a synthetic read would repeat it.
                if self.lookup(&target.name).is_none() {
                    return self.assign(stmt.span, target, value);
                }
                // `x op= e` is `x = x op e`, the `op` spanning the statement.
                let lhs = ast::Expr {
                    kind: ast::ExprKind::Name(target.name.clone()),
                    span: target.span,
                };
                let value = ast::Expr {
                    kind: ast::ExprKind::Binary {
                        op: *op,
                        lhs: Box::new(lhs),
                        rhs: Box::new(value.clone()),
                    },
                    span: stmt.span,
                };
                self.assign(stmt.span, target, &value)
            }
            StmtKind::Expr(e) => self.expr(e).map(tir::Stmt::Expr),
        }
    }

    /// `return`, of type `never` even when it fails.
    fn return_stmt(
        &mut self,
        span: Span,
        value: Option<&ast::Expr>,
    ) -> Result<tir::Stmt, Poisoned> {
        let never = || Poisoned {
            ty: Some(Type::Never),
        };
        let value = match value {
            None => match &self.ret {
                Some(ret) if *ret != Type::Unit => {
                    let d = Diagnostic::error(
                        span,
                        format!("`return` needs a value of type {}", self.show(ret)),
                    );
                    self.diagnostics.push(d);
                    return Err(never());
                }
                _ => None,
            },
            Some(value) if self.ret == Some(Type::Unit) => {
                if type_of(&self.value(value)).is_some() {
                    self.diagnostics.push(Diagnostic::error(
                        value.span,
                        "this function returns no value",
                    ));
                }
                return Err(never());
            }
            Some(value) => {
                let ret = self.ret.clone();
                let value = self.expect(value, ret.as_ref()).map_err(|_| never())?;
                Some(Box::new(value))
            }
        };
        Ok(tir::Stmt::Expr(tir::Expr {
            kind: ExprKind::Return(value),
            ty: Type::Never,
            span,
        }))
    }

    /// `break` or `continue`, of type `never` even outside a loop.
    fn jump(&mut self, span: Span, keyword: &str, kind: ExprKind) -> Result<tir::Stmt, Poisoned> {
        if self.loop_depth == 0 {
            self.diagnostics.push(Diagnostic::error(
                span,
                format!("`{keyword}` outside a loop"),
            ));
            return Err(Poisoned {
                ty: Some(Type::Never),
            });
        }
        Ok(tir::Stmt::Expr(tir::Expr {
            kind,
            ty: Type::Never,
            span,
        }))
    }

    /// `target = value`; a `let` or a parameter is reported, and `value` still checked.
    fn assign(
        &mut self,
        span: Span,
        target: &ast::Ident,
        value: &ast::Expr,
    ) -> Result<tir::Stmt, Poisoned> {
        let binding = self.lookup(&target.name).map(|b| {
            b.reassigned |= b.kind == BindingKind::Var;
            (b.local, b.ty.clone(), b.kind, b.span)
        });
        let (local, want) = match binding {
            None => {
                self.diagnostics.push(not_found(&target.name, target.span));
                (None, None)
            }
            Some((local, ty, kind, declared)) => {
                let cannot_assign =
                    || Diagnostic::error(span, format!("cannot assign to `{}`", target.name));
                match kind {
                    BindingKind::Var => {}
                    BindingKind::Let => self.diagnostics.push(
                        cannot_assign()
                            .label("cannot assign twice")
                            .secondary(declared, "declared with `let` here")
                            .help("declare it with `var`"),
                    ),
                    BindingKind::Param => self.diagnostics.push(
                        cannot_assign()
                            .secondary(declared, "declared as a parameter here")
                            .help(format!(
                                "copy it into a `var`: `var {0} = {0}`",
                                target.name
                            )),
                    ),
                }
                (Some(local), ty)
            }
        };
        let value = self.expect(value, want.as_ref());
        match (local, value) {
            (Some(local), Ok(value)) => Ok(tir::Stmt::Assign {
                place: tir::Place::Local(local),
                value,
            }),
            _ => Err(Poisoned {
                ty: Some(Type::Unit),
            }),
        }
    }

    /// Adds `name` to the innermost scope as a new local. A name already in that scope is
    /// reported and keeps its first binding.
    pub(super) fn declare(
        &mut self,
        name: &ast::Ident,
        ty: Option<Type>,
        kind: BindingKind,
    ) -> tir::LocalId {
        let local = tir::LocalId(self.locals.len());
        self.locals.push(tir::Local {
            name: Some(name.name.clone()),
            // An `Error` local never reaches `tir`: the error behind it stops the program.
            ty: ty.clone().unwrap_or(Type::Unit),
            mutable: kind == BindingKind::Var,
        });
        let scope = self.scopes.last_mut().expect("a scope is open");
        if let Some(first) = scope.get(&name.name) {
            self.diagnostics.push(
                Diagnostic::error(
                    name.span,
                    format!("`{}` is already declared in this scope", name.name),
                )
                .secondary(first.span, "first declared here"),
            );
        } else {
            scope.insert(
                name.name.clone(),
                Binding {
                    local,
                    ty,
                    kind,
                    span: name.span,
                    reassigned: false,
                },
            );
        }
        local
    }

    /// Closes the innermost scope, warning about each `var` that was never reassigned.
    pub(super) fn pop_scope(&mut self) {
        let scope = self.scopes.pop().expect("a scope is open");
        for (name, b) in scope {
            if b.kind == BindingKind::Var && !b.reassigned {
                self.diagnostics.push(
                    Diagnostic::warning(b.span, format!("`{name}` is never reassigned"))
                        .help("declare it with `let`"),
                );
            }
        }
    }

    pub(super) fn lookup(&mut self, name: &str) -> Option<&mut Binding> {
        self.scopes
            .iter_mut()
            .rev()
            .find_map(|scope| scope.get_mut(name))
    }
}

/// The type a statement gives its block when it comes last.
fn stmt_type(stmt: &tir::Stmt) -> Type {
    match stmt {
        tir::Stmt::Let { .. } | tir::Stmt::Assign { .. } => Type::Unit,
        tir::Stmt::Expr(e) => e.ty.clone(),
    }
}

/// `while c { b }` is `Loop { If c { b } else { Break } }`.
fn lower_while(cond: tir::Expr, body: tir::Block, span: Span) -> tir::Expr {
    let expr = |kind, ty| tir::Expr { kind, ty, span };
    let exit = tir::Block {
        stmts: vec![tir::Stmt::Expr(expr(ExprKind::Break, Type::Never))],
        value: None,
        ty: Type::Never,
    };
    // The `else` branch is `never`, so the `if` has the body's type.
    let ty = body.ty.clone();
    let branch = expr(
        ExprKind::If {
            cond: Box::new(cond),
            then_block: body,
            else_block: Some(exit),
        },
        ty.clone(),
    );
    let looped = tir::Block {
        stmts: Vec::new(),
        value: Some(Box::new(branch)),
        ty,
    };
    expr(ExprKind::Loop(looped), Type::Unit)
}

/// The variable a place names; a field place lands in Task 10.
fn place_name(place: &ast::Expr) -> ast::Ident {
    match &place.kind {
        ast::ExprKind::Name(name) => ast::Ident {
            name: name.clone(),
            span: place.span,
        },
        _ => panic!("classes land in Task 10"),
    }
}
