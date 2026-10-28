//! Offline messages and one shared GUI/CLI country-based language resolver.
use crate::{fail, platform, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicU8, Ordering},
        OnceLock,
    },
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

pub const TAGS: [&str; 6] = ["en", "zh-CN", "zh-TW", "ja", "ko", "de"];
pub const NAMES: [&str; 6] = [
    "English",
    "简体中文",
    "繁體中文",
    "日本語",
    "한국어",
    "Deutsch",
];
static CURRENT: AtomicU8 = AtomicU8::new(0);
static MAPS: OnceLock<[BTreeMap<String, String>; 6]> = OnceLock::new();
fn maps() -> &'static [BTreeMap<String, String>; 6] {
    MAPS.get_or_init(|| {
        [
            include_str!(concat!(env!("OUT_DIR"), "/en.json")),
            include_str!(concat!(env!("OUT_DIR"), "/zh-CN.json")),
            include_str!(concat!(env!("OUT_DIR"), "/zh-TW.json")),
            include_str!(concat!(env!("OUT_DIR"), "/ja.json")),
            include_str!(concat!(env!("OUT_DIR"), "/ko.json")),
            include_str!(concat!(env!("OUT_DIR"), "/de.json")),
        ]
        .map(|s| serde_json::from_str(s).expect("validated embedded locale"))
    })
}
pub fn language() -> &'static str {
    TAGS[CURRENT.load(Ordering::Relaxed) as usize]
}
pub fn index(tag: &str) -> Option<usize> {
    TAGS.iter().position(|t| *t == tag)
}
pub fn set(tag: &str) -> Result<()> {
    if let Some(i) = index(tag) {
        CURRENT.store(i as u8, Ordering::Relaxed);
        Ok(())
    } else {
        CURRENT.store(0, Ordering::Relaxed);
        fail(t("language.invalid"))
    }
}
pub fn has_key(key: &str) -> bool {
    maps()[0].contains_key(key)
}
pub fn t(key: &str) -> &'static str {
    let i = CURRENT.load(Ordering::Relaxed) as usize;
    maps()[i]
        .get(key)
        .filter(|s| !s.trim().is_empty())
        .or_else(|| maps()[0].get(key).filter(|s| !s.trim().is_empty()))
        .map(String::as_str)
        .unwrap_or("Message unavailable. The operation cannot continue safely.")
}
pub fn format(key: &str, args: &[String]) -> String {
    render(t(key), args)
}
fn render(template: &str, args: &[String]) -> String {
    let mut result = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            if chars.peek() == Some(&'{') {
                chars.next();
                result.push('{');
                continue;
            }
            let mut n = String::new();
            for c in chars.by_ref() {
                if c == '}' {
                    break;
                }
                n.push(c);
            }
            if let Ok(i) = n.parse::<usize>() {
                if let Some(value) = args.get(i) {
                    result.push_str(value);
                } else {
                    result.push_str("[missing argument]");
                }
            } else {
                result.push('{');
                result.push_str(&n);
                result.push('}');
            }
        } else if c == '}' && chars.peek() == Some(&'}') {
            chars.next();
            result.push('}');
        } else {
            result.push(c);
        }
    }
    result
}
pub fn force_warning() -> &'static str {
    let key = "handles.force_close_warning";
    if has_key(key) {
        t(key)
    } else {
        "This force-closes file/directory handles in other applications and may crash them, lose unsaved work, or corrupt files. It applies only to your current selection. This cannot be undone; full copy and verification checks still apply."
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Choice {
    pub requested_language: String,
    pub effective_language: String,
    pub source: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    language: String,
}
pub fn from_country(country: Option<&str>) -> &'static str {
    match country.map(str::to_ascii_uppercase).as_deref() {
        Some("CN") => "zh-CN",
        Some("TW" | "HK" | "MO") => "zh-TW",
        Some("JP") => "ja",
        Some("KR") => "ko",
        Some("DE" | "AT" | "LI") => "de",
        _ => "en",
    }
}
#[link(name = "kernel32")]
extern "system" {
    fn GetUserGeoID(class: u32) -> i32;
    fn GetGeoInfoW(id: i32, kind: u32, data: *mut u16, len: i32, language: u16) -> i32;
}
fn country() -> Option<String> {
    type Geo = unsafe extern "system" fn(*mut u16, i32) -> i32;
    let module = unsafe { GetModuleHandleW(platform::wide("kernel32.dll").as_ptr()) };
    let mut b = [0u16; 32];
    if let Some(p) = unsafe { GetProcAddress(module, c"GetUserDefaultGeoName".as_ptr() as _) } {
        let f: Geo = unsafe { std::mem::transmute(p) };
        let n = unsafe { f(b.as_mut_ptr(), b.len() as i32) };
        if n <= 1 {
            return None;
        }
        return String::from_utf16(&b[..(n as usize - 1).min(b.len())]).ok();
    }
    // Legacy geography APIs read the same user country setting on Windows 7/8.
    let id = unsafe { GetUserGeoID(16) };
    if id == -1 {
        return None;
    }
    let n = unsafe { GetGeoInfoW(id, 4, b.as_mut_ptr(), b.len() as i32, 0) };
    if n <= 1 {
        return None;
    }
    String::from_utf16(&b[..(n as usize - 1).min(b.len())]).ok()
}
fn settings_path() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    if !base.is_absolute() || base.to_string_lossy().starts_with(r"\\") {
        return None;
    }
    Some(base.join("FreePort/language.json"))
}
pub fn resolve(
    explicit: Option<&str>,
    saved: Option<&str>,
    country: Option<&str>,
) -> Result<Choice> {
    let (requested, source) = if let Some(e) = explicit {
        (e, "argument")
    } else if let Some(s) = saved {
        (s, "preference")
    } else {
        ("auto", "system_country")
    };
    if requested != "auto" && index(requested).is_none() {
        return fail("Invalid language. Choose auto, en, zh-CN, zh-TW, ja, ko, or de.");
    }
    let effective = if requested == "auto" {
        from_country(country)
    } else {
        requested
    };
    Ok(Choice {
        requested_language: requested.into(),
        effective_language: effective.into(),
        source: if requested == "auto" && effective == "en" && country.is_none() {
            "english_fallback"
        } else {
            source
        }
        .into(),
    })
}
pub fn init(explicit: Option<&str>) -> Result<Choice> {
    let mut invalid = false;
    let saved = settings_path().and_then(|p| {
        if !p.exists() {
            return None;
        }
        let result = (|| -> Result<Settings> {
            if fs::metadata(&p)?.len() > 4096 {
                return fail("settings_too_large");
            }
            Ok(serde_json::from_slice(&fs::read(p)?)?)
        })();
        match result {
            Ok(s) if s.language == "auto" || index(&s.language).is_some() => Some(s.language),
            _ => {
                invalid = true;
                None
            }
        }
    });
    let country = country();
    let choice = if explicit.is_none() && invalid {
        Choice {
            requested_language: "en".into(),
            effective_language: "en".into(),
            source: "invalid_preference".into(),
        }
    } else {
        resolve(explicit, saved.as_deref(), country.as_deref())?
    };
    set(&choice.effective_language)?;
    Ok(choice)
}
pub fn save_preference(requested: &str) -> Result<Choice> {
    let c = resolve(Some(requested), None, country().as_deref())?;
    let path = settings_path().ok_or_else(|| t("language.save_failed"))?;
    fs::create_dir_all(path.parent().ok_or("settings_parent")?)?;
    platform::atomic_json(
        &path,
        &Settings {
            language: requested.into(),
        },
    )?;
    set(&c.effective_language)?;
    Ok(c)
}
pub fn describe_error(error: &(dyn std::error::Error + 'static)) -> String {
    if let Some(e) = error.downcast_ref::<std::io::Error>() {
        use std::io::ErrorKind::*;
        let key = match e.kind() {
            PermissionDenied => "error.io.permission",
            NotFound => "error.io.not_found",
            AlreadyExists => "error.io.exists",
            InvalidInput | InvalidData => "error.io.invalid",
            _ => "error.io.other",
        };
        format(
            "error.native",
            &[
                e.raw_os_error()
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "-".into()),
                t(key).into(),
            ],
        )
    } else if let Some(e) = error.downcast_ref::<serde_json::Error>() {
        format("error.input", &[format!("{}:{}", e.line(), e.column())])
    } else {
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn country_matrix() {
        for (c, e) in [
            ("CN", "zh-CN"),
            ("cn", "zh-CN"),
            ("TW", "zh-TW"),
            ("HK", "zh-TW"),
            ("MO", "zh-TW"),
            ("JP", "ja"),
            ("KR", "ko"),
            ("DE", "de"),
            ("AT", "de"),
            ("LI", "de"),
            ("US", "en"),
            ("SG", "en"),
            ("CH", "en"),
            ("", "en"),
            ("001", "en"),
            (" CN ", "en"),
        ] {
            assert_eq!(from_country(Some(c)), e);
        }
        assert_eq!(from_country(None), "en");
    }
    #[test]
    fn explicit_and_saved_priorities() {
        assert_eq!(
            resolve(Some("de"), Some("ja"), Some("CN"))
                .unwrap()
                .effective_language,
            "de"
        );
        assert_eq!(
            resolve(Some("auto"), Some("ja"), Some("CN"))
                .unwrap()
                .effective_language,
            "zh-CN"
        );
        assert_eq!(
            resolve(None, Some("ko"), Some("JP"))
                .unwrap()
                .effective_language,
            "ko"
        );
        assert_eq!(resolve(None, None, None).unwrap().effective_language, "en");
        for bad in ["EN", "zh", "fr", "auto ", "--yes", ""] {
            assert!(resolve(Some(bad), Some("de"), Some("CN")).is_err());
        }
    }
    #[test]
    fn interpolated_paths_are_not_templates() {
        assert_eq!(
            render(
                "Path: {0}; code: {1}",
                &["E:\\path{1}\\{0}".into(), "5".into()]
            ),
            "Path: E:\\path{1}\\{0}; code: 5"
        );
    }
    fn parameters(s: &str) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        let mut rest = s;
        while let Some(i) = rest.find('{') {
            rest = &rest[i + 1..];
            if let Some(j) = rest.find('}') {
                out.insert(rest[..j].into());
                rest = &rest[j + 1..];
            } else {
                panic!("unclosed parameter");
            }
        }
        out
    }
    #[test]
    fn all_six_catalogs_are_complete() {
        let maps = maps();
        let keys = maps[0].keys().collect::<Vec<_>>();
        assert!(!keys.is_empty());
        for key in [
            "handles.force_close_warning",
            "cli.help",
            "common.ok",
            "common.cancel",
            "language.auto",
        ] {
            assert!(maps[0].contains_key(key), "{key}");
        }
        for m in maps {
            assert_eq!(m.keys().collect::<Vec<_>>(), keys);
            for (k, v) in m {
                assert!(!v.trim().is_empty(), "{k}");
                assert_eq!(parameters(v), parameters(&maps[0][k]), "{k}");
            }
        }
    }
}
