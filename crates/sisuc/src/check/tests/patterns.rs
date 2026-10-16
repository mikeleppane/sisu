//! `_` patterns: `let _ = e` and `for _ in ...` bind no name.

use super::*;

#[test]
fn let_underscore_drops_at_once() {
    let printed =
        lowered("fn make() -> [i64] { [1] }\nfn main() {\n    let _ = make()\n    print(1)\n}");
    assert!(printed.contains("(call make) (print 1)"), "{printed}");
    assert!(!printed.contains("(let"), "{printed}");
}

/// A `let` has no value, whatever it lowers to.
#[test]
fn let_underscore_ends_a_block_without_a_value() {
    clean("fn make() -> [i64] { [1] }\nfn main() {\n    let _ = make()\n}");
    clean(
        "fn make() -> [i64] { [1] }\nfn f() -> i64 {\n    let _ = make()\n    1\n}\nfn main() {}",
    );
    codes(
        "fn make() -> [i64] { [1] }\nfn f() -> [i64] {\n    let _ = make()\n}\nfn main() {}",
        &[(Code::BodyHasNoValue, (4, 1))],
    );
}

#[test]
fn let_underscore_checks_its_type_and_value() {
    clean("fn main() {\n    let _: i64 = 1\n}");
    codes(
        "fn main() {\n    let _: bool = 1\n}",
        &[(Code::Mismatch, (2, 19))],
    );
    codes(
        "fn main() {\n    let _ = nope\n}",
        &[(Code::BindingNotFound, (2, 13))],
    );
}

#[test]
fn let_underscore_declares_no_name() {
    codes(
        "fn main() {\n    let _ = 1\n    let _ = 2\n    print(1)\n}",
        &[],
    );
}

#[test]
fn if_let_and_while_let_take_underscore() {
    clean("fn f(n: i64?) {\n    if let _ = n {}\n    while let _ = n {}\n}\nfn main() {}");
    codes(
        "fn main() {\n    if let _ = 5 {}\n}",
        &[(Code::UnwrapNeedsOptional, (2, 16))],
    );
}
