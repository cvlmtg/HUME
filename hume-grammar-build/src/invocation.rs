use std::ffi::OsString;
use std::path::Path;

use crate::toolchain::Flavor;
use crate::{Scanner, Sources};

/// The argument lists, in run order, that build `sources` into `library`.
/// Each runs in `work` after the compiler's own arguments.
///
/// A C++ scanner is compiled to an object of its own first, since one driver
/// invocation cannot compile both languages. The link then goes through the
/// C++ driver (see `Compiler::find`), which links the C++ runtime.
pub(crate) fn steps(
    flavor: Flavor,
    sources: &Sources,
    work: &Path,
    library: &Path,
) -> Vec<Vec<OsString>> {
    match flavor {
        Flavor::Gnu => gnu(sources, work, library),
        Flavor::Msvc => msvc(sources, work, library),
    }
}

fn gnu(sources: &Sources, work: &Path, library: &Path) -> Vec<Vec<OsString>> {
    let mut steps = Vec::new();
    let mut link: Vec<OsString> = vec![
        "-shared".into(),
        "-fno-exceptions".into(),
        "-I".into(),
        (&sources.dir).into(),
        "-o".into(),
        library.into(),
    ];
    if let Some(Scanner::Cpp(scanner)) = &sources.scanner {
        let object = work.join("scanner.o");
        steps.push(vec![
            "-fno-exceptions".into(),
            "-I".into(),
            (&sources.dir).into(),
            "-std=c++14".into(),
            "-c".into(),
            scanner.into(),
            "-o".into(),
            (&object).into(),
        ]);
        // An object listed after `-xc` would be read as C source.
        link.push(object.into());
    }
    link.extend(["-xc".into(), "-std=c11".into()]);
    if let Some(Scanner::C(scanner)) = &sources.scanner {
        link.push(scanner.into());
    }
    link.push((&sources.parser).into());
    if cfg!(target_os = "linux") {
        link.push("-Wl,-z,relro,-z,now".into());
    }
    steps.push(link);
    steps
}

/// `cl` writes each object, and the import library and export file beside
/// the DLL, into the directory it runs in, which is `work`.
fn msvc(sources: &Sources, work: &Path, library: &Path) -> Vec<Vec<OsString>> {
    let mut steps = Vec::new();
    let mut link: Vec<OsString> = vec![
        "/nologo".into(),
        "/LD".into(),
        "/utf-8".into(),
        "/I".into(),
        (&sources.dir).into(),
        "/std:c11".into(),
    ];
    match &sources.scanner {
        Some(Scanner::Cpp(scanner)) => {
            let object = work.join("scanner.obj");
            let mut fo = OsString::from("/Fo");
            fo.push(&object);
            steps.push(vec![
                "/nologo".into(),
                "/utf-8".into(),
                "/I".into(),
                (&sources.dir).into(),
                "/std:c++14".into(),
                "/c".into(),
                scanner.into(),
                fo,
            ]);
            link.push(object.into());
        }
        Some(Scanner::C(scanner)) => link.push(scanner.into()),
        None => {}
    }
    link.push((&sources.parser).into());
    let mut out = OsString::from("/out:");
    out.push(library);
    link.extend(["/link".into(), out]);
    steps.push(link);
    steps
}

#[cfg(test)]
mod tests;
