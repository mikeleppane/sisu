//! Expressions, checked bottom-up and lowered to `tir`.

use super::prelude::PRELUDE;
use super::{
    Binding, Checked, CheckedBlock, Checker, Declared, Poisoned, UNKNOWN, block_expr, block_type,
    built_in_type, optional_of, payload, tail, type_of, value_span, wrap, wrap_block,
};
use crate::ast::{self, CompareOp, ExprKind, UnaryOp};
use crate::diagnostic::{
    Definition, Diagnostic, DiagnosticKind, Literal, Span, Suggestion, Unwrap, alias, suggestion,
};
use crate::symbol::Symbol;
use crate::tir::{self, Type};

impl Checker {
    /// `expected` is the type the context wants, `None` when it wants none. It only types a
    /// `None`, a `[]` and an integer literal, and flows into `if` and `if let` branches, a
    /// block's value, array elements, arithmetic and unary `-`; it never wraps.
    pub(super) fn expr(&mut self, e: &ast::Expr, expected: Option<&Type>) -> Checked {
        // An assignment target's chain runs only through field bases, method receivers and
        // indexed bases; any other expression inside it is an ordinary read.
        let outer = self.in_target;
        self.in_target &= matches!(
            e.kind,
            ExprKind::Field { .. } | ExprKind::MethodCall { .. } | ExprKind::Index { .. }
        );
        let checked = self.expr_kind(e, expected);
        self.in_target = outer;
        checked
    }

    /// The integer literal `value`, written as `text`, typed by `expected`.
    fn integer(&mut self, at: Span, value: i64, text: &str, expected: Option<&Type>) -> Checked {
        let ty = literal_type(expected);
        if ty == Type::U8 && !(0..=i64::from(u8::MAX)).contains(&value) {
            // Only `-9223372036854775808` parses to a negative literal.
            self.diagnostics.push(if value < 0 {
                DiagnosticKind::NegatedU8 { at }.into()
            } else {
                DiagnosticKind::IntegerDoesNotFit {
                    text: text.to_string(),
                    at,
                }
                .into()
            });
            return Err(Poisoned::typed(ty));
        }
        Ok(tir::Expr {
            kind: tir::ExprKind::Int(value),
            ty,
            span: at,
        })
    }

    /// `expr` for each kind of expression.
    fn expr_kind(&mut self, e: &ast::Expr, expected: Option<&Type>) -> Checked {
        let node = |kind, ty| {
            Ok(tir::Expr {
                kind,
                ty,
                span: e.span,
            })
        };
        match &e.kind {
            ExprKind::Int { value, text } => self.integer(e.span, *value, text, expected),
            ExprKind::Bool(b) => node(tir::ExprKind::Bool(*b), Type::Bool),
            ExprKind::Char(c) => node(tir::ExprKind::Char(*c), Type::Char),
            ExprKind::Str(s) => node(tir::ExprKind::Str(s.clone()), Type::String),
            ExprKind::Interpolate(parts) => self.interpolate(e.span, parts),
            ExprKind::ArrayLit(elements) => self.array_lit(e.span, elements, expected),
            ExprKind::ArrayFill { value, len } => self.array_fill(e.span, value, len, expected),
            ExprKind::Index { base, index } => self.index(e.span, base, index),
            ExprKind::NoneLit => self.none(e.span, expected),
            ExprKind::Name(name) => self.name(e.span, *name),
            ExprKind::Call { callee, args } => self.call(e.span, callee, args),
            ExprKind::Todo { keyword, args } => self.todo(e.span, *keyword, args),
            ExprKind::SelfValue => {
                let Some(class) = self.self_class else {
                    self.diagnostics
                        .push(DiagnosticKind::SelfOutsideMethod { at: e.span }.into());
                    return Err(Poisoned::error());
                };
                node(tir::ExprKind::Local(tir::LocalId(0)), Type::Class(class))
            }
            ExprKind::Field { base, name, safe } => {
                self.member(e.span, *safe, base, name, |c, checked| {
                    c.field(e.span, checked, base, name)
                })
            }
            ExprKind::MethodCall {
                receiver,
                method,
                args,
                safe,
            } => self.member(e.span, *safe, receiver, method, |c, checked| {
                c.method_call(e.span, checked, receiver, method, args)
            }),
            ExprKind::Coalesce { lhs, rhs } => self.coalesce(e.span, lhs, rhs),
            ExprKind::Is { lhs, rhs } => self.is(e.span, lhs, rhs),
            ExprKind::Unary { op, operand } => {
                let (want, (operand, mismatched)) = match op {
                    UnaryOp::Neg => (Type::I64, self.negated(e.span, operand, expected)?),
                    UnaryOp::Not => (Type::Bool, self.operand(operand, Some(&Type::Bool))),
                };
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
            ExprKind::Binary {
                op,
                op_span,
                lhs,
                rhs,
            } => self.binary(e.span, (*op, *op_span), lhs, rhs, expected, false),
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
            ExprKind::Block(block) => self.block(block, expected).map(|block| {
                let ty = block.ty.clone();
                tir::Expr {
                    kind: tir::ExprKind::Block(block),
                    ty,
                    span: e.span,
                }
            }),
        }
    }

    /// A read of `name`: a local, a constant, or an unknown name, which is reported.
    fn name(&mut self, span: Span, name: Symbol) -> Checked {
        match self.lookup(name) {
            Some(Binding {
                local,
                ty: Some(ty),
                ..
            }) => Ok(tir::Expr {
                kind: tir::ExprKind::Local(*local),
                ty: ty.clone(),
                span,
            }),
            // `Error`: its diagnostic was recorded where it was bound.
            Some(_) => Err(Poisoned::error()),
            None => self.constant_use(name, span).unwrap_or_else(|| {
                let d = self.not_found(name, span);
                self.diagnostics.push(d);
                Err(Poisoned::error())
            }),
        }
    }

    /// The operand of unary `-`, `expected` passed down, and whether it was reported. A `u8`
    /// operand is an error; any other is checked against `i64`.
    fn negated(
        &mut self,
        span: Span,
        operand: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<(Checked, bool), Poisoned> {
        // A bare `None` expects an `i64`, as in `None + 1`.
        let expected = expected.or_else(|| none_literal(operand).then_some(&Type::I64));
        let before = self.diagnostics.len();
        let checked = self.value(operand, expected);
        if type_of(&checked) == Some(&Type::U8) {
            // A literal's "does not fit" is moot once its negation is an error.
            if matches!(operand.kind, ExprKind::Int { .. }) {
                self.diagnostics.truncate(before);
            }
            self.diagnostics
                .push(DiagnosticKind::NegatedU8 { at: span }.into());
            return Err(Poisoned::error());
        }
        Ok(self.against(operand, checked, Some(&Type::I64)))
    }

    /// A comparison chain: every operand is of one integer type, or every one a `string`.
    fn compare(&mut self, span: Span, operands: &[ast::Expr], ops: &[CompareOp]) -> Checked {
        let operands: Vec<_> = operands.iter().collect();
        let (checked, ty, mismatched) = self.integers(&operands, None, Operator::Compare);
        let operands = checked.into_iter().collect::<Result<Vec<_>, _>>().ok();
        match operands {
            Some(operands) if ty == Some(Type::String) => {
                Ok(self.string_chain(span, operands, ops))
            }
            operands => typed(
                span,
                Type::Bool,
                mismatched,
                operands.map(|operands| tir::ExprKind::Compare {
                    operands,
                    ops: ops.to_vec(),
                }),
            ),
        }
    }

    /// An expression whose value is used: not `unit` or `never`.
    pub(super) fn value(&mut self, e: &ast::Expr, expected: Option<&Type>) -> Checked {
        let checked = self.expr(e, expected);
        let never = match type_of(&checked) {
            Some(Type::Unit) => false,
            Some(Type::Never) => true,
            _ => return checked,
        };
        self.diagnostics
            .push(DiagnosticKind::NoValue { at: e.span, never }.into());
        Err(Poisoned::error())
    }

    /// A value of type `want`; `None` is `Error`, which every type matches. A `T` where `want`
    /// is a `T?` is wrapped.
    pub(super) fn expect(&mut self, e: &ast::Expr, want: Option<&Type>) -> Checked {
        self.operand(e, want).0
    }

    /// `expect`, and whether it reported a mismatch. A mismatch is the failure of the rule
    /// that wants `want`, so that rule's expression is `Error`.
    pub(super) fn operand(&mut self, e: &ast::Expr, want: Option<&Type>) -> (Checked, bool) {
        let checked = self.value(e, Some(want.unwrap_or(&UNKNOWN)));
        self.settled(e, checked, want)
    }

    /// `operand` for `e`, already checked against `want`'s expectation.
    pub(super) fn settled(
        &mut self,
        e: &ast::Expr,
        checked: Checked,
        want: Option<&Type>,
    ) -> (Checked, bool) {
        // `want` was rejected inside `e`, which `against` cannot tell from an `Error` operand.
        if matches!(checked, Err(Poisoned { mismatch: true, .. })) {
            return (checked, true);
        }
        self.against(e, checked, want)
    }

    /// `operand` for `e`, already checked.
    pub(super) fn against(
        &mut self,
        e: &ast::Expr,
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
            let d = self.mismatch(e.span, Some(e), want, got);
            self.diagnostics.push(d);
            return (Err(Poisoned::error()), true);
        }
        (checked, false)
    }

    /// Once an operand is `Error`, the other is checked against `Error` too: with that
    /// operand's type unknown, the operator may be the mistake. So `x + true` draws no
    /// diagnostic when `x` is `Error`, and `true && 1` draws one. A `compound` operator stands
    /// for a compound assignment, a statement that a rewrite of the operator would break. `op`
    /// comes with its token's span, where arithmetic on a `char` is reported.
    pub(super) fn binary(
        &mut self,
        span: Span,
        (op, op_span): (ast::BinaryOp, Span),
        lhs: &ast::Expr,
        rhs: &ast::Expr,
        expected: Option<&Type>,
        compound: bool,
    ) -> Checked {
        let Some((tir_op, want)) = operator(op) else {
            return self.equal(span, op == ast::BinaryOp::Ne, lhs, rhs);
        };
        if want == Type::Bool {
            let lhs = self.operand(lhs, Some(&want));
            return self.arithmetic(span, (tir_op, op_span), want, lhs, rhs);
        }
        let operator = if tir_op == tir::BinaryOp::Add {
            Operator::Plus { compound }
        } else {
            Operator::Arithmetic
        };
        let (checked, ty, mismatched) = self.binary_operands(span, &[lhs, rhs], expected, operator);
        if checked.iter().any(|c| type_of(c) == Some(&Type::Char)) {
            self.report(DiagnosticKind::ArithmeticOnChar { at: op_span });
            return Err(Poisoned::error());
        }
        let [l, r]: [Checked; 2] = checked
            .try_into()
            .unwrap_or_else(|_| panic!("`integers` should return one result per operand"));
        if ty == Some(Type::String) {
            return self.concat(span, l, r, mismatched);
        }
        let kind = l.ok().zip(r.ok()).map(|(l, r)| tir::ExprKind::Binary {
            op: tir_op,
            lhs: Box::new(l),
            rhs: Box::new(r),
        });
        match ty {
            Some(ty) => typed(span, ty, mismatched, kind),
            None => Err(Poisoned::error()),
        }
    }

    /// `integers` for the binary operator at `span`. A `+` that a `string` makes a join under an
    /// expected `u8` is checked again with none, so that no operand is held to a `u8` it was
    /// never going to meet: its `string` meets that `u8` once, where the caller checks it. The
    /// join is remembered, so an operand checked again checks a join inside it once, with no
    /// expected type: each level of nesting adds a check, where it would double them.
    fn binary_operands(
        &mut self,
        span: Span,
        operands: &[&ast::Expr],
        expected: Option<&Type>,
        operator: Operator,
    ) -> (Vec<Checked>, Option<Type>, bool) {
        let joins = operator == (Operator::Plus { compound: false });
        if !joins || literal_type(expected) != Type::U8 {
            return self.integers(operands, expected, operator);
        }
        if self.joins.contains(&span) {
            return self.integers(operands, None, operator);
        }
        let (diagnostics, locals) = (self.diagnostics.len(), self.locals.len());
        let result = self.integers(operands, expected, operator);
        if result.1 != Some(Type::String) {
            return result;
        }
        // The first check is dropped, and its locals with it, so none is left that nothing binds.
        self.diagnostics.truncate(diagnostics);
        self.locals.truncate(locals);
        self.joins.insert(span);
        self.integers(operands, None, operator)
    }

    /// The operands of one integer operator, checked in source order; the type they share,
    /// `None` when it is `Error`; and whether one was reported against it. The type is `u8`
    /// when `expected` is `u8` or `u8?`. Otherwise the first concrete operand (of `rank` 0) is
    /// checked first, with no expected type, and gives it: `u8` beside a `u8`, `char` beside a
    /// `char` in a comparison, else `i64`. With none, it is `i64`, so `None + 1` expects an
    /// `i64`. Once an operand is `Error`, the later ones are checked against `Error`. An operand
    /// of `arithmetic` that is a `char` is left unreported, as the poison `typed` `char`: its
    /// caller reports the operator, so what the operator's type finds wrong with the other
    /// operands is dropped, in either order. In a comparison, a `char` after a number reports
    /// that number, as a `char` before one reports the number.
    ///
    /// The type is `string` beside a `string`, in a comparison and in `+`. In `+` a `string`
    /// drives from either side: an operand of another type is reported against `string`, with
    /// the guess to interpolate, the one before a `string` included, in place of what its own
    /// type was found to lack. A join after an `Error` is `Error`. An expected `u8` is
    /// `binary_operands`' to drop.
    fn integers(
        &mut self,
        operands: &[&ast::Expr],
        expected: Option<&Type>,
        operator: Operator,
    ) -> (Vec<Checked>, Option<Type>, bool) {
        let arithmetic = operator != Operator::Compare;
        let compound = operator == (Operator::Plus { compound: true });
        let mut first = None;
        let mut ty = if literal_type(expected) == Type::U8 {
            Some(Type::U8)
        } else if let Some(k) = operands.iter().position(|e| rank(e) == 0) {
            let checked = self.value(operands[k], None);
            let ty = type_of(&checked).map(|ty| match payload(ty) {
                Type::Char if !arithmetic => Type::Char,
                Type::String if operator != Operator::Arithmetic => Type::String,
                _ => literal_type(Some(ty)),
            });
            first = Some((k, checked));
            ty
        } else {
            Some(Type::I64)
        };
        let mut want = ty.clone();
        let leader = first.as_ref().map(|(k, _)| *k);
        let (mut checked, mut mismatched) = (Vec::with_capacity(operands.len()), false);
        // The diagnostics of the operator's type: mismatches, and a literal's, which that type
        // types. They wait until a `char` operand is ruled out.
        let mut deferred = Vec::new();
        // The type of the operand checked last, as it was found, before it was held to `want`.
        let mut before = None;
        // Whether a concrete operand was `Error` as found, which its own diagnostic reported.
        let mut failed = false;
        for (i, e) in operands.iter().enumerate() {
            let start = self.diagnostics.len();
            let value = match first.take_if(|(k, _)| *k == i) {
                Some((_, operand)) => operand,
                None => self.value(e, Some(want.as_ref().unwrap_or(&UNKNOWN))),
            };
            if rank(e) != 0 {
                deferred.extend(self.diagnostics.drain(start..));
            }
            let char_value = type_of(&value) == Some(&Type::Char);
            let got = type_of(&value).cloned();
            let joins = matches!(operator, Operator::Plus { .. });
            let (operand, m) = if let Some(got) = got
                .as_ref()
                .filter(|got| joins && want == Some(Type::String) && **got != Type::String)
            {
                deferred.push(self.concat_mismatch(e, got, operands, compound));
                (Err(Poisoned::error()), true)
            } else if joins && !compound && got == Some(Type::String) && ty != Some(Type::String) {
                // A `string` in `+` makes it a join, whatever the operand before it, if any, was
                // held to: that operand is reported against `string` instead. Under an expected
                // `u8`, that the `+` is a join is all this check finds: `binary_operands` checks
                // it again with none.
                ty = Some(Type::String);
                if let Some(other) = before.take() {
                    // All that is deferred is that operand's, the first of two.
                    deferred.clear();
                    let d = self.concat_mismatch(operands[0], &other, operands, false);
                    self.diagnostics.push(d);
                    checked[0] = Err(Poisoned::error());
                    want = None;
                    (Err(Poisoned::error()), true)
                } else {
                    // An `Error` before it may be the mistake, so the join is `Error` too.
                    (value, true)
                }
            } else if arithmetic && char_value {
                (Err(Poisoned::typed(Type::Char)), false)
            } else if let Some(k) = leader
                .filter(|_| char_value && !arithmetic && matches!(want, Some(Type::I64 | Type::U8)))
            {
                // A `char` after a number in a comparison: the number is the one converted.
                if let Some(got) = type_of(&checked[k]).cloned() {
                    let d = self.mismatch(operands[k].span, Some(operands[k]), &Type::Char, &got);
                    self.diagnostics.push(d);
                }
                checked[k] = Err(Poisoned::error());
                want = None;
                (Err(Poisoned::error()), true)
            } else {
                let start = self.diagnostics.len();
                let settled = self.settled(e, value, want.as_ref());
                deferred.extend(self.diagnostics.drain(start..));
                settled
            };
            if type_of(&operand).is_none() {
                want = None;
            }
            mismatched |= m;
            failed |= got.is_none() && rank(e) == 0;
            checked.push(operand);
            before = got;
        }
        // The operator is rejected whatever stands beside the `char`, unless that is an `Error`:
        // then the operator may be the mistake, as for any other type, and the sum is `Error`.
        if !(arithmetic && checked.iter().any(|c| type_of(c) == Some(&Type::Char))) {
            self.diagnostics.append(&mut deferred);
        } else if failed {
            for c in &mut checked {
                *c = Err(Poisoned::error());
            }
            ty = None;
        }
        (checked, ty, mismatched)
    }

    /// `lhs op rhs`, its `lhs` already checked against `want`, the type `op` takes; `op` comes
    /// with its token's span.
    pub(super) fn arithmetic(
        &mut self,
        span: Span,
        (op, op_span): (tir::BinaryOp, Span),
        want: Type,
        (l, l_mismatched): (Checked, bool),
        rhs: &ast::Expr,
    ) -> Checked {
        let expected = type_of(&l).and(Some(&want));
        let checked = self.value(rhs, Some(expected.unwrap_or(&UNKNOWN)));
        // Only a compound assignment's number gets here with a `char` to its right.
        if want != Type::Bool && type_of(&checked) == Some(&Type::Char) {
            self.report(DiagnosticKind::ArithmeticOnChar { at: op_span });
            return Err(Poisoned::error());
        }
        let (r, r_mismatched) = self.settled(rhs, checked, expected);
        let kind = l.ok().zip(r.ok()).map(|(l, r)| tir::ExprKind::Binary {
            op,
            lhs: Box::new(l),
            rhs: Box::new(r),
        });
        typed(span, want, l_mismatched || r_mismatched, kind)
    }

    /// `lhs == rhs`, or `!=`. The side of lower `rank` is checked first, the left on a tie,
    /// and the other expects its type made optional. A bare `None` beside a value that is not
    /// optional is reported: the answer is fixed. So is a `u8` beside an `i64`, at the side
    /// checked second, and a `char` beside a `u8` or an `i64`, at the side that is not the `char`.
    fn equal(&mut self, span: Span, negated: bool, lhs: &ast::Expr, rhs: &ast::Expr) -> Checked {
        let swapped = rank(rhs) < rank(lhs);
        let (first, second) = if swapped { (rhs, lhs) } else { (lhs, rhs) };
        let checked = self.value(first, None);
        let want = type_of(&checked).map_or_else(|| UNKNOWN.clone(), |ty| optional_of(ty.clone()));
        let other = self.value(second, Some(&want));
        // `second` is the bare `None`, if either is.
        if none_literal(second)
            && let Some(ty) = type_of(&checked)
            && !matches!(ty, Type::Optional(_))
        {
            let value = path(first);
            self.diagnostics
                .push(DiagnosticKind::NeverNone { value, at: span }.into());
            return Err(Poisoned::error());
        }
        // The numbers are `i64`, `u8` and `char`: none is compared with another.
        let number = |ty: &Type| matches!(ty, Type::I64 | Type::U8 | Type::Char);
        if let (Some(want), Some(got)) = (type_of(&checked), type_of(&other))
            && number(payload(want))
            && number(payload(got))
            && payload(want) != payload(got)
        {
            // A number beside a `char` is the one converted; else the second is held to the first.
            let d = if *payload(got) == Type::Char {
                self.mismatch(first.span, Some(first), payload(got), want)
            } else {
                self.mismatch(second.span, Some(second), payload(want), got)
            };
            self.diagnostics.push(d);
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
            // It goes before the diagnostics of `other`, which the help waits for.
            let at = self.diagnostics.len();
            let checked = self.value(other, Some(&UNKNOWN));
            // `== None` compiles only beside an optional, or a side whose type is unknown.
            let optional = type_of(&checked).is_none_or(|ty| matches!(ty, Type::Optional(_)));
            let tested = path(other).filter(|_| optional);
            let d = DiagnosticKind::IsTestsNone { tested, at: span }.into();
            self.diagnostics.insert(at, d);
            return Err(Poisoned::error());
        }
        let (l, r) = (self.value(lhs, None), self.value(rhs, None));
        for (side, checked) in [(lhs, &l), (rhs, &r)] {
            // Once an operand is `Error`, the later one is not held to `is`.
            let Some(ty) = type_of(checked) else { break };
            if matches!(payload(ty), Type::Array(_) | Type::String) || !ty.is_counted() {
                let kind = DiagnosticKind::IsNeedsObjects {
                    found: self.type_name(ty),
                    at: side.span,
                };
                self.diagnostics.push(kind.into());
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
                    let kind = DiagnosticKind::CannotCompare {
                        left: self.type_name(&lt),
                        right: self.type_name(&rt),
                        at: span,
                    };
                    self.diagnostics.push(kind.into());
                    return Err(Poisoned::error());
                }
            }
            _ => (l, r),
        };
        match (l, r) {
            (Ok(l), Ok(r)) => Ok(self.spilled(span, Type::Bool, vec![l, r], |operands| {
                let [lhs, rhs]: [tir::Expr; 2] = operands.try_into().expect(
                    "`spilled` should hand its closure one operand per input, and there are two",
                );
                kind(Box::new(lhs), Box::new(rhs))
            })),
            _ => Err(Poisoned::typed(Type::Bool)),
        }
    }

    /// The right side of `??`, checked against `expected`: a value, or one that never finishes,
    /// but not `unit`.
    fn fallback(&mut self, rhs: &ast::Expr, expected: &Type) -> Checked {
        let checked = self.expr(rhs, Some(expected));
        if type_of(&checked) == Some(&Type::Unit) {
            self.report(DiagnosticKind::NoValue {
                at: rhs.span,
                never: false,
            });
            return Err(Poisoned::error());
        }
        checked
    }

    /// `lhs ?? rhs`: `rhs` is a `T` or a `T?` beside the `T?` `lhs`, and gives the result its
    /// type, or never finishes, and the result is a `T`.
    fn coalesce(&mut self, span: Span, lhs: &ast::Expr, rhs: &ast::Expr) -> Checked {
        let scrutinee = self.value(lhs, None);
        let Some(Type::Optional(payload)) = type_of(&scrutinee) else {
            if type_of(&scrutinee).is_some() {
                self.diagnostics
                    .push(DiagnosticKind::CoalesceNotOptional { at: lhs.span }.into());
            }
            // Only its own diagnostics matter.
            let _ = self.fallback(rhs, &UNKNOWN);
            return Err(Poisoned::error());
        };
        let payload = (**payload).clone();
        let optional = optional_of(payload.clone());
        let other = self.fallback(rhs, &optional);
        let ty = match type_of(&other) {
            // `a ?? panic("x")`: only the payload reaches the result.
            Some(Type::Never) => payload.clone(),
            Some(ty) if *ty == payload || *ty == optional => ty.clone(),
            Some(ty) => {
                let d = self.mismatch(rhs.span, Some(rhs), &payload, ty);
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
    /// receiver that is not optional is reported, and the member still checked on it. `name` is
    /// the member's name.
    fn member(
        &mut self,
        span: Span,
        safe: bool,
        receiver: &ast::Expr,
        name: &ast::Ident,
        member: impl FnOnce(&mut Self, Checked) -> Checked,
    ) -> Checked {
        let scrutinee = self.value(receiver, None);
        if !safe {
            return member(self, scrutinee);
        }
        let Some(Type::Optional(payload)) = type_of(&scrutinee) else {
            if type_of(&scrutinee).is_some() {
                let kind = DiagnosticKind::SafeAccessNotOptional {
                    value: path(receiver),
                    at: receiver.span,
                    access: Span::new(receiver.span.end(), name.span.start()),
                };
                self.diagnostics.push(kind.into());
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
        let (kind, mismatch) = match expected {
            Some(ty @ Type::Optional(_)) => {
                return Ok(tir::Expr {
                    kind: tir::ExprKind::None,
                    ty: ty.clone(),
                    span,
                });
            }
            Some(Type::Never) => return Err(Poisoned::error()),
            Some(ty) => (
                DiagnosticKind::Mismatch {
                    want: vec![self.type_name(ty)],
                    got: "None".to_string(),
                    at: span,
                    other: None,
                    fix: None,
                    conversion: false,
                },
                true,
            ),
            None => (
                DiagnosticKind::CannotInfer {
                    literal: Literal::None,
                    at: span,
                },
                false,
            ),
        };
        self.diagnostics.push(kind.into());
        Err(Poisoned { ty: None, mismatch })
    }

    /// Without an `else`, `unit`. With one, the branches joined (see `branches`). A condition
    /// that is not `bool` makes the `if` `Error`, or `Never` when its branches are.
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
        // A failed join keeps its own `mismatch`, and branches that never end keep `Never`.
        let ty = match ty {
            Ok(Type::Never) if mismatched => Err(Poisoned::typed(Type::Never)),
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
    /// A `value` that is not optional makes it `Error`, or `Never` when its branches are.
    fn if_let(
        &mut self,
        span: Span,
        name: &ast::Ident,
        value: &ast::Expr,
        then_block: &ast::Block,
        else_block: Option<&ast::Block>,
        expected: Option<&Type>,
    ) -> Checked {
        let (scrutinee, bound, mismatched) = self.unwrapped(Unwrap::IfLet, value);
        let mut bind = None;
        let (then, other, ty) = self.branches(then_block, else_block, expected, |c, expected| {
            let (local, block) = c.unwrapping_block("if let", name, bound, then_block, expected);
            bind = Some(local);
            block
        });
        let bind = bind.expect("`branches` should check the first block, which sets `bind`");
        // A failed join keeps its own `mismatch`, and branches that never end keep `Never`.
        let ty = match ty {
            Ok(Type::Never) if mismatched => Err(Poisoned::typed(Type::Never)),
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

    /// The optional `value` that `keyword` unwraps, the type it binds (`None` is `Error`), and
    /// whether `value` was reported for not being optional.
    pub(super) fn unwrapped(
        &mut self,
        keyword: Unwrap,
        value: &ast::Expr,
    ) -> (Checked, Option<Type>, bool) {
        let scrutinee = self.value(value, None);
        match type_of(&scrutinee) {
            Some(Type::Optional(payload)) => {
                let bound = Some((**payload).clone());
                (scrutinee, bound, false)
            }
            Some(ty) => {
                let kind = DiagnosticKind::UnwrapNeedsOptional {
                    keyword,
                    found: self.type_name(ty),
                    at: value.span,
                };
                self.diagnostics.push(kind.into());
                (Err(Poisoned::error()), None, true)
            }
            None => (scrutinee, None, false),
        }
    }

    /// The blocks of an `if` or `if let`, `then` checking the first, and their type: `unit`
    /// without an `else`. With no `expected` type, the branch whose value is of lower `rank`
    /// is checked first, the first branch on a tie, and the other expects its type made
    /// optional. Then a `never` branch, and after it an `Error` one, takes the other's type; a
    /// `T` and a `T?` join to `T?`, the `T` branch wrapped; other types that differ are
    /// reported. A join that fails for a branch that rejected `expected` is a mismatch too.
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
        if expected.is_none() && block_rank(else_block) < block_rank(then_block) {
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
                    // A branch with no value is that mistake, not a type mismatch.
                    _ if t == Type::Unit => DiagnosticKind::NoValue {
                        at: value_span(then_block),
                        never: false,
                    }
                    .into(),
                    _ if o == Type::Unit => DiagnosticKind::NoValue {
                        at: value_span(else_block),
                        never: false,
                    }
                    .into(),
                    Some(w) if fits(w, &t) => {
                        self.mismatch(value_span(else_block), tail(else_block), w, &o)
                    }
                    Some(w) => self.mismatch(value_span(then_block), tail(then_block), w, &t),
                    None => self.mismatch(value_span(else_block), tail(else_block), &t, &o),
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
        let declared = (
            self.class_ids.get(&callee.name).copied(),
            self.function_ids.get(&callee.name).copied(),
        );
        // A broken name, a constant's included, poisons every use, a built-in's too. The item's
        // own error is reported; its arguments may hold more.
        if matches!(
            declared,
            (Some(Declared::Broken), _) | (_, Some(Declared::Broken))
        ) || matches!(self.constants.get(&callee.name), Some(Declared::Broken))
        {
            self.stray(args);
            return Err(Poisoned::error());
        }
        if let Some(checked) = self.prelude_call(span, callee, args) {
            return checked;
        }
        if let Some(to @ (Type::I64 | Type::U8 | Type::Char | Type::String)) =
            built_in_type(callee.name.as_str())
        {
            return self.convert(span, to, args);
        }
        match declared {
            (Some(Declared::Known(class)), _) => self.construct(span, class, args),
            (_, Some(Declared::Known(func))) => self.invoke(span, func, None, args),
            _ => self.unknown_call(callee, args),
        }
    }

    /// A call of `callee`, which names nothing callable. `len(xs)`, `sum(xs)` and
    /// `contains(xs, v)`, where `xs` is an array with that method, point to the method.
    fn unknown_call(&mut self, callee: &ast::Ident, args: &[ast::Arg]) -> Checked {
        let name = callee.name.as_str();
        let idiom = matches!(name, "len" | "sum" | "contains")
            && args.iter().all(|arg| arg.label.is_none());
        let rest = match args.split_first() {
            Some((first, rest)) if idiom => {
                let receiver = self.expr(&first.value, Some(&UNKNOWN));
                // A poisoned array is not known to be one.
                if let Ok(tir::Expr {
                    ty: Type::Array(element),
                    ..
                }) = &receiver
                    && (name != "sum" || **element == Type::I64)
                {
                    let element = (**element).clone();
                    let next = rest.first().map(|arg| arg.value.span);
                    // The rewrite deletes the text before the array and between it and `next`,
                    // so a comment there makes it a guess.
                    let deleted = [
                        callee.span.start()..first.value.span.start(),
                        first.value.span.end()..next.map_or(first.value.span.end(), Span::start),
                    ];
                    let certain = self.idiom_arguments(name, &element, rest)
                        && !self
                            .comments
                            .iter()
                            .any(|c| deleted.iter().any(|d| d.contains(c)));
                    self.report(DiagnosticKind::MethodAsFunction {
                        name: callee.name.to_string(),
                        at: callee.span,
                        receiver: first.value.span,
                        next,
                        wrap: !postfix(&first.value),
                        certain,
                    });
                    return Err(Poisoned::error());
                }
                rest
            }
            _ => args,
        };
        let functions = self
            .function_ids
            .keys()
            .map(|function| function.as_str())
            .chain(PRELUDE)
            .chain(["i64", "u8", "char", "string"])
            .map(|function| (function, Definition::Function));
        let suggestion = match alias(name) {
            Some(prelude) => Some(Suggestion {
                kind: Definition::Function,
                name: prelude.to_string(),
            }),
            None => suggestion(name, functions.chain(self.class_names())),
        };
        self.report(DiagnosticKind::FunctionNotFound {
            name: callee.name.to_string(),
            at: callee.span,
            suggestion,
        });
        // No `labels`: the name may have meant a class, whose constructor takes them.
        self.stray(rest);
        Err(Poisoned::error())
    }

    /// Checks the arguments after the array of an idiom call of the method `name` on an array of
    /// `element`, each once. Whether the method takes them: `contains` one of type `element`, or
    /// an integer literal that fits it, that checked cleanly, and `len` and `sum` none.
    fn idiom_arguments(&mut self, name: &str, element: &Type, rest: &[ast::Arg]) -> bool {
        let [value] = rest else {
            self.stray(rest);
            return rest.is_empty() && name != "contains";
        };
        let checked = self.expr(&value.value, Some(&UNKNOWN));
        name == "contains"
            && checked.is_ok_and(|checked| {
                checked.ty == *element
                    || matches!(
                        (&value.value.kind, element),
                        (ExprKind::Int { value, .. }, Type::U8) if u8::try_from(*value).is_ok()
                    )
            })
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
        // The arguments are not part of an assignment target's chain; `expr` restores the flag.
        let target = std::mem::take(&mut self.in_target);
        match type_of(&receiver) {
            Some(ty @ Type::Array(_)) => {
                let ty = ty.clone();
                return self.array_method(span, receiver, &ty, method, args);
            }
            Some(Type::String) => return self.string_method(span, receiver, method, args),
            _ => {}
        }
        let Some(func) = self.find_method(type_of(&receiver), receiver_expr, method, target) else {
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
            self.report(DiagnosticKind::ArgumentCount {
                name,
                want: params.len(),
                found: args.len(),
                at: span,
            });
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
    pub(super) fn spilled(
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
    pub(super) fn labels(&mut self, args: &[ast::Arg]) -> bool {
        let mut labeled = false;
        for label in args.iter().filter_map(|a| a.label.as_ref()) {
            self.report(DiagnosticKind::LabeledArgument { at: label.span });
            labeled = true;
        }
        labeled
    }

    /// `C(f: e, ...)` names every field of `C`, in declaration order. Only the first mistake
    /// is reported; every argument is still checked for its own.
    fn construct(&mut self, span: Span, class: tir::ClassId, args: &[ast::Arg]) -> Checked {
        let info = &self.classes[class.0];
        let name = info.name.name.as_str();
        let fields: Vec<(Symbol, Option<Type>)> = info
            .fields
            .iter()
            .map(|f| (f.name.name, f.ty.clone()))
            .collect();
        let names: Vec<_> = fields.iter().map(|(f, _)| *f).collect();
        let mut failed = false;
        let mut checked = Vec::with_capacity(args.len());
        for (i, arg) in args.iter().enumerate() {
            if !failed && let Some(d) = misplaced(name, &names, i, arg) {
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
        if !failed && let Some(missing) = names.get(args.len()) {
            self.report(DiagnosticKind::MissingField {
                field: missing.to_string(),
                fields: names.iter().map(ToString::to_string).collect(),
                at: span,
            });
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
        let Some((class, index)) = self.find_field(type_of(&base), receiver, name, false) else {
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
    /// there is none, reported unless `base` is `Error`. `target` when the field is assigned,
    /// where neither `()` nor `?.` can follow the access.
    pub(super) fn find_field(
        &mut self,
        base: Option<&Type>,
        receiver: &ast::Expr,
        name: &ast::Ident,
        target: bool,
    ) -> Option<(tir::ClassId, usize)> {
        let ty = base?;
        let method = |owner, certain: bool| DiagnosticKind::MethodNotField {
            name: name.name.to_string(),
            owner,
            at: name.span,
            call: (!target).then_some(certain),
        };
        let d = match ty {
            Type::Class(class) => {
                let info = &self.classes[class.0];
                if let Some(index) = info.fields.iter().position(|f| f.name.name == name.name) {
                    return Some((*class, index));
                }
                match info.methods.get(&name.name) {
                    Some(Declared::Known(func)) => {
                        let no_arguments = self.signatures[func.0].params.is_empty();
                        method(self.type_name(ty), no_arguments).into()
                    }
                    // Its own error is reported, and what it held is unknown.
                    Some(Declared::Broken) => return None,
                    None => {
                        let fields = info.fields.iter().map(|f| f.name.name.as_str());
                        no_member(Definition::Field, self.type_name(ty), name, fields)
                    }
                }
            }
            Type::Array(_) if name.name.as_str() == "len" => {
                method(self.type_name(ty), true).into()
            }
            Type::Optional(_) => may_be_none(receiver, name, !target && !self.in_target),
            _ => no_member(Definition::Field, self.type_name(ty), name, []),
        };
        self.diagnostics.push(d);
        None
    }

    /// The method `name` of `receiver`, a value of type `ty`. `None` when there is none,
    /// reported unless `ty` is `Error`. `target` when the call is in an assignment target's
    /// chain, where `?.` cannot follow the receiver.
    fn find_method(
        &mut self,
        ty: Option<&Type>,
        receiver: &ast::Expr,
        name: &ast::Ident,
        target: bool,
    ) -> Option<tir::FuncId> {
        let ty = ty?;
        let d = match ty {
            Type::Class(class) => {
                let info = &self.classes[class.0];
                match info.methods.get(&name.name) {
                    Some(&Declared::Known(func)) => return Some(func),
                    // Its own error is reported; the caller checks the arguments.
                    Some(Declared::Broken) => return None,
                    None => {}
                }
                if info.fields.iter().any(|f| f.name.name == name.name) {
                    DiagnosticKind::FieldNotMethod {
                        name: name.name.to_string(),
                        owner: self.type_name(ty),
                        at: name.span,
                    }
                    .into()
                } else {
                    let methods = info.methods.keys().map(|method| method.as_str());
                    no_member(Definition::Method, self.type_name(ty), name, methods)
                }
            }
            Type::Optional(_) => may_be_none(receiver, name, !target),
            _ => no_member(Definition::Method, self.type_name(ty), name, []),
        };
        self.diagnostics.push(d);
        None
    }

    /// Checks the arguments of a call that cannot take them, for their own mistakes.
    pub(super) fn stray(&mut self, args: &[ast::Arg]) {
        for arg in args {
            // Only its diagnostics matter.
            let _ = self.expr(&arg.value, Some(&UNKNOWN));
        }
    }
}

/// The operator whose operands `integers` checks.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Operator {
    /// A comparison chain.
    Compare,
    /// `+`, which also joins `string`s; `compound` for a compound assignment's.
    Plus { compound: bool },
    /// `- * / %`.
    Arithmetic,
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

/// An integer literal, or `+ - * / %` and unary `-` over such operands only (spec rule 2).
pub(super) fn int_literal(e: &ast::Expr) -> bool {
    match &e.kind {
        ExprKind::Int { .. } => true,
        ExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        } => int_literal(operand),
        ExprKind::Binary { op, lhs, rhs, .. } => {
            operator(*op).is_some_and(|(_, ty)| ty == Type::I64)
                && int_literal(lhs)
                && int_literal(rhs)
        }
        _ => false,
    }
}

/// The type literals take beside a value of type `ty`: `u8` if `ty` is `u8` or `u8?`, else `i64`.
pub(super) fn literal_type(ty: Option<&Type>) -> Type {
    match ty.map(payload) {
        Some(Type::U8) => Type::U8,
        _ => Type::I64,
    }
}

/// Which side of an `if`, `if let` or `==` with no expected type is checked first, the lower
/// first: a concrete value is 0, an `int_literal` 1, a bare `None` or `[]` 2.
pub(super) fn rank(e: &ast::Expr) -> u8 {
    match &e.kind {
        ExprKind::NoneLit => 2,
        ExprKind::ArrayLit(elements) if elements.is_empty() => 2,
        _ if int_literal(e) => 1,
        _ => 0,
    }
}

/// The `rank` of `block`'s value; a block that ends in no expression is concrete.
fn block_rank(block: &ast::Block) -> u8 {
    tail(block).map_or(0, rank)
}

/// Whether `e` is the bare literal `None`.
pub(super) fn none_literal(e: &ast::Expr) -> bool {
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
pub(super) fn fits(want: &Type, ty: &Type) -> bool {
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

/// "`node` may be `None`", at the optional `receiver` of a `.` before `name`. With `fix`, a
/// `?.` may take the `.`'s place.
fn may_be_none(receiver: &ast::Expr, name: &ast::Ident, fix: bool) -> Diagnostic {
    DiagnosticKind::MayBeNone {
        value: path(receiver),
        at: receiver.span,
        access: fix.then(|| Span::new(receiver.span.end(), name.span.start())),
    }
    .into()
}

/// A name, `self`, or a chain of `.` and `?.` fields over one, as written.
pub(super) fn path(e: &ast::Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Name(name) => Some(name.to_string()),
        ExprKind::SelfValue => Some("self".to_owned()),
        ExprKind::Field { base, name, safe } => {
            Some(format!("{}{}{}", path(base)?, ast::dot(*safe), name.name))
        }
        _ => None,
    }
}

/// Whether `expr` takes a `.method()` as written, without parentheses around it.
fn postfix(expr: &ast::Expr) -> bool {
    matches!(
        expr.kind,
        ExprKind::Name(_)
            | ExprKind::SelfValue
            | ExprKind::Field { .. }
            | ExprKind::Call { .. }
            | ExprKind::MethodCall { .. }
            | ExprKind::Index { .. }
            | ExprKind::ArrayLit(_)
            | ExprKind::ArrayFill { .. }
    )
}

/// "`P` has no field `z`": `owner` is the type's name, and `members` are its members of `kind`.
pub(super) fn no_member<'a>(
    kind: Definition,
    owner: String,
    name: &ast::Ident,
    members: impl IntoIterator<Item = &'a str>,
) -> Diagnostic {
    DiagnosticKind::NoMember {
        owner,
        kind,
        name: name.name.to_string(),
        at: name.span,
        suggestion: suggestion(
            name.name.as_str(),
            members.into_iter().map(|member| (member, kind)),
        ),
    }
    .into()
}

/// The mistake in argument `i` of a constructor of `class`, whose fields are `fields`, if any.
fn misplaced(class: &str, fields: &[Symbol], i: usize, arg: &ast::Arg) -> Option<Diagnostic> {
    let Some(field) = fields.get(i) else {
        let start = arg
            .label
            .as_ref()
            .map_or(arg.value.span, |l| l.span)
            .start();
        return Some(
            DiagnosticKind::TooManyArguments {
                class: class.to_string(),
                fields: fields.len(),
                at: Span::new(start, arg.value.span.end()),
            }
            .into(),
        );
    };
    let Some(label) = &arg.label else {
        return Some(
            DiagnosticKind::ArgumentNeedsLabel {
                field: field.to_string(),
                at: arg.value.span,
            }
            .into(),
        );
    };
    if label.name == *field {
        None
    } else if fields.contains(&label.name) {
        Some(
            DiagnosticKind::FieldOutOfOrder {
                field: field.to_string(),
                found: label.name.to_string(),
                fields: fields.iter().map(ToString::to_string).collect(),
                at: label.span,
            }
            .into(),
        )
    } else {
        Some(no_member(
            Definition::Field,
            class.to_string(),
            label,
            fields.iter().map(|field| field.as_str()),
        ))
    }
}

/// The node `kind` makes, of type `ty`. `kind` is `None` when an operand failed; the node
/// keeps `ty`, which its rule fixes whatever its operands are, unless the rule itself
/// reported a `mismatched` operand: a rule that failed makes its expression `Error`.
pub(super) fn typed(
    span: Span,
    ty: Type,
    mismatched: bool,
    kind: Option<tir::ExprKind>,
) -> Checked {
    match kind {
        Some(kind) => Ok(tir::Expr { kind, ty, span }),
        None => Err(Poisoned::of((!mismatched).then_some(ty))),
    }
}
