#![expect(clippy::disallowed_methods, reason = "a build script has no runtime to block; it waits for qsb, lrelease and git")]

use cxx_qt_build::{CxxQtBuilder, QmlFile, QmlModule};
use qt_build_utils::QResourceFile;
use std::path::Path;

fn qml_files(dir: &Path, out: &mut Vec<String>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            qml_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "qml" || e == "js") {
            out.push(p.to_string_lossy().into_owned());
        }
    }
}

fn stale(src: &Path, dst: &Path) -> bool {
    let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    mtime(dst).is_none_or(|d| mtime(src).is_some_and(|s| s > d))
}

fn stamp(src: &Path, dst: &str) {
    let mtime = std::fs::metadata(src).and_then(|m| m.modified()).unwrap();
    std::fs::File::options().write(true).open(dst).and_then(|f| f.set_modified(mtime)).unwrap();
}

fn shaders() -> Vec<String> {
    let qsb = ["/usr/lib/qt6/bin/qsb", "/usr/lib/qt6/libexec/qsb"].into_iter().find(|p| Path::new(p).exists()).unwrap_or("qsb");
    let mut out = Vec::new();
    for entry in std::fs::read_dir("shaders").unwrap().flatten() {
        let src = entry.path();
        if !src.extension().is_some_and(|e| e == "frag" || e == "vert") {
            continue;
        }
        let dst = format!("{}.qsb", src.display());
        println!("cargo:rerun-if-changed={}", src.display());
        out.push(dst.clone());
        if !stale(&src, Path::new(&dst)) {
            continue;
        }
        let status = std::process::Command::new(qsb).args(["--qt6", "-o", &dst]).arg(&src).status().expect("qsb");
        assert!(status.success(), "qsb failed for {}", src.display());
        stamp(&src, &dst);
    }
    out
}

fn fonts() -> Vec<String> {
    println!("cargo:rerun-if-changed=fonts");
    let mut out: Vec<String> = std::fs::read_dir("fonts").unwrap().flatten().map(|e| e.path().to_string_lossy().into_owned()).collect();
    out.sort();
    out
}

fn translations() -> Vec<QResourceFile> {
    let home = std::env::var("HOME").unwrap_or_default();
    let local = format!("{home}/.local/bin/lrelease");
    let lrelease = ["/usr/lib/qt6/bin/lrelease", "/usr/lib/qt6/libexec/lrelease", local.as_str()]
        .into_iter()
        .find(|p| Path::new(p).exists())
        .unwrap_or("lrelease");
    let dir = std::env::var("OUT_DIR").unwrap();
    let mut out = Vec::new();
    for entry in std::fs::read_dir("i18n").unwrap().flatten() {
        let src = entry.path();
        let Some(lang) = src.file_stem().and_then(|s| s.to_str()).and_then(|s| s.strip_prefix("snellar_")) else {
            continue;
        };
        if src.extension().is_none_or(|e| e != "ts") {
            continue;
        }
        let dst = format!("{dir}/qml_{lang}.qm");
        println!("cargo:rerun-if-changed={}", src.display());
        out.push(QResourceFile::new(&dst).alias(format!("qml/i18n/qml_{lang}.qm")));
        if !stale(&src, Path::new(&dst)) {
            continue;
        }
        let status = std::process::Command::new(lrelease)
            .arg(&src)
            .args(["-qm", &dst])
            .status()
            .expect("lrelease not found, install qt6-l10n-tools");
        assert!(status.success(), "lrelease failed for {}", src.display());
        stamp(&src, &dst);
    }
    out
}

fn commit() -> String {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    if let Some(head) = git(&["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={head}");
    }
    if let Some(branch) =
        git(&["symbolic-ref", "-q", "HEAD"]).and_then(|r| git(&["rev-parse", "--git-path", &r])).filter(|p| Path::new(p).exists())
    {
        println!("cargo:rerun-if-changed={branch}");
    }
    git(&["rev-parse", "--short=9", "HEAD"]).unwrap_or_else(|| "unknown".into())
}

fn main() {
    println!("cargo:rustc-env=SNELLAR_COMMIT={}", commit());
    let mut files = Vec::new();
    qml_files(Path::new("qml"), &mut files);
    println!("cargo:rerun-if-changed=qml");

    let singletons = [
        "qml/Theme.qml",
        "qml/Api.qml",
        "qml/Errors.qml",
        "qml/Nav.qml",
        "qml/Tone.qml",
        "qml/Nudges.qml",
        "qml/Imports.qml",
        "qml/Words.qml",
    ];
    let module =
        QmlModule::new("Snellar").qml_files(files.iter().map(|f| QmlFile::from(f.as_str()).singleton(singletons.contains(&f.as_str()))));

    CxxQtBuilder::new_qml_module(module)
        .crate_include_root(None)
        .qt_module("Gui")
        .qt_module("Quick")
        .qt_module("QuickControls2")
        .qrc_resources(shaders().into_iter().chain(fonts()).map(QResourceFile::new).chain(translations()))
        .files(["src/bridge.rs"])
        .cpp_file("src/qtlog.cpp")
        .cpp_file("src/windowbar.cpp")
        .build();
}
