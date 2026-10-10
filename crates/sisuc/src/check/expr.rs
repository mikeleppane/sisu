//! Expressions, checked bottom-up and lowered to `tir`.

use super::{Binding, Checked, Checker, Poisoned, block_type, not_found, type_of, value_span};
use crate::ast::{self, ExprKind, UnaryOp};
use crate::diagnostic::{Diagnostic, Span};
use crate::tir::{self, Type};

impl Checker {
    pub(super) fn expr(&mut self, e: &ast::Expr) -> Checked {
        let node = |kind, ty| {
            Ok(tir::Expr {
                kind,
                ty,
                span: e.span,
            })
        };
        match &e.kind {
            ExprKind::Int(n) => node(tir::ExprKind::Int(*n), Type::I64),
            ExprKind::Bool(b) => node(tir::ExprKind::Bool(*b), Type::Bool),
            ExprKind::Name(name) => match self.lookup(name) {
                Some(Binding {
                    local,
                    ty: Some(ty),
                    ..
                }) => node(tir::ExprKind::Local(*local), ty.clone()),
                // `Error`: its diagnostic was recorded where it was bound.
                Some(_) => Err(Poisoned { ty: None }),
                None => {
                    self.diagnostics.push(not_found(name, e.span));
                    Err(Poisoned { ty: None })
                }
            },
            ExprKind::Call { callee, args } => self.call(e.span, callee, args),
            ExprKind::SelfValue | ExprKind::Field { .. } | ExprKind::MethodCall { .. } => {
                panic!("classes land in Task 10")
            }
            ExprKind::Unary { op, operand } => {
                let want = match op {
                    UnaryOp::Neg => Type::I64,
                    UnaryOp::Not => Type::Bool,
                };
                let (operand, mismatched) = self.operand(operand, Some(&want));
                typed(
                    e.span,
                    want,
                    mismatched,
                    operand.ok().map(|operand| tir::ExprKind::Unary {
                        op: *op,
                        operand: Box::new(operand),
                    }),
                )
            }
            ExprKind::Binary { op, lhs, rhs } => self.binary(e.span, *op, lhs, rhs),
            ExprKind::Compare { operands, ops } => {
                // Once an operand is `Error`, the rest expect `Error`, as in `binary`.
                let mut want = Some(&Type::I64);
                let (mut checked, mut mismatched) = (Vec::new(), false);
                for operand in operands {
                    let (operand, m) = self.operand(operand, want);
                    if type_of(&operand).is_none() {
                        want = None;
                    }
                    mismatched |= m;
                    checked.push(operand);
                }
                let operands = checked.into_iter().collect::<Result<Vec<_>, _>>().ok();
                typed(
                    e.span,
                    Type::Bool,
                    mismatched,
                    operands.map(|operands| tir::ExprKind::Compare {
                        operands,
                        ops: ops.clone(),
                    }),
                )
            }
            ExprKind::If {
                cond,
                then_block,
                else_block,
            } => self.if_expr(e.span, cond, then_block, else_block.as_ref()),
        }
    }

    /// An expression whose value is used: not `unit` or `never`.
    pub(super) fn value(&mut self, e: &ast::Expr) -> Checked {
        let checked = self.expr(e);
        let message = match type_of(&checked) {
            Some(Type::Unit) => "expression has no value",
            Some(Type::Never) => "unreachable code",
            _ => return checked,
        };
        self.diagnostics.push(Diagnostic::error(e.span, message));
        Err(Poisoned { ty: None })
    }

    /// A value of type `want`; `None` is `Error`, which every type matches.
    pub(super) fn expect(&mut self, e: &ast::Expr, want: Option<&Type>) -> Checked {
        self.operand(e, want).0
    }

    /// `expect`, and whether it reported a mismatch. A mismatch is the failure of the rule
    /// that wants `want`, so that rule's expression is `Error`.
    fn operand(&mut self, e: &ast::Expr, want: Option<&Type>) -> (Checked, bool) {
        let checked = self.value(e);
        if let (Some(want), Some(got)) = (want, type_of(&checked))
            && want != got
        {
            let d = self.mismatch(e.span, want, got);
            self.diagnostics.push(d);
            return (Err(Poisoned { ty: None }), true);
        }
        (checked, false)
    }

    /// Once an operand is `Error`, the other is checked against `Error` too: with that
    /// operand's type unknown, the operator may be the mistake. So `x + true` draws no
    /// diagnostic when `x` is `Error`, and `true && 1` draws one.
    fn binary(
        &mut self,
        span: Span,
        op: ast::BinaryOp,
        lhs: &ast::Expr,
        rhs: &ast::Expr,
    ) -> Checked {
        let (op, want) = match op {
            ast::BinaryOp::Eq => return self.equal(span, false, lhs, rhs),
            ast::BinaryOp::Ne => return self.equal(span, true, lhs, rhs),
            ast::BinaryOp::Add => (tir::BinaryOp::Add, Type::I64),
            ast::BinaryOp::Sub => (tir::BinaryOp::Sub, Type::I64),
            ast::BinaryOp::Mul => (tir::BinaryOp::Mul, Type::I64),
            ast::BinaryOp::Div => (tir::BinaryOp::Div, Type::I64),
            ast::BinaryOp::Rem => (tir::BinaryOp::Rem, Type::I64),
            ast::BinaryOp::And => (tir::BinaryOp::And, Type::Bool),
            ast::BinaryOp::Or => (tir::BinaryOp::Or, Type::Bool),
        };
        let (l, l_mismatched) = self.operand(lhs, Some(&want));
        let (r, r_mismatched) = self.operand(rhs, type_of(&l).and(Some(&want)));
        let kind = l.ok().zip(r.ok()).map(|(l, r)| tir::ExprKind::Binary {
            op,
            lhs: Box::new(l),
            rhs: Box::new(r),
        });
        typed(span, want, l_mismatched || r_mismatched, kind)
    }

    fn equal(&mut self, span: Span, negated: bool, lhs: &ast::Expr, rhs: &ast::Expr) -> Checked {
        let (l, r) = (self.value(lhs), self.value(rhs));
        if let (Some(lt), Some(rt)) = (type_of(&l), type_of(&r))
            && lt != rt
        {
            let message = format!("cannot compare {} with {}", self.show(lt), self.show(rt));
            self.diagnostics.push(Diagnostic::error(span, message));
            return Err(Poisoned { ty: None });
        }
        let kind = l.ok().zip(r.ok()).map(|(l, r)| tir::ExprKind::Equal {
            negated,
            lhs: Box::new(l),
            rhs: Box::new(r),
        });
        typed(span, Type::Bool, false, kind)
    }

    /// Without an `else`, `unit`. With one, the branches' common type; a `never` branch takes
    /// the other's, then an `Error` branch takes the other's. A condition that is not `bool`
    /// makes the `if` `Error`.
    fn if_expr(
        &mut self,
        span: Span,
        cond: &ast::Expr,
        then_block: &ast::Block,
        else_block: Option<&ast::Block>,
    ) -> Checked {
        let (cond, mismatched) = self.operand(cond, Some(&Type::Bool));
        let then = self.block(then_block);
        let Some(else_block) = else_block else {
            let kind = cond
                .ok()
                .zip(then.ok())
                .map(|(cond, then)| tir::ExprKind::If {
                    cond: Box::new(cond),
                    then_block: then,
                    else_block: None,
                });
            return typed(span, Type::Unit, mismatched, kind);
        };
        let other = self.block(else_block);
        let ty = match (block_type(&then), block_type(&other)) {
            // `never` first, so an `Error` branch and a `never` one make `Error`.
            (Some(Type::Never), ty) | (ty, Some(Type::Never)) => ty.cloned(),
            (None, None) => None,
            (Some(t), None) | (None, Some(t)) => Some(t.clone()),
            (Some(t), Some(o)) if t == o => Some(t.clone()),
            (Some(t), Some(o)) => {
                let d = self.mismatch(value_span(else_block), t, o);
                self.diagnostics.push(d);
                None
            }
        };
        match (cond, then, other, ty) {
            (Ok(cond), Ok(then), Ok(other), Some(ty)) => Ok(tir::Expr {
                kind: tir::ExprKind::If {
                    cond: Box::new(cond),
                    then_block: then,
                    else_block: Some(other),
                },
                ty,
                span,
            }),
            (.., ty) => Err(Poisoned {
                ty: ty.filter(|_| !mismatched),
            }),
        }
    }

    fn call(&mut self, span: Span, callee: &ast::Ident, args: &[ast::Arg]) -> Checked {
        assert!(
            args.iter().all(|a| a.label.is_none()),
            "classes land in Task 10"
        );
        if callee.name == "print" {
            return self.print(span, args);
        }
        let Some(&func) = self.function_ids.get(&callee.name) else {
            self.diagnostics.push(Diagnostic::error(
                callee.span,
                format!("cannot find function `{}`", callee.name),
            ));
            self.stray(args);
            return Err(Poisoned { ty: None });
        };
        let sig = &self.signatures[func.0];
        let (params, ret) = (sig.params.clone(), sig.ret.clone());
        if params.len() != args.len() {
            let plural = if params.len() == 1 { "" } else { "s" };
            self.diagnostics.push(Diagnostic::error(
                span,
                format!(
                    "`{}` takes {} argument{plural}, found {}",
                    callee.name,
                    params.len(),
                    args.len()
                ),
            ));
            self.stray(args);
            return Err(Poisoned { ty: None });
        }
        let (checked, mismatched): (Vec<_>, Vec<_>) = args
            .iter()
            .zip(&params)
            .map(|(arg, param)| self.operand(&arg.value, param.as_ref()))
            .unzip();
        match (checked.into_iter().collect(), ret) {
            (Ok(args), Some(ret)) => Ok(tir::Expr {
                kind: tir::ExprKind::Call { func, args },
                ty: ret,
                span,
            }),
            (_, ret) => Err(Poisoned {
                ty: ret.filter(|_| !mismatched.contains(&true)),
            }),
        }
    }

    fn print(&mut self, span: Span, args: &[ast::Arg]) -> Checked {
        // Not `value()`: a `unit` argument needs this message, not "expression has no value".
        let wrong = || Diagnostic::error(span, "`print` takes one `i64` or `bool`");
        let [arg] = args else {
            self.diagnostics.push(wrong());
            self.stray(args);
            return Err(Poisoned { ty: None });
        };
        let checked = self.expr(&arg.value);
        let error = match type_of(&checked) {
            Some(Type::Unit) => wrong(),
            Some(Type::Never) => Diagnostic::error(arg.value.span, "unreachable code"),
            _ => {
                let kind = checked.ok().map(|arg| tir::ExprKind::Print(Box::new(arg)));
                return typed(span, Type::Unit, false, kind);
            }
        };
        self.diagnostics.push(error);
        Err(Poisoned { ty: None })
    }

    /// Checks the arguments of a call that cannot take them, for their own mistakes.
    fn stray(&mut self, args: &[ast::Arg]) {
        for arg in args {
            // Only its diagnostics matter.
            let _ = self.expr(&arg.value);
        }
    }
}

/// The node `kind` makes, of type `ty`. `kind` is `None` when an operand failed; the node
/// keeps `ty`, which its rule fixes whatever its operands are, unless the rule itself
/// reported a `mismatched` operand: a rule that failed makes its expression `Error`.
fn typed(span: Span, ty: Type, mismatched: bool, kind: Option<tir::ExprKind>) -> Checked {
    match kind {
        Some(kind) => Ok(tir::Expr { kind, ty, span }),
        None => Err(Poisoned {
            ty: (!mismatched).then_some(ty),
        }),
    }
}
