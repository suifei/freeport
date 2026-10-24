#![windows_subsystem = "windows"]
mod native_dialog;
use freeport::i18n as l;
use freeport::{
    engine::{self, Control},
    handles,
    inventory::Row,
    platform as p, rules, scan,
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    ptr,
    sync::{
        atomic::Ordering,
        mpsc::{self, Receiver},
    },
    thread,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::*,
    UI::{Controls::*, Input::KeyboardAndMouse::EnableWindow, WindowsAndMessaging::*},
};
const LANG: usize = 113;
const SCAN: usize = 101;
const ADD: usize = 102;
const APPLY: usize = 103;
const STOP: usize = 104;
const ALL: usize = 105;
const NONE: usize = 106;
const MODE: usize = 107;
const BROWSE: usize = 108;
const REQUEST: usize = 109;
const FORCE: usize = 110;
const RULES: usize = 111;
const RECOVER: usize = 112;
enum Event {
    Report(scan::Report),
    Measured(usize, Row),
    Progress(String),
    Done(std::result::Result<String, String>),
}
struct Ui {
    hwnd: HWND,
    language: HWND,
    requested: String,
    dpi: i32,
    font: HFONT,
    drives: Vec<(PathBuf, HWND)>,
    mode: HWND,
    list: HWND,
    target: HWND,
    force: HWND,
    request: HWND,
    status: HWND,
    log: HWND,
    title: HWND,
    description: HWND,
    controls: Vec<(usize, HWND)>,
    rows: Vec<Row>,
    busy: bool,
    control: Control,
    receiver: Option<Receiver<Event>>,
    lines: VecDeque<String>,
}
impl Default for Ui {
    fn default() -> Self {
        Self {
            hwnd: ptr::null_mut(),
            language: ptr::null_mut(),
            requested: "auto".into(),
            dpi: 96,
            font: ptr::null_mut(),
            drives: vec![],
            mode: ptr::null_mut(),
            list: ptr::null_mut(),
            target: ptr::null_mut(),
            force: ptr::null_mut(),
            request: ptr::null_mut(),
            status: ptr::null_mut(),
            log: ptr::null_mut(),
            title: ptr::null_mut(),
            description: ptr::null_mut(),
            controls: vec![],
            rows: vec![],
            busy: false,
            control: Control::default(),
            receiver: None,
            lines: VecDeque::new(),
        }
    }
}
unsafe fn text(h: HWND, s: &str) {
    SetWindowTextW(h, p::wide(s).as_ptr());
}
unsafe fn read(h: HWND) -> String {
    let n = GetWindowTextLengthW(h);
    let mut b = vec![0u16; n as usize + 1];
    let got = GetWindowTextW(h, b.as_mut_ptr(), b.len() as i32);
    String::from_utf16_lossy(&b[..got as usize])
}
unsafe fn check(h: HWND) -> bool {
    SendMessageW(h, BM_GETCHECK, 0, 0) == BST_CHECKED as isize
}
unsafe fn setcheck(h: HWND, on: bool) {
    SendMessageW(
        h,
        BM_SETCHECK,
        if on { BST_CHECKED } else { BST_UNCHECKED } as usize,
        0,
    );
}
unsafe fn msg(h: HWND, s: &str, flags: u32) -> i32 {
    native_dialog::message(h, s, flags)
}
unsafe fn child(ui: &mut Ui, id: usize, class: &str, label: &str, style: u32) -> HWND {
    let h = CreateWindowExW(
        0,
        p::wide(class).as_ptr(),
        p::wide(label).as_ptr(),
        WS_CHILD | WS_VISIBLE | style,
        0,
        0,
        10,
        10,
        ui.hwnd,
        id as _,
        GetModuleHandleW(ptr::null()),
        ptr::null(),
    );
    SendMessageW(h, WM_SETFONT, ui.font as usize, 1);
    ui.controls.push((id, h));
    h
}
unsafe fn button(ui: &mut Ui, id: usize, label: &str) -> HWND {
    child(ui, id, "BUTTON", label, WS_TABSTOP | BS_PUSHBUTTON as u32)
}
unsafe fn init(ui: &mut Ui) {
    let dc = GetDC(ui.hwnd);
    ui.dpi = GetDeviceCaps(dc, LOGPIXELSX as i32).max(96);
    ReleaseDC(ui.hwnd, dc);
    ui.font = CreateFontW(
        -13 * ui.dpi / 96,
        0,
        0,
        0,
        400,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        0,
        0,
        CLEARTYPE_QUALITY as u32,
        0,
        p::wide("Segoe UI").as_ptr(),
    );
    ui.title = child(ui, 1, "STATIC", "FreePort", 0);
    ui.description = child(ui, 2, "STATIC", l::t("msg.1918b38f27"), 0);
    ui.language = child(
        ui,
        LANG,
        "COMBOBOX",
        "",
        CBS_DROPDOWNLIST as u32 | WS_TABSTOP | WS_VSCROLL,
    );
    SendMessageW(
        ui.language,
        CB_ADDSTRING,
        0,
        p::wide(l::t("language.auto")).as_ptr() as isize,
    );
    for name in l::NAMES {
        SendMessageW(
            ui.language,
            CB_ADDSTRING,
            0,
            p::wide(name).as_ptr() as isize,
        );
    }
    SendMessageW(
        ui.language,
        CB_SETCURSEL,
        if ui.requested == "auto" {
            0
        } else {
            l::index(&ui.requested).map(|n| n + 1).unwrap_or(0)
        },
        0,
    );
    SendMessageW(ui.language, CB_SETDROPPEDWIDTH, 400, 0);
    ui.mode = child(
        ui,
        MODE,
        "COMBOBOX",
        "",
        CBS_DROPDOWNLIST as u32 | WS_TABSTOP | WS_VSCROLL,
    );
    for s in [l::t("msg.36c271c0cb"), l::t("msg.8bceb93fde")] {
        SendMessageW(ui.mode, CB_ADDSTRING, 0, p::wide(s).as_ptr() as isize);
    }
    SendMessageW(ui.mode, CB_SETCURSEL, 0, 0);
    for (i, v) in scan::volumes().into_iter().enumerate() {
        let h = child(
            ui,
            200 + i,
            "BUTTON",
            &format!("{}  {}", v.root.display(), freeport::human(v.free)),
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        );
        setcheck(h, false);
        ui.drives.push((v.root, h));
    }
    button(ui, SCAN, l::t("msg.63dec1aba1"));
    button(ui, ADD, l::t("msg.568626f4b7"));
    button(ui, ALL, l::t("msg.3a5040b68a"));
    button(ui, NONE, l::t("msg.ab9084a640"));
    button(ui, RULES, l::t("msg.4bf5e1709a"));
    button(ui, RECOVER, l::t("msg.5cc23ae008"));
    ui.list = child(
        ui,
        3,
        "SysListView32",
        "",
        WS_BORDER | WS_TABSTOP | LVS_REPORT | LVS_SHOWSELALWAYS,
    );
    SendMessageW(
        ui.list,
        LVM_SETEXTENDEDLISTVIEWSTYLE,
        0,
        (LVS_EX_CHECKBOXES | LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize,
    );
    for (i, (s, width)) in [
        (l::t("msg.403f528bf2"), 160),
        (l::t("msg.4265f23e53"), 490),
        (l::t("msg.59899b92d9"), 100),
        (l::t("msg.7e9aad99d0"), 350),
    ]
    .into_iter()
    .enumerate()
    {
        let mut name = p::wide(s);
        let mut col: LVCOLUMNW = std::mem::zeroed();
        col.mask = LVCF_TEXT | LVCF_WIDTH;
        col.cx = width;
        col.pszText = name.as_mut_ptr();
        SendMessageW(ui.list, LVM_INSERTCOLUMNW, i, &col as *const _ as isize);
    }
    child(ui, 4, "STATIC", l::t("msg.134b2da520"), 0);
    ui.target = child(
        ui,
        5,
        "EDIT",
        r"D:\Dev",
        WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32,
    );
    button(ui, BROWSE, l::t("msg.e7f468a900"));
    ui.request = child(
        ui,
        REQUEST,
        "BUTTON",
        l::t("msg.1ace177481"),
        BS_AUTOCHECKBOX as u32 | BS_MULTILINE as u32 | WS_TABSTOP,
    );
    ui.force = child(
        ui,
        FORCE,
        "BUTTON",
        l::t("msg.f8077addd1"),
        BS_AUTOCHECKBOX as u32 | BS_MULTILINE as u32 | WS_TABSTOP,
    );
    setcheck(ui.request, false);
    setcheck(ui.force, false);
    button(ui, APPLY, l::t("msg.f6fa97faea"));
    button(ui, STOP, l::t("msg.ca4d973c0b"));
    ui.status = child(ui, 6, "STATIC", l::t("msg.309b0fb3de"), 0);
    ui.log = child(
        ui,
        7,
        "EDIT",
        "",
        WS_BORDER | WS_VSCROLL | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | ES_READONLY as u32,
    );
    SetTimer(ui.hwnd, 1, 60, None);
    layout(ui);
}
unsafe fn move_id(ui: &Ui, id: usize, x: i32, y: i32, w: i32, h: i32) {
    if let Some((_, c)) = ui.controls.iter().find(|(i, _)| *i == id) {
        MoveWindow(
            *c,
            x * ui.dpi / 96,
            y * ui.dpi / 96,
            w.max(1) * ui.dpi / 96,
            h.max(1) * ui.dpi / 96,
            1,
        );
    }
}
unsafe fn layout(ui: &Ui) {
    let mut rect: RECT = std::mem::zeroed();
    GetClientRect(ui.hwnd, &mut rect);
    let w = rect.right * 96 / ui.dpi;
    let h = rect.bottom * 96 / ui.dpi;
    let pad = 18;
    move_id(ui, 1, pad, 14, 200, 25);
    move_id(ui, LANG, w - 360, 12, 342, 250);
    move_id(ui, 2, pad, 44, w - 36, 22);
    move_id(ui, MODE, pad, 76, 320, 250);
    move_id(ui, RULES, w - 470, 74, 210, 28);
    move_id(ui, RECOVER, w - 250, 74, 232, 28);
    let cols = ((w - 36) / 190).max(1) as usize;
    for (i, _) in ui.drives.iter().enumerate() {
        move_id(
            ui,
            200 + i,
            pad + (i % cols) as i32 * 190,
            112 + (i / cols) as i32 * 25,
            188,
            24,
        );
    }
    let bar = 112 + ui.drives.len().div_ceil(cols) as i32 * 25 + 10;
    move_id(ui, SCAN, pad, bar, 165, 30);
    move_id(ui, ADD, pad + 175, bar, 190, 30);
    move_id(ui, ALL, w - 258, bar, 120, 30);
    move_id(ui, NONE, w - 128, bar, 110, 30);
    let top = bar + 40;
    move_id(ui, 3, pad, top, w - 36, (h - 280 - top).max(80));
    let offset = (80 - (h - 280 - top)).max(0);
    let h = h + offset;
    move_id(ui, 4, pad, h - 268, w - 36, 22);
    move_id(ui, 5, pad, h - 242, w - 200, 28);
    move_id(ui, BROWSE, w - 172, h - 242, 154, 28);
    move_id(ui, REQUEST, pad, h - 206, w - 36, 28);
    move_id(ui, FORCE, pad, h - 176, w - 330, 34);
    move_id(ui, APPLY, w - 304, h - 176, 180, 32);
    move_id(ui, STOP, w - 114, h - 176, 96, 32);
    move_id(ui, 6, pad, h - 134, w - 36, 25);
    move_id(ui, 7, pad, h - 108, w - 36, (90 - offset).max(20));
}
unsafe fn refresh(ui: &mut Ui) {
    let ids = selected(ui);
    let mode = SendMessageW(ui.mode, CB_GETCURSEL, 0, 0);
    text(ui.hwnd, l::t("msg.7540ee4199"));
    for (id, key) in [
        (2, "msg.1918b38f27"),
        (SCAN, "msg.63dec1aba1"),
        (ADD, "msg.568626f4b7"),
        (ALL, "msg.3a5040b68a"),
        (NONE, "msg.ab9084a640"),
        (RULES, "msg.4bf5e1709a"),
        (RECOVER, "msg.5cc23ae008"),
        (4, "msg.134b2da520"),
        (BROWSE, "msg.e7f468a900"),
        (REQUEST, "msg.1ace177481"),
        (FORCE, "msg.f8077addd1"),
        (APPLY, "msg.f6fa97faea"),
        (STOP, "msg.ca4d973c0b"),
    ] {
        if let Some((_, c)) = ui.controls.iter().find(|(i, _)| *i == id) {
            text(*c, l::t(key));
        }
    }
    SendMessageW(ui.mode, CB_RESETCONTENT, 0, 0);
    for key in ["msg.36c271c0cb", "msg.8bceb93fde"] {
        SendMessageW(
            ui.mode,
            CB_ADDSTRING,
            0,
            p::wide(l::t(key)).as_ptr() as isize,
        );
    }
    SendMessageW(ui.mode, CB_SETCURSEL, mode as usize, 0);
    SendMessageW(ui.language, CB_DELETESTRING, 0, 0);
    SendMessageW(
        ui.language,
        CB_INSERTSTRING,
        0,
        p::wide(l::t("language.auto")).as_ptr() as isize,
    );
    SendMessageW(
        ui.language,
        CB_SETCURSEL,
        if ui.requested == "auto" {
            0
        } else {
            l::index(&ui.requested).map(|n| n + 1).unwrap_or(0)
        },
        0,
    );
    for (i, key) in [
        "msg.403f528bf2",
        "msg.4265f23e53",
        "msg.59899b92d9",
        "msg.7e9aad99d0",
    ]
    .into_iter()
    .enumerate()
    {
        let mut name = p::wide(l::t(key));
        let col = LVCOLUMNW {
            mask: LVCF_TEXT,
            pszText: name.as_mut_ptr(),
            ..std::mem::zeroed()
        };
        SendMessageW(ui.list, LVM_SETCOLUMNW, i, &col as *const _ as isize);
    }
    scan::relabel(&mut ui.rows);
    SendMessageW(ui.list, LVM_DELETEALLITEMS, 0, 0);
    for i in 0..ui.rows.len() {
        row(ui, i);
        if ids.contains(&ui.rows[i].candidate.id) {
            let item = LVITEMW {
                stateMask: LVIS_STATEIMAGEMASK,
                state: 2 << 12,
                ..std::mem::zeroed()
            };
            SendMessageW(ui.list, LVM_SETITEMSTATE, i, &item as *const _ as isize);
        }
    }
    text(ui.status, l::t("language.changed"));
    layout(ui);
}
unsafe fn selected(ui: &Ui) -> Vec<String> {
    ui.rows
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            ((SendMessageW(ui.list, LVM_GETITEMSTATE, *i, LVIS_STATEIMAGEMASK as isize) as u32)
                >> 12)
                == 2
        })
        .map(|(_, r)| r.candidate.id.clone())
        .collect()
}
unsafe fn row(ui: &Ui, i: usize) {
    let r = &ui.rows[i];
    let state = if let Some(t) = &r.link_target {
        l::format("msg.0ca6431378", &[format!("{}", t.display())])
    } else if !r.busy.is_empty() {
        l::format("msg.bfa79d8418", &[format!("{}", r.busy.join(", "))])
    } else {
        let key = format!("status.{}", r.status);
        if l::has_key(&key) {
            l::t(&key).into()
        } else {
            l::t("status.unknown").into()
        }
    };
    for (col, s) in [
        r.candidate.name.clone(),
        r.candidate.source.display().to_string(),
        if r.status == "unmeasured" {
            l::t("msg.8284301049").into()
        } else {
            freeport::human(r.stats.bytes)
        },
        state,
    ]
    .into_iter()
    .enumerate()
    {
        let mut s = p::wide(s);
        let mut item: LVITEMW = std::mem::zeroed();
        item.iItem = i as i32;
        item.iSubItem = col as i32;
        item.pszText = s.as_mut_ptr();
        item.mask = LVIF_TEXT;
        if col == 0 {
            SendMessageW(ui.list, LVM_INSERTITEMW, 0, &item as *const _ as isize);
        } else {
            SendMessageW(ui.list, LVM_SETITEMTEXTW, i, &item as *const _ as isize);
        }
    }
}
unsafe fn log(ui: &mut Ui, s: String) {
    text(ui.status, &s);
    ui.lines.push_back(s);
    while ui.lines.len() > 120 {
        ui.lines.pop_front();
    }
    text(
        ui.log,
        &ui.lines.iter().cloned().collect::<Vec<_>>().join("\r\n"),
    );
    SendMessageW(ui.log, EM_SETSEL, usize::MAX, -1);
    SendMessageW(ui.log, EM_SCROLLCARET, 0, 0);
}
unsafe fn busy(ui: &mut Ui, on: bool) {
    ui.busy = on;
    for (id, h) in &ui.controls {
        if [
            SCAN, ADD, APPLY, RULES, RECOVER, MODE, REQUEST, FORCE, BROWSE, LANG,
        ]
        .contains(id)
        {
            EnableWindow(*h, (!on) as i32);
        }
    }
}
unsafe fn browse(owner: HWND) -> Option<PathBuf> {
    native_dialog::choose(
        owner,
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(r"C:\")),
        false,
    )
}
unsafe fn rule_file(owner: HWND) -> Option<PathBuf> {
    native_dialog::choose(
        owner,
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(r"C:\")),
        true,
    )
}
unsafe fn start_scan(ui: &mut Ui, roots: Vec<PathBuf>) {
    if roots.is_empty() {
        msg(ui.hwnd, l::t("msg.619d2feebb"), MB_OK | MB_ICONINFORMATION);
        return;
    }
    let mode = if SendMessageW(ui.mode, CB_GETCURSEL, 0, 0) == 0 {
        "migrate"
    } else {
        "restore"
    }
    .to_string();
    let (tx, rx) = mpsc::channel();
    ui.receiver = Some(rx);
    ui.control = Control::default();
    let control = ui.control.clone();
    busy(ui, true);
    log(ui, l::t("msg.75bcc287ac").into());
    thread::spawn(move || {
        let result = (|| -> freeport::Result<()> {
            let progress = |s| {
                let _ = tx.send(Event::Progress(s));
            };
            let mut report = scan::run_with_control(&roots, &mode, &progress, &control)?;
            let _ = tx.send(Event::Report(report.clone()));
            if mode == "migrate" {
                for (i, r) in report.rows.iter_mut().enumerate() {
                    control.check()?;
                    scan::measure_with_control(std::slice::from_mut(r), &progress, &control)?;
                    let _ = tx.send(Event::Measured(i, r.clone()));
                }
            }
            Ok(())
        })();
        let _ = tx.send(Event::Done(
            result
                .map(|_| l::t("msg.7af6958306").into())
                .map_err(|e| l::describe_error(e.as_ref())),
        ));
    });
}
unsafe fn apply(ui: &mut Ui) {
    let ids = selected(ui);
    if ids.is_empty() {
        msg(ui.hwnd, l::t("msg.5c3c8200cc"), MB_OK | MB_ICONINFORMATION);
        return;
    }
    let restore = SendMessageW(ui.mode, CB_GETCURSEL, 0, 0) != 0;
    let store = engine::default_store();
    let rows = ui.rows.clone();
    let destination = PathBuf::from(read(ui.target));
    let control = Control {
        options: handles::Options {
            request_app_exit: check(ui.request),
            force_close_handles: check(ui.force),
        },
        ..Control::default()
    };
    let plan = if restore {
        None
    } else {
        match engine::make_plan(&rows, &ids, &destination) {
            Ok(p) => Some(p),
            Err(e) => {
                msg(
                    ui.hwnd,
                    &l::describe_error(e.as_ref()),
                    MB_OK | MB_ICONERROR,
                );
                return;
            }
        }
    };
    let selected: Vec<_> = rows
        .iter()
        .filter(|r| ids.contains(&r.candidate.id))
        .map(|r| r.candidate.source.clone())
        .collect();
    let mut preview = if restore {
        l::t("msg.d0b032ac5c").to_string()
    } else {
        l::t("msg.5a514567e2").to_string()
    };
    if let Some(p) = &plan {
        for i in &p.items {
            preview.push_str(&format!(
                "\n{}\n→ {}\n",
                i.source.display(),
                i.target.display()
            ));
        }
    } else {
        for p in &selected {
            preview.push_str(&format!("\n{}", p.display()));
        }
    }
    if control.options.force_close_handles {
        preview.push_str(l::t("msg.f4c54ea5fb"));
    }
    if msg(ui.hwnd, &preview, MB_OKCANCEL | MB_ICONQUESTION) != IDOK {
        return;
    }
    let (tx, rx) = mpsc::channel();
    ui.receiver = Some(rx);
    ui.control = control.clone();
    setcheck(ui.force, false);
    setcheck(ui.request, false);
    busy(ui, true);
    thread::spawn(move || {
        let p = |s| {
            let _ = tx.send(Event::Progress(s));
        };
        let result = (|| -> freeport::Result<()> {
            if let Some(plan) = plan {
                engine::migrate(&plan, &store, &control, &p)?;
            } else {
                engine::restore_many(&selected, &store, &control, &p)?;
            }
            Ok(())
        })();
        let _ = tx.send(Event::Done(
            result
                .map(|_| l::t("msg.899db705a6").into())
                .map_err(|e| l::describe_error(e.as_ref())),
        ));
    });
}
unsafe fn recover(ui: &mut Ui) {
    let store = engine::default_store();
    let pending = engine::history(&store).and_then(|rs| {
        rs.into_iter()
            .find(|r| !["completed", "restored", "rolled_back"].contains(&r.phase.as_str()))
            .ok_or_else(|| l::t("msg.940d6b1834").into())
    });
    match pending {
        Err(e) => {
            msg(
                ui.hwnd,
                &l::describe_error(e.as_ref()),
                MB_OK | MB_ICONINFORMATION,
            );
        }
        Ok(record) => {
            if msg(
                ui.hwnd,
                &l::format(
                    "msg.06ee3e938c",
                    &[
                        format!("{}", record.id),
                        format!("{}", record.source.display()),
                        format!("{}", record.phase),
                    ],
                ),
                MB_OKCANCEL | MB_ICONQUESTION,
            ) != IDOK
            {
                return;
            }
            let (tx, rx) = mpsc::channel();
            ui.receiver = Some(rx);
            ui.control = Control::default();
            let c = ui.control.clone();
            busy(ui, true);
            thread::spawn(move || {
                let p = |s| {
                    let _ = tx.send(Event::Progress(s));
                };
                let result = engine::recover(&store, &record.id, &c, &p)
                    .map(|r| l::format("msg.e47730b6da", &[format!("{}", r.phase)]))
                    .map_err(|e| l::describe_error(e.as_ref()));
                let _ = tx.send(Event::Done(result));
            });
        }
    }
}
unsafe extern "system" fn wnd(hwnd: HWND, message: u32, w: usize, l: isize) -> isize {
    if message == WM_NCCREATE {
        let c = &*(l as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, c.lpCreateParams as isize);
    }
    if ![
        WM_CREATE,
        WM_SIZE,
        WM_GETMINMAXINFO,
        WM_COMMAND,
        WM_TIMER,
        WM_CLOSE,
        WM_DESTROY,
    ]
    .contains(&message)
    {
        return DefWindowProcW(hwnd, message, w, l);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, message, w, l);
    }
    let ui = &mut *ptr;
    ui.hwnd = hwnd;
    match message {
        WM_CREATE => {
            init(ui);
            if ui.controls.iter().any(|(_, h)| h.is_null()) {
                -1
            } else {
                0
            }
        }
        WM_SIZE => {
            layout(ui);
            0
        }
        WM_GETMINMAXINFO => {
            let info = &mut *(l as *mut MINMAXINFO);
            info.ptMinTrackSize = POINT {
                x: 920 * ui.dpi / 96,
                y: 700 * ui.dpi / 96,
            };
            0
        }
        WM_COMMAND => {
            let id = w & 0xffff;
            if ui.busy && id != STOP {
                return 0;
            }
            match id {
                SCAN => start_scan(
                    ui,
                    ui.drives
                        .iter()
                        .filter(|(_, h)| check(*h))
                        .map(|(p, _)| p.clone())
                        .collect(),
                ),
                ADD => {
                    if let Some(path) = browse(hwnd) {
                        start_scan(ui, vec![path]);
                    }
                }
                BROWSE => {
                    if let Some(path) = browse(hwnd) {
                        text(ui.target, &path.display().to_string());
                    }
                }
                APPLY => apply(ui),
                STOP => {
                    ui.control.cancel.store(true, Ordering::Relaxed);
                    log(ui, l::t("msg.32d5908bc7").into());
                }
                ALL | NONE => {
                    for i in 0..ui.rows.len() {
                        let mut item: LVITEMW = std::mem::zeroed();
                        item.stateMask = LVIS_STATEIMAGEMASK;
                        item.state = if id == ALL { 2 << 12 } else { 1 << 12 };
                        SendMessageW(ui.list, LVM_SETITEMSTATE, i, &item as *const _ as isize);
                    }
                }
                FORCE => {
                    if check(ui.force)
                        && msg(hwnd, handles::warning(), MB_OKCANCEL | MB_ICONWARNING) != IDOK
                    {
                        setcheck(ui.force, false);
                    }
                }
                MODE => {
                    if w >> 16 == CBN_SELCHANGE as usize {
                        ui.rows.clear();
                        SendMessageW(ui.list, LVM_DELETEALLITEMS, 0, 0);
                        setcheck(ui.force, false);
                        setcheck(ui.request, false);
                        EnableWindow(
                            ui.target,
                            (SendMessageW(ui.mode, CB_GETCURSEL, 0, 0) == 0) as i32,
                        );
                    }
                }
                LANG => {
                    if w >> 16 == CBN_SELCHANGE as usize {
                        let index = SendMessageW(ui.language, CB_GETCURSEL, 0, 0);
                        let choice = if index <= 0 {
                            "auto"
                        } else {
                            l::TAGS.get(index as usize - 1).copied().unwrap_or("en")
                        };
                        match l::save_preference(choice) {
                            Ok(c) => {
                                ui.requested = c.requested_language;
                                refresh(ui);
                            }
                            Err(e) => {
                                msg(hwnd, &l::describe_error(e.as_ref()), MB_OK | MB_ICONERROR);
                                SendMessageW(
                                    ui.language,
                                    CB_SETCURSEL,
                                    if ui.requested == "auto" {
                                        0
                                    } else {
                                        l::index(&ui.requested).map(|n| n + 1).unwrap_or(0)
                                    },
                                    0,
                                );
                            }
                        }
                    }
                }
                RULES => {
                    if let Some(file) = rule_file(hwnd) {
                        match rules::import(&file, &engine::default_store()) {
                            Ok(p) => {
                                log(
                                    ui,
                                    l::format("msg.e29e2be580", &[format!("{}", p.revision)]),
                                );
                            }
                            Err(e) => {
                                msg(hwnd, &l::describe_error(e.as_ref()), MB_OK | MB_ICONERROR);
                            }
                        }
                    }
                }
                RECOVER => recover(ui),
                _ => {}
            }
            0
        }
        WM_TIMER => {
            let events: Vec<_> = ui
                .receiver
                .as_ref()
                .map(|rx| rx.try_iter().take(200).collect())
                .unwrap_or_default();
            for event in events {
                match event {
                    Event::Report(report) => {
                        ui.rows = report.rows;
                        SendMessageW(ui.list, LVM_DELETEALLITEMS, 0, 0);
                        for i in 0..ui.rows.len() {
                            row(ui, i);
                        }
                        for warning in report.warnings {
                            log(ui, warning);
                        }
                    }
                    Event::Measured(i, r) => {
                        if i < ui.rows.len() {
                            let checked = ((SendMessageW(
                                ui.list,
                                LVM_GETITEMSTATE,
                                i,
                                LVIS_STATEIMAGEMASK as isize,
                            ) as u32)
                                >> 12)
                                == 2;
                            ui.rows[i] = r;
                            SendMessageW(ui.list, LVM_DELETEITEM, i, 0);
                            row(ui, i);
                            if checked {
                                let mut item: LVITEMW = std::mem::zeroed();
                                item.stateMask = LVIS_STATEIMAGEMASK;
                                item.state = 2 << 12;
                                SendMessageW(
                                    ui.list,
                                    LVM_SETITEMSTATE,
                                    i,
                                    &item as *const _ as isize,
                                );
                            }
                        }
                    }
                    Event::Progress(s) => log(ui, s),
                    Event::Done(result) => {
                        busy(ui, false);
                        match result {
                            Ok(s) => log(ui, s),
                            Err(e) => {
                                log(ui, e.clone());
                                msg(hwnd, &e, MB_OK | MB_ICONERROR);
                            }
                        }
                    }
                }
            }
            0
        }
        WM_CLOSE => {
            if ui.busy {
                ui.control.cancel.store(true, Ordering::Relaxed);
                log(ui, l::t("msg.640ae839a9").into());
            } else {
                DestroyWindow(hwnd);
            }
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, 1);
            DeleteObject(ui.font);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, message, w, l),
    }
}
fn main() {
    if handles::maybe_helper() {
        return;
    }
    let args: Vec<String> = std::env::args().collect();
    let explicit = args
        .iter()
        .position(|a| a == "--lang")
        .and_then(|i| args.get(i + 1))
        .map(String::as_str);
    let choice = match l::init(explicit) {
        Ok(c) => c,
        Err(e) => {
            unsafe {
                msg(
                    ptr::null_mut(),
                    &l::describe_error(e.as_ref()),
                    MB_OK | MB_ICONERROR,
                );
            }
            return;
        }
    };
    unsafe {
        SetProcessDPIAware();
        let common = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES,
        };
        InitCommonControlsEx(&common);
        let class = p::wide("FreePortNativeWindow");
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(wnd);
        wc.hInstance = GetModuleHandleW(ptr::null());
        wc.hCursor = LoadCursorW(ptr::null_mut(), IDC_ARROW);
        wc.hbrBackground = (COLOR_WINDOW + 1) as _;
        wc.lpszClassName = class.as_ptr();
        RegisterClassW(&wc);
        let mut ui = Box::new(Ui::default());
        ui.requested = choice.requested_language;
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class.as_ptr(),
            p::wide(l::t("msg.7540ee4199")).as_ptr(),
            WS_OVERLAPPEDWINDOW,
            80,
            40,
            1180,
            740,
            ptr::null_mut(),
            ptr::null_mut(),
            wc.hInstance,
            &mut *ui as *mut _ as _,
        );
        if hwnd.is_null() {
            msg(
                ptr::null_mut(),
                &l::format(
                    "msg.a8078003c7",
                    &[l::describe_error(&std::io::Error::last_os_error())],
                ),
                MB_OK | MB_ICONERROR,
            );
            return;
        }
        if args.iter().any(|a| a == "--self-test") {
            let defaults = (
                !check(ui.request),
                !check(ui.force),
                selected(&ui).is_empty(),
            );
            let initial_language = l::language();
            let mut switches_preserved = true;
            ui.rows.push(Row {
                candidate: freeport::catalog::custom(
                    std::path::Path::new(r"E:\freeport-self-test\cache"),
                    "fixture",
                    "custom",
                ),
                stats: Default::default(),
                status: "ready".into(),
                link_target: None,
                busy: vec![],
            });
            row(&ui, 0);
            let item = LVITEMW {
                stateMask: LVIS_STATEIMAGEMASK,
                state: 2 << 12,
                ..std::mem::zeroed()
            };
            SendMessageW(ui.list, LVM_SETITEMSTATE, 0, &item as *const _ as isize);
            setcheck(ui.request, true);
            setcheck(ui.force, true);
            let ids = selected(&ui);
            let destination = read(ui.target);
            let mut labels = std::collections::BTreeMap::new();
            for tag in l::TAGS {
                l::set(tag).unwrap();
                ui.requested = tag.into();
                refresh(&mut ui);
                switches_preserved &= check(ui.request)
                    && check(ui.force)
                    && selected(&ui) == ids
                    && read(ui.target) == destination;
                labels.insert(tag, read(ui.description));
            }
            let dialogs_localized = native_dialog::self_test(hwnd);
            println!(
                "{}",
                serde_json::json!({"window_is_native":true,"language":initial_language,"request_app_exit":!defaults.0,"force_close_handles":!defaults.1,"selected_count":if defaults.2{0}else{1},"controls_count":ui.controls.len(),"language_switch_preserves_state":switches_preserved,"dialogs_localized":dialogs_localized,"labels":labels})
            );
            DestroyWindow(hwnd);
            return;
        }
        if choice.source == "invalid_preference" {
            msg(hwnd, l::t("language.corrupt"), MB_OK | MB_ICONWARNING);
        }
        ShowWindow(hwnd, SW_SHOW);
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(hwnd, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}
