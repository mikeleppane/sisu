//! Builds LLVM IR through inkwell and writes it out as a native object file.

use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::intrinsics::Intrinsic;
use inkwell::memory_buffer::MemoryBuffer;
use inkwell::module::Module;
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::{BasicValue, BasicValueEnum, FunctionValue, IntValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate, OptimizationLevel};

use crate::diagnostic::{Span, line_col};
use crate::tir::{
    BinaryOp, Block, CompareOp, Expr, ExprKind, Function, Place, Program, Stmt, Type, UnaryOp,
};

/// Compiles a program that passed `check` into an LLVM module.
pub(crate) fn compile<'ctx>(
    context: &'ctx Context,
    program: &Program,
    path: &str,
    source: &str,
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
    // `sisu_panic(msg, len)` prints the message and exits; it never returns.
    let panic = module.add_function(
        "sisu_panic",
        void.fn_type(
            &[
                context.ptr_type(AddressSpace::default()).into(),
                i64_type.into(),
            ],
            false,
        ),
        None,
    );
    panic.add_attribute(
        AttributeLoc::Function,
        context.create_enum_attribute(Attribute::get_named_enum_kind_id("noreturn"), 0),
    );

    let mut codegen = Codegen {
        context,
        module,
        builder: context.create_builder(),
        print_int,
        print_bool,
        panic,
        path,
        source,
        functions: Vec::new(),
        locals: Vec::new(),
        loops: Vec::new(),
    };
    // Declare every function first so that calls can refer to functions defined later.
    for f in &program.functions {
        codegen.declare(f);
    }
    for (i, f) in program.functions.iter().enumerate() {
        let function = codegen.functions[i];
        codegen.define(f, function);
    }
    codegen.c_main();
    codegen.module
}

fn zeroext(context: &Context) -> Attribute {
    context.create_enum_attribute(Attribute::get_named_enum_kind_id("zeroext"), 0)
}

/// Where a local's value lives.
#[derive(Clone, Copy)]
enum Slot<'ctx> {
    /// A `let` binding or a parameter: an SSA value.
    Value(BasicValueEnum<'ctx>),
    /// A `var`: an `alloca` of this type, loaded and stored until `mem2reg` promotes it.
    Alloca(PointerValue<'ctx>, BasicTypeEnum<'ctx>),
}

/// An open `loop`: `Break` branches to `end`, a body that falls through back to `body`.
struct LoopFrame<'ctx> {
    body: BasicBlock<'ctx>,
    end: BasicBlock<'ctx>,
}

struct Codegen<'ctx, 'src> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    print_int: FunctionValue<'ctx>,
    print_bool: FunctionValue<'ctx>,
    panic: FunctionValue<'ctx>,
    /// The input path as given, and its text: panic messages name a position in it.
    path: &'src str,
    source: &'src str,
    /// Indexed by `FuncId`.
    functions: Vec<FunctionValue<'ctx>>,
    /// The current function's locals, indexed by `LocalId`; `None` until bound, and for `unit`.
    locals: Vec<Option<Slot<'ctx>>>,
    loops: Vec<LoopFrame<'ctx>>,
}

impl<'ctx> Codegen<'ctx, '_> {
    /// The LLVM type of a value of `ty`; `None` for `unit` and `never`, which have no value.
    fn llvm_type(&self, ty: &Type) -> Option<BasicTypeEnum<'ctx>> {
        match ty {
            Type::I64 => Some(self.context.i64_type().into()),
            Type::Bool => Some(self.context.bool_type().into()),
            Type::Unit | Type::Never => None,
        }
    }

    fn declare(&mut self, f: &Function) {
        let params: Vec<BasicMetadataTypeEnum> = f
            .params
            .iter()
            .map(|p| {
                self.llvm_type(&f.locals[p.0].ty)
                    .expect("checked: a parameter has a value type")
                    .into()
            })
            .collect();
        let fn_type = match self.llvm_type(&f.ret) {
            Some(ret) => ret.fn_type(&params, false),
            None => self.context.void_type().fn_type(&params, false),
        };
        let function = self
            .module
            .add_function(&format!("sisu.{}", f.name), fn_type, None);
        self.functions.push(function);
    }

    fn define(&mut self, f: &Function, function: FunctionValue<'ctx>) {
        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);
        // Every `var` gets its `alloca` up front, in the entry block: see `entry_alloca`.
        self.locals = f
            .locals
            .iter()
            .map(|local| {
                let ty = self.llvm_type(&local.ty).filter(|_| local.mutable)?;
                let name = local.name.as_deref().unwrap_or("");
                Some(Slot::Alloca(self.entry_alloca(ty, name), ty))
            })
            .collect();
        for (p, v) in f.params.iter().zip(function.get_param_iter()) {
            self.locals[p.0] = Some(Slot::Value(v));
        }
        let value = self.block(&f.body);
        if !self.terminated() {
            // The declared return type picks the `ret`: a unit function discards any body value.
            let value = self
                .llvm_type(&f.ret)
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
        let sisu_main = self
            .module
            .get_function("sisu.main")
            .expect("checked: the program has a `main`");
        self.builder
            .build_call(sisu_main, &[], "")
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

    /// The block's value, `None` for `unit` or `never`.
    fn block(&mut self, block: &Block) -> Option<BasicValueEnum<'ctx>> {
        for stmt in &block.stmts {
            // A `never` statement ended the block; what follows is unreachable.
            if self.terminated() {
                return None;
            }
            self.stmt(stmt);
        }
        let value = block.value.as_ref()?;
        if self.terminated() {
            return None;
        }
        self.expr(value)
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { local, init } => {
                let Some(value) = self.expr(init) else {
                    return;
                };
                match self.locals[local.0] {
                    Some(Slot::Alloca(ptr, _)) => self.store(ptr, value),
                    _ => self.locals[local.0] = Some(Slot::Value(value)),
                }
            }
            Stmt::Assign {
                place: Place::Local(local),
                value,
            } => {
                let Some(value) = self.expr(value) else {
                    return;
                };
                let Some(Slot::Alloca(ptr, _)) = self.locals[local.0] else {
                    unreachable!("checked: only a `var` is assigned")
                };
                self.store(ptr, value);
            }
            Stmt::Expr(e) => {
                self.expr(e);
            }
        }
    }

    /// An expression the checker typed `i64` or `bool`.
    fn value(&mut self, e: &Expr) -> IntValue<'ctx> {
        self.expr(e)
            .expect("checked: the expression has a value")
            .into_int_value()
    }

    /// The expression's value, `None` for `unit` or `never`.
    fn expr(&mut self, e: &Expr) -> Option<BasicValueEnum<'ctx>> {
        let value = match &e.kind {
            ExprKind::Int(n) => self.context.i64_type().const_int(n.cast_unsigned(), false),
            ExprKind::Bool(v) => self.context.bool_type().const_int(u64::from(*v), false),
            ExprKind::Local(local) => {
                return self.locals[local.0].map(|slot| match slot {
                    Slot::Value(value) => value,
                    Slot::Alloca(ptr, ty) => {
                        // Name the load after the local (the alloca carries its name).
                        let name = ptr.get_name().to_string_lossy().into_owned();
                        self.builder
                            .build_load(ty, ptr, &name)
                            .expect("builder is positioned")
                    }
                });
            }
            ExprKind::Call { func, args } => {
                let args: Vec<_> = args
                    .iter()
                    .map(|a| {
                        self.expr(a)
                            .expect("checked: an argument has a value")
                            .into()
                    })
                    .collect();
                return self
                    .builder
                    .build_call(self.functions[func.0], &args, "call")
                    .expect("builder is positioned")
                    .try_as_basic_value()
                    .basic();
            }
            ExprKind::Print(operand) => {
                self.print(operand);
                return None;
            }
            ExprKind::Unary { op, operand } => {
                let operand = self.value(operand);
                match op {
                    UnaryOp::Neg => {
                        let zero = self.context.i64_type().const_zero();
                        self.checked("llvm.ssub.with.overflow", zero, operand, e.span)
                    }
                    UnaryOp::Not => self
                        .builder
                        .build_not(operand, "not")
                        .expect("builder is positioned"),
                }
            }
            ExprKind::Binary { op, lhs, rhs } => self.binary(*op, lhs, rhs, e.span),
            ExprKind::Compare { operands, ops } => self.compare(operands, ops),
            ExprKind::Equal { negated, lhs, rhs } => {
                let (l, r) = (self.value(lhs), self.value(rhs));
                let predicate = if *negated {
                    IntPredicate::NE
                } else {
                    IntPredicate::EQ
                };
                self.int_compare(predicate, l, r)
            }
            ExprKind::If {
                cond,
                then_block,
                else_block,
            } => return self.if_expr(cond, then_block, else_block.as_ref(), &e.ty),
            ExprKind::Loop(body) => {
                self.loop_expr(body);
                return None;
            }
            ExprKind::Break => {
                let end = self
                    .loops
                    .last()
                    .expect("checked: `break` is in a loop")
                    .end;
                self.branch_to(end);
                return None;
            }
            ExprKind::Return(value) => {
                let value = value.as_ref().and_then(|e| self.expr(e));
                // The checker rejects a `never` return value; this keeps codegen safe anyway.
                if !self.terminated() {
                    self.builder
                        .build_return(value.as_ref().map(|v| v as &dyn BasicValue))
                        .expect("builder is positioned");
                }
                return None;
            }
        };
        Some(value.into())
    }

    /// An `alloca` at the start of the function's entry block, wherever the `var` sits:
    /// `mem2reg` promotes only entry-block allocas, and one in a loop body would grow the
    /// stack on every iteration.
    fn entry_alloca(&self, ty: BasicTypeEnum<'ctx>, name: &str) -> PointerValue<'ctx> {
        let entry = self
            .current_function()
            .get_first_basic_block()
            .expect("the function has an entry block");
        let builder = self.context.create_builder();
        match entry.get_first_instruction() {
            Some(first) => builder.position_before(&first),
            None => builder.position_at_end(entry),
        }
        builder
            .build_alloca(ty, name)
            .expect("builder is positioned")
    }

    fn store(&self, ptr: PointerValue<'ctx>, value: BasicValueEnum<'ctx>) {
        self.builder
            .build_store(ptr, value)
            .expect("builder is positioned");
    }

    /// `loop`: the body block runs until a `Break` branches to the end block, where codegen
    /// continues; a body that falls through branches back to its start.
    fn loop_expr(&mut self, body: &Block) {
        let function = self.current_function();
        let frame = LoopFrame {
            body: self.context.append_basic_block(function, "loop.body"),
            end: self.context.append_basic_block(function, "loop.end"),
        };
        let (start, end) = (frame.body, frame.end);
        self.branch_to(start);
        self.builder.position_at_end(start);
        self.loops.push(frame);
        self.block(body);
        self.loops.pop();
        if !self.terminated() {
            self.branch_to(start);
        }
        self.builder.position_at_end(end);
    }

    fn print(&mut self, operand: &Expr) {
        let arg = self.value(operand);
        let is_bool = operand.ty == Type::Bool;
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
    }

    /// `span` is the operator expression's: arithmetic panics point at its start.
    fn binary(&mut self, op: BinaryOp, lhs: &Expr, rhs: &Expr, span: Span) -> IntValue<'ctx> {
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            return self.short_circuit(op == BinaryOp::And, lhs, rhs);
        }
        let (l, r) = (self.value(lhs), self.value(rhs));
        match op {
            BinaryOp::Add => self.checked("llvm.sadd.with.overflow", l, r, span),
            BinaryOp::Sub => self.checked("llvm.ssub.with.overflow", l, r, span),
            BinaryOp::Mul => self.checked("llvm.smul.with.overflow", l, r, span),
            BinaryOp::Div | BinaryOp::Rem => self.division(op == BinaryOp::Rem, l, r, span),
            BinaryOp::And | BinaryOp::Or => unreachable!("handled above"),
        }
    }

    fn int_compare(
        &self,
        predicate: IntPredicate,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
    ) -> IntValue<'ctx> {
        self.builder
            .build_int_compare(predicate, lhs, rhs, "cmp")
            .expect("builder is positioned")
    }

    /// `lhs op rhs` through `intrinsic`, an `llvm.s*.with.overflow` that returns the wrapped
    /// result and an overflow flag; the flag branches to an "integer overflow" panic.
    fn checked(
        &self,
        intrinsic: &str,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
        span: Span,
    ) -> IntValue<'ctx> {
        let function = Intrinsic::find(intrinsic)
            .expect("LLVM has the overflow intrinsics")
            .get_declaration(&self.module, &[self.context.i64_type().into()])
            .expect("the intrinsic is overloaded on i64");
        let pair = self
            .builder
            .build_call(function, &[lhs.into(), rhs.into()], "checked")
            .expect("builder is positioned")
            .try_as_basic_value()
            .basic()
            .expect("the intrinsic returns a pair")
            .into_struct_value();
        let field = |index, name| {
            self.builder
                .build_extract_value(pair, index, name)
                .expect("the pair has two fields")
                .into_int_value()
        };
        let (result, overflow) = (field(0, "result"), field(1, "overflow"));
        self.panic_if(overflow, span, "integer overflow");
        result
    }

    /// `lhs / rhs` or `lhs % rhs`: panics on a zero divisor, then on `i64::MIN / -1`, whose
    /// quotient does not fit (`sdiv` and `srem` are undefined for both).
    fn division(
        &self,
        rem: bool,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
        span: Span,
    ) -> IntValue<'ctx> {
        let i64_type = self.context.i64_type();
        let zero = self.int_compare(IntPredicate::EQ, rhs, i64_type.const_zero());
        self.panic_if(zero, span, "division by zero");
        let min = i64_type.const_int(i64::MIN.cast_unsigned(), false);
        let minus_one = i64_type.const_int((-1_i64).cast_unsigned(), false);
        let overflow = self
            .builder
            .build_and(
                self.int_compare(IntPredicate::EQ, lhs, min),
                self.int_compare(IntPredicate::EQ, rhs, minus_one),
                "overflow",
            )
            .expect("builder is positioned");
        self.panic_if(overflow, span, "integer overflow");
        if rem {
            self.builder.build_int_signed_rem(lhs, rhs, "rem")
        } else {
            self.builder.build_int_signed_div(lhs, rhs, "div")
        }
        .expect("builder is positioned")
    }

    /// Branches to a fresh panic block when `fails` holds, and continues in a fresh block
    /// otherwise. The panic block calls `sisu_panic` with `<path>:<line>:<col>: <message>`
    /// for the start of `span`.
    fn panic_if(&self, fails: IntValue<'ctx>, span: Span, message: &str) {
        let function = self.current_function();
        let panic_block = self.context.append_basic_block(function, "panic");
        let ok = self.context.append_basic_block(function, "ok");
        self.builder
            .build_conditional_branch(fails, panic_block, ok)
            .expect("builder is positioned");
        self.builder.position_at_end(panic_block);
        let (line, col) = line_col(self.source, span.start);
        let text = format!("{}:{line}:{col}: {message}", self.path);
        let msg = self
            .builder
            .build_global_string_ptr(&text, "panic.msg")
            .expect("builder is positioned");
        let len = self.context.i64_type().const_int(
            u64::try_from(text.len()).expect("a message fits in u64"),
            false,
        );
        self.builder
            .build_call(self.panic, &[msg.as_pointer_value().into(), len.into()], "")
            .expect("builder is positioned");
        self.builder
            .build_unreachable()
            .expect("builder is positioned");
        self.builder.position_at_end(ok);
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
        .into_int_value()
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
            .into_int_value()
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

    /// `if`/`else`. The merge block exists only if a branch falls through; its `phi`, built
    /// when `ty` has a value, takes one edge per such branch, from the block where that
    /// branch ended.
    fn if_expr(
        &mut self,
        cond: &Expr,
        then_block: &Block,
        else_block: Option<&Block>,
        ty: &Type,
    ) -> Option<BasicValueEnum<'ctx>> {
        let cond = self.value(cond);
        let function = self.current_function();
        let then_start = self.context.append_basic_block(function, "then");
        let else_start = self.context.append_basic_block(function, "else");
        self.builder
            .build_conditional_branch(cond, then_start, else_start)
            .expect("builder is positioned");
        self.builder.position_at_end(then_start);
        let then_value = self.block(then_block);
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
        let else_value = self.block(else_block);
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
        let ty = self.llvm_type(ty)?;
        let incoming: Vec<_> = arms
            .into_iter()
            .map(|(value, end)| (value.expect("checked: a branch has the `if`'s type"), end))
            .collect();
        Some(self.phi(ty, &incoming))
    }

    fn branch_to(&self, block: BasicBlock<'ctx>) {
        self.builder
            .build_unconditional_branch(block)
            .expect("builder is positioned");
    }

    fn phi<V: BasicValue<'ctx>>(
        &self,
        ty: impl BasicType<'ctx>,
        incoming: &[(V, BasicBlock<'ctx>)],
    ) -> BasicValueEnum<'ctx> {
        let phi = self
            .builder
            .build_phi(ty, "phi")
            .expect("builder is positioned");
        for (value, block) in incoming {
            phi.add_incoming(&[(value, *block)]);
        }
        phi.as_basic_value()
    }
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

/// Verifies `module`, then runs `mem2reg`, which turns each `var`'s `alloca` into SSA values.
pub(crate) fn run_mem2reg(module: &Module<'_>, machine: &TargetMachine) -> Result<(), String> {
    module.verify().map_err(|e| e.to_string())?;
    module
        .run_passes("mem2reg", machine, PassBuilderOptions::create())
        .map_err(|e| e.to_string())
}

/// Compiles `module` to object code for `machine`, in memory.
pub(crate) fn object_code(
    module: &Module<'_>,
    machine: &TargetMachine,
) -> Result<MemoryBuffer<'static>, String> {
    module.set_triple(&machine.get_triple());
    module.set_data_layout(&machine.get_target_data().get_data_layout());
    module.verify().map_err(|e| e.to_string())?;
    machine
        .write_to_memory_buffer(module, FileType::Object)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::check;
    use crate::lexer::lex;
    use crate::parser::parse;

    /// Lexes, parses, checks (no errors allowed) and compiles `src`, then verifies the module.
    fn compiled<'ctx>(context: &'ctx Context, src: &str) -> Module<'ctx> {
        let program = parse(&lex(src).expect("source lexes")).expect("source parses");
        let (program, diagnostics) = check(&program);
        let program = program.unwrap_or_else(|| panic!("{diagnostics:?}"));
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
    fn verifies_loop_body_ending_in_return() {
        compiled(
            &Context::create(),
            "fn f() -> i64 {\n    while true { return 1 }\n    2\n}\nfn main() { print(f()) }",
        );
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
    fn verifies_else_if_with_short_circuit_in_branch() {
        compiled(
            &Context::create(),
            "fn f(a: i64) -> i64 {\n    if a < 0 { -1 } else if a == 0 && true { 0 } else { 1 }\n}\nfn main() { print(f(2)) }",
        );
    }

    #[test]
    fn print_bool_passes_a_zero_extended_i1() {
        let context = Context::create();
        let ir = compiled(&context, "fn main() { print(true) }")
            .print_to_string()
            .to_string();
        for wanted in [
            "declare void @sisu_print_bool(i1 zeroext)",
            "call void @sisu_print_bool(i1 zeroext",
        ] {
            assert!(ir.contains(wanted), "missing {wanted:?} in\n{ir}");
        }
    }

    #[test]
    fn arithmetic_calls_overflow_intrinsics() {
        let context = Context::create();
        let ir = compiled(
            &context,
            "fn f(a: i64, b: i64) -> i64 { a * b - a / b }\n\nfn main() {\n    print(-f(1, 2))\n}",
        )
        .print_to_string()
        .to_string();
        for wanted in [
            "@llvm.smul.with.overflow.i64",
            "@llvm.ssub.with.overflow.i64",
            "sdiv",
            "call void @sisu_panic",
        ] {
            assert!(ir.contains(wanted), "missing {wanted:?} in\n{ir}");
        }
        // `a * b` and `a * b - a / b` start at column 31, `a / b` at 39, `-f(1, 2)` at 4:11.
        for message in [
            "test.sisu:1:31: integer overflow",
            "test.sisu:1:39: division by zero",
            "test.sisu:1:39: integer overflow",
            "test.sisu:4:11: integer overflow",
        ] {
            assert_panics_with(&ir, message);
        }
    }

    #[test]
    fn division_guards_precede_the_division() {
        let context = Context::create();
        let ir = compiled(
            &context,
            "fn d(a: i64, b: i64) -> i64 { a / b }\nfn r(a: i64, b: i64) -> i64 { a % b }\nfn main() {}",
        )
        .print_to_string()
        .to_string();
        for (name, op) in [("d", "sdiv"), ("r", "srem")] {
            let header = format!("define i64 @sisu.{name}(");
            let body = ir
                .split(&header)
                .nth(1)
                .and_then(|rest| rest.split("\n}").next())
                .unwrap_or_else(|| panic!("no {header} in\n{ir}"));
            let lines: Vec<&str> = body.lines().map(str::trim).collect();
            let find = |wanted: &dyn Fn(&str) -> bool, what: &str| {
                instruction(&lines, wanted).unwrap_or_else(|| panic!("no {what} in\n{body}"))
            };
            let (op_at, _, op_text) = find(&|t| t.starts_with(&format!("{op} i64 ")), op);
            let (a, b) = op_text[op.len() + " i64 ".len()..]
                .split_once(", ")
                .unwrap_or_else(|| panic!("two operands in {op_text:?}"));
            // The zero guard tests the divisor; the `MIN / -1` guard tests both operands.
            let zero_test = format!("icmp eq i64 {b}, 0");
            let min_test = format!("icmp eq i64 {a}, -9223372036854775808");
            let neg_test = format!("icmp eq i64 {b}, -1");
            let (zero_at, zero, _) = find(&|t| t == zero_test, &zero_test);
            let (min_at, min, _) = find(&|t| t == min_test, &min_test);
            let (neg_at, neg, _) = find(&|t| t == neg_test, &neg_test);
            let (and_at, overflow, _) = find(
                &|t| t == format!("and i1 {min}, {neg}") || t == format!("and i1 {neg}, {min}"),
                "the `and` of the `MIN / -1` tests",
            );
            // Each guard branches on its own test: true to a panic block, false on to `ok`.
            let branch = |cond: &str| {
                let at = lines
                    .iter()
                    .position(|l| l.starts_with(&format!("br i1 {cond}, ")))
                    .unwrap_or_else(|| panic!("no branch on {cond} in\n{body}"));
                let labels: Vec<_> = lines[at].split("label %").skip(1).collect();
                assert!(
                    matches!(labels.as_slice(), [t, f] if t.starts_with("panic") && f.starts_with("ok")),
                    "{}",
                    lines[at]
                );
                at
            };
            // The zero guard, then the `MIN / -1` guard, then the operation itself.
            let order = [
                zero_at,
                branch(zero),
                min_at.min(neg_at),
                min_at.max(neg_at),
                and_at,
                branch(overflow),
                op_at,
            ];
            assert!(order.is_sorted(), "{order:?} in\n{body}");
            assert_eq!(
                lines.iter().filter(|l| l.starts_with("br i1 ")).count(),
                2,
                "{body}"
            );
        }
        for message in [
            "test.sisu:1:31: division by zero",
            "test.sisu:1:31: integer overflow",
            "test.sisu:2:31: division by zero",
            "test.sisu:2:31: integer overflow",
        ] {
            assert_panics_with(&ir, message);
        }
    }

    /// The first instruction `<value> = <text>` in `lines` whose text is `wanted`: its line
    /// index, the value it defines and its text.
    fn instruction<'a>(
        lines: &[&'a str],
        wanted: &dyn Fn(&str) -> bool,
    ) -> Option<(usize, &'a str, &'a str)> {
        lines.iter().enumerate().find_map(|(i, line)| {
            let (value, text) = line.split_once(" = ")?;
            wanted(text).then_some((i, value, text))
        })
    }

    /// Asserts that `ir` has a `sisu_panic` call that passes `message` and its byte length.
    fn assert_panics_with(ir: &str, message: &str) {
        let constant = format!("c\"{message}\\00\"");
        let global = ir
            .lines()
            .find(|l| l.ends_with(&constant) || l.contains(&format!("{constant},")))
            .and_then(|l| l.split(" = ").next())
            .unwrap_or_else(|| panic!("no constant {constant} in\n{ir}"));
        let call = format!("@sisu_panic(ptr {global}, i64 {})", message.len());
        assert!(ir.contains(&call), "missing {call:?} in\n{ir}");
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
}
