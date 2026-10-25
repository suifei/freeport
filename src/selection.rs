//! Shared directory selection. A checked ancestor is one transaction, never many.
use crate::{catalog, engine::Control, fail, inventory::Row, platform as p, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub version: u32,
    pub include: Vec<PathBuf>,
    #[serde(default)]
    pub exclude: Vec<PathBuf>,
}
impl Default for Selection {
    fn default() -> Self {
        Self {
            version: 1,
            include: vec![],
            exclude: vec![],
        }
    }
}
impl Selection {
    pub fn from_ids(rows: &[Row], ids: &[String]) -> Result<Self> {
        let mut s = Self::default();
        for id in ids {
            s.include.push(
                rows.iter()
                    .find(|r| &r.candidate.id == id)
                    .ok_or(crate::i18n::t("selection.invalid"))?
                    .candidate
                    .source
                    .clone(),
            );
        }
        Ok(s)
    }
    /// Most specific explicit intent wins. Unloaded descendants inherit it too.
    pub fn contains(&self, path: &Path) -> bool {
        self.include
            .iter()
            .map(|p| (p, true))
            .chain(self.exclude.iter().map(|p| (p, false)))
            .filter(|(r, _)| p::within(path, r))
            .max_by_key(|(r, _)| r.components().count())
            .map(|(_, b)| b)
            .unwrap_or(false)
    }
    pub fn state(&self, path: &Path) -> u32 {
        let on = self.contains(path);
        let different = self.include.iter().any(|r| p::within(r, path) && !on)
            || self.exclude.iter().any(|r| p::within(r, path) && on);
        if different {
            3
        } else if on {
            2
        } else {
            1
        }
    }
    pub fn set(&mut self, path: &Path, on: bool) {
        self.include.retain(|r| !p::within(r, path));
        self.exclude.retain(|r| !p::within(r, path));
        let inherited = self.contains(path);
        if inherited != on {
            if on {
                self.include.push(path.into())
            } else {
                self.exclude.push(path.into())
            }
        }
    }
    pub fn validate(&self, rows: &[Row]) -> Result<()> {
        if self.version != 1
            || self.include.is_empty()
            || self.include.len() + self.exclude.len() > 16384
        {
            return fail(crate::i18n::t("selection.invalid"));
        }
        for path in self.include.iter().chain(&self.exclude) {
            // Reject invalid/aliased paths before compiling a plan; never widen a rule boundary.
            let excluded = self.exclude.iter().any(|r| p::same(r, path));
            let actual = if excluded && fs::symlink_metadata(path).is_ok_and(|m| p::reparse(&m)) {
                let parent = path.parent().ok_or(crate::i18n::t("selection.invalid"))?;
                p::absolute(parent)?.join(
                    path.file_name()
                        .ok_or(crate::i18n::t("selection.invalid"))?,
                )
            } else {
                p::absolute(path)?
            };
            if !p::same(path, &actual)
                || !rows.iter().any(|r| p::within(path, &r.candidate.source))
                || self.include.iter().any(|x| p::same(x, path))
                    && self.exclude.iter().any(|x| p::same(x, path))
            {
                return fail(crate::i18n::t("selection.invalid"));
            }
            if fs::symlink_metadata(path)
                .map(|m| !m.is_dir() || (p::reparse(&m) && !excluded))
                .unwrap_or(true)
            {
                return fail(crate::i18n::t("selection.link"));
            }
        }
        if self
            .exclude
            .iter()
            .any(|x| !self.include.iter().any(|r| p::within(x, r)))
        {
            return fail(crate::i18n::t("selection.invalid"));
        }
        Ok(())
    }
    pub fn restore_sources(&self, rows: &[Row]) -> Result<Vec<PathBuf>> {
        if self.version != 1
            || self.include.is_empty()
            || self.include.len() + self.exclude.len() > 16384
        {
            return fail(crate::i18n::t("selection.invalid"));
        }
        for path in self.include.iter().chain(&self.exclude) {
            if !rows.iter().any(|r| {
                p::same(&r.candidate.source, path)
                    && r.status == "linked"
                    && r.link_target.is_some()
            }) {
                return fail(crate::i18n::t("selection.invalid"));
            }
        }
        let mut paths = BTreeMap::new();
        for r in rows {
            if self.contains(&r.candidate.source) && r.status == "linked" && r.link_target.is_some()
            {
                paths.insert(p::key(&r.candidate.source), r.candidate.source.clone());
            }
        }
        if paths.is_empty() {
            return fail(crate::i18n::t("selection.invalid"));
        }
        Ok(paths.into_values().collect())
    }
}
/// Component-aware, deterministic, idempotent outer-root reduction.
pub fn roots(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut unique = BTreeMap::new();
    for p in paths {
        unique.entry(p::key(&p)).or_insert(p);
    }
    let mut paths: Vec<_> = unique.into_values().collect();
    paths.sort_by_key(|p| (p.components().count(), p::key(p)));
    let mut out: Vec<PathBuf> = vec![];
    for path in paths {
        if !out.iter().any(|p| p::within(&path, p)) {
            out.push(path);
        }
    }
    out.sort_by_key(|p| p::key(p));
    out
}
pub struct Compiled {
    pub rows: Vec<Row>,
    pub retained: Vec<PathBuf>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Planned {
    pub plan: crate::engine::Plan,
    pub retained: Vec<PathBuf>,
}
pub fn plan(
    rows: &[Row],
    selection: &Selection,
    destination: &Path,
    store: &Path,
    control: &Control,
    progress: &dyn Fn(String),
) -> Result<Planned> {
    let mut compiled = compile(rows, selection, control)?;
    for row in &mut compiled.rows {
        if row.status == "unmeasured" {
            crate::scan::measure_with_control(std::slice::from_mut(row), progress, control)?;
        }
    }
    let ids = compiled
        .rows
        .iter()
        .map(|r| r.candidate.id.clone())
        .collect::<Vec<_>>();
    Ok(Planned {
        plan: crate::engine::make_plan_with_store(&compiled.rows, &ids, destination, store)?,
        retained: compiled.retained,
    })
}
pub fn compile(rows: &[Row], selection: &Selection, control: &Control) -> Result<Compiled> {
    control.check()?;
    selection.validate(rows)?;
    let mut result = Compiled {
        rows: vec![],
        retained: vec![],
    };
    let mut stack = roots(selection.include.clone());
    let mut visited = 0usize;
    while let Some(path) = stack.pop() {
        control.check()?;
        visited += 1;
        if visited > 100_000 {
            return fail(crate::i18n::t("selection.limit"));
        }
        let state = selection.state(&path);
        if state == 1 {
            result.retained.push(path);
            continue;
        }
        let meta = fs::symlink_metadata(&path)?;
        if !meta.is_dir() || p::reparse(&meta) {
            return fail(crate::i18n::t("selection.link"));
        }
        if state == 2 {
            result.rows.push(
                rows.iter()
                    .find(|r| p::same(&r.candidate.source, &path))
                    .cloned()
                    .unwrap_or_else(|| Row {
                        candidate: catalog::custom(
                            &path,
                            crate::i18n::t("msg.73fc5c2ef0"),
                            "custom",
                        ),
                        stats: Default::default(),
                        status: "unmeasured".into(),
                        link_target: None,
                        busy: vec![],
                    }),
            );
        } else {
            // A partially checked directory remains real. Its direct files stay put.
            result.retained.push(path.clone());
            for entry in fs::read_dir(&path)? {
                control.check()?;
                let entry = entry?;
                let m = fs::symlink_metadata(entry.path())?;
                if m.is_dir() || p::reparse(&m) {
                    stack.push(entry.path());
                }
            }
        }
    }
    result.rows.sort_by_key(|r| p::key(&r.candidate.source));
    result.retained.sort_by_key(|p| p::key(p));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roots_are_order_independent_and_component_aware() {
        let input = vec![r"C:\a\b\c", r"C:\ab", r"C:\a", r"c:\A\b", r"C:\a"];
        let expected = vec![PathBuf::from(r"C:\a"), PathBuf::from(r"C:\ab")];
        for shift in 0..input.len() {
            let mut x = input.clone();
            x.rotate_left(shift);
            for reversed in [false, true] {
                let mut x = x.clone();
                if reversed {
                    x.reverse();
                }
                let actual = roots(x.into_iter().map(PathBuf::from));
                assert_eq!(
                    actual.iter().map(|p| p::key(p)).collect::<Vec<_>>(),
                    expected.iter().map(|p| p::key(p)).collect::<Vec<_>>()
                );
                assert_eq!(roots(actual.clone()), actual);
            }
        }
    }
    #[test]
    fn cascade_exclusion_and_reinclude() {
        let mut s = Selection::default();
        let a = Path::new(r"C:\a");
        let b = Path::new(r"C:\a\b");
        let c = Path::new(r"C:\a\b\c");
        s.set(a, true);
        assert_eq!(s.state(c), 2);
        s.set(b, false);
        assert_eq!(s.state(a), 3);
        assert_eq!(s.state(c), 1);
        s.set(c, true);
        assert_eq!(s.state(b), 3);
        assert_eq!(s.state(c), 2);
        s.set(a, false);
        assert_eq!(s.state(c), 1);
        assert!(s.include.is_empty());
    }
    #[test]
    fn ten_thousand_cascade_boundaries() {
        for i in 0..10_000 {
            let root = PathBuf::from(format!(r"E:\目录-{i}"));
            let child = root.join("b");
            let mut s = Selection::default();
            s.set(&root, true);
            s.set(&child, false);
            assert_eq!(s.state(&root), 3);
            assert!(!s.contains(&child.join("unloaded")));
            assert!(!s.contains(&PathBuf::from(format!(r"E:\目录-{i}-other"))));
            s.set(&child, true);
            assert_eq!(s.state(&root), 2);
        }
    }
    #[test]
    fn partial_compilation_retains_direct_files_and_excluded_subtrees() {
        let raw = std::env::temp_dir().join(format!("fp-selection-{}", std::process::id()));
        fs::create_dir_all(raw.join("one/deep")).unwrap();
        fs::create_dir_all(raw.join("two")).unwrap();
        fs::write(raw.join("direct.txt"), b"unchanged").unwrap();
        // Scan stores platform::absolute paths. temp_dir() on the Windows runner is an
        // 8.3 alias, and validation rejects that spelling before compilation.
        let dir = p::absolute(&raw).unwrap();
        let row = Row {
            candidate: catalog::custom(&dir, "fixture", "custom"),
            stats: Default::default(),
            status: "ready".into(),
            link_target: None,
            busy: vec![],
        };
        let mut s = Selection::default();
        s.set(&dir, true);
        s.set(&dir.join("one/deep"), false);
        let out = compile(&[row], &s, &Control::default()).unwrap();
        assert_eq!(out.rows.len(), 1);
        assert!(p::same(&out.rows[0].candidate.source, &dir.join("two")));
        assert!(out.retained.iter().any(|p| p == &dir));
        assert_eq!(fs::read(dir.join("direct.txt")).unwrap(), b"unchanged");
        // Explicit fixture cleanup only; no recursive deletion is needed.
        fs::remove_file(dir.join("direct.txt")).unwrap();
        fs::remove_dir(dir.join("one/deep")).unwrap();
        fs::remove_dir(dir.join("one")).unwrap();
        fs::remove_dir(dir.join("two")).unwrap();
        fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn selection_json_rejects_unknown_fields_and_wrong_types() {
        for json in [
            r#"{"version":1,"include":[],"execute":"cmd.exe"}"#,
            r#"{"version":1,"include":"C:\\a"}"#,
            r#"{"version":-1,"include":[]}"#,
        ] {
            assert!(serde_json::from_str::<Selection>(json).is_err());
        }
    }
    #[test]
    fn shared_restore_targets_are_independent_logical_entries() {
        let make = |path: &str| Row {
            candidate: catalog::custom(Path::new(path), "link", "link"),
            stats: Default::default(),
            status: "linked".into(),
            link_target: Some(PathBuf::from(r"D:\shared")),
            busy: vec![],
        };
        let rows = vec![make(r"C:\one"), make(r"C:\two")];
        let mut s = Selection::default();
        s.set(&rows[0].candidate.source, true);
        s.set(&rows[1].candidate.source, true);
        assert_eq!(s.restore_sources(&rows).unwrap().len(), 2);
        s.set(&rows[0].candidate.source, false);
        assert_eq!(
            s.restore_sources(&rows).unwrap(),
            vec![PathBuf::from(r"C:\two")]
        );
    }
}
