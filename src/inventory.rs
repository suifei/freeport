use crate::{catalog::Candidate, platform, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Stats {
    pub bytes: u64,
    pub files: u64,
    pub dirs: u64,
    pub links: u64,
    pub errors: u64,
    #[serde(default)]
    pub link_details: Vec<LinkDetail>,
    #[serde(default)]
    pub directory_totals: Vec<DirectoryTotal>,
    #[serde(default)]
    pub details_truncated: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinkDetail {
    pub path: PathBuf,
    pub target: Option<PathBuf>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DirectoryTotal {
    pub path: PathBuf,
    pub bytes: u64,
    pub links: u64,
    pub errors: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Row {
    pub candidate: Candidate,
    pub stats: Stats,
    pub status: String,
    pub link_target: Option<PathBuf>,
    pub busy: Vec<String>,
}
impl Row {
    pub fn selectable(&self) -> bool {
        self.status == "ready"
            && self.stats.errors == 0
            && self.stats.links == 0
            && self.stats.files > 0
            && self.busy.is_empty()
    }
}

pub fn measure(path: &Path) -> Stats {
    measure_with_control(path, &crate::engine::Control::default()).unwrap_or_else(|_| Stats {
        errors: 1,
        ..Stats::default()
    })
}
pub fn measure_with_control(path: &Path, control: &crate::engine::Control) -> Result<Stats> {
    measure_internal(path, control, false)
}
pub fn measure_for_display(path: &Path, control: &crate::engine::Control) -> Result<Stats> {
    measure_internal(path, control, true)
}
fn measure_internal(
    path: &Path,
    control: &crate::engine::Control,
    detailed: bool,
) -> Result<Stats> {
    let mut s = Stats::default();
    let mut totals = std::collections::BinaryHeap::new();
    let mut stack = vec![(path.to_path_buf(), None)];
    while let Some((p, start)) = stack.pop() {
        control.check()?;
        if let Some((bytes, links, errors)) = start {
            if detailed {
                let entry = (
                    p.components().count(),
                    p,
                    s.bytes - bytes,
                    s.links - links,
                    s.errors - errors,
                );
                if totals.len() < 16384 {
                    totals.push(entry);
                } else {
                    s.details_truncated = true;
                    if totals.peek().is_some_and(|old| &entry < old) {
                        totals.pop();
                        totals.push(entry);
                    }
                }
            }
            continue;
        }
        match fs::symlink_metadata(&p) {
            Ok(m) if platform::reparse(&m) => {
                s.links += 1;
                if detailed && s.link_details.len() < 128 {
                    s.link_details.push(LinkDetail {
                        target: junction::get_target(&p).or_else(|_| fs::read_link(&p)).ok(),
                        path: p,
                    });
                } else if detailed {
                    s.details_truncated = true;
                }
            }
            Ok(m) if m.is_dir() => {
                s.dirs += 1;
                if detailed {
                    stack.push((p.clone(), Some((s.bytes, s.links, s.errors))));
                }
                match fs::read_dir(p) {
                    Ok(ds) => {
                        for d in ds {
                            match d {
                                Ok(d) => stack.push((d.path(), None)),
                                Err(_) => s.errors += 1,
                            }
                        }
                    }
                    Err(_) => s.errors += 1,
                }
            }
            Ok(m) => {
                s.bytes += m.len();
                s.files += 1;
            }
            Err(_) => s.errors += 1,
        }
    }
    s.directory_totals = totals
        .into_iter()
        .map(|(_, path, bytes, links, errors)| DirectoryTotal {
            path,
            bytes,
            links,
            errors,
        })
        .collect();
    s.directory_totals.sort_by(|a, b| {
        platform::key(a.path.parent().unwrap_or(&a.path))
            .cmp(&platform::key(b.path.parent().unwrap_or(&b.path)))
            .then_with(|| b.bytes.cmp(&a.bytes))
            .then_with(|| platform::key(&a.path).cmp(&platform::key(&b.path)))
    });
    Ok(s)
}
pub fn scan(home: &Path, extra: &[PathBuf], progress: impl Fn(String)) -> Vec<Row> {
    let ps = platform::processes();
    let mut rows = vec![];
    for candidate in crate::catalog::discover(home, extra) {
        progress(crate::i18n::format(
            "msg.66a30391b5",
            &[format!("{}", candidate.name)],
        ));
        let linked = fs::symlink_metadata(&candidate.source)
            .map(|m| platform::reparse(&m))
            .unwrap_or(false);
        let target = if linked {
            junction::get_target(&candidate.source)
                .or_else(|_| fs::read_link(&candidate.source))
                .ok()
        } else {
            None
        };
        let busy = ps
            .iter()
            .filter(|(pid, n)| {
                *pid != std::process::id()
                    && candidate
                        .processes
                        .iter()
                        .any(|x| x.eq_ignore_ascii_case(n))
            })
            .map(|(p, n)| format!("{n} (PID {p})"))
            .collect();
        let stats = if linked {
            Stats::default()
        } else {
            measure(&candidate.source)
        };
        let status = if linked {
            "linked"
        } else if stats.errors > 0 {
            "unreadable"
        } else if stats.links > 0 {
            "nested_links"
        } else {
            "ready"
        };
        rows.push(Row {
            candidate,
            stats,
            status: status.into(),
            link_target: target,
            busy,
        });
    }
    rows.sort_by(|a, b| b.stats.bytes.cmp(&a.stats.bytes));
    rows
}
pub fn save_report(path: &Path, rows: &[Row]) -> Result<()> {
    platform::atomic_json(path, &rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recursive_totals_include_direct_files_and_keep_sibling_boundaries() {
        let root = std::env::temp_dir().join(format!("fp-usage-{}", std::process::id()));
        fs::create_dir_all(root.join("large/nested")).unwrap();
        fs::create_dir_all(root.join("small")).unwrap();
        fs::write(root.join("direct"), [0; 3]).unwrap();
        fs::write(root.join("large/one"), [0; 7]).unwrap();
        fs::write(root.join("large/nested/two"), [0; 11]).unwrap();
        fs::write(root.join("small/one"), [0; 5]).unwrap();
        let stats = measure_for_display(&root, &Default::default()).unwrap();
        assert_eq!(
            (
                stats.bytes,
                stats.files,
                stats.dirs,
                stats.errors,
                stats.links
            ),
            (26, 4, 4, 0, 0)
        );
        let total = |rel: &str| {
            stats
                .directory_totals
                .iter()
                .find(|d| platform::same(&d.path, &root.join(rel)))
                .unwrap()
                .bytes
        };
        assert_eq!(
            (
                total(""),
                total("large"),
                total("large/nested"),
                total("small")
            ),
            (26, 18, 11, 5)
        );
        let siblings: Vec<_> = stats
            .directory_totals
            .iter()
            .filter(|d| d.path.parent().is_some_and(|p| platform::same(p, &root)))
            .map(|d| d.bytes)
            .collect();
        assert_eq!(siblings, vec![18, 5]);
        for file in ["direct", "large/one", "large/nested/two", "small/one"] {
            fs::remove_file(root.join(file)).unwrap();
        }
        for dir in ["large/nested", "large", "small", ""] {
            fs::remove_dir(root.join(dir)).unwrap();
        }
    }
    #[test]
    fn cancelled_display_measurement_does_no_io() {
        let control = crate::engine::Control::default();
        control
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(measure_for_display(Path::new(r"C:\must-not-open"), &control).is_err());
    }
}
