use super::*;
use steel::steel_vm::engine::Engine;

/// One full collection frees every cell no root reaches; the allocations
/// after it sweep the cursor past every freed slot, so a dead cell is
/// overwritten with `"junk"`.
const CHURN: &str = r#"
    (#%gc-collect)
    (let loop ((i 0)) (when (< i 200000) (box "junk") (loop (+ i 1))))
"#;

fn last_value(engine: &mut Engine, source: &str) -> SteelVal {
    engine
        .compile_and_run_raw_program(source.to_owned())
        .expect("the source evaluates")
        .pop()
        .expect("the source produces a value")
}

#[test]
fn a_rooted_closure_keeps_its_captured_box_across_a_collection() {
    let mut engine = Engine::new();
    let closure = Rooted::new(last_value(
        &mut engine,
        r#"(let ((cell (box "alive"))) (lambda () (unbox cell)))"#,
    ));

    engine
        .compile_and_run_raw_program(CHURN.to_owned())
        .expect("the churn evaluates");
    let read = engine.call_function_with_args(closure.value().clone(), vec![]);

    assert!(
        matches!(&read, Ok(SteelVal::StringV(s)) if s.as_str() == "alive"),
        "got {read:?}"
    );
}

#[test]
fn only_values_that_can_reach_the_heap_are_rooted() {
    let mut engine = Engine::new();
    let atoms = [
        SteelVal::BoolV(true),
        SteelVal::IntV(1),
        SteelVal::NumV(1.5),
        SteelVal::CharV('x'),
        SteelVal::Void,
        SteelVal::StringV("s".into()),
        SteelVal::SymbolV("s".into()),
    ];
    for atom in atoms {
        assert!(!Rooted::new(atom.clone()).is_rooted(), "{atom:?}");
    }
    for source in ["(list 1 2)", "(hash 'a 1)", "(lambda () 1)", "(box 1)"] {
        let value = last_value(&mut engine, source);
        assert!(Rooted::new(value).is_rooted(), "{source}");
    }
}
