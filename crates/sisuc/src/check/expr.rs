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
            ExprKind::SelfValue => {
                let Some(class) = self.self_class else {
                    self.diagnostics.push(Diagnostic::error(
                        e.span,
                        "`self` is only available in a method",
                    ));
                    return Err(Poisoned { ty: None });
                };
                node(tir::ExprKind::Local(tir::LocalId(0)), Type::Class(class))
            }
            ExprKind::Field { base, name } => self.field(e.span, base, name),
            ExprKind::MethodCall {
                receiver,
                method,
                args,
            } => self.method_call(e.span, receiver, method, args),
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
        self.against(e.span, checked, want)
    }

    /// `operand` for a value already checked, found at `span`.
    pub(super) fn against(
        &mut self,
        span: Span,
        checked: Checked,
        want: Option<&Type>,
    ) -> (Checked, bool) {
        if let (Some(want), Some(got)) = (want, type_of(&checked))
            && want != got
        {
            let d = self.mismatch(span, want, got);
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
        let Some((tir_op, want)) = operator(op) else {
            return self.equal(span, op == ast::BinaryOp::Ne, lhs, rhs);
        };
        let lhs = self.operand(lhs, Some(&want));
        self.arithmetic(span, tir_op, want, lhs, rhs)
    }

    /// `lhs op rhs`, its `lhs` already checked against `want`, the type `op` takes.
    pub(super) fn arithmetic(
        &mut self,
        span: Span,
        op: tir::BinaryOp,
        want: Type,
        (l, l_mismatched): (Checked, bool),
        rhs: &ast::Expr,
    ) -> Checked {
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
        if callee.name == "print" {
            return self.print(span, args);
        }
        if let Some(&class) = self.class_ids.get(&callee.name) {
            return self.construct(span, class, args);
        }
        let Some(&func) = self.function_ids.get(&callee.name) else {
            self.diagnostics.push(Diagnostic::error(
                callee.span,
                format!("cannot find function `{}`", callee.name),
            ));
            self.stray(args);
            return Err(Poisoned { ty: None });
        };
        self.invoke(span, func, None, args)
    }

    fn method_call(
        &mut self,
        span: Span,
        receiver: &ast::Expr,
        method: &ast::Ident,
        args: &[ast::Arg],
    ) -> Checked {
        let receiver = self.value(receiver);
        let Some(func) = self.find_method(type_of(&receiver), method) else {
            self.stray(args);
            return Err(Poisoned { ty: None });
        };
        self.invoke(span, func, Some(receiver), args)
    }

    /// A call of `func`. `receiver` is a method's `self`; `args` match the other parameters.
    fn invoke(
        &mut self,
        span: Span,
        func: tir::FuncId,
        receiver: Option<Checked>,
        args: &[ast::Arg],
    ) -> Checked {
        let labeled = self.labels(args);
        let sig = &self.signatures[func.0];
        let (name, params, ret) = (sig.name.clone(), sig.params.clone(), sig.ret.clone());
        if params.len() != args.len() {
            let plural = if params.len() == 1 { "" } else { "s" };
            self.diagnostics.push(Diagnostic::error(
                span,
                format!(
                    "`{name}` takes {} argument{plural}, found {}",
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
        let failed = labeled || mismatched.contains(&true);
        match (receiver.into_iter().chain(checked).collect(), ret) {
            (Ok(args), Some(ret)) if !failed => Ok(tir::Expr {
                kind: tir::ExprKind::Call { func, args },
                ty: ret,
                span,
            }),
            (_, ret) => Err(Poisoned {
                ty: ret.filter(|_| !failed),
            }),
        }
    }

    /// Reports each labeled argument, as only a constructor takes labels. Whether there was one.
    fn labels(&mut self, args: &[ast::Arg]) -> bool {
        let mut labeled = false;
        for label in args.iter().filter_map(|a| a.label.as_ref()) {
            self.diagnostics.push(Diagnostic::error(
                label.span,
                "labels on arguments come in milestone 10",
            ));
            labeled = true;
        }
        labeled
    }

    /// `C(f: e, ...)` names every field of `C`, in declaration order. Only the first mistake
    /// is reported; every argument is still checked for its own.
    fn construct(&mut self, span: Span, class: tir::ClassId, args: &[ast::Arg]) -> Checked {
        let info = &self.classes[class.0];
        let name = info.name.name.clone();
        let fields: Vec<(String, Option<Type>)> = info
            .fields
            .iter()
            .map(|f| (f.name.name.clone(), f.ty.clone()))
            .collect();
        let names: Vec<_> = fields.iter().map(|(f, _)| format!("`{f}`")).collect();
        let order = format!("name the fields in declaration order: {}", names.join(", "));
        let mut failed = false;
        let mut checked = Vec::with_capacity(args.len());
        for (i, arg) in args.iter().enumerate() {
            if !failed && let Some(d) = misplaced(&name, &fields, &order, i, arg) {
                self.diagnostics.push(d);
                failed = true;
            }
            if failed {
                checked.push(self.expr(&arg.value));
            } else {
                let (arg, mismatched) = self.operand(&arg.value, fields[i].1.as_ref());
                failed = mismatched;
                checked.push(arg);
            }
        }
        if !failed && let Some((missing, _)) = fields.get(args.len()) {
            self.diagnostics
                .push(Diagnostic::error(span, format!("missing field `{missing}`")).help(order));
            failed = true;
        }
        let ty = Type::Class(class);
        match checked.into_iter().collect() {
            Ok(args) if !failed => Ok(tir::Expr {
                kind: tir::ExprKind::New { class, args },
                ty,
                span,
            }),
            _ => Err(Poisoned {
                ty: (!failed).then_some(ty),
            }),
        }
    }

    /// `base.name`.
    fn field(&mut self, span: Span, base: &ast::Expr, name: &ast::Ident) -> Checked {
        let base = self.value(base);
        let Some((class, index)) = self.find_field(type_of(&base), name) else {
            return Err(Poisoned { ty: None });
        };
        let ty = self.classes[class.0].fields[index].ty.clone();
        match (base, ty) {
            (Ok(base), Some(ty)) => Ok(tir::Expr {
                kind: tir::ExprKind::Field {
                    base: Box::new(base),
                    index,
                },
                ty,
                span,
            }),
            (_, ty) => Err(Poisoned { ty }),
        }
    }

    /// The class and index of field `name` of a value of type `base`. `None` when there is
    /// none, reported unless `base` is `Error`.
    pub(super) fn find_field(
        &mut self,
        base: Option<&Type>,
        name: &ast::Ident,
    ) -> Option<(tir::ClassId, usize)> {
        let ty = base?;
        let d = match ty {
            Type::Class(class) => {
                let info = &self.classes[class.0];
                if let Some(index) = info.fields.iter().position(|f| f.name.name == name.name) {
                    return Some((*class, index));
                }
                if info.methods.contains_key(&name.name) {
                    Diagnostic::error(
                        name.span,
                        format!(
                            "`{}` is a method of {}, not a field",
                            name.name,
                            self.show(ty)
                        ),
                    )
                    .help(format!("call it: `{}()`", name.name))
                } else {
                    no_member("field", &self.show(ty), name)
                }
            }
            _ => no_member("field", &self.show(ty), name),
        };
        self.diagnostics.push(d);
        None
    }

    /// The method `name` of a value of type `receiver`. `None` when there is none, reported
    /// unless `receiver` is `Error`.
    fn find_method(&mut self, receiver: Option<&Type>, name: &ast::Ident) -> Option<tir::FuncId> {
        let ty = receiver?;
        let d = match ty {
            Type::Class(class) => {
                let info = &self.classes[class.0];
                if let Some(&func) = info.methods.get(&name.name) {
                    return Some(func);
                }
                if info.fields.iter().any(|f| f.name.name == name.name) {
                    Diagnostic::error(
                        name.span,
                        format!(
                            "`{}` is a field of {}, not a method",
                            name.name,
                            self.show(ty)
                        ),
                    )
                } else {
                    no_member("method", &self.show(ty), name)
                }
            }
            _ => no_member("method", &self.show(ty), name),
        };
        self.diagnostics.push(d);
        None
    }

    fn print(&mut self, span: Span, args: &[ast::Arg]) -> Checked {
        // Not `value()`: a `unit` argument needs this message, not "expression has no value".
        let wrong = || Diagnostic::error(span, "`print` takes one `i64` or `bool`");
        if self.labels(args) {
            self.stray(args);
            return Err(Poisoned { ty: None });
        }
        let [arg] = args else {
            self.diagnostics.push(wrong());
            self.stray(args);
            return Err(Poisoned { ty: None });
        };
        let checked = self.expr(&arg.value);
        let error = match type_of(&checked) {
            Some(Type::Unit | Type::Class(_)) => wrong(),
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

/// The operator and the type it takes, for all but `==` and `!=`.
pub(super) fn operator(op: ast::BinaryOp) -> Option<(tir::BinaryOp, Type)> {
    Some(match op {
        ast::BinaryOp::Eq | ast::BinaryOp::Ne => return None,
        ast::BinaryOp::Add => (tir::BinaryOp::Add, Type::I64),
        ast::BinaryOp::Sub => (tir::BinaryOp::Sub, Type::I64),
        ast::BinaryOp::Mul => (tir::BinaryOp::Mul, Type::I64),
        ast::BinaryOp::Div => (tir::BinaryOp::Div, Type::I64),
        ast::BinaryOp::Rem => (tir::BinaryOp::Rem, Type::I64),
        ast::BinaryOp::And => (tir::BinaryOp::And, Type::Bool),
        ast::BinaryOp::Or => (tir::BinaryOp::Or, Type::Bool),
    })
}

/// "`P` has no field `z`": `owner` is the type as `show` quotes it.
fn no_member(kind: &str, owner: &str, name: &ast::Ident) -> Diagnostic {
    Diagnostic::error(name.span, format!("{owner} has no {kind} `{}`", name.name))
}

/// The mistake in argument `i` of a constructor of `class`, if any: `order` is the help
/// that lists the fields.
fn misplaced(
    class: &str,
    fields: &[(String, Option<Type>)],
    order: &str,
    i: usize,
    arg: &ast::Arg,
) -> Option<Diagnostic> {
    let Some((field, _)) = fields.get(i) else {
        let start = arg.label.as_ref().map_or(arg.value.span, |l| l.span).start;
        let plural = if fields.len() == 1 { "" } else { "s" };
        return Some(Diagnostic::error(
            Span::new(start, arg.value.span.end),
            format!(
                "too many arguments: `{class}` has {} field{plural}",
                fields.len()
            ),
        ));
    };
    let Some(label) = &arg.label else {
        return Some(
            Diagnostic::error(arg.value.span, "this argument needs a label")
                .help(format!("label it with its field: `{field}: ...`")),
        );
    };
    if label.name == *field {
        None
    } else if fields.iter().any(|(f, _)| *f == label.name) {
        Some(
            Diagnostic::error(
                label.span,
                format!("expected field `{field}` here, found `{}`", label.name),
            )
            .help(order),
        )
    } else {
        Some(no_member("field", &format!("`{class}`"), label))
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
