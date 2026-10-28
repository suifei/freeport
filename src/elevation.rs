//! One-shot UAC worker over an authenticated local named pipe, no credential storage.
use crate::{
    engine::{self, Control, Plan},
    handles::Options,
    inventory::Row,
    platform as p, scan, Result,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::File,
    os::windows::io::{AsRawHandle, FromRawHandle},
    path::{Path, PathBuf},
    ptr,
    sync::atomic::Ordering,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, *},
    Storage::FileSystem::*,
    System::{Pipes::*, Threading::*, IO::*},
    UI::{Shell::*, WindowsAndMessaging::SW_HIDE},
};
const MAX_FRAME: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Job {
    Plan {
        rows: Vec<Row>,
        selection: crate::selection::Selection,
        destination: PathBuf,
        store: PathBuf,
    },
    Scan {
        roots: Vec<PathBuf>,
        mode: String,
        store: PathBuf,
        measure: bool,
    },
    Migrate {
        plan: Plan,
        store: PathBuf,
        options: Options,
    },
    Restore {
        sources: Vec<PathBuf>,
        store: PathBuf,
        options: Options,
    },
    Recover {
        id: String,
        store: PathBuf,
        options: Options,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Update {
    Progress(String),
    Report(scan::Report),
    Measured(usize, Row),
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u32,
    language: String,
    home: Option<PathBuf>,
    job: Job,
}
#[derive(Serialize, Deserialize)]
enum Reply {
    Update(Update),
    Complete(Value),
    Failed(String),
}

pub fn is_elevated() -> Result<bool> {
    unsafe {
        let mut h = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut h) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let token = File::from_raw_handle(h);
        let mut e = TOKEN_ELEVATION::default();
        let mut n = 0;
        if GetTokenInformation(
            token.as_raw_handle(),
            TokenElevation,
            &mut e as *mut _ as _,
            std::mem::size_of_val(&e) as u32,
            &mut n,
        ) == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(e.TokenIsElevated != 0)
    }
}
pub fn needs_volume_access(roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| {
        root.parent().is_none() && {
            let s = root.to_string_lossy();
            s.len() >= 2
                && p::native_file(
                    Path::new(&format!(r"\\.\{}", &s[..2])),
                    0x80000000,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    0,
                )
                .err()
                .and_then(|e| {
                    e.downcast_ref::<std::io::Error>()
                        .and_then(|e| e.raw_os_error())
                })
                .is_some_and(|c| c == 5 || c == 1314)
        }
    })
}
pub fn perform(job: &Job, control: &Control, update: &dyn Fn(Update)) -> Result<Value> {
    let progress = |s| update(Update::Progress(s));
    match job {
        Job::Plan {
            rows,
            selection,
            destination,
            store,
        } => Ok(serde_json::to_value(crate::selection::plan(
            rows,
            selection,
            destination,
            store,
            control,
            &progress,
        )?)?),
        Job::Scan {
            roots,
            mode,
            store,
            measure,
        } => {
            let mut report = scan::run_with_store(roots, mode, &progress, control, store)?;
            update(Update::Report(report.clone()));
            if *measure && mode == "migrate" {
                for (i, row) in report.rows.iter_mut().enumerate() {
                    scan::measure_with_control(std::slice::from_mut(row), &progress, control)?;
                    update(Update::Measured(i, row.clone()));
                }
                report.rows.sort_by(|a, b| {
                    b.stats
                        .bytes
                        .cmp(&a.stats.bytes)
                        .then_with(|| p::key(&a.candidate.source).cmp(&p::key(&b.candidate.source)))
                });
            }
            Ok(serde_json::to_value(report)?)
        }
        Job::Migrate {
            plan,
            store,
            options,
        } => {
            let c = Control {
                options: options.clone(),
                ..control.clone()
            };
            Ok(serde_json::to_value(engine::migrate(
                plan, store, &c, &progress,
            )?)?)
        }
        Job::Restore {
            sources,
            store,
            options,
        } => {
            let c = Control {
                options: options.clone(),
                ..control.clone()
            };
            Ok(serde_json::to_value(engine::restore_many(
                sources, store, &c, &progress,
            )?)?)
        }
        Job::Recover { id, store, options } => {
            let c = Control {
                options: options.clone(),
                ..control.clone()
            };
            Ok(serde_json::to_value(engine::recover(
                store, id, &c, &progress,
            )?)?)
        }
    }
}
fn process(pid: u32) -> Result<File> {
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, pid) };
    if h.is_null() {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { File::from_raw_handle(h) })
}
fn created(f: &File) -> Result<u64> {
    let mut a = FILETIME::default();
    let mut b = a;
    let mut c = a;
    let mut d = a;
    if unsafe { GetProcessTimes(f.as_raw_handle(), &mut a, &mut b, &mut c, &mut d) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(((a.dwHighDateTime as u64) << 32) | a.dwLowDateTime as u64)
}
fn same_image(f: &File) -> Result<()> {
    let mut b = vec![0u16; 32768];
    let mut n = b.len() as u32;
    if unsafe { QueryFullProcessImageNameW(f.as_raw_handle(), 0, b.as_mut_ptr(), &mut n) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if p::identity(&PathBuf::from(String::from_utf16(&b[..n as usize])?))?
        != p::identity(&std::env::current_exe()?)?
    {
        return crate::fail(crate::i18n::t("elevation.peer"));
    }
    Ok(())
}
fn current_sid() -> Result<String> {
    unsafe {
        let mut h = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut h) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let token = File::from_raw_handle(h);
        let mut n = 0;
        GetTokenInformation(token.as_raw_handle(), TokenUser, ptr::null_mut(), 0, &mut n);
        if n == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut b = vec![0usize; (n as usize).div_ceil(std::mem::size_of::<usize>())];
        if GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            b.as_mut_ptr() as _,
            n,
            &mut n,
        ) == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let user = &*(b.as_ptr() as *const TOKEN_USER);
        let mut s = ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut s) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut len = 0;
        while *s.add(len) != 0 {
            len += 1;
        }
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(s, len));
        LocalFree(s as _);
        Ok(sid)
    }
}
fn event() -> Result<File> {
    let h = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
    if h.is_null() {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { File::from_raw_handle(h) })
}
fn finish_io(
    pipe: &File,
    ov: &mut OVERLAPPED,
    control: &Control,
    timeout: Option<Duration>,
) -> Result<u32> {
    let start = Instant::now();
    loop {
        let cancelled = control.check().err();
        let timedout = timeout.is_some_and(|t| start.elapsed() > t);
        if cancelled.is_some() || timedout {
            unsafe {
                CancelIoEx(pipe.as_raw_handle(), ov);
                WaitForSingleObject(ov.hEvent, INFINITE);
            }
            return Err(cancelled.unwrap_or_else(|| crate::i18n::t("elevation.timeout").into()));
        }
        match unsafe { WaitForSingleObject(ov.hEvent, 100) } {
            WAIT_OBJECT_0 => {
                let mut n = 0;
                if unsafe { GetOverlappedResult(pipe.as_raw_handle(), ov, &mut n, 0) } == 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                return Ok(n);
            }
            WAIT_TIMEOUT => {}
            _ => {
                let e = std::io::Error::last_os_error();
                let mut n = 0;
                unsafe {
                    CancelIoEx(pipe.as_raw_handle(), ov);
                    GetOverlappedResult(pipe.as_raw_handle(), ov, &mut n, 1);
                }
                return Err(e.into());
            }
        }
    }
}
fn bytes(pipe: &File, b: &mut [u8], write: bool, control: &Control) -> Result<()> {
    let mut at = 0;
    while at < b.len() {
        control.check()?;
        let ev = event()?;
        let mut ov = OVERLAPPED {
            hEvent: ev.as_raw_handle(),
            ..Default::default()
        };
        let mut n = 0;
        let count = (b.len() - at).min(65536) as u32;
        let ok = unsafe {
            if write {
                WriteFile(
                    pipe.as_raw_handle(),
                    b[at..].as_ptr(),
                    count,
                    &mut n,
                    &mut ov,
                )
            } else {
                ReadFile(
                    pipe.as_raw_handle(),
                    b[at..].as_mut_ptr(),
                    count,
                    &mut n,
                    &mut ov,
                )
            }
        };
        if ok == 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
                return Err(e.into());
            }
            n = finish_io(
                pipe,
                &mut ov,
                control,
                if write {
                    Some(Duration::from_secs(30))
                } else {
                    None
                },
            )?;
        }
        if n == 0 {
            return crate::fail(crate::i18n::t("elevation.disconnected"));
        }
        at += n as usize;
    }
    Ok(())
}
fn send<T: Serialize>(pipe: &File, value: &T, control: &Control) -> Result<()> {
    let mut data = serde_json::to_vec(value)?;
    if data.len() > MAX_FRAME {
        return crate::fail(crate::i18n::t("selection.limit"));
    }
    bytes(pipe, &mut (data.len() as u32).to_le_bytes(), true, control)?;
    bytes(pipe, &mut data, true, control)
}
fn receive<T: serde::de::DeserializeOwned>(pipe: &File, control: &Control) -> Result<T> {
    let mut head = [0u8; 4];
    bytes(pipe, &mut head, false, control)?;
    let n = u32::from_le_bytes(head) as usize;
    if n == 0 || n > MAX_FRAME {
        return crate::fail(crate::i18n::t("selection.limit"));
    }
    let mut data = vec![0; n];
    bytes(pipe, &mut data, false, control)?;
    Ok(serde_json::from_slice(&data)?)
}
pub fn execute(
    job: &Job,
    elevate: bool,
    control: &Control,
    update: &dyn Fn(Update),
) -> Result<Value> {
    if !elevate || is_elevated()? {
        return perform(job, control, update);
    }
    let current_executable = std::env::current_exe()?;
    let _image_guard = p::stable_read_guard(&current_executable)?;
    let parent = process(std::process::id())?;
    let birth = created(&parent)?;
    let name = format!(
        r"\\.\pipe\SaveMyCDrive-{}-{}-{}",
        std::process::id(),
        birth,
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    );
    let mut sd = ptr::null_mut();
    let sddl = format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{})", current_sid()?);
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            p::wide(sddl).as_ptr(),
            1,
            &mut sd,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd,
        bInheritHandle: 0,
    };
    let h = unsafe {
        CreateNamedPipeW(
            p::wide(&name).as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            65536,
            65536,
            0,
            &sa,
        )
    };
    let error = std::io::Error::last_os_error();
    unsafe {
        LocalFree(sd as _);
    }
    if h == INVALID_HANDLE_VALUE {
        return Err(error.into());
    }
    let pipe = unsafe { File::from_raw_handle(h) };
    let ev = event()?;
    let mut ov = OVERLAPPED {
        hEvent: ev.as_raw_handle(),
        ..Default::default()
    };
    let connected = unsafe { ConnectNamedPipe(h, &mut ov) };
    let pending = connected == 0
        && std::io::Error::last_os_error().raw_os_error() == Some(ERROR_IO_PENDING as i32);
    if connected == 0 && !pending {
        return Err(std::io::Error::last_os_error().into());
    }
    let executable = p::wide(&current_executable);
    let verb = p::wide("runas");
    let parameters = p::wide(format!(
        "--smcd-worker {} {} {}",
        name,
        std::process::id(),
        birth
    ));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: verb.as_ptr(),
        lpFile: executable.as_ptr(),
        lpParameters: parameters.as_ptr(),
        nShow: SW_HIDE,
        ..unsafe { std::mem::zeroed() }
    };
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        let e = std::io::Error::last_os_error();
        unsafe {
            CancelIoEx(h, &ov);
            WaitForSingleObject(ev.as_raw_handle(), INFINITE);
        }
        return crate::fail(format!("{} ({})", crate::i18n::t("elevation.denied"), e));
    }
    if info.hProcess.is_null() {
        unsafe {
            CancelIoEx(h, &ov);
            WaitForSingleObject(ev.as_raw_handle(), INFINITE);
        }
        return crate::fail(crate::i18n::t("elevation.peer"));
    }
    let child = unsafe { File::from_raw_handle(info.hProcess) };
    if pending {
        finish_io(&pipe, &mut ov, control, Some(Duration::from_secs(120)))?;
    }
    let mut pid = 0;
    if unsafe { GetNamedPipeClientProcessId(h, &mut pid) } == 0
        || pid != unsafe { GetProcessId(child.as_raw_handle()) }
    {
        return crate::fail(crate::i18n::t("elevation.peer"));
    }
    same_image(&child)?;
    send(
        &pipe,
        &Request {
            version: 1,
            language: crate::i18n::language().into(),
            home: std::env::var_os("USERPROFILE").map(PathBuf::from),
            job: job.clone(),
        },
        control,
    )?;
    let result = (|| loop {
        match receive::<Reply>(&pipe, control)? {
            Reply::Update(u) => update(u),
            Reply::Complete(value) => return Ok(value),
            Reply::Failed(e) => return crate::fail(e),
        }
    })();
    unsafe {
        DisconnectNamedPipe(h);
    }
    // Never kill a worker that may be committing a transaction. Its watchdog requests safe cancellation.
    if result.is_err() {
        unsafe {
            WaitForSingleObject(child.as_raw_handle(), 5000);
        }
    }
    result
}
fn worker(args: &[String]) -> Result<()> {
    if args.len() != 5 || !args[2].starts_with(r"\\.\pipe\SaveMyCDrive-") || args[2].len() > 200 {
        return crate::fail("Invalid worker request");
    }
    let expected: u32 = args[3].parse()?;
    let expected_birth: u64 = args[4].parse()?;
    let parent = process(expected)?;
    if created(&parent)? != expected_birth {
        return crate::fail("Worker peer mismatch");
    }
    same_image(&parent)?;
    let pipe = p::native_file(Path::new(&args[2]), 0xC0000000, 0, FILE_FLAG_OVERLAPPED)?;
    let mut pid = 0;
    if unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut pid) } == 0
        || pid != expected
    {
        return crate::fail("Worker server mismatch");
    }
    if unsafe { WaitForSingleObject(parent.as_raw_handle(), 0) } != WAIT_TIMEOUT
        || created(&process(pid)?)? != expected_birth
    {
        return crate::fail("Worker parent is no longer the authenticated process");
    }
    let control = Control::default();
    let request: Request = receive(&pipe, &control)?;
    if request.version != 1 || !is_elevated()? {
        return crate::fail("Worker privilege/protocol mismatch");
    }
    crate::i18n::set(&request.language)?;
    if let Some(home) = request.home {
        std::env::set_var("USERPROFILE", home);
    }
    let watch = pipe.try_clone()?;
    let watch_control = control.clone();
    std::thread::spawn(move || loop {
        let mut available = 0;
        if unsafe {
            PeekNamedPipe(
                watch.as_raw_handle(),
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                &mut available,
                ptr::null_mut(),
            )
        } == 0
            || unsafe { WaitForSingleObject(parent.as_raw_handle(), 0) } == WAIT_OBJECT_0
        {
            watch_control.cancel.store(true, Ordering::Relaxed);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    });
    let transport = Control::default();
    let update = |u| {
        if send(&pipe, &Reply::Update(u), &transport).is_err() {
            control.cancel.store(true, Ordering::Relaxed);
        }
    };
    let result = perform(&request.job, &control, &update);
    let reply = match result {
        Ok(v) => Reply::Complete(v),
        Err(e) => Reply::Failed(crate::i18n::describe_error(e.as_ref())),
    };
    send(&pipe, &reply, &transport)
}
pub fn maybe_worker() -> bool {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_none_or(|a| a != "--smcd-worker") {
        return false;
    }
    let code = if worker(&args).is_ok() { 0 } else { 1 };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pair() -> (File, File) {
        let name = format!(
            r"\\.\pipe\SaveMyCDrive-unit-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let h = unsafe {
            CreateNamedPipeW(
                p::wide(&name).as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                65536,
                65536,
                0,
                ptr::null(),
            )
        };
        assert_ne!(h, INVALID_HANDLE_VALUE);
        let server = unsafe { File::from_raw_handle(h) };
        let client = p::native_file(Path::new(&name), 0xC0000000, 0, FILE_FLAG_OVERLAPPED).unwrap();
        let ev = event().unwrap();
        let mut ov = OVERLAPPED {
            hEvent: ev.as_raw_handle(),
            ..Default::default()
        };
        let ok = unsafe { ConnectNamedPipe(h, &mut ov) };
        assert!(
            ok != 0
                || std::io::Error::last_os_error().raw_os_error()
                    == Some(ERROR_PIPE_CONNECTED as i32)
        );
        (server, client)
    }
    #[test]
    fn local_pipe_round_trip_and_peer_pid() {
        let (server, client) = pair();
        let pid = std::process::id();
        let mut actual = 0;
        assert_ne!(
            unsafe { GetNamedPipeClientProcessId(server.as_raw_handle(), &mut actual) },
            0
        );
        assert_eq!(pid, actual);
        let thread = std::thread::spawn(move || {
            send(
                &client,
                &Reply::Complete(serde_json::json!({"path":"日本語/目录","value":7})),
                &Control::default(),
            )
            .unwrap()
        });
        let message: Reply = receive(&server, &Control::default()).unwrap();
        thread.join().unwrap();
        assert!(matches!(message,Reply::Complete(v) if v["value"]==7));
    }
    #[test]
    fn oversized_frames_are_rejected_before_allocation() {
        let (server, client) = pair();
        let thread = std::thread::spawn(move || {
            bytes(
                &client,
                &mut ((MAX_FRAME + 1) as u32).to_le_bytes(),
                true,
                &Control::default(),
            )
            .unwrap()
        });
        assert!(receive::<Reply>(&server, &Control::default()).is_err());
        thread.join().unwrap();
    }
    #[test]
    fn waiting_pipe_read_is_cancelled_without_killing_worker() {
        let (server, _client) = pair();
        let control = Control::default();
        let other = control.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            other.cancel.store(true, Ordering::Relaxed);
        });
        let start = Instant::now();
        assert!(receive::<Reply>(&server, &control).is_err());
        assert!(start.elapsed() < Duration::from_secs(3));
        thread.join().unwrap();
    }
    #[test]
    fn malformed_worker_identity_is_rejected() {
        assert!(worker(&[
            "smcd.exe".into(),
            "--smcd-worker".into(),
            r"\\server\pipe\other".into(),
            "0".into(),
            "0".into()
        ])
        .is_err());
        assert!(worker(&[
            "smcd.exe".into(),
            "--smcd-worker".into(),
            r"\\.\pipe\SaveMyCDrive-unit".into(),
            "0".into(),
            "0".into()
        ])
        .is_err());
    }
}
