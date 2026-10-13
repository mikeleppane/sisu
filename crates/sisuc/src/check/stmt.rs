//! Statements, blocks and scopes.

use std::collections::HashMap;

use super::array::optional_array;
use super::expr::local;
use super::{Binding, BindingKind, CheckedBlock, Checker, Poisoned, block_expr, type_of};
use crate::ast::{self, StmtKind};
use crate::diagnostic::{DiagnosticKind, Span, Unwrap};
use crate::tir::{self, ExprKind, Type};

impl Checker {
    /// `never` once a statement is `never`, else the type of the last statement (`unit` when
    /// there is none). Warns "unreachable code" on the first statement after a `never` one.
    /// Every statement is checked, also after one fails. `expected` is for the block's value.
    pub(super) fn block(&mut self, block: &ast::Block, expected: Option<&Type>) -> CheckedBlock {
        self.scopes.push(HashMap::new());
        self.scoped_block(block, expected)
    }

    /// `block`, in the scope just opened for it, which it closes.
    fn scoped_block(&mut self, block: &ast::Block, expected: Option<&Type>) -> CheckedBlock {
        let mut stmts = Vec::new();
        let mut ty = Some(Type::Unit);
        let (mut failed, mut warned, mut mismatch) = (false, false, false);
        for (i, stmt) in block.stmts.iter().enumerate() {
            if ty == Some(Type::Never) && !warned {
                self.report(DiagnosticKind::UnreachableCode { at: stmt.span });
                warned = true;
            }
            let last = i + 1 == block.stmts.len();
            let outer_jump = std::mem::take(&mut self.invalid_jump);
            let checked = self.stmt(stmt, expected.filter(|_| last));
            let invalid_jump = self.invalid_jump;
            // Nested blocks leave their flag set, so an enclosing statement sees it too.
            self.invalid_jump |= outer_jump;
            let stmt_ty = match checked {
                Ok(lowered) => {
                    // `let _ = e` lowers to an expression, but a `let` has no value.
                    let ty = if matches!(stmt.kind, StmtKind::Let { .. }) {
                        Type::Unit
                    } else {
                        stmt_type(&lowered)
                    };
                    stmts.push(lowered);
                    Some(ty)
                }
                Err(poisoned) => {
                    failed = true;
                    mismatch = last && poisoned.mismatch;
                    // An invalid jump made the block `never` only by error; what follows is no
                    // new news. Any other failed `never` statement still leaves the block.
                    warned |= invalid_jump && poisoned.ty == Some(Type::Never);
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
            // A tail that rejected `expected` fails the block, unless a `never` came first.
            ty => {
                let mismatch = mismatch && ty.is_none();
                return Err(Poisoned { ty, mismatch });
            }
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

    /// The statement, or its type when it failed. `expected` is for an expression statement.
    fn stmt(&mut self, stmt: &ast::Stmt, expected: Option<&Type>) -> Result<tir::Stmt, Poisoned> {
        let unit = || Poisoned::typed(Type::Unit);
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
                    None => self.value(init, None),
                };
                // `let _ = e` binds nothing: the value is an expression statement, which
                // codegen releases at once.
                if name.name == "_" {
                    return init.map(tir::Stmt::Expr).map_err(|_| unit());
                }
                // The initializer's type, which a failed one keeps only while it is still
                // known (`if` with one failed branch); an unknown declared type is `Error`.
                let ty = match declared {
                    Some(None) => None,
                    _ => type_of(&init).cloned(),
                };
                let kind = if mutable.is_some() {
                    BindingKind::Var
                } else {
                    BindingKind::Let
                };
                let local = self.declare(name, ty, kind, *mutable);
                Ok(tir::Stmt::Let {
                    local,
                    init: init.map_err(|_| unit())?,
                })
            }
            StmtKind::While { cond, body } => {
                self.loop_depth += 1;
                let cond = self.expect(cond, Some(&Type::Bool));
                let body = self.block(body, None);
                self.loop_depth -= 1;
                let (Ok(cond), Ok(body)) = (cond, body) else {
                    return Err(unit());
                };
                Ok(tir::Stmt::Expr(lower_loop(
                    body,
                    stmt.span,
                    |then_block, exit| ExprKind::If {
                        cond: Box::new(cond),
                        then_block,
                        else_block: Some(exit),
                    },
                )))
            }
            StmtKind::WhileLet { name, value, body } => {
                self.loop_depth += 1;
                let (scrutinee, bound, _) = self.unwrapped(Unwrap::WhileLet, value);
                let (bind, body) = self.unwrapping_block("while let", name, bound, body, None);
                self.loop_depth -= 1;
                let (Ok(scrutinee), Ok(body)) = (scrutinee, body) else {
                    return Err(unit());
                };
                Ok(tir::Stmt::Expr(lower_loop(
                    body,
                    stmt.span,
                    |then_block, exit| ExprKind::IfSome {
                        bind,
                        scrutinee: Box::new(scrutinee),
                        then_block,
                        else_block: Some(exit),
                    },
                )))
            }
            StmtKind::For { name, source, body } => self.for_loop(stmt.span, name, source, body),
            StmtKind::Return(value) => self.return_stmt(stmt.span, value.as_ref()),
            StmtKind::Break => self.jump(stmt.span, "break", ExprKind::Break),
            StmtKind::Continue => self.jump(stmt.span, "continue", ExprKind::Continue),
            StmtKind::Assign { target, value } => self.assign(stmt.span, target, value),
            StmtKind::CompoundAssign { op, target, value } => {
                self.compound_assign(stmt.span, *op, target, value)
            }
            StmtKind::Expr(e) => self.expr(e, expected).map(tir::Stmt::Expr),
        }
    }

    /// `for`, lowered to a `Loop` as in the spec's table: over an array `s` through `s[i]`
    /// with `i < s.len()`, over a range through `i < end`. The locals are the source's, the
    /// fresh ones, then the loop variable.
    fn for_loop(
        &mut self,
        span: Span,
        name: &ast::Ident,
        source: &ast::ForSource,
        body: &ast::Block,
    ) -> Result<tir::Stmt, Poisoned> {
        let expr = |kind, ty| tir::Expr { kind, ty, span };
        let (header, bound) = match source {
            ast::ForSource::Array(e) => self.for_array(span, e),
            ast::ForSource::Range { start, end } => self.for_range(span, start, end),
        };
        self.loop_depth += 1;
        let (x, body) = self.unwrapping_block("for", name, bound, body, None);
        self.loop_depth -= 1;
        let (Some(header), Ok(body)) = (header, body) else {
            return Err(Poisoned::typed(Type::Unit));
        };
        // The body keeps its own type, so codegen releases a counted tail value.
        let body_ty = body.ty.clone();
        let ForHeader {
            mut stmts,
            counter,
            limit,
            item,
        } = header;
        let at = |local_id| local(local_id, Type::I64, span);
        let next = ExprKind::Binary {
            op: tir::BinaryOp::Add,
            lhs: Box::new(at(counter)),
            rhs: Box::new(expr(ExprKind::Int(1), Type::I64)),
        };
        // The counter moves on before the body runs, so `continue` needs nothing more.
        let then_block = tir::Block {
            stmts: vec![
                tir::Stmt::Let {
                    local: x,
                    init: item,
                },
                tir::Stmt::Assign {
                    place: tir::Place::Local(counter),
                    value: expr(next, Type::I64),
                },
                tir::Stmt::Expr(expr(ExprKind::Block(body), body_ty)),
            ],
            value: None,
            ty: Type::Unit,
        };
        let cond = ExprKind::Compare {
            operands: vec![at(counter), at(limit)],
            ops: vec![tir::CompareOp::Lt],
        };
        stmts.push(tir::Stmt::Expr(lower_loop(
            then_block,
            span,
            |then_block, exit| ExprKind::If {
                cond: Box::new(expr(cond, Type::Bool)),
                then_block,
                else_block: Some(exit),
            },
        )));
        Ok(tir::Stmt::Expr(block_expr(stmts, None, Type::Unit, span)))
    }

    /// The header of `for x in e`, and the type of `x`: the element type, `None` (`Error`) when
    /// `e` is no array.
    fn for_array(&mut self, span: Span, e: &ast::Expr) -> (Option<ForHeader>, Option<Type>) {
        let checked = self.value(e, None);
        let (element, d) = match type_of(&checked) {
            Some(Type::Array(element)) => (Some((**element).clone()), None),
            Some(Type::Optional(payload)) if matches!(**payload, Type::Array(_)) => {
                (None, Some(optional_array(e)))
            }
            Some(ty) => {
                let found = self.type_name(ty);
                let d = DiagnosticKind::ForNeedsArray { found, at: e.span };
                (None, Some(d.into()))
            }
            None => (None, None),
        };
        self.diagnostics.extend(d);
        let ty = type_of(&checked).cloned().unwrap_or(Type::Unit);
        let array = self.fresh(ty.clone());
        let len = self.fresh(Type::I64);
        let counter = self.fresh_var(Type::I64);
        let (Ok(checked), Some(element)) = (checked, element.clone()) else {
            return (None, element);
        };
        let expr = |kind, ty| tir::Expr { kind, ty, span };
        let read = || local(array, ty.clone(), e.span);
        let stmts = vec![
            tir::Stmt::Let {
                local: array,
                init: checked,
            },
            tir::Stmt::Let {
                local: len,
                init: expr(ExprKind::Len(Box::new(read())), Type::I64),
            },
            tir::Stmt::Let {
                local: counter,
                init: expr(ExprKind::Int(0), Type::I64),
            },
        ];
        let item = ExprKind::Index {
            base: Box::new(read()),
            index: Box::new(local(counter, Type::I64, span)),
        };
        let header = ForHeader {
            stmts,
            counter,
            limit: len,
            item: expr(item, element.clone()),
        };
        (Some(header), Some(element))
    }

    /// The header of `for x in start..end`, and the type of `x`, `i64`.
    fn for_range(
        &mut self,
        span: Span,
        start: &ast::Expr,
        end: &ast::Expr,
    ) -> (Option<ForHeader>, Option<Type>) {
        let start = self.expect(start, Some(&Type::I64));
        let end = self.expect(end, Some(&Type::I64));
        let counter = self.fresh_var(Type::I64);
        let limit = self.fresh(Type::I64);
        let (Ok(start), Ok(end)) = (start, end) else {
            return (None, Some(Type::I64));
        };
        let stmts = vec![
            tir::Stmt::Let {
                local: counter,
                init: start,
            },
            tir::Stmt::Let {
                local: limit,
                init: end,
            },
        ];
        let item = local(counter, Type::I64, span);
        let header = ForHeader {
            stmts,
            counter,
            limit,
            item,
        };
        (Some(header), Some(Type::I64))
    }

    /// `return`, of type `never` even when it fails.
    fn return_stmt(
        &mut self,
        span: Span,
        value: Option<&ast::Expr>,
    ) -> Result<tir::Stmt, Poisoned> {
        let never = || Poisoned::typed(Type::Never);
        let value = match value {
            None => match &self.ret {
                Some(ret) if *ret != Type::Unit => {
                    let want = self.type_name(ret);
                    self.report(DiagnosticKind::ReturnNeedsValue { want, at: span });
                    return Err(never());
                }
                _ => None,
            },
            Some(value) if self.ret == Some(Type::Unit) => {
                if type_of(&self.value(value, None)).is_some() {
                    self.report(DiagnosticKind::UnexpectedReturnValue { at: value.span });
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
    fn jump(
        &mut self,
        span: Span,
        keyword: &'static str,
        kind: ExprKind,
    ) -> Result<tir::Stmt, Poisoned> {
        if self.loop_depth == 0 {
            self.report(DiagnosticKind::JumpOutsideLoop { keyword, at: span });
            self.invalid_jump = true;
            return Err(Poisoned::typed(Type::Never));
        }
        Ok(tir::Stmt::Expr(tir::Expr {
            kind,
            ty: Type::Never,
            span,
        }))
    }

    /// `block`, its scope holding `name` bound to a `ty`, as `keyword` (`if let`, `while let` or
    /// `for`) binds it: the block cannot declare `name` again. The bound local, and the block.
    /// A `for _` binds a fresh unnamed local and declares no name.
    pub(super) fn unwrapping_block(
        &mut self,
        keyword: &'static str,
        name: &ast::Ident,
        ty: Option<Type>,
        block: &ast::Block,
        expected: Option<&Type>,
    ) -> (tir::LocalId, CheckedBlock) {
        self.scopes.push(HashMap::new());
        let bind = if name.name == "_" {
            self.fresh(ty.unwrap_or(Type::Unit))
        } else {
            self.declare(name, ty, BindingKind::Unwrapped(keyword), None)
        };
        (bind, self.scoped_block(block, expected))
    }

    /// A new local with no source name, for a value the lowering binds.
    pub(super) fn fresh(&mut self, ty: Type) -> tir::LocalId {
        self.fresh_local(ty, false)
    }

    /// `fresh`, mutable: a counter the lowering steps.
    fn fresh_var(&mut self, ty: Type) -> tir::LocalId {
        self.fresh_local(ty, true)
    }

    fn fresh_local(&mut self, ty: Type, mutable: bool) -> tir::LocalId {
        let local = tir::LocalId(self.locals.len());
        self.locals.push(tir::Local {
            name: None,
            ty,
            mutable,
        });
        local
    }

    /// When an operand exits and one before it is counted, binds every operand through the
    /// last exiting one to a fresh local, in source order, so each read of them comes after
    /// every exit. The bindings, and the operands with those reads in place.
    pub(super) fn spill(&mut self, operands: Vec<tir::Expr>) -> (Vec<tir::Stmt>, Vec<tir::Expr>) {
        let last_exit = operands.iter().rposition(tir::Expr::exits);
        let first_counted = operands.iter().position(|e| e.ty.is_counted());
        let Some(last_exit) = last_exit.filter(|&exit| first_counted.is_some_and(|c| c < exit))
        else {
            return (Vec::new(), operands);
        };
        let mut spills = Vec::new();
        let operands = operands
            .into_iter()
            .enumerate()
            .map(|(i, operand)| {
                if i > last_exit {
                    return operand;
                }
                let bind = self.fresh(operand.ty.clone());
                let read = local(bind, operand.ty.clone(), operand.span);
                spills.push(tir::Stmt::Let {
                    local: bind,
                    init: operand,
                });
                read
            })
            .collect();
        (spills, operands)
    }

    /// Adds `name` to the innermost scope as a new local; `keyword` is the `mut` of a
    /// `let mut`. A name already in that scope is reported and poisoned.
    pub(super) fn declare(
        &mut self,
        name: &ast::Ident,
        ty: Option<Type>,
        kind: BindingKind,
        keyword: Option<Span>,
    ) -> tir::LocalId {
        let local = tir::LocalId(self.locals.len());
        self.locals.push(tir::Local {
            name: Some(name.name.clone()),
            // An `Error` local never reaches `tir`: the error behind it stops the program.
            ty: ty.clone().unwrap_or(Type::Unit),
            mutable: kind == BindingKind::Var,
        });
        let scope = self
            .scopes
            .last_mut()
            .expect("a scope should be open while a block is checked");
        let mut binding = Binding {
            local,
            ty,
            kind,
            span: name.span,
            keyword,
            reassigned: false,
        };
        if let Some(first) = scope.get(&name.name) {
            self.diagnostics.push(
                DiagnosticKind::AlreadyDeclared {
                    name: name.name.clone(),
                    at: name.span,
                    first: first.span,
                }
                .into(),
            );
            // A use after it may mean either binding, so neither is judged: the name becomes
            // `Error`, can be assigned, and never warns.
            binding = Binding {
                ty: None,
                kind: BindingKind::Var,
                reassigned: true,
                ..binding
            };
        }
        scope.insert(name.name.clone(), binding);
        local
    }

    /// Closes the innermost scope, warning about each `let mut` that was never reassigned.
    pub(super) fn pop_scope(&mut self) {
        let scope = self
            .scopes
            .pop()
            .expect("a scope should be open while a block is checked");
        for (name, b) in scope {
            if let Some(keyword) = b.keyword
                && !b.reassigned
            {
                self.report(DiagnosticKind::NeverReassigned {
                    name,
                    at: b.span,
                    keyword,
                });
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

/// What a `for` binds before its `Loop`: `stmts` bind the source and the counter, which runs
/// to `limit`, and `item` is the value of the loop variable for the current counter.
struct ForHeader {
    stmts: Vec<tir::Stmt>,
    counter: tir::LocalId,
    limit: tir::LocalId,
    item: tir::Expr,
}

/// The type a statement gives its block when it comes last.
fn stmt_type(stmt: &tir::Stmt) -> Type {
    match stmt {
        tir::Stmt::Let { .. } | tir::Stmt::Assign { .. } => Type::Unit,
        tir::Stmt::Expr(e) => e.ty.clone(),
    }
}

/// `while c { b }` is `Loop { If c { b } else { Break } }`, and `while let` the same with an
/// `IfSome`: `branch` makes that node of the body and the `Break` block.
fn lower_loop(
    body: tir::Block,
    span: Span,
    branch: impl FnOnce(tir::Block, tir::Block) -> ExprKind,
) -> tir::Expr {
    let expr = |kind, ty| tir::Expr { kind, ty, span };
    let exit = tir::Block {
        stmts: vec![tir::Stmt::Expr(expr(ExprKind::Break, Type::Never))],
        value: None,
        ty: Type::Never,
    };
    // The `else` branch is `never`, so the branch has the body's type.
    let ty = body.ty.clone();
    let branch = expr(branch(body, exit), ty.clone());
    let looped = tir::Block {
        stmts: Vec::new(),
        value: Some(Box::new(branch)),
        ty,
    };
    expr(ExprKind::Loop(looped), Type::Unit)
}
