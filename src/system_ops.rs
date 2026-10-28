//! Thin adapters for Windows' own robocopy and cmd built-ins.
//! FreePort supplies validation/journaling; these tools do the copying/linking.
use crate::{engine::Control, fail, platform as p, Result};
use std::{
    ffi::OsString,
    io::Read,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
    thread,
    time::Duration,
};

#[derive(Clone, Copy)]
pub enum CopyMode {
    Prepare,
    FinishMove,
    FillMissing,
}

fn system_tool(name: &str) -> Result<PathBuf> {
    let mut buffer = vec![0u16; 32768];
    #[link(name = "kernel32")]
    extern "system" {
        fn GetSystemDirectoryW(buffer: *mut u16, length: u32) -> u32;
    }
    let n = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
    if n == 0 || n as usize >= buffer.len() {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(PathBuf::from(String::from_utf16(&buffer[..n as usize])?).join(name))
}
fn capture(mut input: impl Read) -> Vec<u8> {
    let mut output = vec![];
    let mut buffer = [0; 8192];
    while let Ok(n) = input.read(&mut buffer) {
        if n == 0 {
            break;
        }
        let count = n.min((64 * 1024usize).saturating_sub(output.len()));
        output.extend_from_slice(&buffer[..count]);
    }
    output
}
fn run(mut command: Command, robocopy: bool, control: &Control) -> Result<()> {
    control.check()?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x08000000)
        .spawn()?;
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let out = thread::spawn(move || capture(out));
    let err = thread::spawn(move || capture(err));
    let status = loop {
        if control.check().is_err() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = out.join();
            let _ = err.join();
            return fail(crate::i18n::t("msg.0c626b7ea7"));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = out.join();
                let _ = err.join();
                return Err(error.into());
            }
        }
    };
    let output = out.join().unwrap_or_default();
    let errors = err.join().unwrap_or_default();
    let code = status.code().unwrap_or(-1);
    if (robocopy && (0..8).contains(&code)) || (!robocopy && code == 0) {
        return Ok(());
    }
    fail(crate::i18n::format(
        "system.command_failed",
        &[
            if robocopy {
                "robocopy".into()
            } else {
                "cmd".into()
            },
            code.to_string(),
            format!(
                "{} {}",
                decode(&output, !robocopy),
                decode(&errors, !robocopy)
            )
            .trim()
            .to_string(),
        ],
    ))
}
fn decode(bytes: &[u8], force_wide: bool) -> String {
    // Some robocopy builds emit a Unicode BOM followed by OEM error lines,
    // despite /UNICODE. cmd /U built-ins reliably use UTF-16LE.
    let bytes = bytes.strip_prefix(&[0xff, 0xfe]).unwrap_or(bytes);
    if force_wide || bytes.iter().filter(|b| **b == 0).count() > bytes.len() / 8 {
        return String::from_utf16_lossy(
            &bytes
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>(),
        );
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn MultiByteToWideChar(
            cp: u32,
            flags: u32,
            input: *const u8,
            len: i32,
            output: *mut u16,
            capacity: i32,
        ) -> i32;
    }
    let mut output = vec![0u16; bytes.len()];
    let count = unsafe {
        MultiByteToWideChar(
            1,
            0,
            bytes.as_ptr(),
            bytes.len() as i32,
            output.as_mut_ptr(),
            output.len() as i32,
        )
    };
    if count > 0 {
        String::from_utf16_lossy(&output[..count as usize])
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}
fn advertises_modified_files(help: &[u8]) -> bool {
    // The ASCII switch spelling is stable in localized OEM and Unicode help.
    let text = String::from_utf8_lossy(help).to_ascii_uppercase();
    let wide = String::from_utf16_lossy(
        &help
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect::<Vec<_>>(),
    )
    .to_ascii_uppercase();
    [text.as_str(), wide.as_str()]
        .iter()
        .any(|text| text.split_whitespace().any(|word| word == "/IM"))
}
fn supports_modified_files() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        let Ok(tool) = system_tool("robocopy.exe") else {
            return false;
        };
        Command::new(tool)
            .arg("/?")
            .stdin(Stdio::null())
            .creation_flags(0x08000000)
            .output()
            .is_ok_and(|output| advertises_modified_files(&output.stdout))
    })
}
fn copy_args(mode: CopyMode, modified_files: bool) -> Vec<&'static str> {
    let mut args = vec![
        "/E",
        "/COPY:DAT",
        "/R:0",
        "/W:0",
        "/XJ",
        "/SL",
        "/NFL",
        "/NDL",
        "/NJH",
        "/NJS",
        "/NP",
        "/UNICODE",
        // Measured on 54k small files: 28 s single-threaded, 7.6 s at 8, slower at 32.
        "/MT:8",
    ];
    match mode {
        CopyMode::Prepare => {}
        // Data was already copied, hashed and flushed. /IS includes equal files;
        // /NOCOPY leaves the verified destination untouched while /MOVE retires
        // their source names. Unknown/changed source files stay for inspection.
        CopyMode::FinishMove => {
            args.extend(["/MOVE", "/IS", "/NOCOPY", "/XL", "/XC", "/XN", "/XO"]);
            if modified_files {
                args.push("/IM");
            }
        }
        CopyMode::FillMissing => args.extend(["/XC", "/XN", "/XO"]),
    }
    args
}
pub fn robocopy(source: &Path, target: &Path, mode: CopyMode, control: &Control) -> Result<()> {
    let mut command = Command::new(system_tool("robocopy.exe")?);
    command.arg(source).arg(target).args(copy_args(
        mode,
        matches!(mode, CopyMode::FinishMove) && supports_modified_files(),
    ));
    run(command, true, control)
}
fn builtin_line(operation: &str, paths: &[&Path]) -> Result<OsString> {
    let mut line = OsString::from("\"");
    line.push(operation);
    for path in paths {
        let value = path.to_string_lossy();
        if !path.is_absolute() || value.contains(['\0', '\r', '\n', '"', '%']) {
            return fail(crate::i18n::t("system.path"));
        }
        line.push(" \"");
        line.push(path);
        line.push("\"");
    }
    line.push("\"");
    Ok(line)
}
fn builtin(operation: &str, paths: &[&Path], control: &Control) -> Result<()> {
    let mut command = Command::new(system_tool("cmd.exe")?);
    command
        .args(["/d", "/v:off", "/u", "/s", "/c"])
        .raw_arg(builtin_line(operation, paths)?);
    run(command, false, control)
}
pub fn junction(link: &Path, target: &Path, control: &Control) -> Result<()> {
    builtin("mklink /J", &[link, target], control)
}
pub fn validate_junction_paths(link: &Path, target: &Path) -> Result<()> {
    builtin_line("mklink /J", &[link, target]).map(|_| ())
}
pub fn hardlink(link: &Path, target: &Path, control: &Control) -> Result<()> {
    builtin("mklink /H", &[link, target], control)
}
pub fn remove_empty(path: &Path, control: &Control) -> Result<()> {
    // Deliberately no /S: a new/residual file must make RD fail, never be erased.
    if p::reparse(&std::fs::symlink_metadata(path)?) || std::fs::read_dir(path)?.next().is_some() {
        return fail(crate::i18n::t("system.residue"));
    }
    control.check()?;
    // Standard-library RemoveDirectory is the non-recursive RD operation. It
    // atomically refuses residual files, without starting a shell per folder.
    std::fs::remove_dir(path)?;
    if std::fs::symlink_metadata(path).is_ok() {
        return fail(crate::i18n::t("system.residue"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parameters_keep_batch_semantics() {
        let prepare = copy_args(CopyMode::Prepare, false);
        let finish = copy_args(CopyMode::FinishMove, false);
        assert!(prepare.contains(&"/E") && !prepare.contains(&"/MOVE"));
        for flag in ["/E", "/MOVE", "/IS", "/NOCOPY", "/XL", "/XC", "/XN", "/XO"] {
            assert!(finish.contains(&flag));
        }
        assert!(!finish.contains(&"/IM"));
        assert!(copy_args(CopyMode::FinishMove, true).contains(&"/IM"));
        for args in [prepare, finish, copy_args(CopyMode::FillMissing, true)] {
            assert!(args.contains(&"/MT:8"));
            for forbidden in ["/COPYALL", "/SEC", "/MIR", "/PURGE", "/B", "/ZB"] {
                assert!(!args.contains(&forbidden));
            }
        }
    }
    #[test]
    fn help_capability_checks_support_legacy_and_unicode() {
        assert!(!advertises_modified_files(
            b"/IS :: same files\n/IMAGINARY :: unknown"
        ));
        assert!(advertises_modified_files(
            b"/IS :: same files\n/IM :: modified files"
        ));
        let wide: Vec<u8> = "/IM :: modified files"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert!(advertises_modified_files(&wide));
    }
    #[test]
    fn system_output_decodes_unicode_and_oem_ascii() {
        assert_eq!(decode(b"\xff\xfeERROR 5", false), "ERROR 5");
        let wide: Vec<u8> = "ERROR 5: 系统错误"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(decode(&wide, false), "ERROR 5: 系统错误");
        assert_eq!(decode(&wide, true), "ERROR 5: 系统错误");
    }
    #[test]
    fn cmd_paths_are_quoted_and_expansion_is_rejected() {
        for i in 0..5000 {
            let a = PathBuf::from(format!(r"E:\测试 & (data) ! {i}\source"));
            let b = PathBuf::from(format!(r"D:\Dev & (目标) ! {i}"));
            let line = builtin_line("mklink /J", &[&a, &b])
                .unwrap()
                .to_string_lossy()
                .into_owned();
            assert_eq!(
                line,
                format!("\"mklink /J \"{}\" \"{}\"\"", a.display(), b.display())
            );
        }
        for bad in [
            "relative",
            r"E:\%PATH%",
            "E:\\bad\" & injected",
            "E:\\bad\ncmd",
        ] {
            assert!(builtin_line("mklink /J", &[Path::new(bad), Path::new(r"D:\ok")]).is_err());
        }
    }
    #[test]
    fn empty_rd_refuses_files() {
        let path = std::env::temp_dir().join(format!("fp-rd-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("keep"), b"retain").unwrap();
        assert!(remove_empty(&path, &Control::default()).is_err());
        assert_eq!(std::fs::read(path.join("keep")).unwrap(), b"retain");
        std::fs::remove_file(path.join("keep")).unwrap();
        std::fs::remove_dir(path).unwrap();
    }
}
