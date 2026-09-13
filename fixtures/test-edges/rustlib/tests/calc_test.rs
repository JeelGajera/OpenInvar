// A crate's `tests/` directory. Not `#[cfg(test)] mod tests`, which is the
// more common Rust convention — the symbols inside such a module are not
// indexed at all today, so naming it a test file would promise contents
// OpenInvar cannot see.
use rustlib::Calculator;

#[test]
fn adds() {
    let calc = Calculator;
    assert_eq!(calc.add(1, 2), 3);
}
