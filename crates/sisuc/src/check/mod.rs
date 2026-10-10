//! The checker: signatures, scopes, names and types. It lowers the program to `tir` as it
//! checks, and records every error instead of stopping at the first.

mod expr;
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
}

type Checked = Result<tir::Expr, Poisoned>;
type CheckedBlock = Result<tir::Block, Poisoned>;

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
                    if matches!(name.name.as_str(), "i64" | "bool") {
                        self.diagnostics.push(Diagnostic::error(
                            name.span,
                            format!("`{}` is a built-in type", name.name),
                        ));
                    } else if self.first_callable(&mut first, "class", name) {
                        self.class_ids.insert(name.name.clone(), id);
                    }
                    next_function += class
                        .members
                        .iter()
                        .filter(|m| matches!(m, ast::Member::Method(_)))
                        .count();
                }
                ast::Item::Function(f) => {
                    if self.first_callable(&mut first, "function", &f.name) {
                        self.function_ids
                            .insert(f.name.name.clone(), tir::FuncId(next_function));
                    }
                    next_function += 1;
                }
            }
        }
    }

    /// Whether `name` is the first function or class of its name; reports it when not.
    fn first_callable(
        &mut self,
        first: &mut HashMap<String, Span>,
        kind: &str,
        name: &ast::Ident,
    ) -> bool {
        if name.name == "print" {
            self.diagnostics
                .push(duplicate(kind, name).label("`print` is built in"));
            return false;
        }
        self.first_definition(first, kind, name)
    }

    /// Whether `name` is the first of its name in `first`, which it joins; reports it when not.
    fn first_definition(
        &mut self,
        first: &mut HashMap<String, Span>,
        kind: &str,
        name: &ast::Ident,
    ) -> bool {
        if let Some(&span) = first.get(&name.name) {
            self.diagnostics
                .push(duplicate(kind, name).secondary(span, "first defined here"));
            return false;
        }
        first.insert(name.name.clone(), name.span);
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
                        if self.first_definition(&mut first, "field", &field.name) {
                            self.classes[id.0].fields.push(FieldInfo {
                                name: field.name.clone(),
                                ty,
                                mutable: field.mutable,
                            });
                        }
                    }
                    ast::Member::Method(method) => {
                        let func = tir::FuncId(self.signatures.len());
                        if self.first_definition(&mut first, "method", &method.name) {
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

    /// Reports each cycle of class fields once, on its first class in source order, at that
    /// class's first field into the cycle: no first object of such a class can exist.
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
                .expect("a class on a cycle has a field into it");
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

    /// `None`, reported, for an unknown type.
    fn resolve(&mut self, ty: &ast::TypeExpr) -> Option<Type> {
        match ty.name.as_str() {
            "i64" => Some(Type::I64),
            "bool" => Some(Type::Bool),
            name => {
                if let Some(&id) = self.class_ids.get(name) {
                    return Some(Type::Class(id));
                }
                self.diagnostics.push(
                    Diagnostic::error(ty.span, format!("unknown type `{name}`"))
                        .help("the types are `i64`, `bool` and classes"),
                );
                None
            }
        }
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
        let body = self.block(&f.body);
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
                    self.mismatch(value_span(&f.body), want, got)
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

    /// A type as messages quote it: "`i64`", "`Tree`".
    fn show(&self, ty: &Type) -> String {
        match ty {
            Type::Class(id) => format!("`{}`", self.classes[id.0].name.name),
            ty => format!("`{ty}`"),
        }
    }

    fn mismatch(&self, span: Span, want: &Type, got: &Type) -> Diagnostic {
        Diagnostic::error(
            span,
            format!("expected {}, found {}", self.show(want), self.show(got)),
        )
    }
}

/// Where a block's value comes from: its last statement, or the block itself when empty.
fn value_span(block: &ast::Block) -> Span {
    block.stmts.last().map_or(block.span, |s| s.span)
}

fn not_found(name: &str, span: Span) -> Diagnostic {
    Diagnostic::error(span, format!("cannot find `{name}` in this scope"))
}

/// `kind` is the second declaration's: "function", "class", "field" or "method".
fn duplicate(kind: &str, name: &ast::Ident) -> Diagnostic {
    Diagnostic::error(
        name.span,
        format!("{kind} `{}` is defined twice", name.name),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::line_col;
    use crate::lexer::lex;
    use crate::parser::parse;

    fn checked(src: &str) -> (Option<tir::Program>, Vec<Diagnostic>) {
        check(&parse(&lex(src).expect("source lexes")).expect("source parses"))
    }

    fn diags(src: &str) -> Vec<Diagnostic> {
        checked(src).1
    }

    /// The typed tree of a program that checks without a diagnostic.
    fn lowered(src: &str) -> String {
        let (program, ds) = checked(src);
        assert!(ds.is_empty(), "{ds:?}");
        program.expect("no errors").to_string()
    }

    /// Asserts the diagnostics are exactly these errors, in order: `(message, (line, col))`,
    /// and that no `tir` comes out.
    fn errors(src: &str, expected: &[(&str, (usize, usize))]) {
        let (program, ds) = checked(src);
        assert!(program.is_none(), "an error yields no `tir`");
        let found: Vec<_> = ds
            .iter()
            .map(|d| (d.severity, d.message.as_str(), at(src, d.span)))
            .collect();
        let expected: Vec<_> = expected
            .iter()
            .map(|&(message, pos)| (Severity::Error, message, pos))
            .collect();
        assert_eq!(found, expected);
    }

    fn at(src: &str, span: Span) -> (usize, usize) {
        line_col(src, span.start)
    }

    /// Asserts exactly one error with these parts, and no `tir`; `secondary` is
    /// `(line, col, text)` rows.
    fn expect_error(
        src: &str,
        message: &str,
        pos: (usize, usize),
        label: Option<&str>,
        secondary: &[(usize, usize, &str)],
        help: Option<&str>,
    ) {
        let (program, ds) = checked(src);
        assert!(program.is_none(), "an error yields no `tir`");
        assert_eq!(ds.len(), 1, "{ds:?}");
        let d = &ds[0];
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.message, message);
        assert_eq!(at(src, d.span), pos);
        assert_eq!(d.label.as_deref(), label);
        let found: Vec<_> = d
            .secondary
            .iter()
            .map(|(span, text)| {
                let (line, col) = at(src, *span);
                (line, col, text.as_str())
            })
            .collect();
        assert_eq!(found, secondary);
        assert_eq!(d.help.as_deref(), help);
    }

    /// Asserts exactly one error with this message and position, and no label or help.
    fn error(src: &str, message: &str, pos: (usize, usize)) {
        expect_error(src, message, pos, None, &[], None);
    }

    /// Asserts exactly one diagnostic: a warning with this message and position.
    fn warning(src: &str, message: &str, pos: (usize, usize)) {
        let ds = diags(src);
        assert_eq!(ds.len(), 1, "{ds:?}");
        assert_eq!(ds[0].severity, Severity::Warning);
        assert_eq!(ds[0].message, message);
        assert_eq!(at(src, ds[0].span), pos);
    }

    fn clean(src: &str) {
        let ds = diags(src);
        assert!(ds.is_empty(), "{ds:?}");
    }

    #[test]
    fn duplicate_function() {
        expect_error(
            "fn main() {}\nfn main() {}",
            "function `main` is defined twice",
            (2, 4),
            None,
            &[(1, 4, "first defined here")],
            None,
        );
    }

    #[test]
    fn print_is_built_in() {
        expect_error(
            "fn print() {}\nfn main() {}",
            "function `print` is defined twice",
            (1, 4),
            Some("`print` is built in"),
            &[],
            None,
        );
    }

    #[test]
    fn missing_main() {
        let src = "fn f() {}";
        expect_error(src, "no `main` function", (1, 1), None, &[], None);
        assert_eq!(diags(src)[0].span, Span::new(0, 0));
    }

    #[test]
    fn main_with_params() {
        expect_error(
            "fn main(x: i64) {}",
            "`main` takes no parameters and returns no value",
            (1, 4),
            None,
            &[],
            None,
        );
    }

    #[test]
    fn main_with_return_type() {
        expect_error(
            "fn main() -> i64 { 0 }",
            "`main` takes no parameters and returns no value",
            (1, 4),
            None,
            &[],
            None,
        );
    }

    #[test]
    fn unknown_type() {
        expect_error(
            "fn main() { let x: str = 1 }",
            "unknown type `str`",
            (1, 20),
            None,
            &[],
            Some("the types are `i64`, `bool` and classes"),
        );
    }

    #[test]
    fn unknown_name() {
        expect_error(
            "fn main() { print(y) }",
            "cannot find `y` in this scope",
            (1, 19),
            None,
            &[],
            None,
        );
    }

    #[test]
    fn unknown_function() {
        expect_error(
            "fn main() { g() }",
            "cannot find function `g`",
            (1, 13),
            None,
            &[],
            None,
        );
    }

    #[test]
    fn redeclare_in_block() {
        expect_error(
            "fn main() {\n    let x = 1\n    let x = 2\n}",
            "`x` is already declared in this scope",
            (3, 9),
            None,
            &[(2, 9, "first declared here")],
            None,
        );
    }

    #[test]
    fn repeated_param() {
        expect_error(
            "fn f(a: i64, a: i64) {}\nfn main() {}",
            "`a` is already declared in this scope",
            (1, 14),
            None,
            &[(1, 6, "first declared here")],
            None,
        );
    }

    #[test]
    fn shadowing_in_inner_block() {
        let src = "fn main() {\n    let x = 1\n    if true {\n        let x = true\n        print(x)\n    }\n    print(x)\n}";
        assert!(diags(src).is_empty());
    }

    #[test]
    fn functions_in_any_order() {
        assert!(diags("fn main() { print(f(1)) }\nfn f(n: i64) -> i64 { n }").is_empty());
    }

    #[test]
    fn var_may_shadow_a_param() {
        let src = "fn f(n: i64) {\n    var n = n\n    n = 1\n}\nfn main() {}";
        clean(src);
    }

    #[test]
    fn every_error_in_one_run() {
        errors(
            "fn main() {\n    print(a)\n    let b: bool = 1\n    print(1 + true)\n}",
            &[
                ("cannot find `a` in this scope", (2, 11)),
                ("expected `bool`, found `i64`", (3, 19)),
                ("expected `i64`, found `bool`", (4, 15)),
            ],
        );
    }

    #[test]
    fn poisoned_binding_reports_once() {
        errors(
            "fn main() {\n    let x = nope\n    print(x + 1)\n}",
            &[("cannot find `nope` in this scope", (2, 13))],
        );
    }

    #[test]
    fn poisoned_operand_checks_siblings() {
        errors(
            "fn f(a: i64, b: i64) {}\nfn main() {\n    f(nope, 1 + true)\n}",
            &[
                ("cannot find `nope` in this scope", (3, 7)),
                ("expected `i64`, found `bool`", (3, 17)),
            ],
        );
    }

    #[test]
    fn if_with_failed_branch_takes_other_type() {
        errors(
            "fn main() {\n    let y = if true { nope } else { 1 }\n    print(y + true)\n}",
            &[
                ("cannot find `nope` in this scope", (2, 23)),
                ("expected `i64`, found `bool`", (3, 15)),
            ],
        );
    }

    #[test]
    fn unknown_let_type_poisons() {
        errors(
            "fn main() {\n    let x: str = 1\n    print(x + true)\n}",
            &[("unknown type `str`", (2, 12))],
        );
    }

    #[test]
    fn unknown_param_type_still_checks_body() {
        errors(
            "fn f(a: str) -> i64 { true }\nfn main() { f(1) }",
            &[
                ("unknown type `str`", (1, 9)),
                ("expected `i64`, found `bool`", (1, 23)),
            ],
        );
    }

    #[test]
    fn duplicate_checked_against_own_signature() {
        errors(
            "fn f(a: i64) {}\nfn f(b: bool) -> bool { b }\nfn main() {}",
            &[("function `f` is defined twice", (2, 4))],
        );
    }

    #[test]
    fn duplicate_call_resolves_to_first() {
        errors(
            "fn f(a: i64) -> i64 { a }\nfn f(b: bool) -> bool { b }\nfn main() {\n    print(f(1))\n}",
            &[("function `f` is defined twice", (2, 4))],
        );
    }

    #[test]
    fn duplicate_let_keeps_first() {
        errors(
            "fn main() {\n    let x = 1\n    let x = true\n    print(x + 1)\n}",
            &[("`x` is already declared in this scope", (3, 9))],
        );
    }

    #[test]
    fn failed_call_still_checks_arguments() {
        const MISMATCH: &str = "expected `i64`, found `bool`";
        let rows = [
            (
                "unknown function",
                "fn main() {\n    g(1 + true)\n}",
                vec![("cannot find function `g`", (2, 5)), (MISMATCH, (2, 11))],
            ),
            (
                "arity",
                "fn f(a: i64) {}\nfn main() {\n    f(1 + true, 2)\n}",
                vec![
                    ("`f` takes 1 argument, found 2", (3, 5)),
                    (MISMATCH, (3, 11)),
                ],
            ),
            (
                "print arity",
                "fn main() {\n    print(1 + true, 2)\n}",
                vec![
                    ("`print` takes one `i64` or `bool`", (2, 5)),
                    (MISMATCH, (2, 15)),
                ],
            ),
        ];
        for (name, src, expected) in rows {
            println!("row: {name}");
            errors(src, &expected);
        }
    }

    #[test]
    fn duplicate_body_still_checked() {
        errors(
            "fn f() {}\nfn f() { print(nope) }\nfn main() {}",
            &[
                ("function `f` is defined twice", (2, 4)),
                ("cannot find `nope` in this scope", (2, 16)),
            ],
        );
    }

    #[test]
    fn missing_main_and_body_error() {
        errors(
            "fn f() { print(nope) }",
            &[
                ("no `main` function", (1, 1)),
                ("cannot find `nope` in this scope", (1, 16)),
            ],
        );
    }

    #[test]
    fn duplicate_body_uses_own_return_type() {
        errors(
            "fn f() -> i64 { 1 }\nfn f() -> bool { true }\nfn main() {}",
            &[("function `f` is defined twice", (2, 4))],
        );
    }

    #[test]
    fn if_with_failed_and_never_branches_is_error() {
        errors(
            "fn main() {\n    let c = true\n    let x = if c { nope } else { return }\n    print(1)\n}",
            &[("cannot find `nope` in this scope", (3, 20))],
        );
    }

    /// A rule that fails makes its expression `Error`. Each row binds the failed expression
    /// and uses it where a typed result would cascade into a second error.
    #[test]
    fn failed_rule_makes_its_expression_error() {
        const MISMATCH: &str = "expected `i64`, found `bool`";
        const F: &str = "fn f(a: i64) -> i64 { a }\n";
        let rows: [(&str, String, &str, (usize, usize)); 12] = [
            (
                "unary",
                "fn main() {\n    let x = -true\n    print(x == false)\n}".into(),
                MISMATCH,
                (2, 14),
            ),
            (
                "binary lhs",
                "fn main() {\n    let x = true + 1\n    print(x == false)\n}".into(),
                MISMATCH,
                (2, 13),
            ),
            (
                "binary rhs",
                "fn main() {\n    let x = 1 + true\n    print(x == false)\n}".into(),
                MISMATCH,
                (2, 17),
            ),
            (
                "compare",
                "fn main() {\n    let x = 1 < true < 2\n    print(x + 1)\n}".into(),
                MISMATCH,
                (2, 17),
            ),
            (
                "equal",
                "fn main() {\n    let x = 1 == true\n    print(x + 1)\n}".into(),
                "cannot compare `i64` with `bool`",
                (2, 13),
            ),
            (
                "call arity",
                format!("{F}fn main() {{\n    let x = f()\n    print(x == false)\n}}"),
                "`f` takes 1 argument, found 0",
                (3, 13),
            ),
            (
                "call argument",
                format!("{F}fn main() {{\n    let x = f(true)\n    print(x == false)\n}}"),
                MISMATCH,
                (3, 15),
            ),
            (
                "print arity",
                "fn main() {\n    let x = print(1, 2)\n    print(x)\n}".into(),
                "`print` takes one `i64` or `bool`",
                (2, 13),
            ),
            (
                "print unit argument",
                "fn main() {\n    let x = print(print(1))\n    print(x)\n}".into(),
                "`print` takes one `i64` or `bool`",
                (2, 13),
            ),
            (
                "if branches",
                "fn main() {\n    let x = if true { 1 } else { false }\n    print(x == false)\n}"
                    .into(),
                MISMATCH,
                (2, 34),
            ),
            (
                "if condition with else",
                "fn main() {\n    let x = if 1 { 2 } else { 3 }\n    print(x == false)\n}".into(),
                "expected `bool`, found `i64`",
                (2, 16),
            ),
            (
                "if condition without else",
                "fn main() {\n    let x = if 1 { print(2) }\n    print(x)\n}".into(),
                "expected `bool`, found `i64`",
                (2, 16),
            ),
        ];
        for (name, src, message, pos) in rows {
            println!("row: {name}");
            errors(&src, &[(message, pos)]);
        }
    }

    #[test]
    fn warning_only_program_lowers() {
        let src = "fn main() {\n    var x = 1\n    print(x)\n}";
        let (program, ds) = checked(src);
        assert!(program.is_some());
        assert_eq!(ds.len(), 1, "{ds:?}");
        assert_eq!(ds[0].severity, Severity::Warning);
        assert_eq!(ds[0].message, "`x` is never reassigned");
        assert_eq!(at(src, ds[0].span), (2, 9));
    }

    #[test]
    fn warning_and_error_in_position_order() {
        // The warning is found when the scope closes, after the error.
        let src = "fn main() {\n    var x = 1\n    print(nope)\n}";
        let found: Vec<_> = diags(src)
            .iter()
            .map(|d| (d.severity, d.message.clone(), at(src, d.span)))
            .collect();
        assert_eq!(
            found,
            [
                (
                    Severity::Warning,
                    "`x` is never reassigned".to_string(),
                    (2, 9)
                ),
                (
                    Severity::Error,
                    "cannot find `nope` in this scope".to_string(),
                    (3, 11)
                ),
            ]
        );
    }

    #[test]
    fn trailing_expression_is_the_block_value() {
        let (program, ds) =
            checked("fn f() -> i64 {\n    let x = 1\n    x\n}\nfn main() {\n    let y = f()\n}");
        assert!(ds.is_empty(), "{ds:?}");
        let program = program.expect("no errors");
        let f = &program.functions[0].body;
        assert!(matches!(f.stmts.as_slice(), [tir::Stmt::Let { .. }]));
        assert!(matches!(
            f.value.as_deref(),
            Some(tir::Expr {
                kind: tir::ExprKind::Local(_),
                ..
            })
        ));
        let main = &program.functions[1].body;
        assert!(matches!(main.stmts.as_slice(), [tir::Stmt::Let { .. }]));
        assert!(main.value.is_none());
    }

    #[test]
    fn lowers_while_to_loop() {
        assert_eq!(
            lowered("fn main() {\n    var i = 0\n    while i < 3 {\n        i = i + 1\n    }\n}"),
            "(fn main () unit (block (var i#0 0) (loop (block (if (< i#0 3) (block (= i#0 (+ i#0 1))) (block (break)))))))"
        );
    }

    #[test]
    fn lowers_continue() {
        assert_eq!(
            lowered(
                "fn main() {\n    var i = 0\n    while i < 3 {\n        i = i + 1\n        if i == 2 { continue }\n        print(i)\n    }\n}"
            ),
            "(fn main () unit (block (var i#0 0) (loop (block (if (< i#0 3) (block (= i#0 (+ i#0 1)) (if (== i#0 2) (block (continue))) (print i#0)) (block (break)))))))"
        );
    }

    #[test]
    fn break_outside_loop() {
        errors(
            "fn main() { break }",
            &[("`break` outside a loop", (1, 13))],
        );
    }

    #[test]
    fn continue_outside_loop() {
        errors(
            "fn main() { continue }",
            &[("`continue` outside a loop", (1, 13))],
        );
    }

    #[test]
    fn break_after_loop() {
        errors(
            "fn main() {\n    while true {}\n    break\n}",
            &[("`break` outside a loop", (3, 5))],
        );
    }

    #[test]
    fn statement_after_break() {
        warning(
            "fn main() {\n    while true {\n        break\n        print(1)\n    }\n}",
            "unreachable code",
            (4, 9),
        );
    }

    #[test]
    fn shadowing_makes_two_locals() {
        assert_eq!(
            lowered(
                "fn main() {\n    let x = 1\n    if true {\n        let x = true\n        print(x)\n    }\n    print(x)\n}"
            ),
            "(fn main () unit (block (let x#0 1) (if true (block (let x#1 true) (print x#1))) (print x#0)))"
        );
    }

    #[test]
    fn lowers_compound_assign() {
        assert_eq!(
            lowered("fn main() {\n    var x = 1\n    x *= 3\n    print(x)\n}"),
            "(fn main () unit (block (var x#0 1) (= x#0 (* x#0 3)) (print x#0)))"
        );
    }

    #[test]
    fn lowers_not_equal() {
        assert_eq!(
            lowered("fn f(a: i64) -> bool { a != 1 }\nfn main() {}"),
            "(fn f ((a#0 i64)) bool (block (!= a#0 1)))\n(fn main () unit (block))"
        );
    }

    #[test]
    fn unknown_type_in_signature() {
        expect_error(
            "fn f(a: str) {}\nfn main() {}",
            "unknown type `str`",
            (1, 9),
            None,
            &[],
            Some("the types are `i64`, `bool` and classes"),
        );
    }

    #[test]
    fn condition_must_be_bool() {
        error(
            "fn main() { if 1 { } }",
            "expected `bool`, found `i64`",
            (1, 16),
        );
    }

    #[test]
    fn while_condition_must_be_bool() {
        error(
            "fn main() { while 1 { } }",
            "expected `bool`, found `i64`",
            (1, 19),
        );
    }

    #[test]
    fn arithmetic_needs_i64() {
        error(
            "fn main() { print(1 + true) }",
            "expected `i64`, found `bool`",
            (1, 23),
        );
    }

    #[test]
    fn not_needs_bool() {
        error(
            "fn main() { print(!1) }",
            "expected `bool`, found `i64`",
            (1, 20),
        );
    }

    #[test]
    fn equality_needs_same_type() {
        error(
            "fn main() { print(1 == true) }",
            "cannot compare `i64` with `bool`",
            (1, 19),
        );
    }

    #[test]
    fn arity() {
        error(
            "fn f(a: i64) {}\nfn main() { f() }",
            "`f` takes 1 argument, found 0",
            (2, 13),
        );
        error(
            "fn f(a: i64, b: i64) {}\nfn main() { f(1) }",
            "`f` takes 2 arguments, found 1",
            (2, 13),
        );
    }

    #[test]
    fn argument_type() {
        error(
            "fn f(a: i64) {}\nfn main() { f(true) }",
            "expected `i64`, found `bool`",
            (2, 15),
        );
    }

    #[test]
    fn print_no_argument() {
        error(
            "fn main() { print() }",
            "`print` takes one `i64` or `bool`",
            (1, 13),
        );
    }

    #[test]
    fn print_two_arguments() {
        error(
            "fn main() { print(1, 2) }",
            "`print` takes one `i64` or `bool`",
            (1, 13),
        );
    }

    #[test]
    fn print_unit_argument() {
        error(
            "fn g() {}\nfn main() { print(g()) }",
            "`print` takes one `i64` or `bool`",
            (2, 13),
        );
    }

    #[test]
    fn bind_unit() {
        error(
            "fn g() {}\nfn main() { let x = g() }",
            "expression has no value",
            (2, 21),
        );
    }

    #[test]
    fn if_without_else_has_no_value() {
        error(
            "fn main() {\n    let x = if true { 1 }\n}",
            "expression has no value",
            (2, 13),
        );
    }

    #[test]
    fn if_branches_differ() {
        error(
            "fn main() {\n    let x = if true { 1 } else { false }\n}",
            "expected `i64`, found `bool`",
            (2, 34),
        );
    }

    #[test]
    fn body_value_type() {
        error(
            "fn f() -> i64 {\n    true\n}\nfn main() {}",
            "expected `i64`, found `bool`",
            (2, 5),
        );
    }

    #[test]
    fn body_without_value() {
        let src = "fn f() -> i64 {\n    let x = 1\n}\nfn main() {}";
        error(
            src,
            "function `f` must return `i64`, but its body has no value",
            (3, 1),
        );
        assert_eq!(diags(src)[0].span, Span::new(30, 31));
    }

    #[test]
    fn return_value_type() {
        error(
            "fn f() -> i64 {\n    return true\n}\nfn main() {}",
            "expected `i64`, found `bool`",
            (2, 12),
        );
    }

    #[test]
    fn return_needs_value() {
        error(
            "fn f() -> i64 {\n    return\n}\nfn main() {}",
            "`return` needs a value of type `i64`",
            (2, 5),
        );
    }

    #[test]
    fn return_value_in_unit_fn() {
        error(
            "fn main() {\n    return 1\n}",
            "this function returns no value",
            (2, 12),
        );
    }

    #[test]
    fn all_paths_return_is_fine() {
        clean(
            "fn sign(n: i64) -> i64 {\n    if n < 0 { return -1 } else { return 1 }\n}\nfn main() {}",
        );
    }

    #[test]
    fn never_as_return_value() {
        error(
            "fn f(c: bool) -> i64 {\n    return if c { return 1 } else { return 2 }\n}\nfn main() {}",
            "unreachable code",
            (2, 12),
        );
    }

    #[test]
    fn annotated_let_type() {
        error(
            "fn main() { let x: bool = 1 }",
            "expected `bool`, found `i64`",
            (1, 27),
        );
    }

    #[test]
    fn annotated_let_matches() {
        clean("fn main() {\n    let x: bool = true\n    print(x)\n}");
    }

    #[test]
    fn never_as_initializer() {
        error(
            "fn f(c: bool) -> i64 {\n    let x = if c { return 1 } else { return 2 }\n    x\n}\nfn main() {}",
            "unreachable code",
            (2, 13),
        );
    }

    #[test]
    fn never_as_condition() {
        error(
            "fn f(c: bool) -> i64 {\n    while if c { return 1 } else { return 2 } {}\n    1\n}\nfn main() {}",
            "unreachable code",
            (2, 11),
        );
    }

    #[test]
    fn never_as_operand() {
        error(
            "fn f(c: bool) -> i64 {\n    1 + if c { return 1 } else { return 2 }\n}\nfn main() {}",
            "unreachable code",
            (2, 9),
        );
    }

    #[test]
    fn statement_after_return() {
        warning(
            "fn main() {\n    return\n    print(1)\n}",
            "unreachable code",
            (3, 5),
        );
    }

    #[test]
    fn block_after_return_is_never() {
        warning(
            "fn f() -> i64 {\n    return 1\n    print(2)\n}\nfn main() {}",
            "unreachable code",
            (3, 5),
        );
    }

    #[test]
    fn neg_needs_i64() {
        error(
            "fn main() { print(-true) }",
            "expected `i64`, found `bool`",
            (1, 20),
        );
    }

    #[test]
    fn less_needs_i64() {
        error(
            "fn main() { print(true < false) }",
            "expected `i64`, found `bool`",
            (1, 19),
        );
    }

    #[test]
    fn and_needs_bool() {
        error(
            "fn main() { print(1 && true) }",
            "expected `bool`, found `i64`",
            (1, 19),
        );
    }

    #[test]
    fn unit_operand() {
        error(
            "fn g() {}\nfn main() { print(1 + g()) }",
            "expression has no value",
            (2, 23),
        );
    }

    #[test]
    fn unit_argument() {
        error(
            "fn g() {}\nfn f(a: i64) {}\nfn main() { f(g()) }",
            "expression has no value",
            (3, 15),
        );
    }

    #[test]
    fn separate_namespaces() {
        clean("fn f() -> i64 { 1 }\nfn main() {\n    let f = 2\n    print(f)\n    print(f())\n}");
    }

    #[test]
    fn never_branch_takes_other_type() {
        clean(
            "fn f(c: bool) -> i64 {\n    let x = if c { return 1 } else { 2 }\n    let y = if c { 3 } else { return 4 }\n    x + y\n}\nfn main() {}",
        );
    }

    #[test]
    fn unreachable_warned_once_per_block() {
        warning(
            "fn main() {\n    return\n    print(1)\n    print(2)\n}",
            "unreachable code",
            (3, 5),
        );
    }

    #[test]
    fn statements_after_return_are_checked() {
        let src = "fn main() {\n    return\n    print(true + 1)\n}";
        let ds = diags(src);
        let found: Vec<_> = ds
            .iter()
            .map(|d| (d.severity, d.message.as_str(), at(src, d.span)))
            .collect();
        assert_eq!(
            found,
            [
                (Severity::Warning, "unreachable code", (3, 5)),
                (Severity::Error, "expected `i64`, found `bool`", (3, 11)),
            ]
        );
    }

    #[test]
    fn assign_to_let() {
        let src = "fn main() {\n    let n = 0\n    n = 1\n}";
        expect_error(
            src,
            "cannot assign to `n`",
            (3, 5),
            Some("cannot assign twice"),
            &[(2, 9, "declared with `let` here")],
            Some("declare it with `var`"),
        );
        let span = diags(src)[0].span;
        assert_eq!(&src[span.start..span.end], "n = 1");
    }

    #[test]
    fn never_assigned_value() {
        error(
            "fn f(c: bool) -> i64 {\n    var x = 0\n    x = if c { return 1 } else { return 2 }\n    x\n}\nfn main() {}",
            "unreachable code",
            (3, 9),
        );
    }

    #[test]
    fn compound_assign_to_let() {
        expect_error(
            "fn main() {\n    let n = 0\n    n += 1\n}",
            "cannot assign to `n`",
            (3, 5),
            Some("cannot assign twice"),
            &[(2, 9, "declared with `let` here")],
            Some("declare it with `var`"),
        );
    }

    #[test]
    fn assign_to_param() {
        expect_error(
            "fn f(n: i64) {\n    n = 1\n}\nfn main() {}",
            "cannot assign to `n`",
            (2, 5),
            None,
            &[(1, 6, "declared as a parameter here")],
            Some("copy it into a `var`: `var n = n`"),
        );
    }

    #[test]
    fn compound_assign_to_param() {
        expect_error(
            "fn f(n: i64) {\n    n += 1\n}\nfn main() {}",
            "cannot assign to `n`",
            (2, 5),
            None,
            &[(1, 6, "declared as a parameter here")],
            Some("copy it into a `var`: `var n = n`"),
        );
    }

    #[test]
    fn compound_assign_unknown_target_once() {
        error(
            "fn main() {\n    nope += 1\n}",
            "cannot find `nope` in this scope",
            (2, 5),
        );
    }

    #[test]
    fn compound_assign_unknown_target_and_bad_value() {
        errors(
            "fn main() {\n    nope += true + 1\n}",
            &[
                ("cannot find `nope` in this scope", (2, 5)),
                ("expected `i64`, found `bool`", (2, 13)),
            ],
        );
    }

    #[test]
    fn assign_type() {
        error(
            "fn main() {\n    var x = 1\n    x = true\n}",
            "expected `i64`, found `bool`",
            (3, 9),
        );
    }

    #[test]
    fn unused_var() {
        let src = "fn main() {\n    var count = 0\n    print(count)\n}";
        warning(src, "`count` is never reassigned", (2, 9));
        assert_eq!(diags(src)[0].help.as_deref(), Some("declare it with `let`"));
    }

    #[test]
    fn unused_vars_in_source_order() {
        // The inner scope pops first, so this fails unless warnings are sorted.
        let src = "fn main() {\n    var a = 0\n    if true {\n        var b = 0\n        print(b)\n    }\n    print(a)\n}";
        let found: Vec<_> = diags(src)
            .iter()
            .map(|d| (d.severity, d.message.clone(), at(src, d.span)))
            .collect();
        assert_eq!(
            found,
            [
                (
                    Severity::Warning,
                    "`a` is never reassigned".to_string(),
                    (2, 9)
                ),
                (
                    Severity::Warning,
                    "`b` is never reassigned".to_string(),
                    (4, 13)
                ),
            ]
        );
    }

    #[test]
    fn var_reassigned_in_inner_block() {
        clean("fn main() {\n    var n = 0\n    if true { n = 1 }\n    print(n)\n}");
    }

    #[test]
    fn empty_file_has_no_main() {
        let ds = diags("");
        assert_eq!(ds.len(), 1, "{ds:?}");
        assert_eq!(ds[0].message, "no `main` function");
        assert_eq!(at("", ds[0].span), (1, 1));
        assert!(
            ds[0]
                .render("empty.sisu", "")
                .starts_with("error: no `main` function")
        );
    }

    #[test]
    fn class_and_function_share_names() {
        expect_error(
            "class A {}\nfn A() {}\nfn main() {}",
            "function `A` is defined twice",
            (2, 4),
            None,
            &[(1, 7, "first defined here")],
            None,
        );
    }

    #[test]
    fn class_after_function_of_same_name() {
        expect_error(
            "fn A() {}\nclass A {}\nfn main() {}",
            "class `A` is defined twice",
            (2, 7),
            None,
            &[(1, 4, "first defined here")],
            None,
        );
    }

    #[test]
    fn class_defined_twice() {
        // The `bool` argument shows that `X` still names the first class.
        let src =
            "class X {\n    let a: i64\n}\nclass X {\n    let a: bool\n}\nfn main() { X(a: true) }";
        errors(
            src,
            &[
                ("class `X` is defined twice", (4, 7)),
                ("expected `i64`, found `bool`", (7, 18)),
            ],
        );
        let secondary: Vec<_> = diags(src)[0]
            .secondary
            .iter()
            .map(|(span, text)| (at(src, *span), text.clone()))
            .collect();
        assert_eq!(secondary, [((1, 7), "first defined here".to_owned())]);
    }

    #[test]
    fn class_named_print() {
        expect_error(
            "class print {}\nfn main() {}",
            "class `print` is defined twice",
            (1, 7),
            Some("`print` is built in"),
            &[],
            None,
        );
    }

    #[test]
    fn duplicate_member() {
        expect_error(
            "class A {\n    let x: i64\n    fn x(self) {}\n}\nfn main() {}",
            "method `x` is defined twice",
            (3, 8),
            None,
            &[(2, 9, "first defined here")],
            None,
        );
    }

    #[test]
    fn duplicate_field() {
        expect_error(
            "class A {\n    fn x(self) {}\n    var x: i64\n}\nfn main() {}",
            "field `x` is defined twice",
            (3, 9),
            None,
            &[(2, 8, "first defined here")],
            None,
        );
    }

    #[test]
    fn builtin_type_as_class() {
        error(
            "class i64 {}\nfn main() {}",
            "`i64` is a built-in type",
            (1, 7),
        );
    }

    #[test]
    fn never_constructed_self() {
        expect_error(
            "class Node {\n    let next: Node\n}\nfn main() {}",
            "`Node` can never be constructed: field `next` needs a `Node`",
            (2, 9),
            None,
            &[],
            Some("make it optional: `Node?`"),
        );
    }

    #[test]
    fn never_constructed_cycle_of_two() {
        expect_error(
            "class A {\n    let b: B\n}\nclass B {\n    let a: A\n}\nfn main() {}",
            "`A` can never be constructed: field `b` needs a `B`",
            (2, 9),
            None,
            &[],
            Some("make it optional: `B?`"),
        );
    }

    #[test]
    fn never_constructed_once_per_strongly_connected_component() {
        // `A`, `B` and `C` form one component of two cycles, `E` another; `D` only points in.
        errors(
            "class A {\n    let b: B\n    let c: C\n}\nclass B {\n    let a: A\n}\nclass C {\n    let a: A\n}\nclass D {\n    let a: A\n}\nclass E {\n    let e: E\n}\nfn main() {}",
            &[
                (
                    "`A` can never be constructed: field `b` needs a `B`",
                    (2, 9),
                ),
                (
                    "`E` can never be constructed: field `e` needs a `E`",
                    (15, 9),
                ),
            ],
        );
    }

    #[test]
    fn constructor_missing_label() {
        expect_error(
            "class P {\n    let x: i64\n}\nfn main() { P(1) }",
            "this argument needs a label",
            (4, 15),
            None,
            &[],
            Some("label it with its field: `x: ...`"),
        );
    }

    #[test]
    fn constructor_unknown_label() {
        error(
            "class P {\n    let x: i64\n}\nfn main() { P(z: 1) }",
            "`P` has no field `z`",
            (4, 15),
        );
    }

    #[test]
    fn constructor_out_of_order() {
        expect_error(
            "class P {\n    let x: i64\n    let y: i64\n}\nfn main() { P(y: 1, x: 2) }",
            "expected field `x` here, found `y`",
            (5, 15),
            None,
            &[],
            Some("name the fields in declaration order: `x`, `y`"),
        );
    }

    #[test]
    fn constructor_missing_field() {
        expect_error(
            "class P {\n    let x: i64\n    let y: i64\n}\nfn main() { P(x: 1) }",
            "missing field `y`",
            (5, 13),
            None,
            &[],
            Some("name the fields in declaration order: `x`, `y`"),
        );
    }

    #[test]
    fn constructor_extra_argument() {
        error(
            "class P {\n    let x: i64\n}\nfn main() { P(x: 1, y: 2) }",
            "too many arguments: `P` has 1 field",
            (4, 21),
        );
        error(
            "class P {\n    let x: i64\n    let y: i64\n}\nfn main() { P(x: 1, y: 2, z: 3) }",
            "too many arguments: `P` has 2 fields",
            (5, 27),
        );
    }

    #[test]
    fn constructor_field_type() {
        error(
            "class P {\n    let x: i64\n}\nfn main() { P(x: true) }",
            "expected `i64`, found `bool`",
            (4, 18),
        );
    }

    #[test]
    fn label_outside_constructor() {
        error(
            "fn f(a: i64) {}\nfn main() { f(a: 1) }",
            "labels on arguments come in milestone 10",
            (2, 15),
        );
    }

    #[test]
    fn constructor_reports_its_first_mistake_and_later_expression_errors() {
        let class = "class P {\n    let x: i64\n    let y: i64\n}\n";
        errors(
            &format!("{class}fn main() {{ P(1, nope) }}"),
            &[
                ("this argument needs a label", (5, 15)),
                ("cannot find `nope` in this scope", (5, 18)),
            ],
        );
        errors(
            &format!("{class}fn main() {{ P(1, 2) }}"),
            &[("this argument needs a label", (5, 15))],
        );
    }

    #[test]
    fn each_label_on_a_call_is_reported() {
        errors(
            "fn f(a: i64, b: i64) {}\nfn main() { f(a: 1, b: 2) }",
            &[
                ("labels on arguments come in milestone 10", (2, 15)),
                ("labels on arguments come in milestone 10", (2, 21)),
            ],
        );
    }

    #[test]
    fn labels_on_unknown_calls_are_reported() {
        errors(
            "class P {}\nfn main() { P().missing(x: 1 + true) }",
            &[
                ("`P` has no method `missing`", (2, 17)),
                ("labels on arguments come in milestone 10", (2, 25)),
                ("expected `i64`, found `bool`", (2, 32)),
            ],
        );
        errors(
            "fn main() { nope(x: 1 + true) }",
            &[
                ("cannot find function `nope`", (1, 13)),
                ("labels on arguments come in milestone 10", (1, 18)),
                ("expected `i64`, found `bool`", (1, 25)),
            ],
        );
        errors(
            "fn main() { nope.m(x: 1 + true) }",
            &[
                ("cannot find `nope` in this scope", (1, 13)),
                ("labels on arguments come in milestone 10", (1, 20)),
                ("expected `i64`, found `bool`", (1, 27)),
            ],
        );
    }

    #[test]
    fn print_reports_labels_and_still_checks_its_arguments() {
        errors(
            "fn main() { print(z: 1, 2) }",
            &[
                ("`print` takes one `i64` or `bool`", (1, 13)),
                ("labels on arguments come in milestone 10", (1, 19)),
            ],
        );
        errors(
            "class E {}\nfn main() { print(z: E()) }",
            &[
                ("`print` takes one `i64` or `bool`", (2, 13)),
                ("labels on arguments come in milestone 10", (2, 19)),
            ],
        );
    }

    #[test]
    fn unknown_field() {
        error(
            "class P {\n    let x: i64\n}\nfn main() {\n    let p = P(x: 1)\n    print(p.size)\n}",
            "`P` has no field `size`",
            (6, 13),
        );
    }

    #[test]
    fn field_called_as_method() {
        error(
            "class P {\n    let x: i64\n}\nfn main() {\n    let p = P(x: 1)\n    print(p.x())\n}",
            "`x` is a field of `P`, not a method",
            (6, 13),
        );
    }

    #[test]
    fn method_read_as_field() {
        expect_error(
            "class P {\n    fn m(self) -> i64 { 1 }\n}\nfn main() {\n    let p = P()\n    print(p.m)\n}",
            "`m` is a method of `P`, not a field",
            (6, 13),
            None,
            &[],
            Some("call it: `m()`"),
        );
    }

    #[test]
    fn unknown_method() {
        error(
            "class P {\n    fn m(self) -> i64 { 1 }\n}\nfn main() {\n    let p = P()\n    print(p.nope())\n}",
            "`P` has no method `nope`",
            (6, 13),
        );
    }

    #[test]
    fn method_arity() {
        error(
            "class P {\n    fn m(self, a: i64) {}\n}\nfn main() { P().m() }",
            "`P.m` takes 1 argument, found 0",
            (4, 13),
        );
    }

    #[test]
    fn field_on_i64() {
        error(
            "fn main() {\n    let x = 1\n    print(x.f)\n}",
            "`i64` has no field `f`",
            (3, 13),
        );
    }

    #[test]
    fn method_on_i64() {
        error(
            "fn main() {\n    let x = 1\n    print(x.m())\n}",
            "`i64` has no method `m`",
            (3, 13),
        );
    }

    #[test]
    fn self_outside_method() {
        error(
            "fn main() { print(self) }",
            "`self` is only available in a method",
            (1, 19),
        );
    }

    #[test]
    fn bare_name_is_not_a_field() {
        error(
            "class P {\n    let x: i64\n    fn m(self) -> i64 { x }\n}\nfn main() {}",
            "cannot find `x` in this scope",
            (3, 25),
        );
    }

    #[test]
    fn assign_let_field() {
        expect_error(
            "class P {\n    let x: i64\n}\nfn main() {\n    let p = P(x: 1)\n    p.x = 2\n}",
            "cannot assign to `x`",
            (6, 5),
            None,
            &[(2, 9, "declared with `let` here")],
            Some("declare it with `var`"),
        );
    }

    #[test]
    fn print_class() {
        error(
            "class E {}\nfn main() { print(E()) }",
            "`print` takes one `i64` or `bool`",
            (2, 13),
        );
    }

    #[test]
    fn poisoned_receiver_checks_args() {
        errors(
            "fn main() { nope.m(1 + true) }",
            &[
                ("cannot find `nope` in this scope", (1, 13)),
                ("expected `i64`, found `bool`", (1, 24)),
            ],
        );
    }

    #[test]
    fn lowers_methods() {
        assert_eq!(
            lowered(
                "class C {\n    var n: i64\n    fn bump(self, by: i64) {\n        self.n += by\n    }\n}\nfn main() {\n    let c = C(n: 1)\n    c.bump(2)\n    print(c.n)\n}"
            ),
            "(class C (var n i64))\n\
             (fn C.bump ((self#0 C) (by#1 i64)) unit (block (block (let #2 self#0) (= (. #2 n) (+ (. #2 n) by#1)))))\n\
             (fn main () unit (block (let c#0 (new C 1)) (call C.bump c#0 2) (print (. c#0 n))))"
        );
    }

    #[test]
    fn lowers_field_assignments() {
        assert_eq!(
            lowered(
                "class B {\n    var f: i64\n}\nclass A {\n    var b: B\n}\nfn mk(a: A) -> A { a }\nfn main() {\n    let a = A(b: B(f: 1))\n    a.b.f = 2\n    mk(a).b = B(f: 3)\n}"
            ),
            "(class B (var f i64))\n\
             (class A (var b B))\n\
             (fn mk ((a#0 A)) A (block a#0))\n\
             (fn main () unit (block (let a#0 (new A (new B 1))) (= (. (. a#0 b) f) 2) (= (. (call mk a#0) b) (new B 3))))"
        );
    }

    #[test]
    fn assign_field_type() {
        error(
            "class P {\n    var x: i64\n}\nfn main() {\n    let p = P(x: 1)\n    p.x = true\n}",
            "expected `i64`, found `bool`",
            (6, 11),
        );
    }

    #[test]
    fn never_as_print_argument() {
        error(
            "fn f(c: bool) -> i64 {\n    print(if c { return 1 } else { return 2 })\n    1\n}\nfn main() {}",
            "unreachable code",
            (2, 11),
        );
    }

    /// The printed `g` of `g` between the spill tests' shared classes and functions and an
    /// empty `main`.
    fn spilled(g: &str) -> String {
        let src = format!(
            "class B {{}}\nclass P {{\n    var b: B\n    let n: i64\n}}\nfn make() -> B {{ B() }}\nfn f(b: B, n: i64) {{}}\n{g}\nfn main() {{}}"
        );
        lowered(&src)
            .lines()
            .find(|line| line.starts_with("(fn g "))
            .expect("`g` is lowered")
            .to_owned()
    }

    #[test]
    fn spills_call_argument() {
        assert_eq!(
            spilled(
                "fn g(c: bool) -> i64 {\n    f(make(), if c { return 0 } else { 1 })\n    2\n}"
            ),
            "(fn g ((c#0 bool)) i64 (block (block (let #1 (call make)) (let #2 (if c#0 (block (return 0)) (block 1))) (call f #1 #2)) 2))"
        );
    }

    #[test]
    fn spills_keep_source_order() {
        assert_eq!(
            spilled(
                "fn one() -> i64 { 1 }\nfn h(a: i64, b: B, n: i64) {}\nfn g(c: bool) -> i64 {\n    h(one(), make(), if c { return 0 } else { 1 })\n    2\n}"
            ),
            "(fn g ((c#0 bool)) i64 (block (block (let #1 (call one)) (let #2 (call make)) (let #3 (if c#0 (block (return 0)) (block 1))) (call h #1 #2 #3)) 2))"
        );
    }

    #[test]
    fn spills_constructor_argument() {
        assert_eq!(
            spilled(
                "fn g(c: bool) -> P {\n    P(b: make(), n: if c { return P(b: B(), n: 0) } else { 1 })\n}"
            ),
            "(fn g ((c#0 bool)) P (block (block (let #1 (call make)) (let #2 (if c#0 (block (return (new P (new B) 0))) (block 1))) (new P #1 #2))))"
        );
    }

    #[test]
    fn spills_field_assignment_base() {
        assert_eq!(
            spilled(
                "fn g(c: bool, p: P) {\n    while true {\n        p.b = if c { break } else { make() }\n    }\n}"
            ),
            "(fn g ((c#0 bool) (p#1 P)) unit (block (loop (block (if true (block (block (let #2 p#1) (let #3 (if c#0 (block (break)) (block (call make)))) (= (. #2 b) #3))) (block (break)))))))"
        );
    }

    #[test]
    fn no_spill_without_exit() {
        assert_eq!(
            spilled("fn g() {\n    f(make(), 1)\n}"),
            "(fn g () unit (block (call f (call make) 1)))"
        );
    }

    #[test]
    fn no_spill_without_earlier_reference() {
        assert_eq!(
            spilled(
                "fn g(c: bool) -> i64 {\n    f(if c { return 0 } else { make() }, 1)\n    2\n}"
            ),
            "(fn g ((c#0 bool)) i64 (block (call f (if c#0 (block (return 0)) (block (call make))) 1) 2))"
        );
    }

    #[test]
    fn nested_loop_break_still_spills() {
        assert_eq!(
            spilled("fn g() {\n    f(make(), if true { while true { break }\n 1 } else { 1 })\n}"),
            "(fn g () unit (block (block (let #0 (call make)) (let #1 (if true (block (loop (block (if true (block (break)) (block (break))))) 1) (block 1))) (call f #0 #1))))"
        );
    }

    #[test]
    fn spills_method_receiver() {
        assert_eq!(
            spilled(
                "class Q {\n    fn m(self, n: i64) {}\n}\nfn q() -> Q { Q() }\nfn g(c: bool) -> i64 {\n    q().m(if c { return 0 } else { 1 })\n    2\n}"
            ),
            "(fn g ((c#0 bool)) i64 (block (block (let #1 (call q)) (let #2 (if c#0 (block (return 0)) (block 1))) (call Q.m #1 #2)) 2))"
        );
    }

    /// `a.f op= e` reads its bound `a` as the assignment's base before `e` runs.
    #[test]
    fn spills_compound_field_assignment_base() {
        assert_eq!(
            spilled(
                "class Q {\n    var k: i64\n}\nfn g(c: bool, q: Q) {\n    while true {\n        q.k += if c { break } else { 1 }\n    }\n}"
            ),
            "(fn g ((c#0 bool) (q#1 Q)) unit (block (loop (block (if true (block (block (let #2 q#1) (block (let #3 #2) (let #4 (+ (. #2 k) (if c#0 (block (break)) (block 1)))) (= (. #3 k) #4)))) (block (break)))))))"
        );
    }
}
