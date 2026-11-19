use std::path::Path;

fn main() {
    let src = Path::new("../../packaging/update-key.pub");
    println!("cargo:rerun-if-changed=../../packaging");
    let key = std::fs::read_to_string(src).unwrap_or_default();
    assert!(key.is_empty() || key.starts_with("-----BEGIN PUBLIC KEY-----"), "{} is not a PEM public key", src.display());
    let dst = Path::new(&std::env::var("OUT_DIR").unwrap()).join("update-key.pub");
    if std::fs::read_to_string(&dst).ok().as_deref() != Some(key.as_str()) {
        std::fs::write(dst, key).unwrap();
    }
}
