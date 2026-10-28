#![windows_subsystem = "windows"]
mod candidate_list;
mod native_dialog;
use freeport::i18n as l;
use freeport::{
    elevation::{self, Job, Update},
    selection,
};
use freeport::{
    engine::{self, Control},
    handles,
    inventory::Row,
    platform as p, scan,
};
use std::{
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
const ELEVATE: usize = 114;
const SAVE_SELECTION: usize = 115;
const LOAD_SELECTION: usize = 116;
const ABOUT: usize = 117;
const LIST_SYNC: u32 = WM_APP + 9;
const LOG_LINES: i32 = 500;
const CONTACT_QQ: &str = "410000";
// Static control styles (windows-sys keeps them behind an unrelated feature).
const SS_NOPREFIX: u32 = 0x80;
const SS_ENDELLIPSIS: u32 = 0x4000;
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
const RECOVER: usize = 112;
enum Event {
    Invalidate,
    Report(scan::Report),
    Prepared(std::result::Result<Prepared, String>),
    Measured(usize, Row),
    Progress(String),
    Done(std::result::Result<String, String>),
}
struct Prepared {
    job: Job,
    preview: String,
    control: Control,
    elevate: bool,
}
struct Ui {
    hwnd: HWND,
    language: HWND,
    requested: String,
    dpi: i32,
    font: HFONT,
    title_font: HFONT,
    background: HBRUSH,
    drives: Vec<(PathBuf, HWND)>,
    mode: HWND,
    list: HWND,
    table: Option<candidate_list::List>,
    elevate: HWND,
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
}
impl Default for Ui {
    fn default() -> Self {
        Self {
            hwnd: ptr::null_mut(),
            language: ptr::null_mut(),
            requested: "auto".into(),
            dpi: 96,
            font: ptr::null_mut(),
            title_font: ptr::null_mut(),
            background: ptr::null_mut(),
            drives: vec![],
            mode: ptr::null_mut(),
            list: ptr::null_mut(),
            table: None,
            elevate: ptr::null_mut(),
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
unsafe fn font(ui: &Ui, height: i32, weight: i32) -> HFONT {
    CreateFontW(
        -height * ui.dpi / 96,
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        0,
        0,
        CLEARTYPE_QUALITY as u32,
        0,
        p::wide("Segoe UI").as_ptr(),
    )
}
fn drive_label(v: &scan::Volume) -> String {
    l::format(
        "drive.free",
        &[v.root.display().to_string(), freeport::human(v.free)],
    )
}
unsafe fn init(ui: &mut Ui) {
    let dc = GetDC(ui.hwnd);
    ui.dpi = GetDeviceCaps(dc, LOGPIXELSX as i32).max(96);
    ReleaseDC(ui.hwnd, dc);
    ui.font = font(ui, 13, 400);
    ui.title_font = font(ui, 20, 600);
    ui.background = GetSysColorBrush(COLOR_WINDOW);
    ui.title = child(ui, 1, "STATIC", "FreePort", SS_NOPREFIX);
    SendMessageW(ui.title, WM_SETFONT, ui.title_font as usize, 1);
    ui.description = child(ui, 2, "STATIC", l::t("scan.scope"), SS_NOPREFIX);
    button(ui, ABOUT, l::t("about.button"));
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
    button(ui, RECOVER, l::t("msg.5cc23ae008"));
    for (i, v) in scan::volumes().into_iter().enumerate() {
        let h = child(
            ui,
            200 + i,
            "BUTTON",
            &drive_label(&v),
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        );
        setcheck(h, false);
        ui.drives.push((v.root, h));
    }
    button(ui, SCAN, l::t("msg.63dec1aba1"));
    button(ui, ADD, l::t("msg.568626f4b7"));
    button(ui, SAVE_SELECTION, l::t("selection.save"));
    button(ui, LOAD_SELECTION, l::t("selection.load"));
    button(ui, ALL, l::t("msg.3a5040b68a"));
    button(ui, NONE, l::t("msg.ab9084a640"));
    ui.list = child(
        ui,
        3,
        "SysListView32",
        "",
        WS_BORDER | WS_TABSTOP | LVS_REPORT | LVS_SHOWSELALWAYS | LVS_SHAREIMAGELISTS,
    );
    let table = candidate_list::List::new(ui.list);
    table.init();
    ui.table = Some(table);
    child(ui, 4, "STATIC", l::t("msg.134b2da520"), SS_NOPREFIX);
    ui.target = child(
        ui,
        5,
        "EDIT",
        r"D:\Dev",
        WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32,
    );
    button(ui, BROWSE, l::t("msg.e7f468a900"));
    // The app-exit and force-close switches are deferred to the next milestone
    // (docs/requirements.md 2.1); ui.request / ui.force stay null and read unchecked.
    ui.elevate = child(
        ui,
        ELEVATE,
        "BUTTON",
        l::t("elevation.option"),
        BS_AUTOCHECKBOX as u32 | BS_MULTILINE as u32 | WS_TABSTOP,
    );
    setcheck(ui.elevate, false);
    setcheck(ui.request, false);
    setcheck(ui.force, false);
    button(ui, APPLY, l::t("msg.f6fa97faea"));
    button(ui, STOP, l::t("msg.ca4d973c0b"));
    ui.status = child(
        ui,
        6,
        "STATIC",
        l::t("msg.309b0fb3de"),
        SS_NOPREFIX | SS_ENDELLIPSIS,
    );
    ui.log = child(
        ui,
        7,
        "EDIT",
        "",
        WS_BORDER | WS_VSCROLL | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | ES_READONLY as u32,
    );
    SendMessageW(ui.log, EM_SETLIMITTEXT, 0, 0);
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
// Logical (96 DPI) sizes. The window's minimum track size keeps every row on screen.
const MIN_WIDTH: i32 = 860;
const MIN_HEIGHT: i32 = 600;
unsafe fn layout(ui: &Ui) {
    let mut rect: RECT = std::mem::zeroed();
    GetClientRect(ui.hwnd, &mut rect);
    let w = (rect.right * 96 / ui.dpi).max(MIN_WIDTH - 16);
    let h = (rect.bottom * 96 / ui.dpi).max(MIN_HEIGHT - 40);
    let pad = 16;
    let right = w - pad;
    // Header: title, description; About and language on the right.
    move_id(ui, 1, pad, 10, 300, 32);
    move_id(ui, LANG, right - 200, 14, 200, 300);
    move_id(ui, ABOUT, right - 300, 13, 92, 28);
    move_id(ui, 2, pad, 46, w - 2 * pad, 20);
    // Mode and transaction recovery.
    move_id(ui, MODE, pad, 74, 300, 300);
    move_id(ui, RECOVER, right - 200, 72, 200, 28);
    // Drives, wrapping as needed.
    let cols = ((w - 2 * pad) / 210).max(1) as usize;
    for (i, _) in ui.drives.iter().enumerate() {
        move_id(
            ui,
            200 + i,
            pad + (i % cols) as i32 * 210,
            108 + (i / cols) as i32 * 26,
            204,
            24,
        );
    }
    let bar = 108 + ui.drives.len().div_ceil(cols).max(1) as i32 * 26 + 6;
    // Scan on the left; selection tools on the right.
    move_id(ui, SCAN, pad, bar, 150, 30);
    move_id(ui, ADD, pad + 158, bar, 170, 30);
    move_id(ui, NONE, right - 90, bar, 90, 30);
    move_id(ui, ALL, right - 188, bar, 90, 30);
    move_id(ui, LOAD_SELECTION, right - 316, bar, 120, 30);
    move_id(ui, SAVE_SELECTION, right - 444, bar, 120, 30);
    // Bottom block, measured upwards: log, status, run row, target.
    let log_height = ((h - bar) / 4).clamp(96, 220);
    let log_top = h - pad - log_height;
    let status_top = log_top - 26;
    let run_top = status_top - 40;
    let target_top = run_top - 38;
    let label_top = target_top - 22;
    let top = bar + 40;
    move_id(ui, 3, pad, top, w - 2 * pad, (label_top - 8 - top).max(80));
    if let Some(table) = &ui.table {
        table.layout();
    }
    move_id(ui, 4, pad, label_top, w - 2 * pad, 20);
    move_id(ui, 5, pad, target_top, w - 2 * pad - 162, 28);
    move_id(ui, BROWSE, right - 154, target_top, 154, 28);
    move_id(ui, ELEVATE, pad, run_top + 2, w - 2 * pad - 300, 28);
    move_id(ui, STOP, right - 96, run_top, 96, 32);
    move_id(ui, APPLY, right - 288, run_top, 184, 32);
    move_id(ui, 6, pad, status_top, w - 2 * pad, 22);
    move_id(ui, 7, pad, log_top, w - 2 * pad, log_height);
}
unsafe fn refresh(ui: &mut Ui) {
    let _ids = selected(ui);
    let mode = SendMessageW(ui.mode, CB_GETCURSEL, 0, 0);
    text(
        ui.hwnd,
        &format!("{} — {}", l::t("msg.7540ee4199"), env!("CARGO_PKG_VERSION")),
    );
    for (id, key) in [
        (2, "scan.scope"),
        (SCAN, "msg.63dec1aba1"),
        (ADD, "msg.568626f4b7"),
        (ALL, "msg.3a5040b68a"),
        (NONE, "msg.ab9084a640"),
        (SAVE_SELECTION, "selection.save"),
        (LOAD_SELECTION, "selection.load"),
        (ABOUT, "about.button"),
        (RECOVER, "msg.5cc23ae008"),
        (4, "msg.134b2da520"),
        (BROWSE, "msg.e7f468a900"),
        (REQUEST, "msg.1ace177481"),
        (ELEVATE, "elevation.option"),
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
    for (i, v) in scan::volumes().into_iter().enumerate() {
        if let Some((_, h)) = ui.drives.get(i).filter(|(root, _)| *root == v.root) {
            text(*h, &drive_label(&v));
        }
    }
    scan::relabel(&mut ui.rows);
    ui.table.as_mut().unwrap().rebuild(&ui.rows, mode != 0);
    text(ui.status, l::t("language.changed"));
    layout(ui);
}
unsafe fn selected(ui: &Ui) -> Vec<String> {
    let selection = &ui.table.as_ref().unwrap().selection;
    ui.rows
        .iter()
        .filter(|r| selection.state(&r.candidate.source) == 2)
        .map(|r| r.candidate.id.clone())
        .collect()
}
unsafe fn row(ui: &mut Ui, i: usize) {
    ui.table.as_mut().unwrap().update(&ui.rows, i);
}
unsafe fn log(ui: &mut Ui, s: String) {
    text(ui.status, s.lines().next().unwrap_or_default());
    // Append at the end instead of resetting the text, so the view follows new lines.
    let end = GetWindowTextLengthW(ui.log) as usize;
    let line = if end == 0 { s } else { format!("\r\n{s}") };
    SendMessageW(ui.log, EM_SETSEL, end, end as isize);
    SendMessageW(
        ui.log,
        EM_REPLACESEL,
        0,
        p::wide(line.replace("\r\n", "\n").replace('\n', "\r\n")).as_ptr() as isize,
    );
    let lines = SendMessageW(ui.log, EM_GETLINECOUNT, 0, 0) as i32;
    if lines > LOG_LINES {
        let cut = SendMessageW(ui.log, EM_LINEINDEX, (lines - LOG_LINES) as usize, 0);
        SendMessageW(ui.log, EM_SETSEL, 0, cut);
        SendMessageW(ui.log, EM_REPLACESEL, 0, p::wide("").as_ptr() as isize);
    }
    let end = GetWindowTextLengthW(ui.log) as usize;
    SendMessageW(ui.log, EM_SETSEL, end, end as isize);
    SendMessageW(ui.log, EM_SCROLLCARET, 0, 0);
}
unsafe fn about(ui: &Ui) {
    msg(
        ui.hwnd,
        &l::format(
            "about.text",
            &[
                env!("CARGO_PKG_VERSION").to_string(),
                env!("CARGO_PKG_LICENSE").to_string(),
                env!("CARGO_PKG_REPOSITORY").to_string(),
                CONTACT_QQ.to_string(),
            ],
        ),
        MB_OK | MB_ICONINFORMATION,
    );
}
unsafe fn busy(ui: &mut Ui, on: bool) {
    ui.busy = on;
    for (id, h) in &ui.controls {
        if [
            SCAN,
            ADD,
            APPLY,
            RECOVER,
            MODE,
            REQUEST,
            FORCE,
            BROWSE,
            LANG,
            ELEVATE,
            ALL,
            NONE,
            SAVE_SELECTION,
            LOAD_SELECTION,
            3,
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
    let mut elevate = check(ui.elevate);
    if !elevate
        && !elevation::is_elevated().unwrap_or(false)
        && elevation::needs_volume_access(&roots)
    {
        match msg(
            ui.hwnd,
            l::t("elevation.scan_prompt"),
            MB_YESNOCANCEL | MB_ICONQUESTION,
        ) {
            IDYES => elevate = true,
            IDNO => {}
            _ => return,
        }
    }
    let mode = if SendMessageW(ui.mode, CB_GETCURSEL, 0, 0) == 0 {
        "migrate"
    } else {
        "restore"
    }
    .to_string();
    ui.table.as_mut().unwrap().selection = Default::default();
    setcheck(ui.request, false);
    setcheck(ui.force, false);
    setcheck(ui.elevate, false);
    let job = Job::Scan {
        roots,
        mode,
        store: engine::default_store(),
        measure: true,
    };
    launch(ui, job, Control::default(), elevate);
}
unsafe fn launch(ui: &mut Ui, job: Job, control: Control, elevate: bool) {
    let (tx, rx) = mpsc::channel();
    ui.receiver = Some(rx);
    ui.control = control.clone();
    busy(ui, true);
    let scan = matches!(job, Job::Scan { .. });
    log(
        ui,
        l::t(if scan {
            "msg.75bcc287ac"
        } else {
            "operation.running"
        })
        .into(),
    );
    thread::spawn(move || {
        let update = |u| {
            let event = match u {
                Update::Progress(s) => Event::Progress(s),
                Update::Report(r) => Event::Report(r),
                Update::Measured(i, r) => Event::Measured(i, r),
            };
            let _ = tx.send(event);
        };
        let result = elevation::execute(&job, elevate, &control, &update);
        if !scan {
            let _ = tx.send(Event::Invalidate);
        }
        let _ = tx.send(Event::Done(
            result
                .map(|_| {
                    l::t(if scan {
                        "msg.7af6958306"
                    } else {
                        "operation.rescan"
                    })
                    .into()
                })
                .map_err(|e| l::describe_error(e.as_ref())),
        ));
    });
}
unsafe fn apply(ui: &mut Ui) {
    let restore = SendMessageW(ui.mode, CB_GETCURSEL, 0, 0) != 0;
    let choice = ui.table.as_ref().unwrap().selection.clone();
    if choice.include.is_empty() {
        msg(ui.hwnd, l::t("msg.5c3c8200cc"), MB_OK | MB_ICONINFORMATION);
        return;
    }
    let rows = ui.rows.clone();
    let destination = PathBuf::from(read(ui.target));
    let store = engine::default_store();
    let control = Control {
        options: handles::Options {
            request_app_exit: check(ui.request),
            force_close_handles: check(ui.force),
        },
        ..Default::default()
    };
    let elevate = check(ui.elevate);
    let (tx, rx) = mpsc::channel();
    ui.receiver = Some(rx);
    ui.control = control.clone();
    busy(ui, true);
    log(ui, l::t("selection.preparing").into());
    thread::spawn(move || {
        let result = (|| -> freeport::Result<Prepared> {
            let mut preview = l::t(if restore {
                "msg.d0b032ac5c"
            } else {
                "msg.5a514567e2"
            })
            .to_string();
            preview.push_str("\n\n");
            preview.push_str(l::t("system.policy"));
            let job = if restore {
                let sources = choice.restore_sources(&rows)?;
                if sources.is_empty() {
                    return freeport::fail(l::t("selection.invalid"));
                }
                for p in &sources {
                    preview.push_str(&format!("\n{}", p.display()));
                }
                let users = engine::in_use(&sources);
                if !users.is_empty() {
                    preview.push_str("\n");
                    preview.push_str(&handles::advice(&users));
                }
                Job::Restore {
                    sources,
                    store,
                    options: control.options.clone(),
                }
            } else {
                let request = Job::Plan {
                    rows,
                    selection: choice,
                    destination,
                    store: store.clone(),
                };
                let planned = elevation::execute(&request, elevate, &control, &|u| {
                    if let Update::Progress(s) = u {
                        let _ = tx.send(Event::Progress(s));
                    }
                })?;
                let planned: selection::Planned = serde_json::from_value(planned)?;
                let plan = planned.plan;
                for i in &plan.items {
                    preview.push_str(&format!(
                        "\n{}\n→ {}\n",
                        i.source.display(),
                        i.target.display()
                    ));
                }
                if !planned.retained.is_empty() {
                    preview.push_str(l::t("selection.retained"));
                    for p in planned.retained {
                        preview.push_str(&format!("\n{}", p.display()));
                    }
                }
                let sources: Vec<_> = plan.items.iter().map(|i| i.source.clone()).collect();
                let users = handles::users(&sources);
                if !users.is_empty() {
                    preview.push_str("\n");
                    preview.push_str(&handles::advice(&users));
                }
                Job::Migrate {
                    plan,
                    store,
                    options: control.options.clone(),
                }
            };
            if control.options.force_close_handles {
                preview.push_str(handles::warning());
            }
            Ok(Prepared {
                job,
                preview,
                control,
                elevate,
            })
        })()
        .map_err(|e| l::describe_error(e.as_ref()));
        let _ = tx.send(Event::Prepared(result));
    });
}
unsafe fn recover(ui: &mut Ui) {
    let store = engine::default_store();
    let pending = engine::history(&store).and_then(|rs| {
        rs.into_iter()
            .find(|r| !["completed", "restored", "rolled_back"].contains(&r.phase.as_str()))
            .ok_or_else(|| {
                let mut text = l::t("msg.940d6b1834").to_string();
                for dir in engine::leftover_dirs(&store).unwrap_or_default() {
                    text.push('\n');
                    text.push_str(&l::format("history.leftover", &[dir.display().to_string()]));
                }
                text.into()
            })
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
            let c = Control::default();
            launch(
                ui,
                Job::Recover {
                    id: record.id,
                    store,
                    options: c.options.clone(),
                },
                c,
                check(ui.elevate),
            );
        }
    }
}
unsafe extern "system" fn wnd(hwnd: HWND, message: u32, w: usize, l: isize) -> isize {
    if message == WM_NOTIFY && candidate_list::internal() {
        return 0;
    }
    if message == WM_NCCREATE {
        let c = &*(l as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, c.lpCreateParams as isize);
    }
    if ![
        WM_CREATE,
        WM_NOTIFY,
        LIST_SYNC,
        WM_SIZE,
        WM_GETMINMAXINFO,
        WM_CTLCOLORSTATIC,
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
        LIST_SYNC => {
            ui.table.as_ref().unwrap().sync();
            0
        }
        // Labels, checkboxes and the read-only log paint on the window background, not gray.
        WM_CTLCOLORSTATIC => {
            let dc = w as HDC;
            SetBkColor(dc, GetSysColor(COLOR_WINDOW));
            SetTextColor(dc, GetSysColor(COLOR_WINDOWTEXT));
            ui.background as isize
        }
        WM_NOTIFY => {
            let hdr = &*(l as *const NMHDR);
            if ui.table.is_none() || hdr.hwndFrom != ui.list {
                return 0;
            }
            if hdr.code == NM_CUSTOMDRAW {
                return ui.table.as_ref().unwrap().draw(l);
            }
            if hdr.code == LVN_ITEMCHANGING {
                // Checkbox changes go through the selection model; the view is re-synced after.
                let change = &*(l as *const NMLISTVIEW);
                let state = (change.uNewState ^ change.uOldState) & LVIS_STATEIMAGEMASK != 0;
                if state && ui.busy {
                    return 1;
                }
                if ui.table.as_mut().unwrap().changing(change) {
                    PostMessageW(hwnd, LIST_SYNC, 0, 0);
                    return 1;
                }
            } else if hdr.code == LVN_ITEMCHANGED {
                let change = &*(l as *const NMLISTVIEW);
                if change.uChanged & LVIF_STATE != 0
                    && change.uNewState & LVIS_SELECTED != 0
                    && change.uOldState & LVIS_SELECTED == 0
                {
                    let details = ui
                        .table
                        .as_ref()
                        .unwrap()
                        .details(change.iItem as usize, &ui.rows);
                    if !details.is_empty() {
                        log(ui, details);
                    }
                }
            } else if hdr.code == LVN_COLUMNCLICK && !ui.busy {
                let click = &*(l as *const NMLISTVIEW);
                ui.table
                    .as_mut()
                    .unwrap()
                    .column_click(click.iSubItem as usize);
            }
            0
        }
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
                x: MIN_WIDTH * ui.dpi / 96,
                y: MIN_HEIGHT * ui.dpi / 96,
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
                    ui.table.as_mut().unwrap().all(id == ALL);
                }
                SAVE_SELECTION => {
                    let selection = &ui.table.as_ref().unwrap().selection;
                    if selection.include.is_empty() {
                        msg(hwnd, l::t("selection.invalid"), MB_OK | MB_ICONINFORMATION);
                        return 0;
                    }
                    let choice = selection.clone();
                    if let Some(folder) = browse(hwnd) {
                        let path = folder.join("FreePort-selection.json");
                        if msg(
                            hwnd,
                            &format!("{}\n{}", l::t("selection.save_confirm"), path.display()),
                            MB_OKCANCEL | MB_ICONQUESTION,
                        ) == IDOK
                        {
                            if let Err(e) = p::atomic_json(&path, &choice) {
                                msg(hwnd, &l::describe_error(e.as_ref()), MB_OK | MB_ICONERROR);
                            }
                        }
                    }
                }
                LOAD_SELECTION => {
                    if let Some(path) = rule_file(hwnd) {
                        let result = (|| -> freeport::Result<selection::Selection> {
                            let bytes = std::fs::read(path)?;
                            if bytes.len() > 1024 * 1024 {
                                return freeport::fail(l::t("selection.limit"));
                            }
                            let choice: selection::Selection = serde_json::from_slice(&bytes)?;
                            if SendMessageW(ui.mode, CB_GETCURSEL, 0, 0) == 0 {
                                choice.validate(&ui.rows)?;
                            } else {
                                choice.restore_sources(&ui.rows)?;
                            }
                            Ok(choice)
                        })();
                        match result {
                            Ok(choice) => {
                                ui.table.as_mut().unwrap().selection = choice;
                                ui.table.as_ref().unwrap().sync();
                            }
                            Err(e) => {
                                msg(hwnd, &l::describe_error(e.as_ref()), MB_OK | MB_ICONERROR);
                            }
                        }
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
                        ui.table.as_mut().unwrap().selection = Default::default();
                        ui.table.as_mut().unwrap().rebuild(&ui.rows, false);
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
                ABOUT => about(ui),
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
                    Event::Invalidate => {
                        ui.rows.clear();
                        ui.table.as_mut().unwrap().selection = Default::default();
                        ui.table.as_mut().unwrap().rebuild(&ui.rows, false);
                    }
                    Event::Report(report) => {
                        let restore = report.mode == "restore";
                        ui.rows = report.rows;
                        ui.table.as_mut().unwrap().rebuild(&ui.rows, restore);
                        for backend in report.backends {
                            log(ui, backend);
                        }
                        for warning in report.warnings {
                            log(ui, warning);
                        }
                    }
                    Event::Measured(i, r) => {
                        if i < ui.rows.len() {
                            ui.rows[i] = r;
                            row(ui, i);
                        }
                    }
                    Event::Prepared(result) => {
                        busy(ui, false);
                        match result {
                            Ok(prepared) => {
                                if msg(hwnd, &prepared.preview, MB_OKCANCEL | MB_ICONQUESTION)
                                    == IDOK
                                {
                                    setcheck(ui.request, false);
                                    setcheck(ui.force, false);
                                    setcheck(ui.elevate, false);
                                    launch(ui, prepared.job, prepared.control, prepared.elevate);
                                }
                            }
                            Err(e) => {
                                log(ui, e.clone());
                                msg(hwnd, &e, MB_OK | MB_ICONERROR);
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
            DeleteObject(ui.title_font);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, message, w, l),
    }
}
fn main() {
    if elevation::maybe_worker() {
        return;
    }
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
            dwICC: ICC_LISTVIEW_CLASSES | ICC_TREEVIEW_CLASSES,
        };
        InitCommonControlsEx(&common);
        let class = p::wide("FreePortNativeWindow");
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(wnd);
        wc.hInstance = GetModuleHandleW(ptr::null());
        wc.hCursor = LoadCursorW(ptr::null_mut(), IDC_ARROW);
        // Resource 1 is the application icon embedded by build.rs.
        wc.hIcon = LoadIconW(wc.hInstance, 1 as _);
        wc.hbrBackground = (COLOR_WINDOW + 1) as _;
        wc.lpszClassName = class.as_ptr();
        RegisterClassW(&wc);
        let mut ui = Box::new(Ui::default());
        ui.requested = choice.requested_language;
        // Size the window in logical units for the system DPI, inside the work area.
        let screen = GetDC(ptr::null_mut());
        let dpi = GetDeviceCaps(screen, LOGPIXELSX as i32).max(96);
        ReleaseDC(ptr::null_mut(), screen);
        let mut area: RECT = std::mem::zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut area as *mut _ as _, 0);
        let width = (1180 * dpi / 96).min(area.right - area.left);
        let height = (780 * dpi / 96).min(area.bottom - area.top);
        let x = area.left + (area.right - area.left - width) / 2;
        let y = area.top + (area.bottom - area.top - height) / 2;
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class.as_ptr(),
            p::wide(format!(
                "{} — {}",
                l::t("msg.7540ee4199"),
                env!("CARGO_PKG_VERSION")
            ))
            .as_ptr(),
            WS_OVERLAPPEDWINDOW,
            x,
            y,
            width,
            height,
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
            ui.table.as_mut().unwrap().rebuild(&ui.rows, false);
            ui.table
                .as_mut()
                .unwrap()
                .selection
                .set(&ui.rows[0].candidate.source, true);
            ui.table.as_ref().unwrap().sync();
            let ids = selected(&ui);
            let destination = read(ui.target);
            let mut labels = std::collections::BTreeMap::new();
            for tag in l::TAGS {
                l::set(tag).unwrap();
                ui.requested = tag.into();
                refresh(&mut ui);
                switches_preserved &= selected(&ui) == ids && read(ui.target) == destination;
                labels.insert(tag, read(ui.description));
            }
            let dialogs_localized = native_dialog::self_test(hwnd);
            let tree_checks = ui.table.as_mut().unwrap().self_test(&ui.rows);
            println!(
                "{}",
                serde_json::json!({"window_is_native":true,"tree_checks":tree_checks,"administrator_option":check(ui.elevate),"language":initial_language,"request_app_exit":!defaults.0,"force_close_handles":!defaults.1,"selected_count":if defaults.2{0}else{1},"controls_count":ui.controls.len(),"language_switch_preserves_state":switches_preserved,"dialogs_localized":dialogs_localized,"labels":labels})
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
