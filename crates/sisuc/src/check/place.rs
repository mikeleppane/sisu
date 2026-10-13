//! Assignment: places, which root may be written, the temps of compound operators, and spills.

use super::expr::{literal_type, local, operator};
use super::{BindingKind, Checked, Checker, Declared, Poisoned, UNKNOWN, block_expr, type_of};
use crate::ast;
use crate::diagnostic::{DiagnosticKind, Immutable, Span};
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
    /// `false` for a root that is a field without `mut`. A name root that cannot be written is reported but
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
        self.assigned(span, place, value)
    }

    /// The write of the checked `value` to the checked `place`.
    fn assigned(
        &mut self,
        span: Span,
        place: CheckedPlace,
        value: Checked,
    ) -> Result<tir::Stmt, Poisoned> {
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
    /// `op` takes `u8`s when the place holds a `u8`, else `i64`s; `+=` on a `string` joins.
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
            // A `char` has no arithmetic, and a rewrite of the statement would break the
            // assignment: `target` stands for the value, which `assign` accepts.
            if self.lookup(name).is_some_and(|b| b.ty == Some(Type::Char)) {
                self.char_compound(target, value);
                return self.assign(span, target, target);
            }
            // `assign` of `target op value`, whose `op` spans the statement.
            let place = self.place(span, target, false);
            let want = place.ty.as_ref();
            let checked = self.binary(
                span,
                op,
                target,
                value,
                Some(want.unwrap_or(&UNKNOWN)),
                true,
            );
            let expanded = ast::Expr {
                kind: ast::ExprKind::Binary {
                    op,
                    lhs: Box::new(target.clone()),
                    rhs: Box::new(value.clone()),
                },
                span,
            };
            let (checked, _) = self.settled(&expanded, checked, want);
            return self.assigned(span, place, checked);
        }
        let place = self.place(span, target, false);
        let Some(ty) = place.ty else {
            // Only its own mistakes are left to report.
            let _ = self.expr(value, Some(&UNKNOWN));
            return Err(unit());
        };
        if ty == Type::Char {
            self.char_compound(target, value);
            return Err(unit());
        }
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
            None => Err(Poisoned::typed(ty.clone())),
        };
        let value = if ty == Type::String && op == tir::BinaryOp::Add {
            let (value, mismatched) = self.operand(value, Some(&ty));
            self.concat(span, read, value, mismatched)
        } else {
            let read = self.against(target, read, Some(&want));
            self.arithmetic(span, op, want, read, value)
        };
        let (Some((root, temps)), Ok(value), true) = (path, value, place.writable) else {
            return Err(unit());
        };
        let operands = reads(&temps);
        lets.push(self.write(span, root, operands, value));
        Ok(tir::Stmt::Expr(block_expr(lets, None, Type::Unit, span)))
    }

    /// Reports the compound assignment to the `char` place `target`, with no fix, and checks
    /// `value` for its own mistakes.
    fn char_compound(&mut self, target: &ast::Expr, value: &ast::Expr) {
        self.report(DiagnosticKind::ArithmeticOnChar {
            at: target.span,
            convert: None,
        });
        // Only its diagnostics matter.
        let _ = self.expr(value, Some(&UNKNOWN));
    }

    /// The constant `name` as the root of a place that `span` assigns, reported, if `name` names
    /// one. The value is still checked against the constant's type. A broken constant poisons
    /// the assignment without a diagnostic.
    fn constant_place(&mut self, span: Span, name: &str, element: bool) -> Option<CheckedPlace> {
        let Declared::Known(constant) = self.constants.get(name)? else {
            return Some(CheckedPlace::failed());
        };
        let (declared, ty) = (constant.span, constant.ty.clone());
        self.report(DiagnosticKind::AssignToImmutable {
            name: name.to_string(),
            at: span,
            element,
            cause: Immutable::Constant { declared },
        });
        Some(CheckedPlace {
            path: None,
            ty,
            writable: false,
        })
    }

    /// The place `target` names, which `span` assigns. `element` when it is the base of an
    /// index, so the write replaces an element of it.
    fn place(&mut self, span: Span, target: &ast::Expr, element: bool) -> CheckedPlace {
        match &target.kind {
            ast::ExprKind::Name(name) => {
                let Some(b) = self.lookup(name) else {
                    if let Some(place) = self.constant_place(span, name, element) {
                        return place;
                    } else if self.function_ids.contains_key(name) {
                        self.report(DiagnosticKind::AssignToFunction {
                            name: name.clone(),
                            at: target.span,
                        });
                    } else {
                        let d = self.not_found(name, target.span);
                        self.diagnostics.push(d);
                        // The name may have meant any open binding; none of them warns as
                        // unassigned.
                        for b in self.scopes.iter_mut().flat_map(|scope| scope.values_mut()) {
                            b.reassigned = true;
                        }
                    }
                    return CheckedPlace::failed();
                };
                b.reassigned |= b.kind == BindingKind::Var;
                let (local, ty, kind, declared) = (b.local, b.ty.clone(), b.kind, b.span);
                let redeclared = b.shadows_mut;
                let cause = match kind {
                    BindingKind::Var => None,
                    BindingKind::Let => Some(Immutable::Let {
                        declared,
                        redeclared,
                    }),
                    BindingKind::Param => Some(Immutable::Param { declared }),
                    BindingKind::Unwrapped(keyword) => {
                        Some(Immutable::Unwrapped { keyword, declared })
                    }
                };
                if let Some(cause) = cause {
                    self.report(DiagnosticKind::AssignToImmutable {
                        name: name.clone(),
                        at: span,
                        element,
                        cause,
                    });
                }
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
                // A `?.` anywhere in the target's chain would not parse.
                let outer = std::mem::replace(&mut self.in_target, true);
                let checked = self.value(base, None);
                self.in_target = outer;
                let Some((class, index)) = self.find_field(type_of(&checked), base, name, true)
                else {
                    return CheckedPlace::failed();
                };
                let field = &self.classes[class.0].fields[index];
                let (ty, writable) = (field.ty.clone(), field.mutable);
                if !writable {
                    let (name, declared) = (field.name.name.clone(), field.name.span);
                    self.report(DiagnosticKind::AssignToImmutable {
                        name,
                        at: span,
                        element,
                        cause: Immutable::Field { declared },
                    });
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
    /// binds what it must of the operands and the value. A field or element place whose value
    /// is `P + t`, `P` reading the place and `t` unable to exit, is `Append { place, t }`
    /// instead; nothing in it can exit, so nothing spills.
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
        let place = build(root, operands);
        let append = !matches!(place, tir::Place::Local(_))
            && matches!(&value.kind, ExprKind::Concat { lhs, rhs }
                if !rhs.exits() && same_place(&place, lhs));
        let assign = match value.kind {
            ExprKind::Concat { rhs, .. } if append => tir::Stmt::Append { place, value: *rhs },
            _ => tir::Stmt::Assign { place, value },
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

/// Whether `read` reads `place`: the same root local, the same fields, and index
/// expressions that are the same local or the same integer literal.
fn same_place(place: &tir::Place, read: &tir::Expr) -> bool {
    match (place, &read.kind) {
        (tir::Place::Local(local), ExprKind::Local(read)) => local == read,
        (
            tir::Place::Field { base, index },
            ExprKind::Field {
                base: read,
                index: field,
            },
        ) => index == field && same_read(base, read),
        (
            tir::Place::Index { base, index },
            ExprKind::Index {
                base: read,
                index: at,
            },
        ) => same_index(index, at) && same_place(base, read),
        _ => false,
    }
}

/// Whether `a` and `b` read the same place, as `same_place` decides.
fn same_read(a: &tir::Expr, b: &tir::Expr) -> bool {
    match (&a.kind, &b.kind) {
        (ExprKind::Local(a), ExprKind::Local(b)) => a == b,
        (ExprKind::Field { base: a, index: i }, ExprKind::Field { base: b, index: j }) => {
            i == j && same_read(a, b)
        }
        (ExprKind::Index { base: a, index: i }, ExprKind::Index { base: b, index: j }) => {
            same_index(i, j) && same_read(a, b)
        }
        _ => false,
    }
}

/// Whether two indexes are the same local or the same integer literal.
fn same_index(a: &tir::Expr, b: &tir::Expr) -> bool {
    match (&a.kind, &b.kind) {
        (ExprKind::Local(a), ExprKind::Local(b)) => a == b,
        (ExprKind::Int(a), ExprKind::Int(b)) => a == b,
        _ => false,
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
