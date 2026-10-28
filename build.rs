use std::{env, fs, path::PathBuf, process::Command};
fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    // Keep the reviewed JSON readable in Git; omit insignificant whitespace from
    // embedded data. No runtime decoder, compression library or extra DLL.
    for source in [
        "rules/freeport-rules.json",
        "locales/en.json",
        "locales/zh-CN.json",
        "locales/zh-TW.json",
        "locales/ja.json",
        "locales/ko.json",
        "locales/de.json",
    ] {
        println!("cargo:rerun-if-changed={source}");
        let input = fs::read(source).unwrap();
        let (mut quoted, mut escaped) = (false, false);
        let mut compact = Vec::with_capacity(input.len());
        for byte in input {
            if quoted {
                compact.push(byte);
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    quoted = false;
                }
            } else if byte == b'"' {
                quoted = true;
                compact.push(byte);
            } else if !byte.is_ascii_whitespace() {
                compact.push(byte);
            }
        }
        fs::write(
            out.join(PathBuf::from(source).file_name().unwrap()),
            compact,
        )
        .unwrap();
    }
    println!("cargo:rerun-if-changed=resources/freeport.manifest");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest = env::current_dir()
        .unwrap()
        .join("resources/freeport.manifest");
    let rc = out.join("freeport.rc");
    fs::write(
        &rc,
        format!(
            "1 24 \"{}\"\n",
            manifest.to_string_lossy().replace('\\', "/")
        ),
    )
    .unwrap();
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
        let obj = out.join("freeport-resource.o");
        let result = Command::new("windres")
            .args(["-i"])
            .arg(&rc)
            .args(["-O", "coff", "-o"])
            .arg(&obj)
            .status()
            .expect("windres must be installed for the native application manifest");
        assert!(result.success(), "windres failed");
        println!("cargo:rustc-link-arg-bins={}", obj.display());
    } else {
        let res = out.join("freeport.res");
        let result = Command::new("rc.exe")
            .arg("/nologo")
            .arg(format!("/fo{}", res.display()))
            .arg(&rc)
            .status()
            .expect("Windows SDK rc.exe must be available");
        assert!(result.success(), "resource compiler failed");
        println!("cargo:rustc-link-arg-bins={}", res.display());
    }
}
