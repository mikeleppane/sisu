use super::*;
use crate::check::check;
use crate::lexer::lex;
use crate::parser::parse;

/// Lexes, parses, checks (no errors allowed), marks last uses in and compiles `src`, then
/// verifies the module.
fn compiled<'ctx>(context: &'ctx Context, src: &str) -> Module<'ctx> {
    compiled_with(context, src, false)
}

/// `compiled`, with the RC checks if `rc_checks`.
fn compiled_with<'ctx>(context: &'ctx Context, src: &str, rc_checks: bool) -> Module<'ctx> {
    let program = parse(&lex(src).expect("source lexes")).expect("source parses");
    let (program, diagnostics) = check(&program);
    let mut program = program.unwrap_or_else(|| panic!("{diagnostics:?}"));
    crate::last_use::mark(&mut program);
    let machine = target_machine(false).expect("the host is a target");
    let module = compile(context, &program, "test.sisu", src, &machine, rc_checks);
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
        let body = function_body(&ir, &header);
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

/// The body of the function `header` starts: up to its closing `}`.
fn function_body<'a>(ir: &'a str, header: &str) -> &'a str {
    ir.split_once(header)
        .and_then(|(_, rest)| rest.split("\n}").next())
        .unwrap_or_else(|| panic!("no {header} in\n{ir}"))
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
fn class_layout_comes_from_the_data_layout() {
    let context = Context::create();
    let ir = compiled(
        &context,
        "class Q {}\nclass P {\n    x: i64\n    mut y: bool\n    q: Q\n}\nfn main() {\n    let p = P(x: 1, y: true, q: Q())\n    print(p.x)\n}",
    )
    .print_to_string()
    .to_string();
    for wanted in [
        "%sisu.P = type { i64, i64, i1, ptr }",
        "call ptr @sisu_alloc(i64 32, i64 8)",
    ] {
        assert!(ir.contains(wanted), "missing {wanted:?} in\n{ir}");
    }
    // Element 0 is the count, so field `i` is element `i + 1`.
    for element in 1..=3 {
        let gep = format!("i32 0, i32 {element}");
        assert!(
            ir.lines().any(|l| l.contains("getelementptr")
                && l.contains("%sisu.P, ptr ")
                && l.ends_with(&gep)),
            "no field GEP to element {element} in\n{ir}"
        );
    }
}

#[test]
fn empty_class_allocates_its_count() {
    let context = Context::create();
    let ir = compiled(&context, "class E {}\nfn main() { E() }")
        .print_to_string()
        .to_string();
    let lines: Vec<&str> = ir.lines().map(str::trim).collect();
    let (_, object, _) = instruction(&lines, &|t| t == "call ptr @sisu_alloc(i64 8, i64 8)")
        .unwrap_or_else(|| panic!("no 8-byte allocation in\n{ir}"));
    let count = format!("store i64 1, ptr {object},");
    assert!(
        lines.iter().any(|l| l.starts_with(&count)),
        "missing {count:?} in\n{ir}"
    );
}

#[test]
fn method_symbols() {
    let context = Context::create();
    let ir = compiled(
        &context,
        "class P {\n    x: i64\n\n    fn m(self) { print(self.x) }\n}\nfn main() { P(x: 1).m() }",
    )
    .print_to_string()
    .to_string();
    assert!(ir.contains("define void @sisu.P.m(ptr"), "{ir}");
}

#[test]
fn verifies_field_assignment_through_a_call() {
    compiled(
        &Context::create(),
        "class C {\n    mut n: i64\n}\nfn make() -> C { C(n: 0) }\nfn main() {\n    make().n = 1\n}",
    );
}

#[test]
fn helpers_are_generated() {
    let context = Context::create();
    let ir = compiled(&context, "class P {}\nfn main() { P() }")
        .print_to_string()
        .to_string();
    for wanted in [
        "define void @sisu_rc.retain(ptr",
        "define void @sisu_rc.release.P(ptr",
        "define void @sisu_rc.drop.P(ptr",
    ] {
        assert!(ir.contains(wanted), "missing {wanted:?} in\n{ir}");
    }
}

#[test]
fn drop_loops_on_the_chain_field() {
    let context = Context::create();
    let ir = compiled(
        &context,
        "class Node {\n    value: i64\n    next: Node?\n}\nclass Tree {\n    left: Tree?\n    right: Tree?\n    v: i64\n}\nfn main() {}",
    )
    .print_to_string()
    .to_string();
    let node = function_body(&ir, "define void @sisu_rc.drop.Node(ptr");
    assert!(node.contains("phi ptr"), "no loop in\n{node}");
    assert_eq!(node.matches("@sisu_free(").count(), 1, "{node}");
    for unwanted in ["@sisu_rc.drop.Node(", "@sisu_rc.release.Node("] {
        assert!(!node.contains(unwanted), "{unwanted:?} in\n{node}");
    }
    // `right` is the chain field; only `left` is released, by recursion.
    let tree = function_body(&ir, "define void @sisu_rc.drop.Tree(ptr");
    assert_eq!(tree.matches("@sisu_rc.release.Tree(").count(), 1, "{tree}");
    assert!(!tree.contains("@sisu_rc.drop.Tree("), "{tree}");
}

#[test]
fn eq_loops_on_the_chain_field() {
    let context = Context::create();
    let ir = compiled(
        &context,
        "class Node {\n    value: i64\n    next: Node?\n}\nclass First {\n    next: First?\n    value: i64\n}\nfn same(a: Node?, b: Node?, c: First?, d: First?) -> bool { a == b && c == d }\nfn main() {}",
    )
    .print_to_string()
    .to_string();
    // The chain field is compared in a loop, in either field order: no `eq` calls itself.
    for (class, header) in [
        ("Node", "define i1 @sisu_rc.eq.Node(ptr"),
        ("First", "define i1 @sisu_rc.eq.First(ptr"),
    ] {
        let body = function_body(&ir, header);
        assert!(body.contains("phi ptr"), "no loop in\n{body}");
        let own_call = format!("call i1 @sisu_rc.eq.{class}(");
        assert!(!body.contains(&own_call), "{own_call:?} in\n{body}");
    }
}

#[test]
fn helper_names_cannot_collide() {
    let context = Context::create();
    let ir = compiled(
        &context,
        "fn retain() {}\nclass release {\n    fn Tree(self) {}\n}\nclass Tree {}\nfn main() {\n    retain()\n    release().Tree()\n}",
    )
    .print_to_string()
    .to_string();
    // The method `release.Tree` and the helper for class `Tree` must both keep their names:
    // LLVM renames the second of two equal names with a numeric suffix.
    for wanted in [
        "define void @sisu.retain()",
        "define void @sisu.release.Tree(ptr",
        "define void @sisu_rc.release.Tree(ptr",
    ] {
        assert!(ir.contains(wanted), "missing {wanted:?} in\n{ir}");
    }
    assert!(
        !ir.contains("@sisu.release.Tree."),
        "a renamed symbol in\n{ir}"
    );
}

#[test]
fn scalar_optional_is_a_pair() {
    let context = Context::create();
    let ir = compiled(&context, "fn f(x: i64?) -> i64? { x }\nfn main() {}")
        .print_to_string()
        .to_string();
    assert!(
        ir.contains("define { i1, i64 } @sisu.f({ i1, i64 }"),
        "{ir}"
    );
}

/// `!=` on an `i64` or a `bool` is one `icmp ne`, not an `icmp eq` and a `not`.
#[test]
fn scalar_not_equal_is_one_icmp_ne() {
    let context = Context::create();
    let ir = compiled(
        &context,
        "fn f(a: i64, b: i64) -> bool { a != b }\nfn g(a: bool, b: bool) -> bool { a != b }\nfn main() {}",
    )
    .print_to_string()
    .to_string();
    for (name, ty) in [("f", "i64"), ("g", "i1")] {
        let body = function_body(&ir, &format!("define i1 @sisu.{name}("));
        assert!(body.contains(&format!("icmp ne {ty} ")), "{body}");
        assert!(!body.contains("icmp eq"), "{body}");
        assert!(!body.contains("xor"), "{body}");
    }
}

#[test]
fn eq_helper_per_class() {
    let context = Context::create();
    let ir = compiled(&context, "class T {}\nfn main() { print(T() == T()) }")
        .print_to_string()
        .to_string();
    assert!(ir.contains("define i1 @sisu_rc.eq.T(ptr"), "{ir}");
}

#[test]
fn field_read_retains_before_releasing_base() {
    let context = Context::create();
    let ir = compiled(
        &context,
        "class N {}\nclass C {\n    next: N\n}\nfn make() -> C { C(next: N()) }\nfn main() { make().next }",
    )
    .print_to_string()
    .to_string();
    let main = function_body(&ir, "define void @sisu.main()");
    let lines: Vec<&str> = main.lines().map(str::trim).collect();
    let at = |wanted: &dyn Fn(&str) -> bool, what: &str| {
        lines
            .iter()
            .position(|l| wanted(l))
            .unwrap_or_else(|| panic!("no {what} in\n{main}"))
    };
    let (_, next, _) = instruction(&lines, &|t| t.starts_with("load ptr, ptr "))
        .unwrap_or_else(|| panic!("no load of `next` in\n{main}"));
    let order = [
        at(&|l| l.starts_with(&format!("{next} = load ptr")), "load"),
        at(
            &|l| l == format!("call void @sisu_rc.retain(ptr {next})"),
            "retain of the field",
        ),
        at(
            &|l| l.starts_with("call void @sisu_rc.release.C(ptr "),
            "release of the base",
        ),
    ];
    assert!(order.is_sorted(), "{order:?} in\n{main}");
}

/// The object whose count `text`, an instruction `<op> i64 <count>, ...`, uses: the pointer
/// `<count>` is loaded from in `lines`.
fn count_owner<'a>(lines: &[&'a str], text: &str) -> Option<&'a str> {
    let count = text.split_once(" i64 ")?.1.split(", ").next()?;
    let load = format!("{count} = load i64, ptr ");
    lines
        .iter()
        .find_map(|l| l.strip_prefix(load.as_str()))?
        .split(", ")
        .next()
}

/// Asserts that in `body` the RC check of `kind` guards `step`, the first instruction
/// `<op> i64 <count>, ...` that uses an object's count: before `step`, the count of that object
/// is loaded and compared signed against 1, and the branch on the result goes to a block that
/// calls `sisu_rc_fault(i32 <kind>)`.
fn assert_count_checked(body: &str, kind: u32, step: &dyn Fn(&str) -> bool) {
    let lines: Vec<&str> = body.lines().map(str::trim).collect();
    let (step_at, _, step_text) =
        instruction(&lines, step).unwrap_or_else(|| panic!("no count step in\n{body}"));
    let object = count_owner(&lines, step_text)
        .unwrap_or_else(|| panic!("{step_text:?} should use a loaded count in\n{body}"));
    let (test_at, dead, _) = instruction(&lines, &|t| {
        t.starts_with("icmp slt i64 ")
            && t.ends_with(", 1")
            && count_owner(&lines, t) == Some(object)
    })
    .unwrap_or_else(|| panic!("no signed test of {object}'s count against 1 in\n{body}"));
    let branch = format!("br i1 {dead}, label %");
    let branch_at = lines
        .iter()
        .position(|l| l.starts_with(&branch))
        .unwrap_or_else(|| panic!("no branch on {dead} in\n{body}"));
    let fault = lines[branch_at][branch.len()..]
        .split(',')
        .next()
        .expect("a branch should name its true label");
    let fault_at = lines
        .iter()
        .position(|l| l.starts_with(&format!("{fault}:")))
        .unwrap_or_else(|| panic!("no block {fault} in\n{body}"));
    assert_eq!(
        lines.get(fault_at + 1).copied(),
        Some(format!("call void @sisu_rc_fault(i32 {kind})").as_str()),
        "{body}"
    );
    assert!([test_at, branch_at, step_at].is_sorted(), "{body}");
}

#[test]
fn rc_checks_guard_retain_release_and_exit() {
    let context = Context::create();
    // A chain field: its drop loop's phi must name the block the checked decrement ends in.
    // `fill` shares one array among its slots; `write` makes a shared array unique.
    let src = format!(
        "class P {{\n    next: P?\n}}\nfn fill(n: i64) -> [[i64]] {{ [[1]; n] }}\nfn write(xs: [i64]) -> [i64] {{\n    let mut ys = xs\n    ys[0] = 1\n    ys\n}}\n{FIB}"
    );
    let ir = compiled_with(&context, &src, true)
        .print_to_string()
        .to_string();
    let retain = function_body(&ir, "define void @sisu_rc.retain(ptr");
    assert_count_checked(retain, 0, &|t| {
        t.starts_with("add i64 ") && t.ends_with(", 1")
    });
    let release = function_body(&ir, "define void @sisu_rc.release.P(ptr");
    assert_count_checked(release, 1, &|t| {
        t.starts_with("add i64 ") && t.ends_with(", -1")
    });
    let fill = function_body(&ir, "@sisu.fill(");
    // The loop's counter steps with `add nuw`; the count gains its extra references with `add`.
    assert_count_checked(fill, 0, &|t| t.starts_with("add i64 "));
    let unique = function_body(&ir, "define ptr @\"sisu_rc.unique.[i64]\"(ptr");
    assert_count_checked(unique, 1, &|t| {
        t.starts_with("icmp eq i64 ") && t.ends_with(", 1")
    });
    let main = function_body(&ir, "define i32 @main()");
    let (_, after_check) = main
        .split_once("call void @sisu_rc_check_exit()")
        .unwrap_or_else(|| panic!("no exit check in\n{main}"));
    assert!(after_check.contains("ret i32 0"), "{main}");
}

#[test]
fn plain_build_has_no_rc_checks() {
    let context = Context::create();
    let src = format!("class P {{}}\n{FIB}");
    let ir = compiled(&context, &src).print_to_string().to_string();
    for unwanted in ["sisu_rc_fault", "sisu_rc_check_exit"] {
        assert!(!ir.contains(unwanted), "{unwanted:?} in\n{ir}");
    }
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
fn u8_add_uses_unsigned_overflow() {
    let context = Context::create();
    let ir = compiled(
        &context,
        "fn f(a: u8, b: u8) -> u8 { a + b }\nfn main() { print(f(1, 2)) }",
    )
    .print_to_string()
    .to_string();
    for wanted in ["define i8 @sisu.f(i8", "@llvm.uadd.with.overflow.i8"] {
        assert!(ir.contains(wanted), "missing {wanted:?} in\n{ir}");
    }
}

#[test]
fn u8_compare_is_unsigned() {
    let context = Context::create();
    let ir = compiled(
        &context,
        "fn f(a: u8, b: u8) -> bool { a < b }\nfn main() { print(f(1, 2)) }",
    )
    .print_to_string()
    .to_string();
    assert!(ir.contains("icmp ult i8"), "missing `icmp ult i8` in\n{ir}");
}

/// A block is the 16-byte header, then the elements at their ABI size.
#[test]
fn literal_allocates_header_and_elements() {
    let context = Context::create();
    let ir = compiled(
        &context,
        "fn main() {\n    let a = [1, 2, 3]\n    let b = [true, false]\n    let c: [i64?] = [None, 1]\n}",
    )
    .print_to_string()
    .to_string();
    // 16 + 3 * 8, 16 + 2 * 1 and 16 + 2 * 16 bytes, aligned to the header's 8.
    for wanted in [
        "call ptr @sisu_alloc(i64 40, i64 8)",
        "call ptr @sisu_alloc(i64 18, i64 8)",
        "call ptr @sisu_alloc(i64 48, i64 8)",
    ] {
        assert!(ir.contains(wanted), "missing {wanted:?} in\n{ir}");
    }
}

/// Reading `xs[i]` from a local neither retains nor releases the array.
#[test]
fn fast_path_skips_base_count() {
    let context = Context::create();
    let ir = compiled(
        &context,
        "fn f(xs: [i64]) -> [i64] {\n    print(xs[0] + xs[1])\n    xs\n}\nfn main() {}",
    )
    .print_to_string()
    .to_string();
    let body = function_body(&ir, "define ptr @sisu.f(ptr");
    let (before, _) = body
        .split_once("call void @sisu_print_int(")
        .unwrap_or_else(|| panic!("no print in\n{body}"));
    for unwanted in ["@sisu_rc.retain(", "@\"sisu_rc.release.[i64]\"("] {
        assert!(
            !before.contains(unwanted),
            "{unwanted:?} before the print in\n{body}"
        );
    }
}

/// Only `drop.[[Node]]` asks for `release.[Node]`: the worklist must define it too.
#[test]
fn nested_array_helpers_are_defined() {
    let context = Context::create();
    let ir = compiled(
        &context,
        "class Node {}\nfn main() {\n    let g = [[Node()]]\n}",
    )
    .print_to_string()
    .to_string();
    for wanted in [
        "define void @\"sisu_rc.drop.[[Node]]\"(ptr",
        "define void @\"sisu_rc.release.[Node]\"(ptr",
    ] {
        assert!(ir.contains(wanted), "missing {wanted:?} in\n{ir}");
    }
}

/// A base that is not a local is released, after the element is loaded.
#[test]
fn general_path_releases_base() {
    let context = Context::create();
    let ir = compiled(
        &context,
        "fn make() -> [i64] { [1] }\nfn f() -> i64 { make()[0] }\nfn main() {}",
    )
    .print_to_string()
    .to_string();
    let body = function_body(&ir, "define i64 @sisu.f()");
    let lines: Vec<&str> = body.lines().map(str::trim).collect();
    // `f` returns the element it loaded.
    let element = lines
        .iter()
        .find_map(|l| l.strip_prefix("ret i64 "))
        .unwrap_or_else(|| panic!("no `ret i64` in\n{body}"));
    let load = lines
        .iter()
        .position(|l| l.starts_with(&format!("{element} = load i64, ptr ")))
        .unwrap_or_else(|| panic!("no load of {element} in\n{body}"));
    let release = lines
        .iter()
        .position(|l| l.starts_with("call void @\"sisu_rc.release.[i64]\"(ptr "))
        .unwrap_or_else(|| panic!("no release of the base in\n{body}"));
    assert!(load < release, "{body}");
}

/// An index built only from whitelisted nodes keeps the fast path; one with a call or a move
/// takes the general path, which releases the base before the element is printed. `o` and
/// `ys` are read again later in `field` and `length`, so those indexes read them without a
/// move; in `moved` the index's read of `ys` is its last use, which the whitelist excludes.
#[test]
fn fast_path_follows_the_index_whitelist() {
    let context = Context::create();
    let ir = compiled(
        &context,
        "class O {\n    n: i64\n}\nfn g() -> i64 { 0 }\n\
         fn unary(xs: [i64], i: i64) -> [i64] {\n    print(xs[-i + 1])\n    xs\n}\n\
         fn convert(xs: [i64], b: u8) -> [i64] {\n    print(xs[i64(b)])\n    xs\n}\n\
         fn field(xs: [i64], o: O) -> [i64] {\n    print(xs[o.n])\n    print(o.n)\n    xs\n}\n\
         fn length(xs: [i64], ys: [bool]) -> [i64] {\n    print(xs[ys.len()])\n    print(ys.len())\n    xs\n}\n\
         fn call(xs: [i64]) -> [i64] {\n    print(xs[1 + g()])\n    xs\n}\n\
         fn moved(xs: [i64], ys: [bool]) -> [i64] {\n    print(xs[ys.len()])\n    xs\n}\nfn main() {}",
    )
    .print_to_string()
    .to_string();
    for (name, fast) in [
        ("unary", true),
        ("convert", true),
        ("field", true),
        ("length", true),
        ("call", false),
        ("moved", false),
    ] {
        let body = function_body(&ir, &format!("define ptr @sisu.{name}("));
        let (before, _) = body
            .split_once("call void @sisu_print_int(")
            .unwrap_or_else(|| panic!("no print in\n{body}"));
        let released = before.contains("@\"sisu_rc.release.[i64]\"(");
        assert_eq!(released, !fast, "{name}:\n{body}");
    }
}

/// An element write checks the index before `unique` copies the array, so an
/// out-of-bounds write copies nothing, and stores the value into the unique array.
#[test]
fn write_checks_bounds_before_unique() {
    let context = Context::create();
    let ir = compiled(
        &context,
        "fn f(xs: [i64]) -> [i64] {\n    let mut ys = xs\n    ys[0] = 1\n    ys\n}\nfn main() {}",
    )
    .print_to_string()
    .to_string();
    let body = function_body(&ir, "@sisu.f(");
    let lines: Vec<&str> = body.lines().map(str::trim).collect();
    let find = |wanted: &dyn Fn(&str) -> bool, what: &str| {
        instruction(&lines, wanted)
            .unwrap_or_else(|| panic!("no {what} in\n{body}"))
            .0
    };
    // `check_bounds` panics when `index uge len`: `0 <= index < len` negated.
    let bounds = find(&|t| t.starts_with("icmp uge i64 0, "), "bounds check");
    let unique = find(
        &|t| t.starts_with("call ptr @\"sisu_rc.unique.[i64]\"(ptr "),
        "`unique` call",
    );
    let store = lines
        .iter()
        .position(|l| l.starts_with("store i64 1, ptr "))
        .unwrap_or_else(|| panic!("no store of 1 in\n{body}"));
    assert!([bounds, unique, store].is_sorted(), "{body}");
}

/// `Move` takes the local's reference and leaves `null` in its slot, which releasing at the
/// scope's end then skips.
#[test]
fn move_stores_null() {
    let context = Context::create();
    let ir = compiled(&context, "fn f(xs: [i64]) -> [i64] { xs }\nfn main() {}")
        .print_to_string()
        .to_string();
    let body = function_body(&ir, "@sisu.f(");
    let (param, _) = body
        .split_once(')')
        .unwrap_or_else(|| panic!("no parameter list in\n{body}"));
    let param = param.trim_start_matches("ptr ");
    // The slot is where the entry block stores the parameter.
    let entry_store = format!("store ptr {param}, ptr ");
    let slot = body
        .lines()
        .find_map(|l| l.trim().strip_prefix(entry_store.as_str()))
        .and_then(|rest| rest.split(',').next())
        .unwrap_or_else(|| panic!("no store of {param} in\n{body}"));
    assert!(
        body.contains(&format!("store ptr null, ptr {slot},")),
        "{body}"
    );
}

/// `.config/nextest.toml` kills this test after 30 s, matching it by name: a rename must
/// update the override there. Each `+=` is an overflow panic site, and finding its line and
/// column from the start of the text made codegen quadratic in the number of sites.
#[test]
fn many_panic_sites_compile_in_linear_time() {
    let src = format!(
        "fn main() {{\n    let mut x = 0\n{}    print(x)\n}}\n",
        "    x += 1\n".repeat(100_000)
    );
    compiled(&Context::create(), &src);
}
