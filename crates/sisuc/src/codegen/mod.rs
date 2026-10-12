//! Builds LLVM IR through inkwell and writes it out as a native object file.

mod arith;
mod array;
mod rc;

use std::cell::RefCell;
use std::collections::HashMap;

use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::memory_buffer::MemoryBuffer;
use inkwell::module::{Linkage, Module};
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetData, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, StructType};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValue, BasicValueEnum, FunctionValue, IntValue, PointerValue,
};
use inkwell::{AddressSpace, IntPredicate, OptimizationLevel};

use self::rc::Helper;
use crate::diagnostic::{LineIndex, Span};
use crate::tir::{
    Block, ClassId, CompareOp, Expr, ExprKind, Function, LocalId, Place, Program, Stmt, Type,
    UnaryOp,
};

/// The `expect` message for a builder call: codegen positions the builder before it emits code.
const POSITIONED: &str =
    "the builder should be positioned in a block while a function body is emitted";

/// Compiles a program that passed `check` into an LLVM module for `machine`. With `rc_checks`,
/// a retain or release of an object whose count is below 1 calls `sisu_rc_fault`, and `@main`
/// calls `sisu_rc_check_exit` after `sisu.main` returns. `@main` always calls
/// `sisu_flush_stdout` first.
pub(crate) fn compile<'ctx>(
    context: &'ctx Context,
    program: &Program,
    path: &str,
    source: &str,
    machine: &TargetMachine,
    rc_checks: bool,
) -> Module<'ctx> {
    let module = context.create_module("main");
    // The passes and the object file both read the target from the module.
    let target_data = machine.get_target_data();
    module.set_triple(&machine.get_triple());
    module.set_data_layout(&target_data.get_data_layout());
    let i64_type = context.i64_type();
    let ptr_type = context.ptr_type(AddressSpace::default());
    let void = context.void_type();

    // Every class type exists before any body is set, so a field can name any class.
    let classes: Vec<StructType> = program
        .classes
        .iter()
        .map(|class| context.opaque_struct_type(&format!("sisu.{}", class.name)))
        .collect();
    // `sisu_alloc(size, align)` and `sisu_free(ptr, size, align)`, from the runtime.
    let alloc = module.add_function(
        "sisu_alloc",
        ptr_type.fn_type(&[i64_type.into(), i64_type.into()], false),
        None,
    );
    let free = module.add_function(
        "sisu_free",
        void.fn_type(&[ptr_type.into(), i64_type.into(), i64_type.into()], false),
        None,
    );
    // `@sisu_rc.retain(object)`; see `define_helpers`.
    let rc_retain = module.add_function(
        "sisu_rc.retain",
        void.fn_type(&[ptr_type.into()], false),
        Some(Linkage::Internal),
    );

    // The runtime's print functions. `zeroext` keeps the C `bool` ABI: the callee may read
    // all 8 bits of the argument, so the caller must extend the `i1` with zeros.
    let print_int = declare_print(&module, "sisu_print_int", i64_type.into());
    let print_bool = declare_print(&module, "sisu_print_bool", context.bool_type().into());
    print_bool.add_attribute(AttributeLoc::Param(0), zeroext(context));
    // `sisu_panic(msg, len)` prints the message and exits; `sisu_panic_bounds(msg, len,
    // index, length)` prints a bounds panic and exits.
    let panic = declare_noreturn(&module, "sisu_panic", &[ptr_type.into(), i64_type.into()]);
    let bounds = [
        ptr_type.into(),
        i64_type.into(),
        i64_type.into(),
        i64_type.into(),
    ];
    let panic_bounds = declare_noreturn(&module, "sisu_panic_bounds", &bounds);
    // Declared only with the checks, so that a plain build's module stays as it was.
    let rc_fault =
        rc_checks.then(|| declare_noreturn(&module, "sisu_rc_fault", &[context.i32_type().into()]));

    let mut codegen = Codegen {
        context,
        module,
        builder: context.create_builder(),
        print_int,
        print_bool,
        panic,
        panic_bounds,
        alloc,
        free,
        rc_retain,
        rc_fault,
        program,
        helpers: RefCell::new(HashMap::new()),
        pending: RefCell::new(Vec::new()),
        target_data,
        path,
        source,
        lines: LineIndex::new(source),
        classes,
        functions: Vec::new(),
        locals: Vec::new(),
        types: Vec::new(),
        live: Vec::new(),
        loops: Vec::new(),
    };
    // A class is `{ i64 count, fields... }`: field `i` is element `i + 1`.
    for (class, ty) in program.classes.iter().zip(&codegen.classes) {
        let body: Vec<BasicTypeEnum> = std::iter::once(i64_type.into())
            .chain(class.fields.iter().map(|field| {
                codegen
                    .llvm_type(&field.ty)
                    .expect("the checker should give every class field a value type, never `unit` or `never`")
            }))
            .collect();
        ty.set_body(&body, false);
    }
    // Every class gets its helpers, used or not, as in milestone 2.
    for helper in [Helper::Release, Helper::Eq] {
        for id in 0..program.classes.len() {
            codegen.helper(helper, &Type::Class(ClassId(id)));
        }
    }
    // Declare every function first so that calls can refer to functions defined later.
    for f in &program.functions {
        codegen.declare(f);
    }
    for (i, f) in program.functions.iter().enumerate() {
        let function = codegen.functions[i];
        codegen.define(f, function);
    }
    codegen.c_main();
    codegen.define_helpers();
    codegen.module
}

/// Declares the runtime function `name`, which takes `params` and never returns.
fn declare_noreturn<'ctx>(
    module: &Module<'ctx>,
    name: &str,
    params: &[BasicMetadataTypeEnum<'ctx>],
) -> FunctionValue<'ctx> {
    let context = module.get_context();
    let function = module.add_function(name, context.void_type().fn_type(params, false), None);
    function.add_attribute(
        AttributeLoc::Function,
        context.create_enum_attribute(Attribute::get_named_enum_kind_id("noreturn"), 0),
    );
    function
}

/// Declares the runtime print function `name`, which takes one `param` and returns nothing.
/// The runtime is built with `panic = "abort"`, so it never unwinds. A print takes no
/// pointer, so it reaches no memory of the module: it uses only the runtime's own buffer and
/// state (inaccessible memory) and can set `errno`. It can also exit the process, so there
/// is no `willreturn`.
fn declare_print<'ctx>(
    module: &Module<'ctx>,
    name: &str,
    param: BasicMetadataTypeEnum<'ctx>,
) -> FunctionValue<'ctx> {
    // LLVM 22's `MemoryEffects` encoding (`llvm/Support/ModRef.h`): two bits per location,
    // `Ref` 1 and `Mod` 2, lowest first: `ArgMem`, `InaccessibleMem`, `ErrnoMem`, then
    // everything else. So this is `ErrnoMem` 3, `InaccessibleMem` 3, `ArgMem` 0, and none for
    // the rest, globals included.
    const MEMORY: u64 = 0b11_11_00;
    let context = module.get_context();
    let function = module.add_function(name, context.void_type().fn_type(&[param], false), None);
    function.add_attribute(
        AttributeLoc::Function,
        context.create_enum_attribute(Attribute::get_named_enum_kind_id("nounwind"), 0),
    );
    function.add_attribute(
        AttributeLoc::Function,
        context.create_enum_attribute(Attribute::get_named_enum_kind_id("memory"), MEMORY),
    );
    function
}

fn zeroext(context: &Context) -> Attribute {
    context.create_enum_attribute(Attribute::get_named_enum_kind_id("zeroext"), 0)
}

/// Where a local's value lives.
#[derive(Clone, Copy)]
enum Slot<'ctx> {
    /// A `let` binding or a parameter that is not counted: an SSA value.
    Value(BasicValueEnum<'ctx>),
    /// A variable or a counted local: an `alloca` of this type, loaded and stored until
    /// `mem2reg` promotes it. A `Move` stores `null` into a counted local's slot.
    Alloca(PointerValue<'ctx>, BasicTypeEnum<'ctx>),
}

/// An open `loop`: `Break` branches to `end`, `Continue` and a body that falls through
/// branch back to `body`. Both first release the locals above `depth`, the length of the
/// live-locals stack when the loop was entered.
struct LoopFrame<'ctx> {
    body: BasicBlock<'ctx>,
    end: BasicBlock<'ctx>,
    depth: usize,
}

struct Codegen<'ctx, 'src> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    print_int: FunctionValue<'ctx>,
    print_bool: FunctionValue<'ctx>,
    panic: FunctionValue<'ctx>,
    panic_bounds: FunctionValue<'ctx>,
    alloc: FunctionValue<'ctx>,
    free: FunctionValue<'ctx>,
    /// `@sisu_rc.retain`, which every counted type shares.
    rc_retain: FunctionValue<'ctx>,
    /// `sisu_rc_fault(kind)`, declared only with the RC checks: `Some` turns them on.
    rc_fault: Option<FunctionValue<'ctx>>,
    program: &'src Program,
    /// The helpers declared so far, by kind and counted type; see `rc::helper`.
    helpers: RefCell<HashMap<(Helper, Type), FunctionValue<'ctx>>>,
    /// The helpers declared but not yet defined; `define_helpers` empties it.
    pending: RefCell<Vec<(Helper, Type)>>,
    /// The target's layout: the sizes and alignments of classes.
    target_data: TargetData,
    /// The input path as given, and its text: panic messages name a position in it.
    path: &'src str,
    source: &'src str,
    /// The line starts of `source`, so that a panic site costs a binary search.
    lines: LineIndex,
    /// `%sisu.<Class>`, indexed by `ClassId`.
    classes: Vec<StructType<'ctx>>,
    /// Indexed by `FuncId`.
    functions: Vec<FunctionValue<'ctx>>,
    /// The current function's locals, indexed by `LocalId`; `None` until bound, and for `unit`.
    locals: Vec<Option<Slot<'ctx>>>,
    /// The current function's local types, indexed by `LocalId`.
    types: Vec<Type>,
    /// The counted locals in scope, newest last: each holds a reference to release.
    live: Vec<LocalId>,
    loops: Vec<LoopFrame<'ctx>>,
}

impl<'ctx> Codegen<'ctx, '_> {
    /// The LLVM type of a value of `ty`; `None` for `unit` and `never`, which have no value.
    fn llvm_type(&self, ty: &Type) -> Option<BasicTypeEnum<'ctx>> {
        match ty {
            Type::I64 => Some(self.context.i64_type().into()),
            Type::U8 => Some(self.context.i8_type().into()),
            Type::Bool => Some(self.context.bool_type().into()),
            Type::Unit | Type::Never => None,
            // An object or an array is a pointer to its block.
            Type::Class(_) | Type::Array(_) => {
                Some(self.context.ptr_type(AddressSpace::default()).into())
            }
            Type::Optional(payload) => match self
                .llvm_type(payload)
                .expect("the checker should reject an optional of `unit` or `never`, so its payload has a value type")
            {
                // A class optional is the object's pointer, `null` for `None`.
                ptr @ BasicTypeEnum::PointerType(_) => Some(ptr),
                // A scalar optional is `{ i1, T }`: whether it holds a value, then the value.
                value => {
                    let flag = self.context.bool_type().into();
                    Some(self.context.struct_type(&[flag, value], false).into())
                }
            },
        }
    }

    /// The ABI size and alignment of an object of class `id`, from the target's data layout.
    fn class_layout(&self, id: ClassId) -> (u64, u32) {
        let ty = &self.classes[id.0];
        (
            self.target_data.get_abi_size(ty),
            self.target_data.get_abi_alignment(ty),
        )
    }

    /// A pointer to field `index` of `object`, whose type is `ty`.
    fn field_ptr(&self, ty: &Type, object: PointerValue<'ctx>, index: usize) -> PointerValue<'ctx> {
        let Type::Class(id) = ty else {
            unreachable!("the checker should only allow field access on a class-typed value")
        };
        // Element 0 is the count.
        let element = u32::try_from(index + 1).expect(
            "a class should have fewer than 2^32 fields, since field indexes are `u32` in LLVM",
        );
        self.builder
            .build_struct_gep(self.classes[id.0], object, element, "field")
            .expect("the checker should only emit a field index that the class has")
    }

    /// Field `index` of the object `base`, whose type is `ty`. A local base is borrowed: the
    /// local keeps the object alive, so codegen skips its retain and release.
    fn read_field(&mut self, base: &Expr, index: usize, ty: &Type) -> BasicValueEnum<'ctx> {
        let object = self
            .borrow_or_expr(base)
            .expect("the checker should only emit a field access on a class-typed base, which lowers to a pointer")
            .into_pointer_value();
        let field = self.field_ptr(&base.ty, object, index);
        let llvm_ty = self.llvm_type(ty).expect(
            "the checker should give every class field a value type, never `unit` or `never`",
        );
        let value = self
            .builder
            .build_load(llvm_ty, field, "load")
            .expect(POSITIONED);
        // Retain before releasing the base: the base's drop could release the field first.
        self.retain(value, ty);
        if !matches!(base.kind, ExprKind::Local(_)) {
            self.release(object.into(), &base.ty);
        }
        value
    }

    /// `base` for a read that ends before anything can move or replace the base: a local is
    /// loaded without a retain, and the caller then skips the release; any other expression
    /// is evaluated as usual.
    pub(super) fn borrow_or_expr(&mut self, base: &Expr) -> Option<BasicValueEnum<'ctx>> {
        match base.kind {
            ExprKind::Local(local) => self.load_local(local),
            _ => self.expr(base),
        }
    }

    /// `a is b`: whether two class or class-optional values are the same pointer.
    fn same_object(&self, a: PointerValue<'ctx>, b: PointerValue<'ctx>) -> IntValue<'ctx> {
        self.builder
            .build_int_compare(IntPredicate::EQ, a, b, "same")
            .expect(POSITIONED)
    }

    fn call(&self, function: FunctionValue<'ctx>, args: &[BasicMetadataValueEnum<'ctx>]) {
        self.builder
            .build_call(function, args, "")
            .expect(POSITIONED);
    }

    /// Releases a value nobody keeps, if there is one.
    fn discard(&self, value: Option<BasicValueEnum<'ctx>>, ty: &Type) {
        if let Some(value) = value {
            self.release(value, ty);
        }
    }

    /// Releases the locals `live[depth..]`, newest first, without popping them.
    fn release_to(&self, depth: usize) {
        for local in self.live[depth..].iter().rev() {
            let value = self.load_local(*local).expect(
                "a counted local should have a value, since only initialised locals are released",
            );
            self.release(value, &self.types[local.0]);
        }
    }

    /// The value of `local`, without retaining it; `None` for `unit`.
    fn load_local(&self, local: LocalId) -> Option<BasicValueEnum<'ctx>> {
        self.locals[local.0].map(|slot| match slot {
            Slot::Value(value) => value,
            Slot::Alloca(ptr, ty) => {
                // Name the load after the local (the alloca carries its name).
                let name = ptr.get_name().to_string_lossy().into_owned();
                self.builder.build_load(ty, ptr, &name).expect(POSITIONED)
            }
        })
    }

    /// `Move`: the value of `local`, whose reference leaves its slot without a retain. The
    /// slot then holds `null`, which releasing skips.
    fn move_local(&self, local: LocalId) -> BasicValueEnum<'ctx> {
        let Some(Slot::Alloca(ptr, ty)) = self.locals[local.0] else {
            unreachable!(
                "a moved local should have an `alloca`, since only counted locals move and each has one"
            )
        };
        let value = self.builder.build_load(ty, ptr, "moved").expect(POSITIONED);
        self.store(ptr, ty.const_zero());
        value
    }

    /// `New`: allocates the object, then stores its count, 1, and each argument.
    fn new_object(&mut self, class: ClassId, args: &[Expr]) -> PointerValue<'ctx> {
        // Arguments run first: one that returns early then leaves no object behind.
        let args: Vec<_> = args
            .iter()
            .map(|a| self.expr(a).expect("the checker should reject a `unit` or `never` argument, so each argument has a value"))
            .collect();
        let i64_type = self.context.i64_type();
        let (size, align) = self.class_layout(class);
        let object = self
            .builder
            .build_call(
                self.alloc,
                &[
                    i64_type.const_int(size, false).into(),
                    i64_type.const_int(u64::from(align), false).into(),
                ],
                "object",
            )
            .expect(POSITIONED)
            .try_as_basic_value()
            .basic()
            .expect("`sisu_alloc` should return a pointer, as the runtime declares it")
            .into_pointer_value();
        self.store(object, i64_type.const_int(1, false).into());
        let ty = Type::Class(class);
        for (index, arg) in args.into_iter().enumerate() {
            let field = self.field_ptr(&ty, object, index);
            self.store(field, arg);
        }
        object
    }

    fn declare(&mut self, f: &Function) {
        let params: Vec<BasicMetadataTypeEnum> = f
            .params
            .iter()
            .map(|p| {
                self.llvm_type(&f.locals[p.0].ty)
                    .expect("the checker should reject a `unit` or `never` parameter, so each parameter has a value type")
                    .into()
            })
            .collect();
        let fn_type = match self.llvm_type(&f.ret) {
            Some(ret) => ret.fn_type(&params, false),
            None => self.context.void_type().fn_type(&params, false),
        };
        let function = self.module.add_function(
            &format!("sisu.{}", f.name),
            fn_type,
            Some(Linkage::Internal),
        );
        self.functions.push(function);
    }

    fn define(&mut self, f: &Function, function: FunctionValue<'ctx>) {
        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);
        // Every variable and counted local gets its `alloca` up front, in the entry block: see
        // `entry_alloca`.
        self.locals = f
            .locals
            .iter()
            .map(|local| {
                let ty = self
                    .llvm_type(&local.ty)
                    .filter(|_| local.mutable || local.ty.is_counted())?;
                let name = local.name.as_deref().unwrap_or("");
                Some(Slot::Alloca(self.entry_alloca(ty, name), ty))
            })
            .collect();
        self.types = f.locals.iter().map(|local| local.ty.clone()).collect();
        // The parameters own their arguments: they are the bottom of the live-locals stack.
        self.live = Vec::new();
        for (p, v) in f.params.iter().zip(function.get_param_iter()) {
            self.bind(*p, v);
        }
        let value = self.block(&f.body, None);
        if !self.terminated() {
            // The declared return type picks the `ret`: a unit function discards any body value.
            let value = if self.llvm_type(&f.ret).is_some() {
                Some(value.expect("the checker should give a body that falls through a value of the function's return type"))
            } else {
                self.discard(value, &f.body.ty);
                None
            };
            self.release_to(0);
            self.builder
                .build_return(value.as_ref().map(|v| v as &dyn BasicValue))
                .expect(POSITIONED);
        }
    }

    /// `define i32 @main()`, which the C startup code calls: runs `sisu.main`, flushes standard
    /// output, returns 0.
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
            .expect("the checker should reject a program without a `main` function");
        self.builder
            .build_call(sisu_main, &[], "")
            .expect(POSITIONED);
        let void = self.context.void_type().fn_type(&[], false);
        // `print` buffers; this delivers what is left, or panics if standard output fails.
        let flush = self.module.add_function("sisu_flush_stdout", void, None);
        self.call(flush, &[]);
        if self.rc_fault.is_some() {
            let check_exit = self.module.add_function("sisu_rc_check_exit", void, None);
            self.call(check_exit, &[]);
        }
        self.builder
            .build_return(Some(&i32_type.const_zero()))
            .expect(POSITIONED);
    }

    fn current_block(&self) -> BasicBlock<'ctx> {
        self.builder.get_insert_block().expect(POSITIONED)
    }

    fn current_function(&self) -> FunctionValue<'ctx> {
        self.current_block()
            .get_parent()
            .expect("the block the builder is positioned in should belong to a function")
    }

    /// Whether the current block already ends in a terminator, so nothing more can follow.
    fn terminated(&self) -> bool {
        self.current_block().get_terminator().is_some()
    }

    /// The current block, or `None` when it is terminated: the block a branch falls through from.
    fn open_block(&self) -> Option<BasicBlock<'ctx>> {
        (!self.terminated()).then(|| self.current_block())
    }

    /// The block's value, `None` for `unit` or `never`. A block that falls through releases
    /// the locals it bound after evaluating its value; either way they leave the stack. `bind`
    /// is an `IfSome`'s binding, which the block owns like its own locals.
    fn block(
        &mut self,
        block: &Block,
        bind: Option<(LocalId, BasicValueEnum<'ctx>)>,
    ) -> Option<BasicValueEnum<'ctx>> {
        let start = self.live.len();
        if let Some((local, value)) = bind {
            self.bind(local, value);
        }
        let mut value = None;
        // A `never` statement ends the block; what follows is unreachable.
        for stmt in &block.stmts {
            if self.terminated() {
                break;
            }
            self.stmt(stmt);
        }
        if let Some(e) = &block.value
            && !self.terminated()
        {
            value = self.expr(e);
        }
        if !self.terminated() {
            self.release_to(start);
        }
        self.live.truncate(start);
        value
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { local, init } => {
                if let Some(value) = self.expr(init) {
                    self.bind(*local, value);
                }
            }
            Stmt::Assign {
                place: Place::Local(local),
                value,
            } => {
                let Some(value) = self.expr(value) else {
                    return;
                };
                let Some(Slot::Alloca(ptr, ty)) = self.locals[local.0] else {
                    unreachable!(
                        "the checker should only let a variable be assigned, and every variable has an `alloca`"
                    )
                };
                // The value first, then the store, then the release of the old value.
                let counted = &self.types[local.0];
                let old = counted.is_counted().then(|| self.load(ty, ptr));
                self.store(ptr, value);
                self.discard(old, counted);
            }
            Stmt::Assign {
                place: Place::Field { base, index },
                value,
            } => {
                // Left to right: the base, then the value, then the store, then the release
                // of the old value, then of the base, which keeps the object alive until then.
                let object = self
                    .expr(base)
                    .expect("the checker should only emit a field access on a class-typed base, which lowers to a pointer")
                    .into_pointer_value();
                let ty = &value.ty;
                let Some(value) = self.expr(value) else {
                    return;
                };
                let field = self.field_ptr(&base.ty, object, *index);
                let old = ty.is_counted().then(|| self.load(value.get_type(), field));
                self.store(field, value);
                self.discard(old, ty);
                self.release(object.into(), &base.ty);
            }
            Stmt::Assign {
                place: place @ Place::Index { .. },
                value,
            } => self.assign_index(place, value),
            Stmt::Expr(e) => {
                let value = self.expr(e);
                self.discard(value, &e.ty);
            }
        }
    }

    /// Gives `local` the owned `value`; a counted local joins the live-locals stack.
    fn bind(&mut self, local: LocalId, value: BasicValueEnum<'ctx>) {
        match self.locals[local.0] {
            Some(Slot::Alloca(ptr, _)) => self.store(ptr, value),
            _ => self.locals[local.0] = Some(Slot::Value(value)),
        }
        if self.types[local.0].is_counted() {
            self.live.push(local);
        }
    }

    /// `lhs == rhs`, or `!=` when `negated`; releases both operands after comparing them.
    fn equal_expr(&mut self, negated: bool, lhs: &Expr, rhs: &Expr) -> IntValue<'ctx> {
        let (l, r) = (self.operand(lhs), self.operand(rhs));
        let equal = match lhs.ty {
            Type::I64 | Type::U8 | Type::Bool if negated => {
                self.int_compare(IntPredicate::NE, l.into_int_value(), r.into_int_value())
            }
            _ if negated => {
                let equal = self.equal(&lhs.ty, l, r);
                self.builder.build_not(equal, "not").expect(POSITIONED)
            }
            _ => self.equal(&lhs.ty, l, r),
        };
        self.release(l, &lhs.ty);
        self.release(r, &rhs.ty);
        equal
    }

    /// `lhs is rhs`; releases both operands after comparing them.
    fn is_expr(&mut self, lhs: &Expr, rhs: &Expr) -> IntValue<'ctx> {
        let (l, r) = (self.operand(lhs), self.operand(rhs));
        let same = self.same_object(l.into_pointer_value(), r.into_pointer_value());
        self.release(l, &lhs.ty);
        self.release(r, &rhs.ty);
        same
    }

    /// An operand of `==` or `is`.
    fn operand(&mut self, e: &Expr) -> BasicValueEnum<'ctx> {
        self.expr(e)
            .expect("the checker should reject a `unit` or `never` operand, so it has a value")
    }

    /// An expression the checker typed `i64` or `bool`.
    fn value(&mut self, e: &Expr) -> IntValue<'ctx> {
        self.expr(e)
            .expect("the checker should type this expression `i64` or `bool`, so it has a value")
            .into_int_value()
    }

    /// The expression's value, `None` for `unit` or `never`.
    fn expr(&mut self, e: &Expr) -> Option<BasicValueEnum<'ctx>> {
        let value = match &e.kind {
            ExprKind::Int(n) => self
                .llvm_type(&e.ty)
                .expect("the checker should type an integer literal as an integer type, which has an LLVM type")
                .into_int_type()
                .const_int(n.cast_unsigned(), false),
            ExprKind::Bool(v) => self.context.bool_type().const_int(u64::from(*v), false),
            ExprKind::ArrayLit(elements) => return Some(self.array_lit(&e.ty, elements).into()),
            ExprKind::ArrayFill { value, len } => {
                return Some(self.array_fill(&e.ty, value, len).into());
            }
            ExprKind::Index { base, index } => return Some(self.index(&e.ty, base, index)),
            ExprKind::Len(base) => self.len(base),
            ExprKind::Sum(base) => self.sum(base, e.span),
            ExprKind::Contains { base, value } => self.contains(base, value),
            ExprKind::Local(local) => {
                let value = self.load_local(*local)?;
                self.retain(value, &e.ty);
                return Some(value);
            }
            ExprKind::Move(local) => return Some(self.move_local(*local)),
            ExprKind::Call { func, args } => {
                let args: Vec<_> = args
                    .iter()
                    .map(|a| {
                        self.expr(a)
                            .expect("the checker should reject a `unit` or `never` argument, so each argument has a value")
                            .into()
                    })
                    .collect();
                return self
                    .builder
                    .build_call(self.functions[func.0], &args, "call")
                    .expect(POSITIONED)
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
                        .expect(POSITIONED),
                }
            }
            ExprKind::Binary { op, lhs, rhs } => self.binary(*op, lhs, rhs, e.span),
            ExprKind::Compare { operands, ops } => self.compare(operands, ops),
            ExprKind::Equal { negated, lhs, rhs } => self.equal_expr(*negated, lhs, rhs),
            ExprKind::Is { lhs, rhs } => self.is_expr(lhs, rhs),
            ExprKind::If {
                cond,
                then_block,
                else_block,
            } => {
                let cond = self.value(cond);
                return self.if_expr(cond, then_block, None, else_block.as_ref(), &e.ty);
            }
            ExprKind::Loop(body) => {
                self.loop_expr(body);
                return None;
            }
            ExprKind::Break => {
                let frame = self.loops.last().expect("the checker should reject `break` outside a loop");
                let (end, depth) = (frame.end, frame.depth);
                self.release_to(depth);
                self.branch_to(end);
                return None;
            }
            ExprKind::Continue => {
                let frame = self.loops.last().expect("the checker should reject `continue` outside a loop");
                let (body, depth) = (frame.body, frame.depth);
                self.release_to(depth);
                self.branch_to(body);
                return None;
            }
            ExprKind::New { class, args } => return Some(self.new_object(*class, args).into()),
            ExprKind::Field { base, index } => return Some(self.read_field(base, *index, &e.ty)),
            ExprKind::Block(block) => return self.block(block, None),
            ExprKind::Convert(_) => self.convert(e),
            ExprKind::None | ExprKind::Wrap(_) | ExprKind::IfSome { .. } => {
                return self.optional(e);
            }
            ExprKind::Return(value) => {
                self.return_expr(value.as_deref());
                return None;
            }
        };
        Some(value.into())
    }

    /// `return`: the value, then the release of every live local, then the `ret`.
    fn return_expr(&mut self, value: Option<&Expr>) {
        let value = value.and_then(|e| self.expr(e));
        // The checker rejects a `never` return value; this keeps codegen safe anyway.
        if !self.terminated() {
            self.release_to(0);
            self.builder
                .build_return(value.as_ref().map(|v| v as &dyn BasicValue))
                .expect(POSITIONED);
        }
    }

    /// `None`, `Wrap` and `IfSome`: a class optional is a pointer, a scalar one a `{ i1, T }`.
    fn optional(&mut self, e: &Expr) -> Option<BasicValueEnum<'ctx>> {
        match &e.kind {
            // `null`, or `{ false, 0 }`.
            ExprKind::None => Some(
                self.llvm_type(&e.ty)
                    .expect("the checker should type `None` as an optional, which has an LLVM type")
                    .const_zero(),
            ),
            ExprKind::Wrap(operand) => {
                let value = self.expr(operand)?;
                // A class needs no wrapping: its pointer is never `null`.
                if e.ty.is_counted() {
                    return Some(value);
                }
                let pair = self
                    .llvm_type(&e.ty)
                    .expect("the checker should type `Wrap` as an optional, which has an LLVM type")
                    .into_struct_type();
                let some = self.context.bool_type().const_int(1, false);
                let pair = self
                    .builder
                    .build_insert_value(pair.const_zero(), some, 0, "some")
                    .expect("a scalar optional should be lowered to a `{ flag, value }` pair");
                let pair = self
                    .builder
                    .build_insert_value(pair, value, 1, "wrap")
                    .expect("a scalar optional should be lowered to a `{ flag, value }` pair");
                Some(pair.as_basic_value_enum())
            }
            ExprKind::IfSome {
                bind,
                scrutinee,
                then_block,
                else_block,
            } => {
                // A class optional's reference moves into `bind`. On the `None` path it is
                // `null`, so it holds nothing to release. A scalar pair binds its value.
                let value = self
                    .expr(scrutinee)
                    .expect("the checker should reject an `if let` scrutinee that is not an optional, so it has a value");
                let (some, bound) = if scrutinee.ty.is_counted() {
                    let some = self
                        .builder
                        .build_is_not_null(value.into_pointer_value(), "some")
                        .expect(POSITIONED);
                    (some, value)
                } else {
                    let part = |index, name| {
                        self.builder
                            .build_extract_value(value.into_struct_value(), index, name)
                            .expect(
                                "a scalar optional should be lowered to a `{ flag, value }` pair",
                            )
                    };
                    (part(0, "some").into_int_value(), part(1, "value"))
                };
                let bind = Some((*bind, bound));
                self.if_expr(some, then_block, bind, else_block.as_ref(), &e.ty)
            }
            _ => unreachable!("`expr` should pass `optional` only optional nodes"),
        }
    }

    /// An `alloca` at the start of the function's entry block, wherever the variable sits:
    /// `mem2reg` promotes only entry-block allocas, and one in a loop body would grow the
    /// stack on every iteration.
    fn entry_alloca(&self, ty: BasicTypeEnum<'ctx>, name: &str) -> PointerValue<'ctx> {
        let entry = self.current_function().get_first_basic_block().expect(
            "a function with a body should have an entry block before any alloca is emitted",
        );
        let builder = self.context.create_builder();
        match entry.get_first_instruction() {
            Some(first) => builder.position_before(&first),
            None => builder.position_at_end(entry),
        }
        builder.build_alloca(ty, name).expect(POSITIONED)
    }

    /// The value a place held before an assignment.
    fn load(&self, ty: BasicTypeEnum<'ctx>, ptr: PointerValue<'ctx>) -> BasicValueEnum<'ctx> {
        self.builder.build_load(ty, ptr, "old").expect(POSITIONED)
    }

    fn store(&self, ptr: PointerValue<'ctx>, value: BasicValueEnum<'ctx>) {
        self.builder.build_store(ptr, value).expect(POSITIONED);
    }

    /// `loop`: the body block runs until a `Break` branches to the end block, where codegen
    /// continues; a body that falls through branches back to its start.
    fn loop_expr(&mut self, body: &Block) {
        let function = self.current_function();
        let frame = LoopFrame {
            body: self.context.append_basic_block(function, "loop.body"),
            end: self.context.append_basic_block(function, "loop.end"),
            depth: self.live.len(),
        };
        let (start, end) = (frame.body, frame.end);
        self.branch_to(start);
        self.builder.position_at_end(start);
        self.loops.push(frame);
        let value = self.block(body, None);
        self.loops.pop();
        if !self.terminated() {
            self.discard(value, &body.ty);
            self.branch_to(start);
        }
        self.builder.position_at_end(end);
    }

    fn print(&mut self, operand: &Expr) {
        let mut arg = self.value(operand);
        if operand.ty == Type::U8 {
            arg = self
                .builder
                .build_int_z_extend(arg, self.context.i64_type(), "widened")
                .expect(POSITIONED);
        }
        let is_bool = operand.ty == Type::Bool;
        let print = if is_bool {
            self.print_bool
        } else {
            self.print_int
        };
        let call = self
            .builder
            .build_call(print, &[arg.into()], "")
            .expect(POSITIONED);
        if is_bool {
            call.add_attribute(AttributeLoc::Param(0), zeroext(self.context));
        }
    }

    /// Panics with `<path>:<line>:<col>: <message>`, for the start of `span`, when `fails`
    /// holds; see `panic_with`.
    fn panic_if(&self, fails: IntValue<'ctx>, span: Span, message: &str) {
        let text = format!("{}: {message}", self.location(span));
        self.panic_with(fails, &text, self.panic, &[]);
    }

    /// `<path>:<line>:<col>` for the start of `span`.
    fn location(&self, span: Span) -> String {
        let (line, col) = self.lines.line_col(self.source, span.start);
        format!("{}:{line}:{col}", self.path)
    }

    /// Branches to a fresh panic block when `fails` holds, and continues in a fresh block
    /// otherwise. The panic block calls `function`, which never returns, with `text` and its
    /// length, then `extra`.
    fn panic_with(
        &self,
        fails: IntValue<'ctx>,
        text: &str,
        function: FunctionValue<'ctx>,
        extra: &[BasicMetadataValueEnum<'ctx>],
    ) {
        let current = self.current_function();
        let panic_block = self.context.append_basic_block(current, "panic");
        let ok = self.context.append_basic_block(current, "ok");
        self.builder
            .build_conditional_branch(fails, panic_block, ok)
            .expect(POSITIONED);
        self.builder.position_at_end(panic_block);
        let msg = self
            .builder
            .build_global_string_ptr(text, "panic.msg")
            .expect(POSITIONED);
        let len = self.context.i64_type().const_int(
            u64::try_from(text.len()).expect("a panic message should be shorter than 2^64 bytes"),
            false,
        );
        let args: Vec<BasicMetadataValueEnum> = [msg.as_pointer_value().into(), len.into()]
            .into_iter()
            .chain(extra.iter().copied())
            .collect();
        self.builder
            .build_call(function, &args, "")
            .expect(POSITIONED);
        self.builder.build_unreachable().expect(POSITIONED);
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
            .expect(POSITIONED);
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
        let (last_op, first_ops) = ops
            .split_last()
            .expect("a comparison chain should have at least one operator");
        let function = self.current_function();
        let mut false_from = Vec::new();
        let signed = operands[0].ty != Type::U8;
        let mut lhs = self.value(&operands[0]);
        for (op, operand) in first_ops.iter().zip(&operands[1..]) {
            let rhs = self.value(operand);
            let holds = self.compare_pair(*op, signed, lhs, rhs);
            let next = self.context.append_basic_block(function, "chain.next");
            false_from.push((holds, self.current_block(), next));
            self.builder.position_at_end(next);
            lhs = rhs;
        }
        let rhs = self.value(
            operands
                .last()
                .expect("a comparison chain should have at least two operands"),
        );
        let holds = self.compare_pair(*last_op, signed, lhs, rhs);
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
                .expect(POSITIONED);
            incoming.push((no, block));
        }
        incoming.push((holds, last_end));
        self.builder.position_at_end(merge);
        self.phi(self.context.bool_type(), &incoming)
            .into_int_value()
    }

    /// `if`/`else` on `cond`, and `IfSome` with the binding its then block starts with. The
    /// merge block exists only if a branch falls through; its `phi`, built when `ty` has a
    /// value, takes one edge per such branch, from the block where that branch ended.
    fn if_expr(
        &mut self,
        cond: IntValue<'ctx>,
        then_block: &Block,
        bind: Option<(LocalId, BasicValueEnum<'ctx>)>,
        else_block: Option<&Block>,
        ty: &Type,
    ) -> Option<BasicValueEnum<'ctx>> {
        let function = self.current_function();
        let then_start = self.context.append_basic_block(function, "then");
        let else_start = self.context.append_basic_block(function, "else");
        self.builder
            .build_conditional_branch(cond, then_start, else_start)
            .expect(POSITIONED);
        // A `unit` `if` discards its branches' values.
        let discards = *ty == Type::Unit;
        self.builder.position_at_end(then_start);
        let then_value = self.block(then_block, bind);
        if discards && !self.terminated() {
            self.discard(then_value, &then_block.ty);
        }
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
        let else_value = self.block(else_block, None);
        if discards && !self.terminated() {
            self.discard(else_value, &else_block.ty);
        }
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
            .map(|(value, end)| (value.expect("the checker should give every non-diverging `if` branch a value of the `if`'s type"), end))
            .collect();
        Some(self.phi(ty, &incoming))
    }

    fn branch_to(&self, block: BasicBlock<'ctx>) {
        self.builder
            .build_unconditional_branch(block)
            .expect(POSITIONED);
    }

    fn phi<V: BasicValue<'ctx>>(
        &self,
        ty: impl BasicType<'ctx>,
        incoming: &[(V, BasicBlock<'ctx>)],
    ) -> BasicValueEnum<'ctx> {
        let phi = self.builder.build_phi(ty, "phi").expect(POSITIONED);
        for (value, block) in incoming {
            phi.add_incoming(&[(value, *block)]);
        }
        phi.as_basic_value()
    }
}

/// A target machine for the host `sisuc` runs on.
pub(crate) fn target_machine(o2: bool) -> Result<TargetMachine, String> {
    Target::initialize_native(&InitializationConfig::default())?;
    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple).map_err(|e| e.to_string())?;
    // PIC because Ubuntu's `cc` links position-independent executables by default.
    target
        .create_target_machine(
            &triple,
            "generic",
            "",
            if o2 {
                OptimizationLevel::Default
            } else {
                OptimizationLevel::None
            },
            RelocMode::PIC,
            CodeModel::Default,
        )
        .ok_or_else(|| "LLVM cannot create a target machine for this host".to_string())
}

/// The pass pipeline: `default<O2>` with `-O2`, else `mem2reg`, which turns each variable's
/// `alloca` into SSA values.
pub(crate) fn pipeline(o2: bool) -> &'static str {
    if o2 { "default<O2>" } else { "mem2reg" }
}

/// Verifies `module`, then runs the pipeline.
pub(crate) fn optimize(
    module: &Module<'_>,
    machine: &TargetMachine,
    o2: bool,
) -> Result<(), String> {
    module.verify().map_err(|e| e.to_string())?;
    module
        .run_passes(pipeline(o2), machine, PassBuilderOptions::create())
        .map_err(|e| e.to_string())
}

/// Compiles `module` to object code for `machine`, in memory.
pub(crate) fn object_code(
    module: &Module<'_>,
    machine: &TargetMachine,
) -> Result<MemoryBuffer<'static>, String> {
    module.verify().map_err(|e| e.to_string())?;
    machine
        .write_to_memory_buffer(module, FileType::Object)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
