//! Patterns, `match`, and the patterns of `if let` and `while let`.

use rustc_hash::FxHashMap;

use super::enums::reserved_name;
use super::expr::Branch;
use super::{Checked, CheckedBlock, Checker, Poisoned, type_of};
use crate::ast;
use crate::diagnostic::{Definition, DiagnosticKind, Span, Unwrap, suggestion};
use crate::symbol::Symbol;
use crate::tir::{self, Type};

/// The names one pattern binds: where each was first bound, and their locals in order.
#[derive(Default)]
struct Bound {
    names: FxHashMap<Symbol, Span>,
    locals: Vec<tir::LocalId>,
}

/// What `if let` or `while let` does with its value, as `let_value` decides.
pub(super) enum LetValue {
    /// Unwraps an optional, binding `name` to its payload, of type `bound` (`None` is `Error`).
    Unwrap {
        name: ast::Ident,
        bound: Option<Type>,
    },
    /// Tests the value, of type `ty` (`None` is `Error`), against the pattern.
    Match { ty: Option<Type> },
}

/// The checked pattern of an `if let` or `while let`, from `let_block`.
pub(super) enum LetPattern {
    /// An optional's payload bound to `bind`, as `IfSome` binds it.
    Some { bind: tir::LocalId },
    /// An enum tested against `pattern`, which binds `bindings`; `None` when it failed.
    Match {
        pattern: Option<(tir::Pattern, Vec<tir::LocalId>)>,
    },
}

/// Whether `ty` is one `match` takes.
fn matchable(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Enum(_) | Type::I64 | Type::U8 | Type::Bool | Type::Char | Type::String
    )
}

/// The path of a variant pattern as written: `Num` or `Expr.Num`.
fn written(qualifier: Option<&ast::Ident>, name: &ast::Ident) -> String {
    match qualifier {
        Some(q) => format!("{}.{}", q.name, name.name),
        None => name.name.to_string(),
    }
}

/// An empty block of type `unit`: the `_` arm an `if let` with no `else` lowers to.
fn empty_block() -> tir::Block {
    tir::Block {
        stmts: Vec::new(),
        value: None,
        ty: Type::Unit,
    }
}

impl Checker<'_> {
    /// `match scrutinee { arms }`. Each arm is checked in a scope that holds its bindings, and
    /// the arms join as an `if`'s branches do (`join`).
    pub(super) fn match_expr(
        &mut self,
        span: Span,
        scrutinee: &ast::Expr,
        arms: &[ast::Arm],
        expected: Option<&Type>,
    ) -> Checked {
        let checked = self.value(scrutinee, None);
        let mut position = type_of(&checked).cloned();
        let cannot = position.take_if(|ty| !matchable(ty));
        if let Some(ty) = &cannot {
            let ty = self.type_name(ty);
            self.report(DiagnosticKind::CannotMatch {
                ty,
                at: scrutinee.span,
            });
        }
        let mismatched = cannot.is_some();
        let sources: Vec<_> = arms
            .iter()
            .map(|arm| match &arm.body.kind {
                ast::ExprKind::Block(block) => Branch::of_block(block),
                _ => Branch::of_expr(&arm.body),
            })
            .collect();
        let mut patterns: Vec<_> = arms.iter().map(|_| None).collect();
        let (blocks, ty) = self.join(&sources, expected, |c, i, expected| {
            let arm = &arms[i];
            c.scopes.push(FxHashMap::default());
            patterns[i] = c.pattern_in_scope(&arm.pattern, position.as_ref(), "match");
            if let ast::ExprKind::Block(block) = &arm.body.kind {
                return c.scoped_block(block, expected);
            }
            let value = c.expr(&arm.body, expected);
            c.pop_scope();
            value.map(|value| tir::Block {
                stmts: Vec::new(),
                ty: value.ty.clone(),
                value: Some(Box::new(value)),
            })
        });
        // A scrutinee `match` cannot take fails the `match`, as a failed condition fails an `if`.
        // With no arms, only a scrutinee of a known type makes it `never`.
        let ty = match ty {
            Ok(Type::Never) if mismatched && !arms.is_empty() => Err(Poisoned::typed(Type::Never)),
            Ok(_) if mismatched || (arms.is_empty() && position.is_none()) => {
                Err(Poisoned::error())
            }
            ty => ty,
        };
        let arms: Option<Vec<_>> = patterns
            .into_iter()
            .zip(blocks)
            .map(|(pattern, body)| {
                let (pattern, bindings) = pattern?;
                Some(tir::Arm {
                    pattern,
                    bindings,
                    body: body.ok()?,
                })
            })
            .collect();
        match (checked, arms, ty) {
            (Ok(scrutinee), Some(arms), Ok(ty)) => Ok(tir::Expr {
                kind: tir::ExprKind::Match {
                    temp: self.fresh(scrutinee.ty.clone()),
                    scrutinee: Box::new(scrutinee),
                    arms,
                },
                ty,
                span,
            }),
            (.., ty) => Err(ty.map_or_else(|poisoned| poisoned, Poisoned::typed)),
        }
    }

    /// `value`, which `keyword` tests against `pattern`, and what it does with it: unwraps an
    /// optional when `pattern` is a name or `_`, else matches it. A value that is neither an
    /// enum nor optional is reported, as the `bool` says, and becomes `Error`; the pattern is
    /// then checked at an `Error` position, so its names are poisoned.
    pub(super) fn let_value(
        &mut self,
        keyword: Unwrap,
        pattern: &ast::Pattern,
        value: &ast::Expr,
    ) -> (Checked, LetValue, bool) {
        let checked = self.value(value, None);
        let ty = type_of(&checked).cloned();
        let name = match &pattern.kind {
            ast::PatternKind::Name(name) => Some(name.clone()),
            ast::PatternKind::Wild => Some(ast::Ident {
                name: Symbol::intern("_"),
                span: pattern.span,
            }),
            ast::PatternKind::None | ast::PatternKind::Variant { .. } => None,
        };
        match (ty, name) {
            (Some(Type::Optional(payload)), Some(name)) => {
                let bound = Some(*payload);
                (checked, LetValue::Unwrap { name, bound }, false)
            }
            // `pattern_in_scope` reports any other pattern on an optional.
            (ty @ (Some(Type::Optional(_) | Type::Enum(_)) | None), _) => {
                (checked, LetValue::Match { ty }, false)
            }
            (Some(ty), _) => {
                let compare = matches!(
                    ty,
                    Type::I64 | Type::U8 | Type::Bool | Type::String | Type::Char
                );
                let found = self.type_name(&ty);
                self.report(DiagnosticKind::UnwrapNeedsOptional {
                    keyword,
                    found,
                    at: value.span,
                    compare,
                });
                (Err(Poisoned::error()), LetValue::Match { ty: None }, true)
            }
        }
    }

    /// `block`, in a scope holding what `pattern` binds as `let_value` decided, for `keyword`.
    pub(super) fn let_block(
        &mut self,
        keyword: Unwrap,
        value: LetValue,
        pattern: &ast::Pattern,
        block: &ast::Block,
        expected: Option<&Type>,
    ) -> (LetPattern, CheckedBlock) {
        match value {
            LetValue::Unwrap { name, bound } => {
                let (bind, block) =
                    self.unwrapping_block(keyword.word(), &name, bound, block, expected);
                (LetPattern::Some { bind }, block)
            }
            LetValue::Match { ty } => {
                self.scopes.push(FxHashMap::default());
                let pattern = self.pattern_in_scope(pattern, ty.as_ref(), keyword.word());
                (
                    LetPattern::Match { pattern },
                    self.scoped_block(block, expected),
                )
            }
        }
    }

    /// `pattern` at a position of type `ty` (`None` is `Error`), its names declared in the
    /// innermost scope as `keyword` binds them. `None` when it failed.
    fn pattern_in_scope(
        &mut self,
        pattern: &ast::Pattern,
        ty: Option<&Type>,
        keyword: &'static str,
    ) -> Option<(tir::Pattern, Vec<tir::LocalId>)> {
        let mut bound = Bound::default();
        let pattern = self.pattern(pattern, ty, keyword, &mut bound)?;
        Some((pattern, bound.locals))
    }

    /// `p` at a position of type `ty`; see `pattern_in_scope`. A failed pattern still declares
    /// its names, poisoned, so the arm's body does not report them.
    fn pattern(
        &mut self,
        p: &ast::Pattern,
        ty: Option<&Type>,
        keyword: &'static str,
        bound: &mut Bound,
    ) -> Option<tir::Pattern> {
        let pattern = |kind| tir::Pattern { kind, span: p.span };
        // An `Error` position matches anything; its own error is reported.
        let Some(ty) = ty else {
            self.poison(p, keyword, bound);
            return None;
        };
        let names_only = !matchable(ty);
        match &p.kind {
            ast::PatternKind::Wild => Some(pattern(tir::PatternKind::Wild)),
            ast::PatternKind::Name(name) => {
                self.name_pattern(name, ty, keyword, bound).map(pattern)
            }
            ast::PatternKind::None if names_only => {
                self.only_names(ty, p.span);
                None
            }
            ast::PatternKind::None => {
                let want = self.type_name(ty);
                self.report(DiagnosticKind::PatternMismatch {
                    want,
                    found: "None".to_string(),
                    at: p.span,
                });
                None
            }
            ast::PatternKind::Variant {
                qualifier,
                name,
                fields,
                parens,
            } => {
                let kind = if names_only {
                    self.only_names(ty, p.span);
                    None
                } else {
                    self.variant_pattern(p.span, ty, qualifier.as_ref(), name, fields, *parens)
                };
                let Some((variant, types)) = kind else {
                    for field in fields {
                        self.poison(field, keyword, bound);
                    }
                    return None;
                };
                // Every field is checked, so the names after a failed one are still bound.
                let fields: Vec<_> = fields
                    .iter()
                    .zip(&types)
                    .map(|(field, ty)| self.pattern(field, ty.as_ref(), keyword, bound))
                    .collect();
                Some(pattern(tir::PatternKind::Variant {
                    variant,
                    fields: fields.into_iter().collect::<Option<_>>()?,
                }))
            }
        }
    }

    /// The name pattern `name` at a position of type `ty`: a variant of `ty`'s enum, else a
    /// binding, unless it is a constant or an upper-case name, which `ty` has no variant of.
    fn name_pattern(
        &mut self,
        name: &ast::Ident,
        ty: &Type,
        keyword: &'static str,
        bound: &mut Bound,
    ) -> Option<tir::PatternKind> {
        let text = name.name.as_str();
        let enum_id = match ty {
            Type::Enum(id) => Some(id.0),
            _ => None,
        };
        if let Some(id) = enum_id
            && let Some(&variant) = self.enums[id].index.get(&name.name)
        {
            if self.enums[id].variants[variant].fields.is_empty() {
                return Some(tir::PatternKind::Variant {
                    variant,
                    fields: Vec::new(),
                });
            }
            self.report(DiagnosticKind::VariantForm {
                path: text.to_string(),
                payload: true,
                held: false,
                at: name.span,
            });
            return None;
        }
        // A name the prelude takes is a binding, which `declare` reports.
        if reserved_name(text).is_none() {
            if self.constants.contains_key(&name.name) {
                self.report(DiagnosticKind::ConstantInPattern {
                    name: text.to_string(),
                    at: name.span,
                });
                return None;
            }
            if text.starts_with(char::is_uppercase) {
                let kind = if let Some(id) = enum_id {
                    let info = &self.enums[id];
                    let variants = info
                        .variants
                        .iter()
                        .map(|v| (v.name.name.as_str(), Definition::Variant));
                    DiagnosticKind::UnknownVariant {
                        enum_name: info.name.name.to_string(),
                        name: text.to_string(),
                        at: name.span,
                        suggestion: suggestion(text, variants),
                    }
                } else {
                    // Read as the binding the fix makes it, so the body does not cascade. The
                    // name is not checked as a binding's: this error is the one it gets.
                    if bound.names.insert(name.name, name.span).is_none() {
                        let kind = super::BindingKind::Unwrapped(keyword);
                        bound.locals.push(self.insert_local(name, None, kind, None));
                    }
                    DiagnosticKind::NotVariantHere {
                        name: text.to_string(),
                        at: name.span,
                    }
                };
                self.report(kind);
                return None;
            }
        }
        self.bind(name, Some(ty.clone()), keyword, bound)
            .map(tir::PatternKind::Bind)
    }

    /// The variant that `qualifier.name(fields)` (`parens` is `None` without parentheses)
    /// names at a position of type `ty`, an enum or a type `==` compares, and its field types;
    /// `None` when it was reported.
    fn variant_pattern(
        &mut self,
        span: Span,
        ty: &Type,
        qualifier: Option<&ast::Ident>,
        name: &ast::Ident,
        fields: &[ast::Pattern],
        parens: Option<Span>,
    ) -> Option<(usize, Vec<Option<Type>>)> {
        let path = written(qualifier, name);
        let named = match qualifier {
            Some(q) => Some(self.resolve(&ast::TypeExpr {
                kind: ast::TypeKind::Name(q.name),
                optional: false,
                span: q.span,
            })?),
            None => None,
        };
        let id = match ty {
            Type::Enum(id) if named.as_ref().is_none_or(|named| named == ty) => id.0,
            _ => {
                let want = self.type_name(ty);
                self.report(DiagnosticKind::PatternMismatch {
                    want,
                    found: path,
                    at: span,
                });
                return None;
            }
        };
        let info = &self.enums[id];
        let Some(&variant) = info.index.get(&name.name) else {
            let variants = info
                .variants
                .iter()
                .map(|v| (v.name.name.as_str(), Definition::Variant));
            let kind = DiagnosticKind::UnknownVariant {
                enum_name: info.name.name.to_string(),
                name: name.name.to_string(),
                at: name.span,
                suggestion: suggestion(name.name.as_str(), variants),
            };
            self.report(kind);
            return None;
        };
        let types = info.variants[variant].fields.clone();
        let kind = match parens {
            None if !types.is_empty() => DiagnosticKind::VariantForm {
                path,
                payload: true,
                held: false,
                at: name.span,
            },
            // The fix deletes the parentheses, so what they hold (even a comment) makes it unsafe.
            Some(parens) if types.is_empty() => DiagnosticKind::VariantForm {
                path,
                payload: false,
                held: !fields.is_empty() || self.comment_in(&[parens.range()]),
                at: parens,
            },
            _ if fields.len() != types.len() => DiagnosticKind::SubpatternCount {
                variant: name.name.to_string(),
                fields: types.len(),
                found: fields.len(),
                at: span,
            },
            _ => return Some((variant, types)),
        };
        self.report(kind);
        None
    }

    /// "a `ty` position takes only a name or `_`", at `at`.
    fn only_names(&mut self, ty: &Type, at: Span) {
        let ty = self.type_name(ty);
        self.report(DiagnosticKind::OnlyNamePosition { ty, at });
    }

    /// Declares `name`, of type `ty`, as `keyword` binds it, unless this pattern binds it
    /// already, which is reported.
    fn bind(
        &mut self,
        name: &ast::Ident,
        ty: Option<Type>,
        keyword: &'static str,
        bound: &mut Bound,
    ) -> Option<tir::LocalId> {
        if let Some(&first) = bound.names.get(&name.name) {
            self.report(DiagnosticKind::BoundTwice {
                name: name.name.to_string(),
                at: name.span,
                first,
            });
            return None;
        }
        bound.names.insert(name.name, name.span);
        let local = self.declare(name, ty, super::BindingKind::Unwrapped(keyword), None);
        bound.locals.push(local);
        Some(local)
    }

    /// Declares the names in `p`, a pattern that failed, as `Error`, so that uses of them
    /// report nothing more. An upper-case name is taken for a variant and binds nothing.
    fn poison(&mut self, p: &ast::Pattern, keyword: &'static str, bound: &mut Bound) {
        match &p.kind {
            ast::PatternKind::Name(name)
                if !name.name.as_str().starts_with(char::is_uppercase)
                    && !bound.names.contains_key(&name.name) =>
            {
                self.bind(name, None, keyword, bound);
            }
            ast::PatternKind::Variant { fields, .. } => {
                for field in fields {
                    self.poison(field, keyword, bound);
                }
            }
            _ => {}
        }
    }

    /// `if let pattern = value`'s lowering on an enum, given its checked parts: a `match` of
    /// `pattern => then` and `_ => else`, or of `_ => {}` when there is no `else`.
    pub(super) fn if_let_match(
        &mut self,
        scrutinee: tir::Expr,
        (pattern, bindings): (tir::Pattern, Vec<tir::LocalId>),
        then_block: tir::Block,
        else_block: Option<tir::Block>,
    ) -> tir::ExprKind {
        let span = pattern.span;
        let other = tir::Pattern {
            kind: tir::PatternKind::Wild,
            span,
        };
        tir::ExprKind::Match {
            temp: self.fresh(scrutinee.ty.clone()),
            scrutinee: Box::new(scrutinee),
            arms: vec![
                tir::Arm {
                    pattern,
                    bindings,
                    body: then_block,
                },
                tir::Arm {
                    pattern: other,
                    bindings: Vec::new(),
                    body: else_block.unwrap_or_else(empty_block),
                },
            ],
        }
    }
}
