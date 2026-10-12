//! The checker: signatures, scopes, names and types. It lowers the program to `tir` as it
//! checks, and records every error instead of stopping at the first.

mod array;
mod expr;
mod place;
mod stmt;

use std::collections::HashMap;

use crate::ast;
use crate::diagnostic::{Diagnostic, Severity, Span};
use crate::tir::{self, Type};

/// An expression whose rule failed; its diagnostic is recorded. `ty` is its type when
/// still known (an `if` with one failed branch takes the other's), else it is the poison
/// type `Error`, which matches every type and draws no further diagnostic.
///
/// Poison spreads to sibling operands by one rule. For a homogeneous operator (`+ - * / %
/// && ||`, comparison chains), once an operand is `Error` the later operands are still
/// checked for their own errors but not against the operator's type. Call and constructor
/// arguments are each checked against their own parameter, so one bad argument does not
/// excuse another.
struct Poisoned {
    ty: Option<Type>,
    /// Whether the expected type passed down was rejected (a `None` that is not optional, or
    /// a block or `if` whose value is one): the rule that passed it failed, so its expression
    /// is `Error` too, where an `Error` operand would excuse it.
    mismatch: bool,
}

impl Poisoned {
    /// `Error`.
    fn error() -> Self {
        Self::of(None)
    }

    /// A failed expression whose type is still known.
    fn typed(ty: Type) -> Self {
        Self::of(Some(ty))
    }

    fn of(ty: Option<Type>) -> Self {
        Self {
            ty,
            mismatch: false,
        }
    }
}

type Checked = Result<tir::Expr, Poisoned>;
type CheckedBlock = Result<tir::Block, Poisoned>;

/// The expected type a position passes down when its own type is `Error`. No position expects
/// `never`, so it stands for `Error` there: a `None` it reaches is `Error`, with no diagnostic.
static UNKNOWN: Type = Type::Never;

/// A type of `None` is `Error`: its error is reported where the signature names it.
struct Signature {
    /// `f`, or `P.m` for a method.
    name: String,
    /// Without a method's `self`.
    params: Vec<Option<Type>>,
    ret: Option<Type>,
}

struct ClassInfo {
    name: ast::Ident,
    /// The first field of each name, in declaration order.
    fields: Vec<FieldInfo>,
    /// The first method of each name.
    methods: HashMap<String, tir::FuncId>,
}

struct FieldInfo {
    name: ast::Ident,
    /// `None` is `Error`.
    ty: Option<Type>,
    mutable: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum BindingKind {
    Let,
    Var,
    Param,
    /// Bound by an `if let` or a `while let`: the keyword.
    Unwrapped(&'static str),
}

struct Binding {
    local: tir::LocalId,
    /// `None` is `Error`.
    ty: Option<Type>,
    kind: BindingKind,
    span: Span,
    reassigned: bool,
}

struct Checker {
    /// The first function of each name.
    function_ids: HashMap<String, tir::FuncId>,
    /// Indexed by `FuncId`, so a duplicate function has a signature of its own.
    signatures: Vec<Signature>,
    /// Indexed by `ClassId`, so a duplicate class has fields and methods of its own.
    classes: Vec<ClassInfo>,
    /// The first class of each name.
    class_ids: HashMap<String, tir::ClassId>,
    diagnostics: Vec<Diagnostic>,
    // The rest is per body.
    scopes: Vec<HashMap<String, Binding>>,
    locals: Vec<tir::Local>,
    /// `None` is `Error`.
    ret: Option<Type>,
    /// The class of the method being checked, whose `self` is local 0.
    self_class: Option<tir::ClassId>,
    /// Loops open around the code being checked; a `while` condition counts as inside.
    loop_depth: usize,
}

/// Diagnostics sorted by span start (stable); the program only when none is an error.
pub(crate) fn check(program: &ast::Program) -> (Option<tir::Program>, Vec<Diagnostic>) {
    let mut checker = Checker {
        function_ids: HashMap::new(),
        signatures: Vec::new(),
        classes: Vec::new(),
        class_ids: HashMap::new(),
        diagnostics: Vec::new(),
        scopes: Vec::new(),
        locals: Vec::new(),
        ret: None,
        self_class: None,
        loop_depth: 0,
    };
    checker.collect_names(program);
    let declared = checker.collect_signatures(program);
    checker.never_constructed();
    checker.check_main(&declared);
    // Every body is checked, so this collects into a `Vec` before it gives up on a `None`.
    let functions: Vec<_> = declared
        .into_iter()
        .enumerate()
        .map(|(i, (f, class))| checker.function(f, tir::FuncId(i), class))
        .collect();
    let classes = checker.classes.iter().map(lower_class).collect();
    let mut diagnostics = checker.diagnostics;
    diagnostics.sort_by_key(|d| d.span.start);
    let failed = diagnostics.iter().any(|d| d.severity == Severity::Error);
    let program = functions
        .into_iter()
        .collect::<Option<_>>()
        .zip(classes)
        .filter(|_| !failed)
        .map(|(functions, classes)| tir::Program { classes, functions });
    debug_assert!(
        program.is_some() || failed,
        "a missing program implies an error diagnostic"
    );
    (program, diagnostics)
}

/// The class, unless a field's type is `Error`.
fn lower_class(class: &ClassInfo) -> Option<tir::Class> {
    let fields = class
        .fields
        .iter()
        .map(|f| {
            Some(tir::Field {
                name: f.name.name.clone(),
                ty: f.ty.clone()?,
                mutable: f.mutable,
            })
        })
        .collect::<Option<_>>()?;
    Some(tir::Class {
        name: class.name.name.clone(),
        fields,
    })
}

/// The type of a checked expression; `None` is `Error`.
fn type_of(checked: &Checked) -> Option<&Type> {
    match checked {
        Ok(e) => Some(&e.ty),
        Err(p) => p.ty.as_ref(),
    }
}

/// The type of a checked block; `None` is `Error`.
fn block_type(checked: &CheckedBlock) -> Option<&Type> {
    match checked {
        Ok(b) => Some(&b.ty),
        Err(p) => p.ty.as_ref(),
    }
}

impl Checker {
    /// Pass 1: gives every class a `ClassId` and records the first function and class of each
    /// name, so that types and calls can name any of them. Classes and functions share a
    /// namespace. `FuncId`s run in source order, a class's methods at its place.
    fn collect_names(&mut self, program: &ast::Program) {
        let mut first = HashMap::new();
        let mut next_function = 0;
        for item in &program.items {
            match item {
                ast::Item::Class(class) => {
                    let id = tir::ClassId(self.classes.len());
                    self.classes.push(ClassInfo {
                        name: class.name.clone(),
                        fields: Vec::new(),
                        methods: HashMap::new(),
                    });
                    let name = &class.name;
                    if !self.built_in(name)
                        && self.first_definition(&mut first, "class", None, name)
                    {
                        self.class_ids.insert(name.name.clone(), id);
                    }
                    next_function += class
                        .members
                        .iter()
                        .filter(|m| matches!(m, ast::Member::Method(_)))
                        .count();
                }
                ast::Item::Function(f) => {
                    if !self.built_in(&f.name)
                        && self.first_definition(&mut first, "function", None, &f.name)
                    {
                        self.function_ids
                            .insert(f.name.name.clone(), tir::FuncId(next_function));
                    }
                    next_function += 1;
                }
            }
        }
    }

    /// Whether `name` is a built-in type's or function's, which no class or function may take;
    /// reports it when it is.
    fn built_in(&mut self, name: &ast::Ident) -> bool {
        let what = if built_in_type(&name.name).is_some() {
            "type"
        } else if name.name == "print" {
            "function"
        } else {
            return false;
        };
        self.diagnostics.push(Diagnostic::error(
            name.span,
            format!("`{}` is a built-in {what}", name.name),
        ));
        true
    }

    /// Whether `name` is the first of its name in `first`, which it joins with its `kind`;
    /// reports it when not. `owner` is the class whose members `first` holds.
    fn first_definition(
        &mut self,
        first: &mut HashMap<String, (Span, &'static str)>,
        kind: &'static str,
        owner: Option<&str>,
        name: &ast::Ident,
    ) -> bool {
        if let Some(&(span, first_kind)) = first.get(&name.name) {
            self.diagnostics.push(
                duplicate(first_kind, kind, owner, name).secondary(span, "first defined here"),
            );
            return false;
        }
        first.insert(name.name.clone(), (name.span, kind));
        true
    }

    /// Pass 2: each class's fields and every signature, in `FuncId` order. Returns each
    /// function with the class of a method. A class's fields and methods share a namespace.
    fn collect_signatures<'a>(
        &mut self,
        program: &'a ast::Program,
    ) -> Vec<(&'a ast::Function, Option<tir::ClassId>)> {
        let mut declared = Vec::new();
        let mut next_class = 0;
        for item in &program.items {
            let class = match item {
                ast::Item::Function(f) => {
                    self.signature(f, f.name.name.clone());
                    declared.push((f, None));
                    continue;
                }
                ast::Item::Class(class) => class,
            };
            let id = tir::ClassId(next_class);
            next_class += 1;
            let mut first = HashMap::new();
            for member in &class.members {
                match member {
                    ast::Member::Field(field) => {
                        let ty = self.resolve(&field.ty);
                        if self.first_definition(
                            &mut first,
                            "field",
                            Some(&class.name.name),
                            &field.name,
                        ) {
                            self.classes[id.0].fields.push(FieldInfo {
                                name: field.name.clone(),
                                ty,
                                mutable: field.mutable,
                            });
                        }
                    }
                    ast::Member::Method(method) => {
                        let func = tir::FuncId(self.signatures.len());
                        if self.first_definition(
                            &mut first,
                            "method",
                            Some(&class.name.name),
                            &method.name,
                        ) {
                            self.classes[id.0]
                                .methods
                                .insert(method.name.name.clone(), func);
                        }
                        self.signature(method, format!("{}.{}", class.name.name, method.name.name));
                        declared.push((method, Some(id)));
                    }
                }
            }
        }
        declared
    }

    fn signature(&mut self, f: &ast::Function, name: String) {
        let params = f.params.iter().map(|p| self.resolve(&p.ty)).collect();
        let ret = f
            .ret
            .as_ref()
            .map_or(Some(Type::Unit), |ty| self.resolve(ty));
        self.signatures.push(Signature { name, params, ret });
    }

    /// Reports each strongly connected component of non-optional class fields once
    /// (overlapping cycles share one error), on the component's first class in source order,
    /// at its first field into the component: no first object of such a class can exist.
    fn never_constructed(&mut self) {
        let reach: Vec<Vec<bool>> = (0..self.classes.len()).map(|c| self.reachable(c)).collect();
        for (c, class) in self.classes.iter().enumerate() {
            let in_cycle = |d: usize| reach[c][d] && reach[d][c];
            if !in_cycle(c) || (0..c).any(in_cycle) {
                continue;
            }
            let (field, target) = class
                .fields
                .iter()
                .find_map(|f| match f.ty {
                    Some(Type::Class(d)) if in_cycle(d.0) => Some((f, d)),
                    _ => None,
                })
                .expect(
                    "a class on a reference cycle should have a field that leads into the cycle",
                );
            let target = &self.classes[target.0].name.name;
            self.diagnostics.push(
                Diagnostic::error(
                    field.name.span,
                    format!(
                        "`{}` can never be constructed: field `{}` needs a `{target}`",
                        class.name.name, field.name.name
                    ),
                )
                .help(format!("make it optional: `{target}?`")),
            );
        }
    }

    /// The classes an object of class `from` holds through one field or more.
    fn reachable(&self, from: usize) -> Vec<bool> {
        let mut seen = vec![false; self.classes.len()];
        let mut stack = vec![from];
        while let Some(c) = stack.pop() {
            for field in &self.classes[c].fields {
                if let Some(Type::Class(d)) = field.ty
                    && !seen[d.0]
                {
                    seen[d.0] = true;
                    stack.push(d.0);
                }
            }
        }
        seen
    }

    /// Reports a missing `main`, or one with parameters or a return type.
    fn check_main(&mut self, declared: &[(&ast::Function, Option<tir::ClassId>)]) {
        match self.function_ids.get("main") {
            None => self
                .diagnostics
                .push(Diagnostic::error(Span::new(0, 0), "no `main` function")),
            Some(id) => {
                let main = declared[id.0].0;
                if !main.params.is_empty() || main.ret.is_some() {
                    self.diagnostics.push(Diagnostic::error(
                        main.name.span,
                        "`main` takes no parameters and returns no value",
                    ));
                }
            }
        }
    }

    /// `None`, reported at the name without its `?`, for an unknown type.
    fn resolve(&mut self, ty: &ast::TypeExpr) -> Option<Type> {
        let named = match &ty.kind {
            ast::TypeKind::Array(element) => Type::Array(Box::new(self.resolve(element)?)),
            ast::TypeKind::Name(name) => {
                if let Some(named) = built_in_type(name) {
                    named
                } else if let Some(&id) = self.class_ids.get(name) {
                    Type::Class(id)
                } else {
                    let span = Span::new(ty.span.start, ty.span.start + name.len());
                    self.diagnostics.push(
                        Diagnostic::error(span, format!("unknown type `{name}`")).help(
                            "the types are `i64`, `u8`, `bool`, classes and arrays such as `[i64]`",
                        ),
                    );
                    return None;
                }
            }
        };
        if !ty.optional {
            return Some(named);
        }
        Some(optional_of(named))
    }

    /// Checks `f`'s body against its own signature, `id`. The function, unless its return type
    /// is `Error` or its body failed; an `Error` parameter still yields one, and `check` drops
    /// it for the error behind it.
    /// A method, of `class`, gets `self` as its first local.
    fn function(
        &mut self,
        f: &ast::Function,
        id: tir::FuncId,
        class: Option<tir::ClassId>,
    ) -> Option<tir::Function> {
        let sig = &self.signatures[id.0];
        let (name, param_types, ret) = (sig.name.clone(), sig.params.clone(), sig.ret.clone());
        self.ret.clone_from(&ret);
        self.self_class = class;
        self.scopes.push(HashMap::new());
        // `self` is a keyword, so it needs no binding: `ExprKind::SelfValue` reads local 0.
        let receiver = class.map(|class| {
            self.locals.push(tir::Local {
                name: Some("self".into()),
                ty: Type::Class(class),
                mutable: false,
            });
            tir::LocalId(0)
        });
        let params = receiver
            .into_iter()
            .chain(
                f.params
                    .iter()
                    .zip(param_types)
                    .map(|(p, ty)| self.declare(&p.name, ty, BindingKind::Param)),
            )
            .collect();
        // A `unit` function discards its body's value, so it expects none.
        let expected = match &ret {
            None => Some(&UNKNOWN),
            Some(Type::Unit) => None,
            Some(ret) => Some(ret),
        };
        let mut body = self.block(&f.body, expected);
        if let Some(want @ Type::Optional(payload)) = &ret
            && block_type(&body) == Some(&**payload)
        {
            wrap_block(&mut body, want);
        }
        self.pop_scope();
        let locals = std::mem::take(&mut self.locals);
        match (&ret, block_type(&body)) {
            (Some(want), Some(got))
                if *want != Type::Unit && *got != Type::Never && got != want =>
            {
                let d = if *got == Type::Unit {
                    let end = f.body.span.end;
                    Diagnostic::error(
                        Span::new(end - 1, end),
                        format!(
                            "function `{}` must return {}, but its body has no value",
                            f.name.name,
                            self.show(want)
                        ),
                    )
                } else {
                    self.mismatch(value_span(&f.body), tail(&f.body), want, got)
                };
                self.diagnostics.push(d);
            }
            _ => {}
        }
        Some(tir::Function {
            name,
            params,
            ret: ret?,
            locals,
            body: body.ok()?,
        })
    }

    /// A type as messages quote it: "`i64`", "`Tree`", "`Tree?`".
    fn show(&self, ty: &Type) -> String {
        format!("`{}`", self.type_name(ty))
    }

    fn type_name(&self, ty: &Type) -> String {
        match ty {
            Type::Class(id) => self.classes[id.0].name.name.clone(),
            Type::Array(element) => format!("[{}]", self.type_name(element)),
            Type::Optional(payload) => format!("{}?", self.type_name(payload)),
            ty => ty.to_string(),
        }
    }

    /// "expected `want`, found `got`" at `span`, where `source` is the expression found, when
    /// known. Between `u8` and `i64` it says how to convert `source`.
    fn mismatch(
        &self,
        span: Span,
        source: Option<&ast::Expr>,
        want: &Type,
        got: &Type,
    ) -> Diagnostic {
        let d = Diagnostic::error(
            span,
            format!("expected {}, found {}", self.show(want), self.show(got)),
        );
        let to = match (payload(want), got) {
            (Type::U8, Type::I64) => "u8",
            (Type::I64, Type::U8) => "i64",
            _ => return d,
        };
        let operand = source.and_then(expr::path);
        d.help(format!(
            "convert it: `{to}({})`",
            operand.as_deref().unwrap_or("...")
        ))
    }
}

/// The type a built-in type name names.
fn built_in_type(name: &str) -> Option<Type> {
    match name {
        "i64" => Some(Type::I64),
        "u8" => Some(Type::U8),
        "bool" => Some(Type::Bool),
        _ => None,
    }
}

/// `T` for a `T?`, else `ty`.
fn payload(ty: &Type) -> &Type {
    match ty {
        Type::Optional(payload) => payload,
        ty => ty,
    }
}

/// The expression a block ends in, if it ends in one: its value.
fn tail(block: &ast::Block) -> Option<&ast::Expr> {
    match block.stmts.last() {
        Some(ast::Stmt {
            kind: ast::StmtKind::Expr(e),
            ..
        }) => Some(e),
        _ => None,
    }
}

/// Where a block's value comes from: its last statement, or the block itself when empty.
fn value_span(block: &ast::Block) -> Span {
    block.stmts.last().map_or(block.span, |s| s.span)
}

/// A `Block` expression of `stmts` then `value`, of type `ty`.
fn block_expr(stmts: Vec<tir::Stmt>, value: Option<tir::Expr>, ty: Type, span: Span) -> tir::Expr {
    let block = tir::Block {
        stmts,
        value: value.map(Box::new),
        ty: ty.clone(),
    };
    tir::Expr {
        kind: tir::ExprKind::Block(block),
        ty,
        span,
    }
}

/// `ty?`; an optional is its own, as optionals do not nest.
fn optional_of(ty: Type) -> Type {
    match ty {
        Type::Optional(_) => ty,
        ty => Type::Optional(Box::new(ty)),
    }
}

/// `e`, of type `T`, where its optional `ty` is expected.
fn wrap(e: tir::Expr, ty: &Type) -> tir::Expr {
    tir::Expr {
        span: e.span,
        kind: tir::ExprKind::Wrap(Box::new(e)),
        ty: ty.clone(),
    }
}

/// Makes a block of type `T` one of the optional `ty` by wrapping its value.
fn wrap_block(block: &mut CheckedBlock, ty: &Type) {
    match block {
        Ok(block) => {
            let value = block
                .value
                .take()
                .expect("a block checked as a value type should end in its value");
            block.value = Some(Box::new(wrap(*value, ty)));
            block.ty = ty.clone();
        }
        Err(poisoned) => poisoned.ty = Some(ty.clone()),
    }
}

fn not_found(name: &str, span: Span) -> Diagnostic {
    Diagnostic::error(span, format!("cannot find `{name}` in this scope"))
}

/// `name` taken again. `kind` is the second declaration's and `first` the first's: "function",
/// "class", "field" or "method". `owner` is the class of a field or method.
fn duplicate(first: &str, kind: &str, owner: Option<&str>, name: &ast::Ident) -> Diagnostic {
    let name_text = &name.name;
    let message = if first == kind {
        format!("{kind} `{name_text}` is defined twice")
    } else {
        let owner = owner.map_or_else(String::new, |class| format!(" of `{class}`"));
        format!("`{name_text}` is already a {first}{owner}")
    };
    Diagnostic::error(name.span, message)
}

#[cfg(test)]
mod tests;
