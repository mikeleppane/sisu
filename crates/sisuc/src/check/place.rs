//! Assignment: places, which root may be written, the temps of compound operators, and spills.

use super::expr::{literal_type, local, operator};
use super::{BindingKind, Checker, Poisoned, UNKNOWN, block_expr, not_found, type_of};
use crate::ast;
use crate::diagnostic::{Diagnostic, Span};
use crate::tir::{self, ExprKind, Type};

/// A place, flattened. `operands` are its own operands in evaluation order: a field root's
/// base expression, then each index from the root outwards.
struct Path {
    root: Root,
    operands: Vec<tir::Expr>,
}

#[derive(Clone, Copy)]
enum Root {
    Local(tir::LocalId),
    /// Field `index` of the object that is the first operand.
    Field(usize),
}

struct CheckedPlace {
    /// `None` when a part of the place failed.
    path: Option<Path>,
    /// The type a value written to it needs; `None` is `Error`.
    ty: Option<Type>,
    /// `false` for a `let` field root. A name root that cannot be written is reported but
    /// still lowers, as in milestone 2.
    writable: bool,
}

impl CheckedPlace {
    fn failed() -> Self {
        Self {
            path: None,
            ty: None,
            writable: false,
        }
    }
}

impl Checker {
    /// `target = value`; a root that cannot be written is reported, and `value` still checked.
    pub(super) fn assign(
        &mut self,
        span: Span,
        target: &ast::Expr,
        value: &ast::Expr,
    ) -> Result<tir::Stmt, Poisoned> {
        let place = self.place(span, target, false);
        let value = self.expect(value, place.ty.as_ref());
        match (place.path, value) {
            (Some(path), Ok(value)) if place.writable => {
                Ok(self.write(span, path.root, path.operands, value))
            }
            _ => Err(Poisoned::typed(Type::Unit)),
        }
    }

    /// `x op= e` is `x = x op e`. On any other place, each of its operands is bound to a fresh
    /// local first, in source order, so it runs once: `p[e] op= v` is
    /// `Block { let t = e; Assign { p[t], p[t] op v } }`. The root binding is never copied.
    /// `op` takes `u8`s when the place holds a `u8`, else `i64`s.
    pub(super) fn compound_assign(
        &mut self,
        span: Span,
        op: ast::BinaryOp,
        target: &ast::Expr,
        value: &ast::Expr,
    ) -> Result<tir::Stmt, Poisoned> {
        let unit = || Poisoned::typed(Type::Unit);
        if let ast::ExprKind::Name(name) = &target.kind {
            // `assign` already reports an unknown target; a synthetic read would repeat it.
            if self.lookup(name).is_none() {
                return self.assign(span, target, value);
            }
            // The `op` spans the statement.
            let value = ast::Expr {
                kind: ast::ExprKind::Binary {
                    op,
                    lhs: Box::new(target.clone()),
                    rhs: Box::new(value.clone()),
                },
                span,
            };
            return self.assign(span, target, &value);
        }
        let place = self.place(span, target, false);
        let Some(ty) = place.ty else {
            // Only its own mistakes are left to report.
            let _ = self.expr(value, Some(&UNKNOWN));
            return Err(unit());
        };
        let (op, _) = operator(op).expect(
            "the parser should produce only arithmetic operators for a compound assignment",
        );
        let want = literal_type(Some(&ty));
        let mut lets = Vec::new();
        let path = place.path.map(|path| {
            let temps: Vec<_> = path
                .operands
                .into_iter()
                .map(|init| {
                    let temp = self.fresh(init.ty.clone());
                    let read = (temp, init.ty.clone(), init.span);
                    lets.push(tir::Stmt::Let { local: temp, init });
                    read
                })
                .collect();
            (path.root, temps)
        });
        let reads = |temps: &[(tir::LocalId, Type, Span)]| {
            temps
                .iter()
                .map(|(temp, ty, span)| local(*temp, ty.clone(), *span))
                .collect::<Vec<_>>()
        };
        let read = match &path {
            Some((root, temps)) => Ok(self.read(build(*root, reads(temps)), target.span)),
            None => Err(Poisoned::typed(ty)),
        };
        let read = self.against(target, read, Some(&want));
        let value = self.arithmetic(span, op, want, read, value);
        let (Some((root, temps)), Ok(value), true) = (path, value, place.writable) else {
            return Err(unit());
        };
        let operands = reads(&temps);
        lets.push(self.write(span, root, operands, value));
        Ok(tir::Stmt::Expr(block_expr(lets, None, Type::Unit, span)))
    }

    /// The place `target` names, which `span` assigns. `element` when it is the base of an
    /// index, so the write replaces an element of it.
    fn place(&mut self, span: Span, target: &ast::Expr, element: bool) -> CheckedPlace {
        match &target.kind {
            ast::ExprKind::Name(name) => {
                let Some(b) = self.lookup(name) else {
                    self.diagnostics.push(not_found(name, target.span));
                    return CheckedPlace::failed();
                };
                b.reassigned |= b.kind == BindingKind::Var;
                let (local, ty, kind, declared) = (b.local, b.ty.clone(), b.kind, b.span);
                let d = cannot_assign(span, name, element);
                let d = match kind {
                    BindingKind::Var => None,
                    BindingKind::Let => Some(
                        if element {
                            d
                        } else {
                            d.label("cannot assign twice")
                        }
                        .secondary(declared, "declared with `let` here")
                        .help("declare it with `var`"),
                    ),
                    BindingKind::Param => Some(
                        d.secondary(declared, "declared as a parameter here")
                            .help(format!("copy it into a `var`: `var {name} = {name}`")),
                    ),
                    BindingKind::Unwrapped(keyword) => {
                        Some(d.secondary(declared, format!("bound by `{keyword}` here")))
                    }
                };
                self.diagnostics.extend(d);
                CheckedPlace {
                    path: Some(Path {
                        root: Root::Local(local),
                        operands: Vec::new(),
                    }),
                    ty,
                    writable: true,
                }
            }
            ast::ExprKind::Field { base, name, .. } => {
                let checked = self.value(base, None);
                let Some((class, index)) = self.find_field(type_of(&checked), base, name) else {
                    return CheckedPlace::failed();
                };
                let field = &self.classes[class.0].fields[index];
                let (ty, writable) = (field.ty.clone(), field.mutable);
                if !writable {
                    let d = cannot_assign(span, &field.name.name, element)
                        .secondary(field.name.span, "declared with `let` here")
                        .help("declare it with `var`");
                    self.diagnostics.push(d);
                }
                CheckedPlace {
                    path: checked.ok().map(|base| Path {
                        root: Root::Field(index),
                        operands: vec![base],
                    }),
                    ty,
                    writable,
                }
            }
            ast::ExprKind::Index { base, index } => {
                let inner = self.place(span, base, true);
                let element = self.element(base, inner.ty.as_ref());
                let (index, mismatched) = self.operand(index, Some(&Type::I64));
                let path = inner
                    .path
                    .filter(|_| element.is_some())
                    .zip(index.ok())
                    .map(|(mut path, index)| {
                        path.operands.push(index);
                        path
                    });
                CheckedPlace {
                    path,
                    ty: element.filter(|_| !mismatched),
                    writable: inner.writable,
                }
            }
            _ => unreachable!("the parser should make only a name, a field or an index a place"),
        }
    }

    /// `Assign { place, value }`, the place built from `root` and `operands`, after `spill`
    /// binds what it must of the operands and the value.
    fn write(
        &mut self,
        span: Span,
        root: Root,
        mut operands: Vec<tir::Expr>,
        value: tir::Expr,
    ) -> tir::Stmt {
        operands.push(value);
        let (mut stmts, mut operands) = self.spill(operands);
        let value = operands
            .pop()
            .expect("`spill` should return one operand per input");
        let assign = tir::Stmt::Assign {
            place: build(root, operands),
            value,
        };
        if stmts.is_empty() {
            return assign;
        }
        stmts.push(assign);
        tir::Stmt::Expr(block_expr(stmts, None, Type::Unit, span))
    }

    /// The value `place` holds, read at `span`.
    fn read(&self, place: tir::Place, span: Span) -> tir::Expr {
        let (kind, ty) = match place {
            tir::Place::Local(id) => (ExprKind::Local(id), self.locals[id.0].ty.clone()),
            tir::Place::Field { base, index } => {
                let Type::Class(class) = base.ty else {
                    panic!(
                        "a field place's base should be of a class type, as `find_field` found the field on it"
                    )
                };
                let ty = self.classes[class.0].fields[index].ty.clone().expect(
                    "a place whose type is known should not be a field whose type is `Error`",
                );
                let base = Box::new(base);
                (ExprKind::Field { base, index }, ty)
            }
            tir::Place::Index { base, index } => {
                let base = self.read(*base, span);
                let Type::Array(element) = &base.ty else {
                    panic!("an indexed place should hold an array, as `element` checked")
                };
                let ty = (**element).clone();
                let (base, index) = (Box::new(base), Box::new(index));
                (ExprKind::Index { base, index }, ty)
            }
        };
        tir::Expr { kind, ty, span }
    }
}

/// The place `root` names, with `operands` as its field base and indexes.
fn build(root: Root, operands: Vec<tir::Expr>) -> tir::Place {
    let mut operands = operands.into_iter();
    let mut place = match root {
        Root::Local(id) => tir::Place::Local(id),
        Root::Field(index) => tir::Place::Field {
            base: operands
                .next()
                .expect("a field root's base should be the first operand of its path"),
            index,
        },
    };
    for index in operands {
        place = tir::Place::Index {
            base: Box::new(place),
            index,
        };
    }
    place
}

/// "cannot assign to `name`", or "cannot assign to an element of `name`" for an `element`
/// write, at `span`.
fn cannot_assign(span: Span, name: &str, element: bool) -> Diagnostic {
    let what = if element { "an element of " } else { "" };
    Diagnostic::error(span, format!("cannot assign to {what}`{name}`"))
}
