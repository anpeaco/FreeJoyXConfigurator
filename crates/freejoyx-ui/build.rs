use std::process::Command;

fn main() {
    slint_build::compile("ui/app.slint").unwrap();
    emit_build_rev();
}

fn emit_build_rev() {
    let rev = Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=FREEJOYX_BUILD_REV={rev}");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
}
