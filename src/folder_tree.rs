//! Native folder tree; all checkbox intent lives in the shared selection model.
use freeport::{i18n as l, inventory::Row, platform as p, selection::Selection};
use std::{collections::BTreeMap, fs, path::PathBuf};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::InvalidateRect,
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
}
pub struct Tree {
    pub hwnd: HWND,
    pub selection: Selection,
    nodes: Vec<Node>,
    restore: bool,
}
impl Tree {
    pub fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            selection: Selection::default(),
            nodes: vec![],
            restore: false,
        }
    }
    pub unsafe fn init(&self) {
        // Common Controls v6 supplies checked/unchecked/partial state images on Win7.
        let style = GetWindowLongPtrW(self.hwnd, GWL_STYLE) as u32;
        SetWindowLongPtrW(self.hwnd, GWL_STYLE, (style | TVS_CHECKBOXES) as isize);
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
            hInsertAfter: TVI_SORT,
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
        });
    }
    pub unsafe fn update(&self, rows: &[Row], index: usize) {
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
        Ok(())
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
        native && partial && mixed
    }
}
fn label(path: &std::path::Path, row: Option<&Row>) -> String {
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    if let Some(r) = row {
        let status = if let Some(target) = &r.link_target {
            l::format("msg.0ca6431378", &[target.display().to_string()])
        } else {
            l::t(&format!("status.{}", r.status)).to_string()
        };
        format!(
            "{} — {}   [{}]   {}",
            name,
            r.candidate.name,
            if r.status == "unmeasured" {
                l::t("msg.8284301049").into()
            } else {
                freeport::human(r.stats.bytes)
            },
            status
        )
    } else {
        name
    }
}
