//! Array expressions: literals and their element join, `[v; n]`, indexing and methods.

use super::expr::{fits, literal_type, no_member, none_literal, quoted, rank, typed};
use super::{Checked, Checker, Poisoned, UNKNOWN, optional_of, payload, type_of, wrap};
use crate::ast;
use crate::diagnostic::{Diagnostic, Span};
use crate::tir::{self, Type};

impl Checker {
    /// `[a, b, ...]`. With an expected `[E]` or `[E]?`, each element is checked against `E`.
    /// Without one, the elements join (see `joined`). A `[]` takes its type from `expected`.
    pub(super) fn array_lit(
        &mut self,
        span: Span,
        elements: &[ast::Expr],
        expected: Option<&Type>,
    ) -> Checked {
        if let Some(element) = element_of(expected) {
            let element = element.clone();
            let (checked, mismatched): (Vec<_>, Vec<_>) = elements
                .iter()
                .map(|e| self.operand(e, Some(&element)))
                .unzip();
            return self.array(span, element, checked, mismatched.contains(&true));
        }
        if elements.is_empty() {
            let (d, mismatch) = match expected {
                Some(Type::Never) => return Err(Poisoned::error()),
                Some(ty) => (
                    Diagnostic::error(span, format!("expected {}, found `[]`", self.show(ty))),
                    true,
                ),
                None => (
                    Diagnostic::error(span, "cannot infer the type of `[]`")
                        .help("write the type: `let xs: [i64] = []`"),
                    false,
                ),
            };
            self.diagnostics.push(d);
            return Err(Poisoned { ty: None, mismatch });
        }
        self.joined(span, elements, expected == Some(&UNKNOWN))
    }

    /// The elements of a literal with no expected array type, joined in three waves by `rank`:
    /// the concrete elements; the integer literals, typed beside the first concrete element;
    /// then each `None` and `[]`, against the join made optional. The join runs in source
    /// order as `branches` joins, and a `None` makes the element type optional. With no typed
    /// element, the first `None` or `[]` cannot be inferred, which `quiet` (`UNKNOWN` was
    /// expected) leaves unreported. An element whose type is `Error` poisons the array.
    fn joined(&mut self, span: Span, elements: &[ast::Expr], quiet: bool) -> Checked {
        let mut slots: Vec<Option<Checked>> = elements.iter().map(|_| None).collect();
        for (slot, e) in slots.iter_mut().zip(elements) {
            if rank(e) == 0 {
                *slot = Some(self.value(e, None));
            }
        }
        let literal = literal_type(slots.iter().flatten().next().and_then(type_of));
        for (slot, e) in slots.iter_mut().zip(elements) {
            if rank(e) == 1 {
                *slot = Some(self.value(e, Some(&literal)));
            }
        }
        let mut failed = slots.iter().flatten().any(|c| type_of(c).is_none());
        let mut joined: Option<Type> = None;
        for (slot, e) in slots.iter().zip(elements).filter(|_| !failed) {
            let Some(ty) = slot.as_ref().and_then(type_of) else {
                continue;
            };
            match &joined {
                Some(t) if fits(t, ty) => {}
                Some(t) if !fits(ty, t) => {
                    let d = self.mismatch(e.span, Some(e), t, ty);
                    self.diagnostics.push(d);
                    failed = true;
                    break;
                }
                _ => joined = Some(ty.clone()),
            }
        }
        let Some(joined) = joined.filter(|_| !failed) else {
            for (i, e) in elements.iter().filter(|e| rank(e) == 2).enumerate() {
                let expected = (i > 0 || failed || quiet).then_some(&UNKNOWN);
                // Only its own diagnostics matter.
                let _ = self.value(e, expected);
            }
            return Err(Poisoned::error());
        };
        let want = optional_of(joined.clone());
        for (slot, e) in slots.iter_mut().zip(elements) {
            if rank(e) == 2 {
                *slot = Some(self.value(e, Some(&want)));
            }
        }
        let element = if elements.iter().any(none_literal) {
            want
        } else {
            joined
        };
        let checked = slots
            .into_iter()
            .map(|slot| {
                match slot.expect("the three waves should check every element, one rank each") {
                    Ok(e) if e.ty != element => Ok(wrap(e, &element)),
                    checked => checked,
                }
            })
            .collect();
        self.array(span, element, checked, false)
    }

    /// The `ArrayLit` of `checked`, an `[element]`, unless an element failed. `mismatched`
    /// when the literal's rule reported an element, which makes it `Error`.
    fn array(
        &mut self,
        span: Span,
        element: Type,
        checked: Vec<Checked>,
        mismatched: bool,
    ) -> Checked {
        let ty = Type::Array(Box::new(element));
        match checked.into_iter().collect::<Result<Vec<_>, _>>() {
            Ok(elements) if !mismatched => {
                Ok(self.spilled(span, ty, elements, tir::ExprKind::ArrayLit))
            }
            _ => Err(Poisoned::of((!mismatched).then_some(ty))),
        }
    }

    /// `[value; len]`: `value` is checked against the expected element type, and `len` against
    /// `i64`. A `value` of a class or a class optional is reported, as every slot would hold
    /// the same object; the literal `None` is not.
    pub(super) fn array_fill(
        &mut self,
        span: Span,
        value: &ast::Expr,
        len: &ast::Expr,
        expected: Option<&Type>,
    ) -> Checked {
        let (checked, mismatched) = match element_of(expected) {
            Some(element) => {
                let element = element.clone();
                self.operand(value, Some(&element))
            }
            // A `None` or `[]` here cannot be inferred, unless `expected` is `UNKNOWN`.
            None => (
                self.value(value, expected.filter(|ty| **ty == UNKNOWN)),
                false,
            ),
        };
        // A wrap only adds the `?` that `payload` removes, so this is the unwrapped type's rule.
        let class = type_of(&checked)
            .map(payload)
            .filter(|ty| matches!(ty, Type::Class(_)) && !none_literal(value))
            .map(|ty| self.type_name(ty));
        if let Some(class) = &class {
            self.diagnostics.push(
                Diagnostic::error(
                    value.span,
                    format!("every element would be the same `{class}` object"),
                )
                .help(format!(
                    "start from `[None; n]` with type `[{class}?]` and fill it in a loop"
                )),
            );
        }
        let (len, len_mismatched) = self.operand(len, Some(&Type::I64));
        let failed = mismatched || class.is_some() || len_mismatched;
        let ty = type_of(&checked).map(|ty| Type::Array(Box::new(ty.clone())));
        match (checked, len, ty) {
            (Ok(value), Ok(len), Some(ty)) if !failed => {
                Ok(self.spilled(span, ty, vec![value, len], |operands| {
                    let (value, len) = pair(operands);
                    tir::ExprKind::ArrayFill { value, len }
                }))
            }
            (.., ty) => Err(Poisoned::of(ty.filter(|_| !failed))),
        }
    }

    /// `base[index]`: `base` is an array, not an optional one, and `index` an `i64`.
    pub(super) fn index(&mut self, span: Span, base: &ast::Expr, index: &ast::Expr) -> Checked {
        let checked = self.value(base, None);
        let (element, d) = match type_of(&checked) {
            Some(Type::Array(element)) => (Some((**element).clone()), None),
            Some(Type::Optional(payload)) if matches!(**payload, Type::Array(_)) => {
                let d = Diagnostic::error(base.span, format!("{} may be `None`", quoted(base)))
                    .help("unwrap it with `if let`");
                (None, Some(d))
            }
            Some(ty) => {
                let d = Diagnostic::error(base.span, format!("cannot index {}", self.show(ty)))
                    .help("only arrays can be indexed");
                (None, Some(d))
            }
            None => (None, None),
        };
        self.diagnostics.extend(d);
        let (index, mismatched) = self.operand(index, Some(&Type::I64));
        match (checked, index, element) {
            (Ok(base), Ok(index), Some(element)) if !mismatched => {
                Ok(self.spilled(span, element, vec![base, index], |operands| {
                    let (base, index) = pair(operands);
                    tir::ExprKind::Index { base, index }
                }))
            }
            (.., element) => Err(Poisoned::of(element.filter(|_| !mismatched))),
        }
    }

    /// `receiver.method(args)` on `receiver`, an array of type `ty`. The methods: `len()`.
    pub(super) fn array_method(
        &mut self,
        span: Span,
        receiver: Checked,
        ty: &Type,
        method: &ast::Ident,
        args: &[ast::Arg],
    ) -> Checked {
        let d = if method.name != "len" {
            no_member("method", &self.show(ty), method)
        } else if !args.is_empty() {
            let message = format!("`len` takes 0 arguments, found {}", args.len());
            Diagnostic::error(span, message)
        } else {
            let kind = receiver.ok().map(|r| tir::ExprKind::Len(Box::new(r)));
            return typed(span, Type::I64, false, kind);
        };
        self.diagnostics.push(d);
        self.labels(args);
        self.stray(args);
        Err(Poisoned::error())
    }
}

/// `E`, when `expected` is `[E]` or `[E]?`.
fn element_of(expected: Option<&Type>) -> Option<&Type> {
    match expected.map(payload) {
        Some(Type::Array(element)) => Some(element),
        _ => None,
    }
}

/// The two operands `spilled` hands back for a node of two.
fn pair(operands: Vec<tir::Expr>) -> (Box<tir::Expr>, Box<tir::Expr>) {
    let [a, b]: [tir::Expr; 2] = operands
        .try_into()
        .expect("`spilled` should hand its closure one operand per input, and there are two");
    (Box::new(a), Box::new(b))
}
