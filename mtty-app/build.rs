fn main() {
    println!("cargo:rerun-if-changed=resources.rc");
    println!("cargo:rerun-if-changed=../assets/icons/mtty.ico");
    #[cfg(windows)]
    embed_icon();
}

#[cfg(windows)]
fn embed_icon() {
    use std::{env, path::PathBuf, process::Command};

    let target = env::var("TARGET").expect("Cargo target");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    let resource = out.join("mtty-icon.res");
    let mut compiler = if target.ends_with("-msvc") {
        let mut command =
            find_msvc_tools::find(&target, "rc.exe").unwrap_or_else(|| Command::new("rc.exe"));
        command.arg("/nologo").arg("/fo").arg(&resource);
        command
    } else {
        let mut command = Command::new("windres");
        command.arg("-O").arg("coff").arg("-o").arg(&resource);
        command
    };
    let status = compiler
        .current_dir(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"))
        .arg("resources.rc")
        .status()
        .expect("Windows resource compiler must be installed");
    assert!(status.success(), "Windows icon resource compilation failed");
    println!("cargo:rustc-link-arg-bin=mtty={}", resource.display());
}
