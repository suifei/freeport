//! Native modal dialogs with application-owned translations, including their buttons.
use freeport::{i18n as l, platform as p};
use std::{fs, path::PathBuf, ptr};
use windows_sys::Win32::{
    Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
};
const DWLP_USER: i32 = (2 * std::mem::size_of::<isize>()) as i32;
fn template(width: i16, height: i16) -> Vec<u32> {
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_MODALFRAME as u32 | DS_CENTER as u32;
    let mut words = vec![
        style as u16,
        (style >> 16) as u16,
        0,
        0,
        0,
        0,
        0,
        width as u16,
        height as u16,
        0,
        0,
    ];
    words.extend(p::wide("FreePort"));
    let mut bytes = Vec::new();
    for word in words {
        bytes.extend(word.to_le_bytes());
    }
    while bytes.len() % 4 != 0 {
        bytes.push(0);
    }
    bytes
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
unsafe fn control(
    owner: HWND,
    id: usize,
    class: &str,
    value: &str,
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> HWND {
    let c = CreateWindowExW(
        0,
        p::wide(class).as_ptr(),
        p::wide(value).as_ptr(),
        WS_CHILD | WS_VISIBLE | style,
        x,
        y,
        w,
        h,
        owner,
        id as _,
        GetModuleHandleW(ptr::null()),
        ptr::null(),
    );
    SendMessageW(c, WM_SETFONT, GetStockObject(DEFAULT_GUI_FONT) as usize, 1);
    c
}
unsafe fn set(h: HWND, s: &str) {
    SetWindowTextW(h, p::wide(s).as_ptr());
}
unsafe fn value(h: HWND) -> String {
    let len = GetWindowTextLengthW(h);
    let mut b = vec![0u16; len as usize + 1];
    let n = GetWindowTextW(h, b.as_mut_ptr(), b.len() as i32);
    String::from_utf16_lossy(&b[..n as usize])
}
struct Message {
    text: String,
    flags: u32,
}
unsafe extern "system" fn message_proc(hwnd: HWND, m: u32, w: usize, param: isize) -> isize {
    if m == WM_INITDIALOG {
        SetWindowLongPtrW(hwnd, DWLP_USER, param);
        let d = &*(param as *const Message);
        let title = if d.flags & 0xf0 == MB_ICONERROR {
            l::t("common.error")
        } else if d.flags & 0xf0 == MB_ICONWARNING {
            l::t("common.warning")
        } else {
            l::t("common.information")
        };
        set(hwnd, &format!("FreePort — {title}"));
        let mut r: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut r);
        control(
            hwnd,
            10,
            "EDIT",
            &d.text,
            WS_TABSTOP
                | WS_VSCROLL
                | ES_MULTILINE as u32
                | ES_AUTOVSCROLL as u32
                | ES_READONLY as u32,
            18,
            16,
            r.right - 36,
            r.bottom - 76,
        );
        let cancel = d.flags & MB_OKCANCEL != 0;
        control(
            hwnd,
            IDOK as usize,
            "BUTTON",
            l::t("common.ok"),
            WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
            r.right - if cancel { 244 } else { 130 },
            r.bottom - 46,
            112,
            30,
        );
        if cancel {
            control(
                hwnd,
                IDCANCEL as usize,
                "BUTTON",
                l::t("common.cancel"),
                WS_TABSTOP | BS_PUSHBUTTON as u32,
                r.right - 122,
                r.bottom - 46,
                104,
                30,
            );
        }
        return 1;
    }
    if m == WM_COMMAND {
        let id = w & 0xffff;
        if id == IDOK as usize || id == IDCANCEL as usize {
            EndDialog(hwnd, id as isize);
            return 1;
        }
    }
    if m == WM_CLOSE {
        EndDialog(hwnd, IDCANCEL as isize);
        return 1;
    }
    0
}
pub unsafe fn message(owner: HWND, text: &str, flags: u32) -> i32 {
    let data = template(380, 190);
    let d = Message {
        text: text.into(),
        flags,
    };
    let result = DialogBoxIndirectParamW(
        GetModuleHandleW(ptr::null()),
        data.as_ptr() as _,
        owner,
        Some(message_proc),
        &d as *const _ as isize,
    );
    if result < 0 {
        IDCANCEL
    } else {
        result as i32
    }
}

struct Picker {
    initial: PathBuf,
    file: bool,
    result: Option<PathBuf>,
    directory: PathBuf,
    children: Vec<PathBuf>,
    edit: HWND,
    list: HWND,
    status: HWND,
}
unsafe fn fill(hwnd: HWND, d: &mut Picker, path: PathBuf) {
    let absolute = match p::absolute(&path) {
        Ok(p) => p,
        Err(e) => {
            set(d.status, &l::describe_error(e.as_ref()));
            return;
        }
    };
    if !absolute.is_dir() {
        set(d.status, l::t("dialog.path_invalid"));
        return;
    }
    d.directory = absolute.clone();
    d.children.clear();
    SendMessageW(d.list, LB_RESETCONTENT, 0, 0);
    set(d.edit, &absolute.display().to_string());
    match fs::read_dir(&absolute) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let path = entry.path();
                let meta = match fs::symlink_metadata(&path) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if p::reparse(&meta) {
                    continue;
                }
                if meta.is_dir()
                    || (d.file
                        && path
                            .extension()
                            .is_some_and(|x| x.eq_ignore_ascii_case("json")))
                {
                    d.children.push(path);
                }
            }
            d.children.sort_by(|a, b| {
                b.is_dir()
                    .cmp(&a.is_dir())
                    .then_with(|| a.file_name().cmp(&b.file_name()))
            });
            for path in &d.children {
                let label = format!(
                    "{}{}",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    if path.is_dir() { "\\" } else { "" }
                );
                SendMessageW(d.list, LB_ADDSTRING, 0, p::wide(label).as_ptr() as isize);
            }
            set(d.status, l::t("dialog.path_hint"));
        }
        Err(e) => set(d.status, &l::describe_error(&e)),
    };
    let _ = hwnd;
}
unsafe fn accept(hwnd: HWND, d: &mut Picker) {
    let path = PathBuf::from(value(d.edit));
    match p::absolute(&path) {
        Ok(p)
            if if d.file {
                p.is_file()
            } else {
                p.is_dir() || !p.exists()
            } =>
        {
            d.result = Some(p);
            EndDialog(hwnd, IDOK as isize);
        }
        _ => set(d.status, l::t("dialog.path_invalid")),
    }
}
unsafe extern "system" fn picker_proc(hwnd: HWND, m: u32, w: usize, param: isize) -> isize {
    if m == WM_INITDIALOG {
        SetWindowLongPtrW(hwnd, DWLP_USER, param);
        let d = &mut *(param as *mut Picker);
        set(
            hwnd,
            if d.file {
                l::t("dialog.rules_title")
            } else {
                l::t("dialog.folder_title")
            },
        );
        let mut r: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut r);
        d.edit = control(
            hwnd,
            10,
            "EDIT",
            "",
            WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32,
            16,
            15,
            r.right - 160,
            27,
        );
        control(
            hwnd,
            12,
            "BUTTON",
            l::t("dialog.parent"),
            WS_TABSTOP | BS_PUSHBUTTON as u32,
            r.right - 132,
            14,
            116,
            29,
        );
        d.list = control(
            hwnd,
            11,
            "LISTBOX",
            "",
            WS_BORDER | WS_VSCROLL | WS_TABSTOP | LBS_NOTIFY as u32 | LBS_NOINTEGRALHEIGHT as u32,
            16,
            52,
            r.right - 32,
            r.bottom - 142,
        );
        d.status = control(
            hwnd,
            13,
            "STATIC",
            "",
            0,
            16,
            r.bottom - 82,
            r.right - 32,
            28,
        );
        control(
            hwnd,
            IDOK as usize,
            "BUTTON",
            l::t("common.ok"),
            WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
            r.right - 244,
            r.bottom - 42,
            112,
            28,
        );
        control(
            hwnd,
            IDCANCEL as usize,
            "BUTTON",
            l::t("common.cancel"),
            WS_TABSTOP | BS_PUSHBUTTON as u32,
            r.right - 124,
            r.bottom - 42,
            108,
            28,
        );
        let mut base = d.initial.clone();
        if !base.is_absolute() || base.to_string_lossy().starts_with(r"\\") {
            base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from(r"C:\"));
        }
        while !base.is_dir() {
            if !base.pop() {
                base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from(r"C:\"));
                break;
            }
        }
        fill(hwnd, d, base);
        return 1;
    }
    if m != WM_COMMAND && m != WM_CLOSE {
        return 0;
    }
    let state = GetWindowLongPtrW(hwnd, DWLP_USER) as *mut Picker;
    if state.is_null() {
        return 0;
    }
    let d = &mut *state;
    if m == WM_COMMAND {
        match w & 0xffff {
            1 => accept(hwnd, d),
            2 => {
                EndDialog(hwnd, IDCANCEL as isize);
            }
            12 => {
                if let Some(parent) = d.directory.parent() {
                    fill(hwnd, d, parent.to_path_buf());
                }
            }
            11 => {
                if w >> 16 == LBN_DBLCLK as usize {
                    let selected = SendMessageW(d.list, LB_GETCURSEL, 0, 0);
                    if selected >= 0 {
                        if let Some(path) = d.children.get(selected as usize).cloned() {
                            if path.is_dir() {
                                fill(hwnd, d, path);
                            } else {
                                set(d.edit, &path.display().to_string());
                                accept(hwnd, d);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        return 1;
    }
    if m == WM_CLOSE {
        EndDialog(hwnd, IDCANCEL as isize);
        return 1;
    }
    0
}
pub unsafe fn choose(owner: HWND, initial: PathBuf, file: bool) -> Option<PathBuf> {
    let data = template(400, 240);
    let mut picker = Picker {
        initial,
        file,
        result: None,
        directory: PathBuf::new(),
        children: vec![],
        edit: ptr::null_mut(),
        list: ptr::null_mut(),
        status: ptr::null_mut(),
    };
    DialogBoxIndirectParamW(
        GetModuleHandleW(ptr::null()),
        data.as_ptr() as _,
        owner,
        Some(picker_proc),
        &mut picker as *mut _ as isize,
    );
    picker.result
}
pub unsafe fn self_test(owner: HWND) -> bool {
    let mut passed = true;
    for tag in l::TAGS {
        l::set(tag).unwrap();
        let data = template(380, 190);
        let d = Message {
            text: l::force_warning().into(),
            flags: MB_OKCANCEL | MB_ICONWARNING,
        };
        let hwnd = CreateDialogIndirectParamW(
            GetModuleHandleW(ptr::null()),
            data.as_ptr() as _,
            owner,
            Some(message_proc),
            &d as *const _ as isize,
        );
        if hwnd.is_null() {
            passed = false;
            continue;
        }
        passed &= value(GetDlgItem(hwnd, IDOK)) == l::t("common.ok")
            && value(GetDlgItem(hwnd, IDCANCEL)) == l::t("common.cancel");
        DestroyWindow(hwnd);
    }
    passed
}
