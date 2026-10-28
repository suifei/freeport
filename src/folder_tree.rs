//! Native folder tree; all checkbox intent lives in the shared selection model.
use freeport::{i18n as l, inventory::Row, platform as p, selection::Selection};
use std::{collections::BTreeMap, fs, path::PathBuf};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::{
        DrawTextW, FillRect, GetSysColor, GetSysColorBrush, InvalidateRect, ScreenToClient,
        SetBkMode, SetTextColor, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW,
        COLOR_WINDOWTEXT, DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, TRANSPARENT,
    },
    Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY,
    UI::{Controls::*, Shell::*, WindowsAndMessaging::*},
};
thread_local! { static INTERNAL:std::cell::Cell<u32>=const { std::cell::Cell::new(0) }; }
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
struct Node {
    path: PathBuf,
    item: HTREEITEM,
    row: Option<usize>,
    loaded: bool,
    navigation: bool,
    bytes: Option<u64>,
    application: String,
    status: String,
}
pub struct Tree {
    pub hwnd: HWND,
    pub selection: Selection,
    nodes: Vec<Node>,
    restore: bool,
    header: std::cell::Cell<HWND>,
    width: std::cell::Cell<i32>,
}
impl Tree {
    pub fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            selection: Selection::default(),
            nodes: vec![],
            restore: false,
            header: std::cell::Cell::new(std::ptr::null_mut()),
            width: std::cell::Cell::new(0),
        }
    }
    pub unsafe fn init(&self) {
        let header = CreateWindowExW(
            0,
            p::wide("SysHeader32").as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE | HDS_FULLDRAG,
            0,
            0,
            0,
            0,
            GetParent(self.hwnd),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        self.header.set(header);
        SendMessageW(
            header,
            WM_SETFONT,
            SendMessageW(self.hwnd, WM_GETFONT, 0, 0) as usize,
            0,
        );
        for (i, key) in [
            "column.folder",
            "column.total",
            "column.application",
            "column.status",
        ]
        .iter()
        .enumerate()
        {
            let mut title = p::wide(l::t(key));
            let item = HDITEMW {
                mask: HDI_TEXT | HDI_WIDTH | HDI_FORMAT,
                cxy: 100,
                pszText: title.as_mut_ptr(),
                fmt: HDF_LEFT as i32,
                ..std::mem::zeroed()
            };
            SendMessageW(header, HDM_INSERTITEMW, i, &item as *const _ as isize);
        }
        // Common Controls v6 supplies checked/unchecked/partial state images on Win7.
        let style = GetWindowLongPtrW(self.hwnd, GWL_STYLE) as u32;
        SetWindowLongPtrW(
            self.hwnd,
            GWL_STYLE,
            (style | TVS_CHECKBOXES | TVS_NOHSCROLL) as isize,
        );
        SendMessageW(
            self.hwnd,
            TVM_SETEXTENDEDSTYLE,
            TVS_EX_PARTIALCHECKBOXES as usize,
            TVS_EX_PARTIALCHECKBOXES as isize,
        );
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
                TVM_SETIMAGELIST,
                TVSIL_NORMAL as usize,
                images as isize,
            );
        }
    }
    pub unsafe fn rebuild(&mut self, rows: &[Row], restore: bool) {
        let _internal = Internal::enter();
        SendMessageW(self.hwnd, WM_SETREDRAW, 0, 0);
        SendMessageW(self.hwnd, TVM_DELETEITEM, 0, TVI_ROOT);
        self.nodes.clear();
        self.restore = restore;
        let mut paths = BTreeMap::new();
        for (i, r) in rows.iter().enumerate() {
            for path in r.candidate.source.ancestors() {
                let entry = paths
                    .entry(p::key(path))
                    .or_insert((path.to_path_buf(), None));
                if p::same(path, &r.candidate.source) {
                    entry.1 = Some(i);
                }
            }
        }
        let mut paths: Vec<_> = paths.into_values().collect();
        paths.sort_by_key(|(p, _)| (p.components().count(), p::key(p)));
        for (path, row) in paths {
            let parent = path
                .parent()
                .and_then(|p| self.nodes.iter().find(|n| p::same(&n.path, p)))
                .map(|n| n.item)
                .unwrap_or(TVI_ROOT);
            let navigation = !rows.iter().any(|r| p::within(&path, &r.candidate.source));
            self.insert(path, row, parent, navigation, rows);
        }
        for n in &self.nodes {
            if n.navigation {
                SendMessageW(self.hwnd, TVM_EXPAND, TVE_EXPAND as usize, n.item);
            }
        }
        self.sync();
        self.refresh_metrics(rows);
        self.sort();
        self.relabel_header();
        SendMessageW(self.hwnd, WM_SETREDRAW, 1, 0);
        InvalidateRect(self.hwnd, std::ptr::null(), 1);
    }
    unsafe fn insert(
        &mut self,
        path: PathBuf,
        row: Option<usize>,
        parent: HTREEITEM,
        navigation: bool,
        rows: &[Row],
    ) {
        let label = label(&path, row.and_then(|i| rows.get(i)));
        let mut text = p::wide(label);
        let mut info: SHFILEINFOW = std::mem::zeroed();
        SHGetFileInfoW(
            p::wide("folder").as_ptr(),
            FILE_ATTRIBUTE_DIRECTORY,
            &mut info,
            std::mem::size_of_val(&info) as u32,
            SHGFI_SYSICONINDEX | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES,
        );
        let linked = row
            .and_then(|i| rows.get(i))
            .is_some_and(|r| r.link_target.is_some() || r.status == "linked");
        let item = TVITEMW {
            mask: TVIF_TEXT | TVIF_PARAM | TVIF_CHILDREN | TVIF_IMAGE | TVIF_SELECTEDIMAGE,
            pszText: text.as_mut_ptr(),
            lParam: self.nodes.len() as isize,
            cChildren: if linked { 0 } else { 1 },
            iImage: info.iIcon,
            iSelectedImage: info.iIcon,
            ..std::mem::zeroed()
        };
        let insertion = TVINSERTSTRUCTW {
            hParent: parent,
            hInsertAfter: TVI_LAST,
            Anonymous: TVINSERTSTRUCTW_0 { item },
        };
        let handle = SendMessageW(
            self.hwnd,
            TVM_INSERTITEMW,
            0,
            &insertion as *const _ as isize,
        );
        self.nodes.push(Node {
            path,
            item: handle,
            row,
            loaded: linked,
            navigation,
            bytes: None,
            application: String::new(),
            status: String::new(),
        });
    }
    pub unsafe fn update(&mut self, rows: &[Row], index: usize) {
        let _internal = Internal::enter();
        if let Some(n) = self.nodes.iter().find(|n| n.row == Some(index)) {
            let mut s = p::wide(label(&n.path, rows.get(index)));
            let item = TVITEMW {
                mask: TVIF_TEXT,
                hItem: n.item,
                pszText: s.as_mut_ptr(),
                ..std::mem::zeroed()
            };
            SendMessageW(self.hwnd, TVM_SETITEMW, 0, &item as *const _ as isize);
        }
        self.refresh_metrics(rows);
        self.sort();
        InvalidateRect(self.hwnd, std::ptr::null(), 0);
        self.sync();
    }
    pub unsafe fn sync(&self) {
        let _internal = Internal::enter();
        for n in &self.nodes {
            let item = TVITEMW {
                mask: TVIF_STATE,
                hItem: n.item,
                stateMask: TVIS_STATEIMAGEMASK,
                state: self.selection.state(&n.path) << 12,
                ..std::mem::zeroed()
            };
            SendMessageW(self.hwnd, TVM_SETITEMW, 0, &item as *const _ as isize);
        }
    }
    fn allowed(&self, n: &Node, rows: &[Row]) -> bool {
        if n.navigation {
            return false;
        }
        if let Some(i) = n.row {
            return if self.restore {
                rows[i].status == "linked" && rows[i].link_target.is_some()
            } else {
                rows[i].status == "ready" && rows[i].stats.links == 0 && rows[i].stats.errors == 0
            };
        }
        !self.restore
            && fs::symlink_metadata(&n.path)
                .map(|m| m.is_dir() && !p::reparse(&m))
                .unwrap_or(false)
    }
    pub unsafe fn toggle(&mut self, item: HTREEITEM, rows: &[Row]) {
        let Some(index) = self.nodes.iter().position(|n| n.item == item) else {
            return;
        };
        let n = &self.nodes[index];
        let on = self.selection.state(&n.path) != 2;
        if !on {
            self.selection.set(&n.path, false);
        } else if n.navigation {
            for r in rows {
                if p::within(&r.candidate.source, &n.path)
                    && (if self.restore {
                        r.status == "linked" && r.link_target.is_some()
                    } else {
                        r.status == "ready" && r.stats.links == 0
                    })
                {
                    self.selection.set(&r.candidate.source, true);
                }
            }
        } else if self.allowed(n, rows) {
            self.selection.set(&n.path, true);
        }
        self.sync();
    }
    pub unsafe fn all(&mut self, rows: &[Row], on: bool) {
        self.selection = Selection::default();
        if on {
            for n in &self.nodes {
                if n.row.is_some() && self.allowed(n, rows) {
                    self.selection.set(&n.path, true);
                }
            }
        }
        self.sync();
    }
    pub unsafe fn expand(&mut self, item: HTREEITEM, rows: &[Row]) -> freeport::Result<()> {
        let _internal = Internal::enter();
        let Some(index) = self.nodes.iter().position(|n| n.item == item) else {
            return Ok(());
        };
        if self.nodes[index].loaded || self.nodes[index].navigation || self.restore {
            return Ok(());
        }
        let path = self.nodes[index].path.clone();
        if p::reparse(&fs::symlink_metadata(&path)?) {
            return Ok(());
        }
        // Read only this branch. Never recurse or follow links while expanding the view.
        let mut children = vec![];
        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            let m = fs::symlink_metadata(entry.path())?;
            if m.is_dir() || p::reparse(&m) {
                children.push((entry.path(), p::reparse(&m)));
            }
            if children.len() > 20000 {
                return freeport::fail(l::t("selection.limit"));
            }
        }
        self.nodes[index].loaded = true;
        for (path, linked) in children {
            if self.nodes.iter().any(|n| p::same(&n.path, &path)) {
                continue;
            }
            self.insert(path, None, item, false, rows);
            if linked {
                self.nodes.last_mut().unwrap().loaded = true;
            }
        }
        self.sync();
        self.refresh_metrics(rows);
        self.sort();
        Ok(())
    }
    unsafe fn relabel_header(&self) {
        for (i, key) in [
            "column.folder",
            "column.total",
            "column.application",
            "column.status",
        ]
        .iter()
        .enumerate()
        {
            let mut title = p::wide(l::t(key));
            let item = HDITEMW {
                mask: HDI_TEXT,
                pszText: title.as_mut_ptr(),
                ..std::mem::zeroed()
            };
            SendMessageW(
                self.header.get(),
                HDM_SETITEMW,
                i,
                &item as *const _ as isize,
            );
        }
    }
    pub unsafe fn layout_header(&self, dpi: i32) {
        let mut rect: RECT = std::mem::zeroed();
        GetWindowRect(self.hwnd, &mut rect);
        let mut origin = POINT {
            x: rect.left,
            y: rect.top,
        };
        ScreenToClient(GetParent(self.hwnd), &mut origin);
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        let header_height = 25 * dpi / 96;
        MoveWindow(
            self.header.get(),
            origin.x,
            origin.y,
            width,
            header_height,
            1,
        );
        MoveWindow(
            self.hwnd,
            origin.x,
            origin.y + header_height,
            width,
            (height - header_height).max(1),
            1,
        );
        if self.width.replace(width) != width {
            for (i, percent) in [40, 15, 20, 25].iter().enumerate() {
                let item = HDITEMW {
                    mask: HDI_WIDTH,
                    cxy: width * percent / 100,
                    ..std::mem::zeroed()
                };
                SendMessageW(
                    self.header.get(),
                    HDM_SETITEMW,
                    i,
                    &item as *const _ as isize,
                );
            }
        }
    }
    pub unsafe fn header_notification(&self, hdr: &NMHDR) -> bool {
        if hdr.hwndFrom != self.header.get() {
            return false;
        }
        InvalidateRect(self.hwnd, std::ptr::null(), 1);
        true
    }
    pub fn details(&self, item: HTREEITEM, rows: &[Row]) -> String {
        let Some(n) = self.nodes.iter().find(|n| n.item == item) else {
            return String::new();
        };
        let mut details = vec![n.path.display().to_string(), n.status.clone()];
        for r in rows {
            if p::within(&n.path, &r.candidate.source) || p::within(&r.candidate.source, &n.path) {
                if p::same(&n.path, &r.candidate.source) {
                    if let Some(k) = &r.candidate.knowledge {
                        details.push(k.purpose.clone());
                    }
                }
                for link in &r.stats.link_details {
                    if p::within(&link.path, &n.path) {
                        details.push(format!(
                            "{} → {}",
                            link.path.display(),
                            link.target
                                .as_ref()
                                .map(|t| t.display().to_string())
                                .unwrap_or_else(|| l::t("status.unreadable").into())
                        ));
                    }
                }
            }
        }
        details.join("\r\n")
    }
    fn refresh_metrics(&mut self, rows: &[Row]) {
        let mut totals = BTreeMap::new();
        let mut links = BTreeMap::new();
        for r in rows {
            for total in &r.stats.directory_totals {
                totals.insert(
                    p::key(&total.path),
                    (total.bytes, total.links, total.errors),
                );
            }
            for link in &r.stats.link_details {
                links.insert(p::key(&link.path), link.target.clone());
            }
        }
        for n in &mut self.nodes {
            n.bytes = None;
            n.application.clear();
            n.status.clear();
            if let Some(i) = n.row {
                let r = &rows[i];
                if !["unmeasured", "linked"].contains(&r.status.as_str()) {
                    n.bytes = Some(r.stats.bytes);
                }
                n.application = r
                    .candidate
                    .knowledge
                    .as_ref()
                    .map(|k| k.application.clone())
                    .unwrap_or_else(|| r.candidate.name.clone());
                n.status = r
                    .link_target
                    .as_ref()
                    .map(|target| l::format("msg.0ca6431378", &[target.display().to_string()]))
                    .unwrap_or_else(|| l::t(&format!("status.{}", r.status)).into());
                if r.stats.links > 0 {
                    n.status.push_str(&format!(" ({})", r.stats.links));
                }
            } else if let Some((bytes, count, errors)) = totals.get(&p::key(&n.path)) {
                n.bytes = Some(*bytes);
                n.status = l::t(if *errors > 0 {
                    "status.unreadable"
                } else if *count > 0 {
                    "status.nested_links"
                } else {
                    "status.ready"
                })
                .into();
            } else if let Some(target) = links.get(&p::key(&n.path)) {
                n.status = target
                    .as_ref()
                    .map(|t| l::format("msg.0ca6431378", &[t.display().to_string()]))
                    .unwrap_or_else(|| l::t("status.linked").into());
            } else if n.navigation {
                let mut covered: Vec<&std::path::Path> = vec![];
                let mut total = 0u64;
                let mut pending = false;
                let mut children: Vec<_> = rows
                    .iter()
                    .filter(|r| p::within(&r.candidate.source, &n.path))
                    .collect();
                children.sort_by_key(|r| r.candidate.source.components().count());
                for r in children {
                    if covered.iter().any(|p| p::within(&r.candidate.source, p)) {
                        continue;
                    }
                    covered.push(&r.candidate.source);
                    pending |= ["unmeasured", "linked"].contains(&r.status.as_str());
                    total = total.saturating_add(r.stats.bytes);
                }
                if !pending {
                    n.bytes = Some(total);
                }
                n.status = l::t("column.candidate_total").into();
            }
        }
    }
    unsafe fn sort(&self) {
        unsafe extern "system" fn compare(a: isize, b: isize, context: isize) -> i32 {
            let tree = &*(context as *const Tree);
            let Some(a) = tree.nodes.get(a as usize) else {
                return 0;
            };
            let Some(b) = tree.nodes.get(b as usize) else {
                return 0;
            };
            let order = b
                .bytes
                .cmp(&a.bytes)
                .then_with(|| p::key(&a.path).cmp(&p::key(&b.path)));
            match order {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            }
        }
        // SORTCHILDRENCB requires wParam=0 and sorts only one sibling group.
        // Explicitly visit each loaded parent; item IDs and intent stay stable.
        for parent in std::iter::once(TVI_ROOT).chain(self.nodes.iter().map(|n| n.item)) {
            let sorting = TVSORTCB {
                hParent: parent,
                lpfnCompare: Some(compare),
                lParam: self as *const _ as isize,
            };
            SendMessageW(
                self.hwnd,
                TVM_SORTCHILDRENCB,
                0,
                &sorting as *const _ as isize,
            );
        }
    }
    pub unsafe fn draw(&self, address: isize) -> isize {
        let draw = &*(address as *const NMTVCUSTOMDRAW);
        match draw.nmcd.dwDrawStage {
            CDDS_PREPAINT => CDRF_NOTIFYITEMDRAW as isize,
            stage if stage == CDDS_ITEMPREPAINT => CDRF_NOTIFYPOSTPAINT as isize,
            stage if stage == CDDS_ITEMPOSTPAINT => {
                let Some(node) = self.nodes.get(draw.nmcd.lItemlParam as usize) else {
                    return CDRF_DODEFAULT as isize;
                };
                let selected = draw.nmcd.uItemState & CDIS_SELECTED != 0;
                let background = if selected {
                    COLOR_HIGHLIGHT
                } else {
                    COLOR_WINDOW
                };
                SetBkMode(draw.nmcd.hdc, TRANSPARENT as i32);
                SetTextColor(
                    draw.nmcd.hdc,
                    GetSysColor(if selected {
                        COLOR_HIGHLIGHTTEXT
                    } else {
                        COLOR_WINDOWTEXT
                    }),
                );
                let values = [
                    node.bytes
                        .map(freeport::human)
                        .unwrap_or_else(|| l::t("msg.8284301049").into()),
                    node.application.clone(),
                    node.status.clone(),
                ];
                for (column, value) in values.iter().enumerate() {
                    let mut rect: RECT = std::mem::zeroed();
                    SendMessageW(
                        self.header.get(),
                        HDM_GETITEMRECT,
                        column + 1,
                        &mut rect as *mut _ as isize,
                    );
                    rect.top = draw.nmcd.rc.top;
                    rect.bottom = draw.nmcd.rc.bottom;
                    FillRect(draw.nmcd.hdc, &rect, GetSysColorBrush(background));
                    rect.left += 6;
                    rect.right -= 5;
                    let mut value = p::wide(value);
                    DrawTextW(
                        draw.nmcd.hdc,
                        value.as_mut_ptr(),
                        -1,
                        &mut rect,
                        DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
                    );
                }
                CDRF_DODEFAULT as isize
            }
            _ => CDRF_DODEFAULT as isize,
        }
    }
    pub unsafe fn dispose(&self) {
        let images = SendMessageW(self.hwnd, TVM_GETIMAGELIST, TVSIL_STATE as usize, 0);
        if images != 0 {
            SendMessageW(self.hwnd, TVM_SETIMAGELIST, TVSIL_STATE as usize, 0);
            ImageList_Destroy(images as _);
        }
    }
    pub unsafe fn self_test(&mut self, rows: &[Row]) -> bool {
        let mut class = [0u16; 64];
        GetClassNameW(self.hwnd, class.as_mut_ptr(), 64);
        let native =
            String::from_utf16_lossy(&class[..class.iter().position(|n| *n == 0).unwrap_or(64)])
                == "SysTreeView32";
        let images = SendMessageW(self.hwnd, TVM_GETIMAGELIST, TVSIL_STATE as usize, 0);
        let partial = images != 0 && ImageList_GetImageCount(images as _) >= 3;
        let Some(path) = rows.first().map(|r| r.candidate.source.clone()) else {
            return false;
        };
        self.selection.set(&path, true);
        self.selection.set(&path.join("unloaded-child"), false);
        self.sync();
        let mut item = TVITEMW {
            mask: TVIF_STATE,
            hItem: self
                .nodes
                .iter()
                .find(|n| p::same(&n.path, &path))
                .unwrap()
                .item,
            stateMask: TVIS_STATEIMAGEMASK,
            ..std::mem::zeroed()
        };
        SendMessageW(self.hwnd, TVM_GETITEMW, 0, &mut item as *mut _ as isize);
        let mixed = (item.state >> 12) == 3;
        self.selection.set(&path, true);
        self.sync();
        let saved = self.selection.clone();
        let root = PathBuf::from(r"E:\freeport-sort-fixture");
        let make = |relative: &str, bytes: u64| Row {
            candidate: freeport::catalog::custom(&root.join(relative), "sort fixture", "custom"),
            stats: freeport::inventory::Stats {
                bytes,
                ..Default::default()
            },
            status: "ready".into(),
            link_target: None,
            busy: vec![],
        };
        let fixture = vec![
            make("small", 2),
            make("large", 100),
            make("large/small", 3),
            make("large/large", 90),
        ];
        self.rebuild(&fixture, false);
        let first_is = |parent: &std::path::Path, expected: &std::path::Path| {
            let parent = self
                .nodes
                .iter()
                .find(|n| p::same(&n.path, parent))
                .unwrap()
                .item;
            let first = SendMessageW(self.hwnd, TVM_GETNEXTITEM, TVGN_CHILD as usize, parent);
            self.nodes
                .iter()
                .any(|n| n.item == first && p::same(&n.path, expected))
        };
        let sorted = first_is(&root, &root.join("large"))
            && first_is(&root.join("large"), &root.join("large/large"));
        self.rebuild(rows, false);
        self.selection = saved;
        self.sync();
        native
            && partial
            && mixed
            && sorted
            && SendMessageW(self.header.get(), HDM_GETITEMCOUNT, 0, 0) == 4
    }
}
fn label(path: &std::path::Path, row: Option<&Row>) -> String {
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let _ = row;
    name
}
