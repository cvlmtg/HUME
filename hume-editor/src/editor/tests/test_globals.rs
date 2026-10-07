//! Regression tests for the locks over process `PATH`: `EnvClaim` is
//! exclusive, `PathReader` is shared. A nested claim, or a claim and a reader
//! on one thread, must panic with a clear message. A hang is the failure
//! these types exist to rule out, so no test here asserts against a timeout.

use super::*;

#[test]
#[should_panic(expected = "already holds an `EnvClaim`")]
fn claiming_twice_on_one_thread_panics_instead_of_hanging() {
    let _outer = claim_env();
    let _inner = claim_env();
}

#[test]
#[should_panic(expected = "holds a `PathReader` and claims `PATH`")]
fn claiming_while_holding_a_reader_panics_instead_of_hanging() {
    let _reader = path_reader();
    let _claim = claim_env();
}

#[test]
#[should_panic(expected = "holds an `EnvClaim` and builds a `Dirs` fixture")]
fn building_a_reader_while_holding_a_claim_panics_instead_of_hanging() {
    let _claim = claim_env();
    let _reader = path_reader();
}

#[test]
fn nested_path_readers_on_one_thread_succeed() {
    let _outer = path_reader();
    let _inner = path_reader();
}

#[test]
fn a_claim_can_be_retaken_after_it_drops() {
    drop(claim_env());
    let _again = claim_env();
}
