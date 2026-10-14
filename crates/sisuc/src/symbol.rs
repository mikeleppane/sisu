//! Interned names. A `Symbol` stands for a name's text, so that copying, comparing and hashing
//! a name costs what an integer does, and each distinct name is allocated once.

use rustc_hash::FxHashMap;
use std::fmt;
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

/// A name, interned: two symbols are equal exactly when their texts are.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Symbol(u32);

#[derive(Default)]
struct Interner {
    ids: FxHashMap<&'static str, Symbol>,
    names: Vec<&'static str>,
}

/// Every name interned in this process. Each text is leaked so that `as_str` can lend it for
/// the rest of the process; a run interns each distinct name of its source once.
static INTERNER: LazyLock<Mutex<Interner>> = LazyLock::new(Mutex::default);

fn interner() -> MutexGuard<'static, Interner> {
    // `intern` changes the tables only after its last fallible step, so a poisoned lock still
    // guards consistent tables.
    INTERNER.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Symbol {
    pub(crate) fn intern(name: &str) -> Symbol {
        let mut interner = interner();
        if let Some(&symbol) = interner.ids.get(name) {
            return symbol;
        }
        let symbol = Symbol(
            u32::try_from(interner.names.len())
                .expect("a process should intern fewer than 2^32 names"),
        );
        let name: &'static str = Box::leak(name.into());
        interner.names.push(name);
        interner.ids.insert(name, symbol);
        symbol
    }

    pub(crate) fn as_str(self) -> &'static str {
        interner().names[self.0 as usize]
    }
}

/// As the name's text, so a symbol debugs as the `String` it replaces.
impl fmt::Debug for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_texts_share_a_symbol() {
        let a = Symbol::intern("symbol_test_a");
        let b = Symbol::intern("symbol_test_b");
        assert_eq!(a, Symbol::intern("symbol_test_a"));
        assert_ne!(a, b);
        assert_eq!((a.as_str(), b.as_str()), ("symbol_test_a", "symbol_test_b"));
        assert_eq!(format!("{a} {b:?}"), "symbol_test_a \"symbol_test_b\"");
    }
}
