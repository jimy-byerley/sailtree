//! compiles the C++ snippets embedded with `cpp!` (the SailfishApp shim)

use std::process::Command;

/// flags reported by pkg-config for the given module
fn pkg_config(args: &[&str]) -> Vec<String> {
    let output = Command::new("pkg-config")
        .args(args)
        .output()
        .expect("pkg-config must be available");
    assert!(
        output.status.success(),
        "pkg-config {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("pkg-config output is utf8")
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

fn main() {
    let mut config = cpp_build::Config::new();
    config.flag_if_supported("-std=c++11");
    // Qt 5.6 headers are noisy with a recent gcc
    config.flag_if_supported("-Wno-deprecated-copy");

    // Qt location is exported by the qttypes build script
    // sailfishapp.h includes Qt headers without their module prefix
    if let Ok(path) = std::env::var("DEP_QT_INCLUDE_PATH") {
        config.include(&path);
        for module in ["QtCore", "QtGui", "QtQml", "QtQuick"] {
            config.include(format!("{path}/{module}"));
        }
    }

    for flag in pkg_config(&["--cflags", "sailfishapp"]) {
        if let Some(path) = flag.strip_prefix("-I") {
            config.include(path);
        } else if let Some(define) = flag.strip_prefix("-D") {
            let mut parts = define.splitn(2, '=');
            config.define(parts.next().unwrap(), parts.next());
        }
    }
    config.build("src/main.rs");

    for flag in pkg_config(&["--libs", "sailfishapp"]) {
        if let Some(path) = flag.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={path}");
        } else if let Some(lib) = flag.strip_prefix("-l") {
            println!("cargo:rustc-link-lib={lib}");
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}
