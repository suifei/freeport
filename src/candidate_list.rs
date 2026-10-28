//! Native candidate list: one row per discovered directory. Checkbox intent lives in the
//! shared selection model; the view only mirrors it.
use savemycdrive::{i18n as l, inventory::Row, platform as p, selection::Selection};
use std::path::PathBuf;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::{GetSysColor, InvalidateRect, COLOR_GRAYTEXT},
    Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY,
    UI::{Controls::*, Shell::*, WindowsAndMessaging::*},
};
thread_local! { static INTERNAL:std::cell::Cell<u32>=const { std::cell::Cell::new(0) }; }
/// True while the list itself changes item state; notifications are then not user intent.
pub fn internal() -> bool {
    INTERNAL.with(|v| v.get() != 0)
}
struct Internal;
impl Internal {
    fn enter() -> Self {
        INTERNAL.with(|v| v.set(v.get() + 1));
        Self
    }
}
impl Drop for Internal {
    fn drop(&mut self) {
        INTERNAL.with(|v| v.set(v.get() - 1));
    }
}
const COLUMNS: [&str; 4] = [
    "column.folder",
    "column.total",
    "column.application",
    "column.status",
];
const STATE_SHIFT: u32 = 12;
pub struct List {
    pub hwnd: HWND,
    pub selection: Selection,
    restore: bool,
    /// Per row: total bytes (None while unmeasured or for links), application, status, pending.
    cells: Vec<(Option<u64>, String, String, bool)>,
    paths: Vec<PathBuf>,
    allowed: Vec<bool>,
    sort: (usize, bool),
    width: std::cell::Cell<i32>,
}
impl List {
    pub fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            selection: Selection::default(),
            restore: false,
            cells: vec![],
            paths: vec![],
            allowed: vec![],
            sort: (1, false),
            width: std::cell::Cell::new(0),
        }
    }
    pub unsafe fn init(&self) {
        SetWindowTheme(self.hwnd, p::wide("Explorer").as_ptr(), std::ptr::null());
        let ex = LVS_EX_CHECKBOXES | LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_LABELTIP;
        SendMessageW(
            self.hwnd,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            ex as usize,
            ex as isize,
        );
        for (i, key) in COLUMNS.iter().enumerate() {
            let mut title = p::wide(l::t(key));
            let column = LVCOLUMNW {
                mask: LVCF_TEXT | LVCF_WIDTH | LVCF_FMT | LVCF_SUBITEM,
                fmt: if i == 1 { LVCFMT_RIGHT } else { LVCFMT_LEFT },
                cx: 100,
                pszText: title.as_mut_ptr(),
                iSubItem: i as i32,
                ..std::mem::zeroed()
            };
            SendMessageW(
                self.hwnd,
                LVM_INSERTCOLUMNW,
                i,
                &column as *const _ as isize,
            );
        }
        // The shell's shared image list must not be destroyed with the control (LVS_SHAREIMAGELISTS).
        let mut info: SHFILEINFOW = std::mem::zeroed();
        let images = SHGetFileInfoW(
            p::wide("folder").as_ptr(),
            FILE_ATTRIBUTE_DIRECTORY,
            &mut info,
            std::mem::size_of_val(&info) as u32,
            SHGFI_SYSICONINDEX | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES,
        );
        if images != 0 {
            SendMessageW(
                self.hwnd,
                LVM_SETIMAGELIST,
                LVSIL_SMALL as usize,
                images as isize,
            );
        }
        self.sort_arrow();
    }
    pub unsafe fn rebuild(&mut self, rows: &[Row], restore: bool) {
        let _internal = Internal::enter();
        SendMessageW(self.hwnd, WM_SETREDRAW, 0, 0);
        SendMessageW(self.hwnd, LVM_DELETEALLITEMS, 0, 0);
        self.restore = restore;
        self.paths = rows.iter().map(|r| r.candidate.source.clone()).collect();
        self.cells = rows.iter().map(cells).collect();
        self.allowed = rows.iter().map(|r| allowed(r, restore)).collect();
        let mut info: SHFILEINFOW = std::mem::zeroed();
        SHGetFileInfoW(
            p::wide("folder").as_ptr(),
            FILE_ATTRIBUTE_DIRECTORY,
            &mut info,
            std::mem::size_of_val(&info) as u32,
            SHGFI_SYSICONINDEX | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES,
        );
        for (i, r) in rows.iter().enumerate() {
            let mut text = p::wide(r.candidate.source.display().to_string());
            let item = LVITEMW {
                mask: LVIF_TEXT | LVIF_PARAM | LVIF_IMAGE,
                iItem: i as i32,
                pszText: text.as_mut_ptr(),
                iImage: info.iIcon,
                lParam: i as isize,
                ..std::mem::zeroed()
            };
            let index = SendMessageW(self.hwnd, LVM_INSERTITEMW, 0, &item as *const _ as isize);
            self.texts(index as usize, i);
        }
        self.sync();
        self.order();
        self.relabel();
        SendMessageW(self.hwnd, WM_SETREDRAW, 1, 0);
        InvalidateRect(self.hwnd, std::ptr::null(), 1);
    }
    unsafe fn find(&self, row: usize) -> Option<usize> {
        let find = LVFINDINFOW {
            flags: LVFI_PARAM,
            lParam: row as isize,
            ..std::mem::zeroed()
        };
        let index = SendMessageW(
            self.hwnd,
            LVM_FINDITEMW,
            usize::MAX,
            &find as *const _ as isize,
        );
        (index >= 0).then_some(index as usize)
    }
    unsafe fn row_at(&self, index: usize) -> Option<usize> {
        let mut item = LVITEMW {
            mask: LVIF_PARAM,
            iItem: index as i32,
            ..std::mem::zeroed()
        };
        (SendMessageW(self.hwnd, LVM_GETITEMW, 0, &mut item as *mut _ as isize) != 0)
            .then_some(item.lParam as usize)
            .filter(|r| *r < self.paths.len())
    }
    unsafe fn texts(&self, index: usize, row: usize) {
        let (bytes, application, status, pending) = &self.cells[row];
        let values = [
            bytes.map(savemycdrive::human).unwrap_or_else(|| {
                // Links are never measured; only show "counting" while a measure is due.
                if *pending {
                    l::t("msg.8284301049")
                } else {
                    "—"
                }
                .into()
            }),
            application.clone(),
            status.clone(),
        ];
        for (column, value) in values.iter().enumerate() {
            let mut value = p::wide(value);
            let item = LVITEMW {
                iSubItem: column as i32 + 1,
                pszText: value.as_mut_ptr(),
                ..std::mem::zeroed()
            };
            SendMessageW(
                self.hwnd,
                LVM_SETITEMTEXTW,
                index,
                &item as *const _ as isize,
            );
        }
    }
    pub unsafe fn update(&mut self, rows: &[Row], row: usize) {
        let _internal = Internal::enter();
        let Some(r) = rows.get(row) else { return };
        if row >= self.cells.len() {
            return;
        }
        self.cells[row] = cells(r);
        self.allowed[row] = allowed(r, self.restore);
        if let Some(index) = self.find(row) {
            self.texts(index, row);
        }
        self.order();
        self.sync();
    }
    pub unsafe fn sync(&self) {
        let _internal = Internal::enter();
        for (row, path) in self.paths.iter().enumerate() {
            let Some(index) = self.find(row) else {
                continue;
            };
            let checked = self.selection.contains(path);
            let item = LVITEMW {
                stateMask: LVIS_STATEIMAGEMASK,
                state: (if checked { 2 } else { 1 }) << STATE_SHIFT,
                ..std::mem::zeroed()
            };
            SendMessageW(
                self.hwnd,
                LVM_SETITEMSTATE,
                index,
                &item as *const _ as isize,
            );
        }
    }
    /// A checkbox click or Space on a row. Returns true to block the native toggle;
    /// the caller then re-syncs every row from the selection model.
    pub unsafe fn changing(&mut self, change: &NMLISTVIEW) -> bool {
        if change.uChanged & LVIF_STATE == 0
            || (change.uNewState ^ change.uOldState) & LVIS_STATEIMAGEMASK == 0
            || change.uOldState & LVIS_STATEIMAGEMASK == 0
        {
            return false;
        }
        let row = change.lParam as usize;
        if let (Some(path), Some(true)) = (self.paths.get(row), self.allowed.get(row)) {
            let on = !self.selection.contains(path);
            self.selection.set(&path.clone(), on);
        }
        true
    }
    pub unsafe fn all(&mut self, on: bool) {
        self.selection = Selection::default();
        if on {
            for (path, allowed) in self.paths.iter().zip(&self.allowed) {
                if *allowed {
                    self.selection.set(path, true);
                }
            }
        }
        self.sync();
    }
    pub unsafe fn column_click(&mut self, column: usize) {
        self.sort = if self.sort.0 == column {
            (column, !self.sort.1)
        } else {
            // Sizes read best largest first; text columns alphabetically.
            (column, column != 1)
        };
        self.order();
        self.sort_arrow();
    }
    unsafe fn order(&self) {
        unsafe extern "system" fn compare(a: isize, b: isize, context: isize) -> i32 {
            let list = &*(context as *const List);
            let (a, b) = (a as usize, b as usize);
            let (Some(x), Some(y)) = (list.cells.get(a), list.cells.get(b)) else {
                return 0;
            };
            let key = |i: usize| p::key(&list.paths[i]);
            let order = match list.sort.0 {
                1 => x.0.cmp(&y.0),
                2 => x.1.to_lowercase().cmp(&y.1.to_lowercase()),
                3 => x.2.to_lowercase().cmp(&y.2.to_lowercase()),
                _ => key(a).cmp(&key(b)),
            };
            let order = if list.sort.1 { order } else { order.reverse() };
            match order.then_with(|| key(a).cmp(&key(b))) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            }
        }
        SendMessageW(
            self.hwnd,
            LVM_SORTITEMS,
            self as *const _ as usize,
            compare as unsafe extern "system" fn(isize, isize, isize) -> i32 as usize as isize,
        );
    }
    unsafe fn sort_arrow(&self) {
        let header = SendMessageW(self.hwnd, LVM_GETHEADER, 0, 0) as HWND;
        for i in 0..COLUMNS.len() {
            let mut item = HDITEMW {
                mask: HDI_FORMAT,
                ..std::mem::zeroed()
            };
            SendMessageW(header, HDM_GETITEMW, i, &mut item as *mut _ as isize);
            item.fmt &= !(HDF_SORTUP | HDF_SORTDOWN);
            if i == self.sort.0 {
                item.fmt |= if self.sort.1 {
                    HDF_SORTUP
                } else {
                    HDF_SORTDOWN
                };
            }
            SendMessageW(header, HDM_SETITEMW, i, &item as *const _ as isize);
        }
    }
    pub unsafe fn relabel(&self) {
        for (i, key) in COLUMNS.iter().enumerate() {
            let mut title = p::wide(l::t(key));
            let column = LVCOLUMNW {
                mask: LVCF_TEXT,
                pszText: title.as_mut_ptr(),
                ..std::mem::zeroed()
            };
            SendMessageW(self.hwnd, LVM_SETCOLUMNW, i, &column as *const _ as isize);
        }
    }
    /// Spread column widths when the list width changes; user-dragged widths otherwise stay.
    pub unsafe fn layout(&self) {
        let mut rect: RECT = std::mem::zeroed();
        GetClientRect(self.hwnd, &mut rect);
        let width = rect.right - rect.left;
        if width <= 0 || self.width.replace(width) == width {
            return;
        }
        for (i, percent) in [44, 13, 18, 25].iter().enumerate() {
            SendMessageW(
                self.hwnd,
                LVM_SETCOLUMNWIDTH,
                i,
                (width * percent / 100) as isize,
            );
        }
    }
    /// Gray out rows that cannot be checked in the current mode.
    pub unsafe fn draw(&self, address: isize) -> isize {
        let draw = &mut *(address as *mut NMLVCUSTOMDRAW);
        match draw.nmcd.dwDrawStage {
            CDDS_PREPAINT => CDRF_NOTIFYITEMDRAW as isize,
            CDDS_ITEMPREPAINT => {
                let row = draw.nmcd.lItemlParam as usize;
                if self.allowed.get(row) == Some(&false) {
                    draw.clrText = GetSysColor(COLOR_GRAYTEXT);
                }
                CDRF_DODEFAULT as isize
            }
            _ => CDRF_DODEFAULT as isize,
        }
    }
    pub fn details(&self, index: usize, rows: &[Row]) -> String {
        let row = unsafe { self.row_at(index) };
        let Some(r) = row.and_then(|i| rows.get(i)) else {
            return String::new();
        };
        let mut details = vec![r.candidate.source.display().to_string()];
        if let Some(k) = &r.candidate.knowledge {
            details.push(k.purpose_text().to_string());
        }
        details.push(self.cells[row.unwrap()].2.clone());
        for link in &r.stats.link_details {
            details.push(format!(
                "{} → {}",
                link.path.display(),
                link.target
                    .as_ref()
                    .map(|t| t.display().to_string())
                    .unwrap_or_else(|| l::t("status.unreadable").into())
            ));
        }
        details.join("\r\n")
    }
    pub unsafe fn self_test(&mut self, rows: &[Row]) -> bool {
        let mut class = [0u16; 64];
        GetClassNameW(self.hwnd, class.as_mut_ptr(), 64);
        let native =
            String::from_utf16_lossy(&class[..class.iter().position(|n| *n == 0).unwrap_or(64)])
                == "SysListView32";
        let ex = SendMessageW(self.hwnd, LVM_GETEXTENDEDLISTVIEWSTYLE, 0, 0) as u32;
        let styled = ex & LVS_EX_CHECKBOXES != 0 && ex & LVS_EX_FULLROWSELECT != 0;
        let Some(path) = rows.first().map(|r| r.candidate.source.clone()) else {
            return false;
        };
        let checked = |list: &Self| {
            let index = list.find(0).unwrap_or(usize::MAX);
            (SendMessageW(
                list.hwnd,
                LVM_GETITEMSTATE,
                index,
                LVIS_STATEIMAGEMASK as isize,
            ) as u32
                >> STATE_SHIFT)
                == 2
        };
        let saved = self.selection.clone();
        self.selection.set(&path, true);
        self.sync();
        let mirrored = checked(self);
        let root = PathBuf::from(r"E:\savemycdrive-sort-fixture");
        let make = |relative: &str, bytes: u64| Row {
            candidate: savemycdrive::catalog::custom(
                &root.join(relative),
                "sort fixture",
                "custom",
            ),
            stats: savemycdrive::inventory::Stats {
                bytes,
                ..Default::default()
            },
            status: "ready".into(),
            link_target: None,
            busy: vec![],
        };
        let fixture = vec![make("small", 2), make("large", 100), make("middle", 50)];
        self.rebuild(&fixture, false);
        let flat = SendMessageW(self.hwnd, LVM_GETITEMCOUNT, 0, 0) == 3;
        let sorted = self.row_at(0) == Some(1) && self.row_at(2) == Some(0);
        self.rebuild(rows, false);
        self.selection = saved;
        self.sync();
        native && styled && mirrored && flat && sorted
    }
}
fn allowed(r: &Row, restore: bool) -> bool {
    if restore {
        r.status == "linked" && r.link_target.is_some()
    } else {
        r.status == "ready" && r.stats.links == 0 && r.stats.errors == 0
    }
}
fn cells(r: &Row) -> (Option<u64>, String, String, bool) {
    let bytes = (!["unmeasured", "linked"].contains(&r.status.as_str())).then_some(r.stats.bytes);
    let application = r
        .candidate
        .knowledge
        .as_ref()
        .map(|k| {
            if k.migration == "review" {
                l::format("rule.review", &[k.application.clone()])
            } else {
                k.application.clone()
            }
        })
        .unwrap_or_else(|| r.candidate.name.clone());
    let mut status = r
        .link_target
        .as_ref()
        .map(|target| l::format("msg.0ca6431378", &[target.display().to_string()]))
        .unwrap_or_else(|| l::t(&format!("status.{}", r.status)).into());
    if r.stats.links > 0 {
        status.push_str(&format!(" ({})", r.stats.links));
    }
    (bytes, application, status, r.status == "unmeasured")
}
