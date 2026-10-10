//! Expressions, checked bottom-up and lowered to `tir`.

use super::{
    Binding, Checked, CheckedBlock, Checker, Poisoned, UNKNOWN, block_expr, block_type, not_found,
    optional_of, type_of, value_span, wrap, wrap_block,
};
use crate::ast::{self, CompareOp, ExprKind, UnaryOp};
use crate::diagnostic::{Diagnostic, Span};
use crate::tir::{self, Type};

impl Checker {
    /// `expected` is the type the context wants, `None` when it wants none. It only types a
    /// `None`, and flows into `if` and `if let` branches and a block's value; it never wraps.
    pub(super) fn expr(&mut self, e: &ast::Expr, expected: Option<&Type>) -> Checked {
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
            ExprKind::NoneLit => self.none(e.span, expected),
            ExprKind::Name(name) => match self.lookup(name) {
                Some(Binding {
                    local,
                    ty: Some(ty),
                    ..
                }) => node(tir::ExprKind::Local(*local), ty.clone()),
                // `Error`: its diagnostic was recorded where it was bound.
                Some(_) => Err(Poisoned::error()),
                None => {
                    self.diagnostics.push(not_found(name, e.span));
                    Err(Poisoned::error())
                }
            },
            ExprKind::Call { callee, args } => self.call(e.span, callee, args),
            ExprKind::SelfValue => {
                let Some(class) = self.self_class else {
                    self.diagnostics.push(Diagnostic::error(
                        e.span,
                        "`self` is only available in a method",
                    ));
                    return Err(Poisoned::error());
                };
                node(tir::ExprKind::Local(tir::LocalId(0)), Type::Class(class))
            }
            ExprKind::Field { base, name, safe } => {
                self.member(e.span, *safe, base, |c, checked| {
                    c.field(e.span, checked, base, name)
                })
            }
            ExprKind::MethodCall {
                receiver,
                method,
                args,
                safe,
            } => self.member(e.span, *safe, receiver, |c, checked| {
                c.method_call(e.span, checked, receiver, method, args)
            }),
            ExprKind::Coalesce { lhs, rhs } => self.coalesce(e.span, lhs, rhs),
            ExprKind::Is { lhs, rhs } => self.is(e.span, lhs, rhs),
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
            ExprKind::Compare { operands, ops } => self.compare(e.span, operands, ops),
            ExprKind::If {
                cond,
                then_block,
                else_block,
            } => self.if_expr(e.span, cond, then_block, else_block.as_ref(), expected),
            ExprKind::IfLet {
                name,
                value,
                then_block,
                else_block,
            } => self.if_let(
                e.span,
                name,
                value,
                then_block,
                else_block.as_ref(),
                expected,
            ),
        }
    }

    /// A comparison chain: every operand is an `i64`.
    fn compare(&mut self, span: Span, operands: &[ast::Expr], ops: &[CompareOp]) -> Checked {
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
            span,
            Type::Bool,
            mismatched,
            operands.map(|operands| tir::ExprKind::Compare {
                operands,
                ops: ops.to_vec(),
            }),
        )
    }

    /// An expression whose value is used: not `unit` or `never`.
    pub(super) fn value(&mut self, e: &ast::Expr, expected: Option<&Type>) -> Checked {
        let checked = self.expr(e, expected);
        let message = match type_of(&checked) {
            Some(Type::Unit) => "expression has no value",
            Some(Type::Never) => "unreachable code",
            _ => return checked,
        };
        self.diagnostics.push(Diagnostic::error(e.span, message));
        Err(Poisoned::error())
    }

    /// A value of type `want`; `None` is `Error`, which every type matches. A `T` where `want`
    /// is a `T?` is wrapped.
    pub(super) fn expect(&mut self, e: &ast::Expr, want: Option<&Type>) -> Checked {
        self.operand(e, want).0
    }

    /// `expect`, and whether it reported a mismatch. A mismatch is the failure of the rule
    /// that wants `want`, so that rule's expression is `Error`.
    fn operand(&mut self, e: &ast::Expr, want: Option<&Type>) -> (Checked, bool) {
        let checked = self.value(e, Some(want.unwrap_or(&UNKNOWN)));
        // `want` was rejected inside `e`, which `against` cannot tell from an `Error` operand.
        if matches!(checked, Err(Poisoned { mismatch: true, .. })) {
            return (checked, true);
        }
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
            if let Type::Optional(payload) = want
                && **payload == *got
            {
                let wrapped = match checked {
                    Ok(e) => Ok(wrap(e, want)),
                    Err(_) => Err(Poisoned::typed(want.clone())),
                };
                return (wrapped, false);
            }
            let d = self.mismatch(span, want, got);
            self.diagnostics.push(d);
            return (Err(Poisoned::error()), true);
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

    /// `lhs == rhs`, or `!=`. The side that is not a bare `None` is checked first, and the
    /// other expects its type made optional. A bare `None` beside a value that is not
    /// optional is reported: the answer is fixed.
    fn equal(&mut self, span: Span, negated: bool, lhs: &ast::Expr, rhs: &ast::Expr) -> Checked {
        let swapped = none_literal(lhs) && !none_literal(rhs);
        let (first, second) = if swapped { (rhs, lhs) } else { (lhs, rhs) };
        let checked = self.value(first, None);
        let want = type_of(&checked).map_or_else(|| UNKNOWN.clone(), |ty| optional_of(ty.clone()));
        let other = self.value(second, Some(&want));
        // `second` is the bare `None`, if either is.
        if none_literal(second)
            && let Some(ty) = type_of(&checked)
            && !matches!(ty, Type::Optional(_))
        {
            let message = format!("{} is not optional, so it is never `None`", quoted(first));
            self.diagnostics.push(Diagnostic::error(span, message));
            return Err(Poisoned::error());
        }
        let (l, r) = if swapped {
            (other, checked)
        } else {
            (checked, other)
        };
        self.compared(span, l, r, |lhs, rhs| tir::ExprKind::Equal {
            negated,
            lhs,
            rhs,
        })
    }

    /// `lhs is rhs`: each a class or its optional, of one class.
    fn is(&mut self, span: Span, lhs: &ast::Expr, rhs: &ast::Expr) -> Checked {
        if none_literal(lhs) || none_literal(rhs) {
            let other = if none_literal(lhs) { rhs } else { lhs };
            let mut d = Diagnostic::error(span, "use `==` to test for `None`");
            if let Some(path) = path(other) {
                d = d.help(format!("`{path} == None`"));
            }
            self.diagnostics.push(d);
            // Only its own diagnostics matter.
            let _ = self.value(other, Some(&UNKNOWN));
            return Err(Poisoned::error());
        }
        let (l, r) = (self.value(lhs, None), self.value(rhs, None));
        for (side, checked) in [(lhs, &l), (rhs, &r)] {
            if let Some(ty) = type_of(checked)
                && !ty.is_counted()
            {
                let message = format!("`is` needs objects; found {}", self.show(ty));
                self.diagnostics
                    .push(Diagnostic::error(side.span, message).help("use `==` to compare values"));
                return Err(Poisoned::error());
            }
        }
        self.compared(span, l, r, |lhs, rhs| tir::ExprKind::Is { lhs, rhs })
    }

    /// The `bool` node `kind` makes of `l` and `r`, of one type once a `T` beside a `T?` is
    /// wrapped; other types that differ are reported.
    fn compared(
        &mut self,
        span: Span,
        l: Checked,
        r: Checked,
        kind: impl FnOnce(Box<tir::Expr>, Box<tir::Expr>) -> tir::ExprKind,
    ) -> Checked {
        let (l, r) = match (type_of(&l).cloned(), type_of(&r).cloned()) {
            (Some(lt), Some(rt)) if lt != rt => {
                if fits(&lt, &rt) {
                    (l, r.map(|e| wrap(e, &lt)))
                } else if fits(&rt, &lt) {
                    (l.map(|e| wrap(e, &rt)), r)
                } else {
                    let message =
                        format!("cannot compare {} with {}", self.show(&lt), self.show(&rt));
                    self.diagnostics.push(Diagnostic::error(span, message));
                    return Err(Poisoned::error());
                }
            }
            _ => (l, r),
        };
        match (l, r) {
            (Ok(l), Ok(r)) => Ok(self.spilled(span, Type::Bool, vec![l, r], |operands| {
                let [lhs, rhs]: [tir::Expr; 2] = operands.try_into().expect("two operands");
                kind(Box::new(lhs), Box::new(rhs))
            })),
            _ => Err(Poisoned::typed(Type::Bool)),
        }
    }

    /// `lhs ?? rhs`: `rhs` is a `T` or a `T?` beside the `T?` `lhs`, and gives the result its
    /// type.
    fn coalesce(&mut self, span: Span, lhs: &ast::Expr, rhs: &ast::Expr) -> Checked {
        let scrutinee = self.value(lhs, None);
        let Some(Type::Optional(payload)) = type_of(&scrutinee) else {
            if type_of(&scrutinee).is_some() {
                self.diagnostics.push(Diagnostic::error(
                    lhs.span,
                    "the left side of `??` is not optional",
                ));
            }
            // Only its own diagnostics matter.
            let _ = self.value(rhs, Some(&UNKNOWN));
            return Err(Poisoned::error());
        };
        let payload = (**payload).clone();
        let optional = optional_of(payload.clone());
        let other = self.value(rhs, Some(&optional));
        let ty = match type_of(&other) {
            Some(ty) if *ty == payload || *ty == optional => ty.clone(),
            Some(ty) => {
                let d = self.mismatch(rhs.span, &payload, ty);
                self.diagnostics.push(d);
                return Err(Poisoned::error());
            }
            None => return Err(Poisoned::error()),
        };
        let bind = self.fresh(payload.clone());
        let read = local(bind, payload, lhs.span);
        let value = if read.ty == ty { read } else { wrap(read, &ty) };
        match (scrutinee, other) {
            (Ok(scrutinee), Ok(other)) => Ok(if_some(bind, scrutinee, value, Some(other), span)),
            _ => Err(Poisoned::typed(ty)),
        }
    }

    /// `receiver.member`, which `member` checks on the checked receiver. With `safe`, it is
    /// `receiver?.member`, and `member` checks it on a fresh local bound to the receiver's
    /// value. Its type `U` makes the result a `U?`, or `unit` for a method with no value. A
    /// receiver that is not optional is reported, and the member still checked on it.
    fn member(
        &mut self,
        span: Span,
        safe: bool,
        receiver: &ast::Expr,
        member: impl FnOnce(&mut Self, Checked) -> Checked,
    ) -> Checked {
        let scrutinee = self.value(receiver, None);
        if !safe {
            return member(self, scrutinee);
        }
        let Some(Type::Optional(payload)) = type_of(&scrutinee) else {
            if type_of(&scrutinee).is_some() {
                let message = format!("{} is not optional", quoted(receiver));
                self.diagnostics
                    .push(Diagnostic::error(receiver.span, message).help("use `.`"));
            }
            // Only its own diagnostics matter.
            let _ = member(self, Err(Poisoned::of(type_of(&scrutinee).cloned())));
            return Err(Poisoned::error());
        };
        let payload = (**payload).clone();
        let bind = self.fresh(payload.clone());
        let member = member(self, Ok(local(bind, payload, receiver.span)));
        let ty = match type_of(&member) {
            Some(Type::Unit) => Type::Unit,
            Some(ty) => optional_of(ty.clone()),
            None => return Err(Poisoned::error()),
        };
        let (Ok(scrutinee), Ok(member)) = (scrutinee, member) else {
            return Err(Poisoned::typed(ty));
        };
        let none = (ty != Type::Unit).then(|| tir::Expr {
            kind: tir::ExprKind::None,
            ty: ty.clone(),
            span,
        });
        let value = if member.ty == ty {
            member
        } else {
            wrap(member, &ty)
        };
        Ok(if_some(bind, scrutinee, value, none, span))
    }

    /// A `None`, of the optional type `expected`.
    fn none(&mut self, span: Span, expected: Option<&Type>) -> Checked {
        let (d, mismatch) = match expected {
            Some(ty @ Type::Optional(_)) => {
                return Ok(tir::Expr {
                    kind: tir::ExprKind::None,
                    ty: ty.clone(),
                    span,
                });
            }
            Some(Type::Never) => return Err(Poisoned::error()),
            Some(ty) => (
                Diagnostic::error(span, format!("expected {}, found `None`", self.show(ty))),
                true,
            ),
            None => (
                Diagnostic::error(span, "cannot infer the type of `None`")
                    .help("write the type: `let x: Tree? = None`"),
                false,
            ),
        };
        self.diagnostics.push(d);
        Err(Poisoned { ty: None, mismatch })
    }

    /// Without an `else`, `unit`. With one, the branches joined (see `branches`). A condition
    /// that is not `bool` makes the `if` `Error`.
    fn if_expr(
        &mut self,
        span: Span,
        cond: &ast::Expr,
        then_block: &ast::Block,
        else_block: Option<&ast::Block>,
        expected: Option<&Type>,
    ) -> Checked {
        let (cond, mismatched) = self.operand(cond, Some(&Type::Bool));
        let (then, other, ty) = self.branches(then_block, else_block, expected, |c, expected| {
            c.block(then_block, expected)
        });
        // A failed join keeps its own `mismatch`.
        let ty = match ty {
            Ok(_) if mismatched => Err(Poisoned::error()),
            ty => ty,
        };
        match (cond, then, other.transpose(), ty) {
            (Ok(cond), Ok(then_block), Ok(else_block), Ok(ty)) => Ok(tir::Expr {
                kind: tir::ExprKind::If {
                    cond: Box::new(cond),
                    then_block,
                    else_block,
                },
                ty,
                span,
            }),
            (.., ty) => Err(ty.map_or_else(|poisoned| poisoned, Poisoned::typed)),
        }
    }

    /// `if let name = value { ... }`, typed as `if`; `name` is in scope in the first block.
    /// A `value` that is not optional makes it `Error`.
    fn if_let(
        &mut self,
        span: Span,
        name: &ast::Ident,
        value: &ast::Expr,
        then_block: &ast::Block,
        else_block: Option<&ast::Block>,
        expected: Option<&Type>,
    ) -> Checked {
        let (scrutinee, bound, mismatched) = self.unwrapped("if let", value);
        let mut bind = None;
        let (then, other, ty) = self.branches(then_block, else_block, expected, |c, expected| {
            let (local, block) = c.unwrapping_block("if let", name, bound, then_block, expected);
            bind = Some(local);
            block
        });
        let bind = bind.expect("`branches` checks the first block");
        // A failed join keeps its own `mismatch`.
        let ty = match ty {
            Ok(_) if mismatched => Err(Poisoned::error()),
            ty => ty,
        };
        match (scrutinee, then, other.transpose(), ty) {
            (Ok(scrutinee), Ok(then_block), Ok(else_block), Ok(ty)) => Ok(tir::Expr {
                kind: tir::ExprKind::IfSome {
                    bind,
                    scrutinee: Box::new(scrutinee),
                    then_block,
                    else_block,
                },
                ty,
                span,
            }),
            (.., ty) => Err(ty.map_or_else(|poisoned| poisoned, Poisoned::typed)),
        }
    }

    /// The optional `value` that `keyword` (`if let` or `while let`) unwraps, the type it binds
    /// (`None` is `Error`), and whether `value` was reported for not being optional.
    pub(super) fn unwrapped(
        &mut self,
        keyword: &str,
        value: &ast::Expr,
    ) -> (Checked, Option<Type>, bool) {
        let scrutinee = self.value(value, None);
        match type_of(&scrutinee) {
            Some(Type::Optional(payload)) => {
                let bound = Some((**payload).clone());
                (scrutinee, bound, false)
            }
            Some(ty) => {
                let d = Diagnostic::error(
                    value.span,
                    format!("`{keyword}` needs an optional; this is {}", self.show(ty)),
                );
                self.diagnostics.push(d);
                (Err(Poisoned::error()), None, true)
            }
            None => (scrutinee, None, false),
        }
    }

    /// The blocks of an `if` or `if let`, `then` checking the first, and their type: `unit`
    /// without an `else`. With no `expected` type, the branch whose value is not a bare `None`
    /// is checked first, and the other expects its type made optional. Then a `never` branch,
    /// and after it an `Error` one, takes the other's type; a `T` and a `T?` join to `T?`, the
    /// `T` branch wrapped; other types that differ are reported. A join that fails for a
    /// branch that rejected `expected` is a mismatch too.
    fn branches(
        &mut self,
        then_block: &ast::Block,
        else_block: Option<&ast::Block>,
        expected: Option<&Type>,
        then: impl FnOnce(&mut Self, Option<&Type>) -> CheckedBlock,
    ) -> (CheckedBlock, Option<CheckedBlock>, Result<Type, Poisoned>) {
        let Some(else_block) = else_block else {
            return (then(self, None), None, Ok(Type::Unit));
        };
        let (mut first, mut other);
        if expected.is_none() && bare_none(then_block) && !bare_none(else_block) {
            other = self.block(else_block, None);
            let after = after_branch(block_type(&other));
            first = then(self, after.as_ref());
        } else {
            first = then(self, expected);
            let after = after_branch(block_type(&first));
            other = self.block(else_block, expected.or(after.as_ref()));
        }
        let (t, o) = (block_type(&first).cloned(), block_type(&other).cloned());
        let ty = match (t, o) {
            // `never` first, so an `Error` branch and a `never` one make `Error`.
            (Some(Type::Never), ty) | (ty, Some(Type::Never)) => ty,
            (None, None) => None,
            (Some(t), None) | (None, Some(t)) => Some(t),
            (Some(t), Some(o)) if t == o => Some(t),
            (Some(t), Some(o)) if after_branch(Some(&t)).as_ref() == Some(&o) => {
                wrap_block(&mut first, &o);
                Some(o)
            }
            (Some(t), Some(o)) if after_branch(Some(&o)).as_ref() == Some(&t) => {
                wrap_block(&mut other, &t);
                Some(t)
            }
            // With a type expected, the branch that fits it neither as is nor as its payload is
            // the mistake; without one, the `else` is, against the first branch.
            (Some(t), Some(o)) => {
                let d = match expected.filter(|w| **w != UNKNOWN) {
                    Some(w) if fits(w, &t) => self.mismatch(value_span(else_block), w, &o),
                    Some(w) => self.mismatch(value_span(then_block), w, &t),
                    None => self.mismatch(value_span(else_block), &t, &o),
                };
                self.diagnostics.push(d);
                None
            }
        };
        let mismatch = [&first, &other]
            .iter()
            .any(|b| matches!(b, Err(Poisoned { mismatch: true, .. })));
        let ty = ty.ok_or(Poisoned { ty: None, mismatch });
        (first, Some(other), ty)
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
            self.labels(args);
            self.stray(args);
            return Err(Poisoned::error());
        };
        self.invoke(span, func, None, args)
    }

    /// `receiver.method(args)`, `receiver` checked from `receiver_expr`.
    fn method_call(
        &mut self,
        span: Span,
        receiver: Checked,
        receiver_expr: &ast::Expr,
        method: &ast::Ident,
        args: &[ast::Arg],
    ) -> Checked {
        let Some(func) = self.find_method(type_of(&receiver), receiver_expr, method) else {
            self.labels(args);
            self.stray(args);
            return Err(Poisoned::error());
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
            return Err(Poisoned::error());
        }
        let (checked, mismatched): (Vec<_>, Vec<_>) = args
            .iter()
            .zip(&params)
            .map(|(arg, param)| self.operand(&arg.value, param.as_ref()))
            .unzip();
        let failed = labeled || mismatched.contains(&true);
        match (receiver.into_iter().chain(checked).collect(), ret) {
            (Ok(args), Some(ret)) if !failed => {
                Ok(self.spilled(span, ret, args, |args| tir::ExprKind::Call { func, args }))
            }
            (_, ret) => Err(Poisoned::of(ret.filter(|_| !failed))),
        }
    }

    /// The node `kind` makes of `operands`, in a block after the bindings `spill` makes of them.
    fn spilled(
        &mut self,
        span: Span,
        ty: Type,
        operands: Vec<tir::Expr>,
        kind: impl FnOnce(Vec<tir::Expr>) -> tir::ExprKind,
    ) -> tir::Expr {
        let (stmts, operands) = self.spill(operands);
        let node = tir::Expr {
            kind: kind(operands),
            ty: ty.clone(),
            span,
        };
        if stmts.is_empty() {
            return node;
        }
        block_expr(stmts, Some(node), ty, span)
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
                checked.push(self.expr(&arg.value, Some(&UNKNOWN)));
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
            Ok(args) if !failed => {
                Ok(self.spilled(span, ty, args, |args| tir::ExprKind::New { class, args }))
            }
            _ => Err(Poisoned::of((!failed).then_some(ty))),
        }
    }

    /// `base.name`, `base` checked from `receiver`.
    fn field(
        &mut self,
        span: Span,
        base: Checked,
        receiver: &ast::Expr,
        name: &ast::Ident,
    ) -> Checked {
        let Some((class, index)) = self.find_field(type_of(&base), receiver, name) else {
            return Err(Poisoned::error());
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
            (_, ty) => Err(Poisoned::of(ty)),
        }
    }

    /// The class and index of field `name` of `receiver`, a value of type `base`. `None` when
    /// there is none, reported unless `base` is `Error`.
    pub(super) fn find_field(
        &mut self,
        base: Option<&Type>,
        receiver: &ast::Expr,
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
            Type::Optional(_) => may_be_none(receiver),
            _ => no_member("field", &self.show(ty), name),
        };
        self.diagnostics.push(d);
        None
    }

    /// The method `name` of `receiver`, a value of type `ty`. `None` when there is none,
    /// reported unless `ty` is `Error`.
    fn find_method(
        &mut self,
        ty: Option<&Type>,
        receiver: &ast::Expr,
        name: &ast::Ident,
    ) -> Option<tir::FuncId> {
        let ty = ty?;
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
            Type::Optional(_) => may_be_none(receiver),
            _ => no_member("method", &self.show(ty), name),
        };
        self.diagnostics.push(d);
        None
    }

    fn print(&mut self, span: Span, args: &[ast::Arg]) -> Checked {
        // Not `value()`: a `unit` argument needs this message, not "expression has no value".
        let wrong = || Diagnostic::error(span, "`print` takes one `i64` or `bool`");
        let labeled = self.labels(args);
        let [arg] = args else {
            self.diagnostics.push(wrong());
            self.stray(args);
            return Err(Poisoned::error());
        };
        let checked = self.expr(&arg.value, None);
        let error = match type_of(&checked) {
            Some(Type::Unit | Type::Class(_) | Type::Optional(_)) => wrong(),
            Some(Type::Never) => Diagnostic::error(arg.value.span, "unreachable code"),
            _ => {
                let kind = checked
                    .ok()
                    .filter(|_| !labeled)
                    .map(|arg| tir::ExprKind::Print(Box::new(arg)));
                return typed(span, Type::Unit, labeled, kind);
            }
        };
        self.diagnostics.push(error);
        Err(Poisoned::error())
    }

    /// Checks the arguments of a call that cannot take them, for their own mistakes.
    fn stray(&mut self, args: &[ast::Arg]) {
        for arg in args {
            // Only its diagnostics matter.
            let _ = self.expr(&arg.value, Some(&UNKNOWN));
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

/// Whether `block`'s value is the bare literal `None`.
fn bare_none(block: &ast::Block) -> bool {
    matches!(
        block.stmts.last(),
        Some(ast::Stmt { kind: ast::StmtKind::Expr(e), .. }) if none_literal(e)
    )
}

/// Whether `e` is the bare literal `None`.
fn none_literal(e: &ast::Expr) -> bool {
    matches!(e.kind, ExprKind::NoneLit)
}

/// A read of `local`, of type `ty`.
pub(super) fn local(local: tir::LocalId, ty: Type, span: Span) -> tir::Expr {
    tir::Expr {
        kind: tir::ExprKind::Local(local),
        ty,
        span,
    }
}

/// An `IfSome` over `scrutinee` whose blocks hold only `then` and `other`, of `then`'s type.
fn if_some(
    bind: tir::LocalId,
    scrutinee: tir::Expr,
    then: tir::Expr,
    other: Option<tir::Expr>,
    span: Span,
) -> tir::Expr {
    let ty = then.ty.clone();
    let block = |value: Option<tir::Expr>| tir::Block {
        stmts: Vec::new(),
        value: value.map(Box::new),
        ty: ty.clone(),
    };
    let (then_block, else_block) = (block(Some(then)), Some(block(other)));
    tir::Expr {
        kind: tir::ExprKind::IfSome {
            bind,
            scrutinee: Box::new(scrutinee),
            then_block,
            else_block,
        },
        ty,
        span,
    }
}

/// Whether a branch of type `ty` fits the expected `want`: as is, or as its payload.
fn fits(want: &Type, ty: &Type) -> bool {
    want == ty || matches!(want, Type::Optional(payload) if **payload == *ty)
}

/// What one branch of an `if` expects after the other, of type `ty`, when the `if` expects
/// nothing: `ty` made optional; `Error` after `Error`; nothing after `unit` or `never`.
fn after_branch(ty: Option<&Type>) -> Option<Type> {
    match ty {
        None => Some(UNKNOWN.clone()),
        Some(Type::Unit | Type::Never) => None,
        Some(ty) => Some(optional_of(ty.clone())),
    }
}

/// "`node` may be `None`", at the optional `receiver` of a `.`.
fn may_be_none(receiver: &ast::Expr) -> Diagnostic {
    Diagnostic::error(receiver.span, format!("{} may be `None`", quoted(receiver)))
        .help("use `?.`, or unwrap it with `if let`")
}

/// `e` as messages quote it: its path in backticks, or "this value".
fn quoted(e: &ast::Expr) -> String {
    path(e).map_or_else(|| "this value".to_owned(), |p| format!("`{p}`"))
}

/// A name, `self`, or a chain of `.` and `?.` fields over one, as written.
fn path(e: &ast::Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Name(name) => Some(name.clone()),
        ExprKind::SelfValue => Some("self".to_owned()),
        ExprKind::Field { base, name, safe } => {
            let dot = if *safe { "?." } else { "." };
            Some(format!("{}{dot}{}", path(base)?, name.name))
        }
        _ => None,
    }
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
        None => Err(Poisoned::of((!mismatched).then_some(ty))),
    }
}
