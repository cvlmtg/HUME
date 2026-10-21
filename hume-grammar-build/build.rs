fn main() {
    // `cc` reads the target triple from cargo's build-script environment,
    // which a running binary does not have.
    let target = std::env::var("TARGET").expect("cargo sets TARGET for build scripts");
    println!("cargo:rustc-env=HUME_GRAMMAR_BUILD_TARGET={target}");
}
