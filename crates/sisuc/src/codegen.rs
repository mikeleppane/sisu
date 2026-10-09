//! Builds LLVM IR through inkwell and writes it out as a native object file.

use std::collections::HashMap;
use std::path::Path;

use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, IntType};
use inkwell::values::{BasicValue, FunctionValue, IntValue};
use inkwell::{IntPredicate, OptimizationLevel};

use crate::ast::{
    BinaryOp, Block, CompareOp, Expr, ExprKind, Function, Program, Stmt, StmtKind, TypeExpr,
    UnaryOp,
};

/// Compiles a program that passed `check` into an LLVM module.
#[cfg_attr(not(test), expect(dead_code, reason = "wired into the CLI in Task 15"))]
// `_path` and `_source` locate panic messages once Task 14 adds checked arithmetic.
pub(crate) fn compile<'ctx>(
    context: &'ctx Context,
    program: &Program,
    _path: &str,
    _source: &str,
) -> Module<'ctx> {
    let module = context.create_module("main");
    let i64_type = context.i64_type();
    let void = context.void_type();

    // The runtime's print functions. `zeroext` keeps the C `bool` ABI: the callee may read
    // all 8 bits of the argument, so the caller must extend the `i1` with zeros.
    let print_int = module.add_function(
        "sisu_print_int",
        void.fn_type(&[i64_type.into()], false),
        None,
    );
    let print_bool = module.add_function(
        "sisu_print_bool",
        void.fn_type(&[context.bool_type().into()], false),
        None,
    );
    print_bool.add_attribute(AttributeLoc::Param(0), zeroext(context));

    let mut codegen = Codegen {
        context,
        module,
        builder: context.create_builder(),
        print_int,
        print_bool,
        scopes: Vec::new(),
    };
    // Declare every function first so that calls can refer to functions defined later.
    for f in &program.functions {
        codegen.declare(f);
    }
    for f in &program.functions {
        codegen.define(f);
    }
    codegen.c_main();
    codegen.module
}

fn zeroext(context: &Context) -> Attribute {
    context.create_enum_attribute(Attribute::get_named_enum_kind_id("zeroext"), 0)
}

/// What a name in scope stands for.
#[derive(Clone, Copy)]
enum Local<'ctx> {
    /// A `let` binding or a parameter: an SSA value.
    Value(IntValue<'ctx>),
}

struct Codegen<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    print_int: FunctionValue<'ctx>,
    print_bool: FunctionValue<'ctx>,
    scopes: Vec<HashMap<String, Local<'ctx>>>,
}

impl<'ctx> Codegen<'ctx> {
    fn int_type(&self, ty: &TypeExpr) -> IntType<'ctx> {
        if ty.name == "bool" {
            self.context.bool_type()
        } else {
            self.context.i64_type()
        }
    }

    fn function(&self, name: &str) -> FunctionValue<'ctx> {
        self.module
            .get_function(&format!("sisu.{name}"))
            .expect("checked: every called function is declared")
    }

    fn declare(&self, f: &Function) {
        let params: Vec<BasicMetadataTypeEnum> = f
            .params
            .iter()
            .map(|p| self.int_type(&p.ty).into())
            .collect();
        let fn_type = match &f.ret {
            Some(ret) => self.int_type(ret).fn_type(&params, false),
            None => self.context.void_type().fn_type(&params, false),
        };
        self.module
            .add_function(&format!("sisu.{}", f.name.name), fn_type, None);
    }

    fn define(&mut self, f: &Function) {
        let function = self.function(&f.name.name);
        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);
        let params = f
            .params
            .iter()
            .zip(function.get_param_iter())
            .map(|(p, v)| (p.name.name.clone(), Local::Value(v.into_int_value())))
            .collect();
        self.scopes = vec![params];
        let value = self.block(&f.body, f.ret.is_some());
        if !self.terminated() {
            // The declared return type picks the `ret`: a unit function discards any body value.
            let value = f
                .ret
                .as_ref()
                .map(|_| value.expect("checked: a body that falls through has the return type"));
            self.builder
                .build_return(value.as_ref().map(|v| v as &dyn BasicValue))
                .expect("builder is positioned");
        }
    }

    /// `define i32 @main()`, which the C startup code calls: runs `sisu.main`, returns 0.
    fn c_main(&self) {
        let i32_type = self.context.i32_type();
        let main = self
            .module
            .add_function("main", i32_type.fn_type(&[], false), None);
        self.builder
            .position_at_end(self.context.append_basic_block(main, "entry"));
        self.builder
            .build_call(self.function("main"), &[], "")
            .expect("builder is positioned");
        self.builder
            .build_return(Some(&i32_type.const_zero()))
            .expect("builder is positioned");
    }

    fn current_block(&self) -> BasicBlock<'ctx> {
        self.builder
            .get_insert_block()
            .expect("builder is positioned")
    }

    fn current_function(&self) -> FunctionValue<'ctx> {
        self.current_block()
            .get_parent()
            .expect("the block belongs to a function")
    }

    /// Whether the current block already ends in a terminator, so nothing more can follow.
    fn terminated(&self) -> bool {
        self.current_block().get_terminator().is_some()
    }

    /// The current block, or `None` when it is terminated: the block a branch falls through from.
    fn open_block(&self) -> Option<BasicBlock<'ctx>> {
        (!self.terminated()).then(|| self.current_block())
    }

    /// The block's value, `None` for `unit` or `never`. `used` says whether the caller needs
    /// the value; only the last statement's value can be used.
    fn block(&mut self, block: &Block, used: bool) -> Option<IntValue<'ctx>> {
        self.scopes.push(HashMap::new());
        let mut value = None;
        for (i, stmt) in block.stmts.iter().enumerate() {
            // A `never` statement ended the block; what follows is unreachable.
            if self.terminated() {
                break;
            }
            value = self.stmt(stmt, used && i + 1 == block.stmts.len());
        }
        self.scopes.pop();
        value
    }

    fn stmt(&mut self, stmt: &Stmt, used: bool) -> Option<IntValue<'ctx>> {
        match &stmt.kind {
            StmtKind::Let {
                mutable: false,
                name,
                init,
                ..
            } => {
                let value = self.value(init);
                self.scopes
                    .last_mut()
                    .expect("a block pushed a scope")
                    .insert(name.name.clone(), Local::Value(value));
                None
            }
            StmtKind::Let { mutable: true, .. }
            | StmtKind::While { .. }
            | StmtKind::Assign { .. } => {
                panic!("codegen for `var`/`while`/assignment lands in stage 5")
            }
            StmtKind::Return(value) => {
                let value = value.as_ref().and_then(|e| self.expr(e));
                // The checker rejects a `never` return value; this keeps codegen safe anyway.
                if !self.terminated() {
                    self.builder
                        .build_return(value.as_ref().map(|v| v as &dyn BasicValue))
                        .expect("builder is positioned");
                }
                None
            }
            StmtKind::Expr(Expr {
                kind:
                    ExprKind::If {
                        cond,
                        then_block,
                        else_block,
                    },
                ..
            }) => self.if_expr(cond, then_block, else_block.as_ref(), used),
            StmtKind::Expr(e) => self.expr(e),
        }
    }

    /// An expression the checker typed `i64` or `bool`.
    fn value(&mut self, e: &Expr) -> IntValue<'ctx> {
        self.expr(e).expect("checked: the expression has a value")
    }

    fn expr(&mut self, e: &Expr) -> Option<IntValue<'ctx>> {
        let value = match &e.kind {
            ExprKind::Int(n) => self.context.i64_type().const_int(n.cast_unsigned(), false),
            ExprKind::Bool(v) => self.context.bool_type().const_int(u64::from(*v), false),
            ExprKind::Name(name) => {
                let local = self
                    .scopes
                    .iter()
                    .rev()
                    .find_map(|scope| scope.get(name))
                    .expect("checked: every name is bound");
                let Local::Value(value) = *local;
                value
            }
            ExprKind::Call { callee, args } => return self.call(&callee.name, args),
            ExprKind::Unary { op, operand } => {
                let operand = self.value(operand);
                match op {
                    UnaryOp::Neg => self.builder.build_int_neg(operand, "neg"),
                    UnaryOp::Not => self.builder.build_not(operand, "not"),
                }
                .expect("builder is positioned")
            }
            ExprKind::Binary { op, lhs, rhs } => self.binary(*op, lhs, rhs),
            ExprKind::Compare { operands, ops } => self.compare(operands, ops),
            ExprKind::If {
                cond,
                then_block,
                else_block,
            } => return self.if_expr(cond, then_block, else_block.as_ref(), true),
        };
        Some(value)
    }

    fn call(&mut self, callee: &str, args: &[Expr]) -> Option<IntValue<'ctx>> {
        let args: Vec<IntValue> = args.iter().map(|a| self.value(a)).collect();
        if callee == "print" {
            let arg = args[0];
            let is_bool = arg.get_type().get_bit_width() == 1;
            let print = if is_bool {
                self.print_bool
            } else {
                self.print_int
            };
            let call = self
                .builder
                .build_call(print, &[arg.into()], "")
                .expect("builder is positioned");
            if is_bool {
                call.add_attribute(AttributeLoc::Param(0), zeroext(self.context));
            }
            return None;
        }
        let args: Vec<_> = args.into_iter().map(Into::into).collect();
        self.builder
            .build_call(self.function(callee), &args, "call")
            .expect("builder is positioned")
            .try_as_basic_value()
            .basic()
            .map(|v| v.into_int_value())
    }

    // shortcut: plain arithmetic wraps on overflow; Task 14 replaces it with checked arithmetic.
    fn binary(&mut self, op: BinaryOp, lhs: &Expr, rhs: &Expr) -> IntValue<'ctx> {
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            return self.short_circuit(op == BinaryOp::And, lhs, rhs);
        }
        let (l, r) = (self.value(lhs), self.value(rhs));
        let b = &self.builder;
        match op {
            BinaryOp::Add => b.build_int_add(l, r, "add"),
            BinaryOp::Sub => b.build_int_sub(l, r, "sub"),
            BinaryOp::Mul => b.build_int_mul(l, r, "mul"),
            BinaryOp::Div => b.build_int_signed_div(l, r, "div"),
            BinaryOp::Rem => b.build_int_signed_rem(l, r, "rem"),
            BinaryOp::Eq => b.build_int_compare(IntPredicate::EQ, l, r, "eq"),
            BinaryOp::Ne => b.build_int_compare(IntPredicate::NE, l, r, "ne"),
            BinaryOp::And | BinaryOp::Or => unreachable!("handled above"),
        }
        .expect("builder is positioned")
    }

    /// `lhs && rhs` or `lhs || rhs`: `rhs` runs only when `lhs` does not decide the result.
    fn short_circuit(&mut self, and: bool, lhs: &Expr, rhs: &Expr) -> IntValue<'ctx> {
        let lhs = self.value(lhs);
        let lhs_end = self.current_block();
        let function = self.current_function();
        let rhs_block = self.context.append_basic_block(function, "rhs");
        let merge = self.context.append_basic_block(function, "logic.end");
        let (on_true, on_false) = if and {
            (rhs_block, merge)
        } else {
            (merge, rhs_block)
        };
        self.builder
            .build_conditional_branch(lhs, on_true, on_false)
            .expect("builder is positioned");
        self.builder.position_at_end(rhs_block);
        let rhs = self.value(rhs);
        let rhs_end = self.current_block();
        self.branch_to(merge);
        self.builder.position_at_end(merge);
        // Skipping `rhs` means `lhs` already decided: `false` for `&&`, `true` for `||`.
        let decided = self.context.bool_type().const_int(u64::from(!and), false);
        self.phi(
            self.context.bool_type(),
            &[(decided, lhs_end), (rhs, rhs_end)],
        )
    }

    /// `a < b <= c` is `a < b && b <= c` with `b` computed once.
    fn compare(&mut self, operands: &[Expr], ops: &[CompareOp]) -> IntValue<'ctx> {
        let (last_op, first_ops) = ops.split_last().expect("a chain has an operator");
        let function = self.current_function();
        let mut false_from = Vec::new();
        let mut lhs = self.value(&operands[0]);
        for (op, operand) in first_ops.iter().zip(&operands[1..]) {
            let rhs = self.value(operand);
            let holds = self.compare_pair(*op, lhs, rhs);
            let next = self.context.append_basic_block(function, "chain.next");
            false_from.push((holds, self.current_block(), next));
            self.builder.position_at_end(next);
            lhs = rhs;
        }
        let rhs = self.value(operands.last().expect("a chain has operands"));
        let holds = self.compare_pair(*last_op, lhs, rhs);
        if false_from.is_empty() {
            return holds;
        }
        let last_end = self.current_block();
        let merge = self.context.append_basic_block(function, "chain.end");
        self.branch_to(merge);
        let no = self.context.bool_type().const_zero();
        let mut incoming = Vec::new();
        for (holds, block, next) in false_from {
            self.builder.position_at_end(block);
            self.builder
                .build_conditional_branch(holds, next, merge)
                .expect("builder is positioned");
            incoming.push((no, block));
        }
        incoming.push((holds, last_end));
        self.builder.position_at_end(merge);
        self.phi(self.context.bool_type(), &incoming)
    }

    fn compare_pair(
        &self,
        op: CompareOp,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
    ) -> IntValue<'ctx> {
        let predicate = match op {
            CompareOp::Lt => IntPredicate::SLT,
            CompareOp::Le => IntPredicate::SLE,
            CompareOp::Gt => IntPredicate::SGT,
            CompareOp::Ge => IntPredicate::SGE,
        };
        self.builder
            .build_int_compare(predicate, lhs, rhs, "cmp")
            .expect("builder is positioned")
    }

    /// `if`/`else`. The merge block exists only if a branch falls through; its `phi` (built
    /// only when the value is `used`) takes one edge per such branch, from the block where
    /// that branch ended.
    fn if_expr(
        &mut self,
        cond: &Expr,
        then_block: &Block,
        else_block: Option<&Block>,
        used: bool,
    ) -> Option<IntValue<'ctx>> {
        let cond = self.value(cond);
        let function = self.current_function();
        let then_start = self.context.append_basic_block(function, "then");
        let else_start = self.context.append_basic_block(function, "else");
        self.builder
            .build_conditional_branch(cond, then_start, else_start)
            .expect("builder is positioned");
        self.builder.position_at_end(then_start);
        let then_value = self.block(then_block, used);
        let then_end = self.open_block();
        self.builder.position_at_end(else_start);
        let Some(else_block) = else_block else {
            // Without `else`, the else block is where both paths meet.
            if let Some(end) = then_end {
                self.builder.position_at_end(end);
                self.branch_to(else_start);
                self.builder.position_at_end(else_start);
            }
            return None;
        };
        let else_value = self.block(else_block, used);
        let else_end = self.open_block();
        let arms: Vec<_> = [(then_value, then_end), (else_value, else_end)]
            .into_iter()
            .filter_map(|(value, end)| Some((value, end?)))
            .collect();
        if arms.is_empty() {
            // Both branches ended in a terminator; so does the `if`.
            return None;
        }
        let merge = self.context.append_basic_block(function, "if.end");
        for (_, end) in &arms {
            self.builder.position_at_end(*end);
            self.branch_to(merge);
        }
        self.builder.position_at_end(merge);
        if !used {
            return None;
        }
        let incoming: Vec<_> = arms
            .into_iter()
            .map(|(value, end)| (value.expect("checked: a used `if` has a value"), end))
            .collect();
        let ty = incoming[0].0.get_type();
        Some(self.phi(ty, &incoming))
    }

    fn branch_to(&self, block: BasicBlock<'ctx>) {
        self.builder
            .build_unconditional_branch(block)
            .expect("builder is positioned");
    }

    fn phi(
        &self,
        ty: IntType<'ctx>,
        incoming: &[(IntValue<'ctx>, BasicBlock<'ctx>)],
    ) -> IntValue<'ctx> {
        let phi = self
            .builder
            .build_phi(ty, "phi")
            .expect("builder is positioned");
        for (value, block) in incoming {
            phi.add_incoming(&[(value, *block)]);
        }
        phi.as_basic_value().into_int_value()
    }
}

/// Builds the module for the fixed program `fn main() { print_int(42) }`.
/// The parser replaces this once it exists.
pub(crate) fn hello_module(context: &Context) -> Module<'_> {
    let module = context.create_module("main");
    let i64_type = context.i64_type();
    let i32_type = context.i32_type();

    // declare void @sisu_print_int(i64): the runtime defines it, the linker finds it.
    let print_int_type = context.void_type().fn_type(&[i64_type.into()], false);
    let print_int = module.add_function("sisu_print_int", print_int_type, None);

    // define i32 @main(): the C startup code calls main and expects an exit status.
    let main_fn = module.add_function("main", i32_type.fn_type(&[], false), None);
    let entry = context.append_basic_block(main_fn, "entry");
    let builder = context.create_builder();
    builder.position_at_end(entry);
    builder
        .build_call(print_int, &[i64_type.const_int(42, false).into()], "")
        .expect("builder is positioned and the argument types match");
    builder
        .build_return(Some(&i32_type.const_zero()))
        .expect("builder is positioned");
    module
}

/// A target machine for the host `sisuc` runs on.
pub(crate) fn target_machine() -> Result<TargetMachine, String> {
    Target::initialize_native(&InitializationConfig::default())?;
    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple).map_err(|e| e.to_string())?;
    // PIC because Ubuntu's `cc` links position-independent executables by default.
    target
        .create_target_machine(
            &triple,
            "generic",
            "",
            OptimizationLevel::None,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .ok_or_else(|| "LLVM cannot create a target machine for this host".to_string())
}

/// Writes `module` to `path` as an object file for `machine`.
pub(crate) fn write_object(
    module: &Module<'_>,
    machine: &TargetMachine,
    path: &Path,
) -> Result<(), String> {
    module.set_triple(&machine.get_triple());
    module.set_data_layout(&machine.get_target_data().get_data_layout());
    module.verify().map_err(|e| e.to_string())?;
    machine
        .write_to_file(module, FileType::Object, path)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::check;
    use crate::diagnostic::Severity;
    use crate::lexer::lex;
    use crate::parser::parse;

    /// Lexes, parses, checks (no errors allowed) and compiles `src`, then verifies the module.
    fn compiled<'ctx>(context: &'ctx Context, src: &str) -> Module<'ctx> {
        let program = parse(&lex(src).expect("source lexes")).expect("source parses");
        let diagnostics = check(&program);
        assert!(
            diagnostics.iter().all(|d| d.severity != Severity::Error),
            "{diagnostics:?}"
        );
        let module = compile(context, &program, "test.sisu", src);
        if let Err(e) = module.verify() {
            panic!("{e}\n{}", module.print_to_string());
        }
        module
    }

    const FIB: &str = "fn fib(n: i64) -> i64 {\n    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }\n}\n\nfn main() {\n    print(fib(30))\n}\n";

    #[test]
    fn verifies_fib() {
        compiled(&Context::create(), FIB);
    }

    #[test]
    fn verifies_all_paths_return() {
        compiled(
            &Context::create(),
            "fn sign(n: i64) -> i64 {\n    if n < 0 { return -1 } else { return 1 }\n}\nfn main() { print(sign(2)) }",
        );
    }

    #[test]
    fn verifies_chain_and_short_circuit() {
        compiled(
            &Context::create(),
            "fn main() {\n    print(1 < 2 <= 3)\n    print(true && false || true)\n}",
        );
    }

    #[test]
    fn verifies_one_returning_branch() {
        let context = Context::create();
        let module = compiled(
            &context,
            "fn a(c: bool) -> i64 {\n    if c { return 1 } else { 2 }\n}\nfn b(c: bool) -> i64 {\n    if c { 1 } else { return 2 }\n}\nfn main() {\n    print(a(true))\n}",
        );
        let ir = module.print_to_string().to_string();
        let phis: Vec<_> = ir.lines().filter(|l| l.contains(" = phi ")).collect();
        assert_eq!(phis.len(), 2, "{ir}");
        for phi in phis {
            assert_eq!(phi.matches('[').count(), 1, "{phi}");
        }
    }

    #[test]
    fn verifies_early_return_in_main() {
        compiled(
            &Context::create(),
            "fn main() {\n    if true { return }\n    print(1)\n}",
        );
    }

    #[test]
    fn verifies_discarded_body_value() {
        compiled(
            &Context::create(),
            "fn g() { 1 }\nfn main() {\n    g()\n    if true { 1 } else { 2 }\n}",
        );
    }

    #[test]
    fn user_function_names_are_prefixed() {
        let context = Context::create();
        let ir = compiled(&context, FIB).print_to_string().to_string();
        for wanted in [
            "define i64 @sisu.fib(i64",
            "define void @sisu.main()",
            "define i32 @main()",
        ] {
            assert!(ir.contains(wanted), "missing {wanted:?} in\n{ir}");
        }
    }

    #[test]
    fn hello_module_calls_print_int_and_returns_zero() {
        let context = Context::create();
        let module = hello_module(&context);
        module.verify().expect("the module is valid IR");
        let expected = r#"; ModuleID = 'main'
source_filename = "main"

declare void @sisu_print_int(i64)

define i32 @main() {
entry:
  call void @sisu_print_int(i64 42)
  ret i32 0
}
"#;
        assert_eq!(module.print_to_string().to_string(), expected);
    }
}
