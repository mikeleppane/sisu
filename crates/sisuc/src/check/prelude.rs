//! The prelude: the functions every file sees without an import, `print` and `panic`. No
//! top-level function, class or constant takes one of their names.

use super::expr::typed;
use super::{Checked, Checker, Poisoned, payload, type_of};
use crate::ast;
use crate::diagnostic::{DiagnosticKind, MismatchFix, Span};
use crate::tir::{self, Type};

/// The prelude's names.
pub(super) const PRELUDE: [&str; 2] = ["print", "panic"];

impl Checker {
    /// A call of the prelude function `callee`; `None` when `callee` names none.
    pub(super) fn prelude_call(
        &mut self,
        span: Span,
        callee: &ast::Ident,
        args: &[ast::Arg],
    ) -> Option<Checked> {
        match callee.name.as_str() {
            "print" => Some(self.print(span, args)),
            "panic" => Some(self.panic(span, args)),
            _ => None,
        }
    }

    fn print(&mut self, span: Span, args: &[ast::Arg]) -> Checked {
        // Not `value()`: a `unit` argument needs this code, not "expression has no value".
        let wrong = || {
            DiagnosticKind::PrintArgument {
                at: span,
                class: None,
            }
            .into()
        };
        let labeled = self.labels(args);
        let [arg] = args else {
            self.diagnostics.push(wrong());
            self.stray(args);
            return Err(Poisoned::error());
        };
        let checked = self.expr(&arg.value, None);
        let error = match type_of(&checked) {
            Some(ty) if matches!(payload(ty), Type::Array(_)) => DiagnosticKind::PrintArray {
                ty: self.type_name(ty),
                optional: matches!(ty, Type::Optional(_)),
                at: arg.value.span,
            }
            .into(),
            Some(ty @ Type::Class(_)) => DiagnosticKind::PrintArgument {
                at: arg.value.span,
                class: Some(self.type_name(ty)),
            }
            .into(),
            Some(Type::Unit | Type::Optional(_)) => wrong(),
            Some(Type::Never) => DiagnosticKind::PrintNever { at: arg.value.span }.into(),
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

    /// `panic(message)`, which never finishes.
    fn panic(&mut self, span: Span, args: &[ast::Arg]) -> Checked {
        let labeled = self.labels(args);
        let [arg] = args else {
            self.report(DiagnosticKind::ArgumentCount {
                name: "panic".to_string(),
                want: 1,
                found: args.len(),
                at: span,
            });
            self.stray(args);
            return Err(Poisoned::error());
        };
        let kind = self
            .message(&arg.value)
            .ok()
            .filter(|_| !labeled)
            .map(|message| tir::ExprKind::Panic(Box::new(message)));
        typed(span, Type::Never, labeled, kind)
    }

    /// A `panic` or `todo` message, an expression of type `string`. A value that interpolation
    /// prints, on one line, gets the fix that wraps it in a literal.
    pub(super) fn message(&mut self, arg: &ast::Expr) -> Checked {
        let checked = self.value(arg, Some(&Type::String));
        let got = match type_of(&checked) {
            Some(Type::String) | None => return checked,
            Some(got) => got,
        };
        let printed = matches!(got, Type::I64 | Type::U8 | Type::Bool | Type::Char);
        self.report(DiagnosticKind::Mismatch {
            want: vec![self.type_name(&Type::String)],
            got: self.type_name(got),
            at: arg.span,
            other: None,
            fix: (printed && self.one_line(arg.span))
                .then_some(MismatchFix::WrapInString(arg.span)),
        });
        Err(Poisoned::error())
    }
}
