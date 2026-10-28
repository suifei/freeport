//! Versioned, data-only directory rules. Rules cannot change the transaction safety policy.
use crate::{
    catalog::{custom, Candidate},
    fail, platform, Result,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub category: String,
    pub suffix: String,
    #[serde(default)]
    pub processes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge: Option<Knowledge>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Knowledge {
    pub application: String,
    pub purpose: String,
    pub kind: String,
    pub confidence: String,
    pub migration: String,
    pub scope: String,
    pub owner_suffix: String,
    pub markers: Vec<Vec<String>>,
    pub sources: Vec<String>,
    pub relocation_env: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pack {
    pub schema_version: u32,
    pub revision: u64,
    pub rules: Vec<Rule>,
}
pub fn builtin() -> Pack {
    serde_json::from_str(include_str!(concat!(
        env!("OUT_DIR"),
        "/freeport-rules.json"
    )))
    .expect("validated bundled rules")
}
pub fn validate(pack: &Pack) -> Result<()> {
    if ![1, 2].contains(&pack.schema_version)
        || pack.revision == 0
        || pack.rules.is_empty()
        || pack.rules.len() > 4096
    {
        return fail(crate::i18n::t("msg.8ac60e18b3"));
    }
    let mut ids = HashSet::new();
    for r in &pack.rules {
        if r.id.is_empty()
            || r.id.len() > 64
            || !r
                .id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            || !ids.insert(&r.id)
        {
            return fail(crate::i18n::t("msg.41d033646f"));
        }
        if r.name.is_empty()
            || r.name.len() > 256
            || r.category.is_empty()
            || r.category.len() > 64
            || r.suffix.len() > 1024
            || r.suffix.contains([':', '\0'])
            || r.suffix.starts_with(['/', '\\'])
        {
            return fail(crate::i18n::t("msg.e83a4cb32d"));
        }
        let s = r.suffix.replace('\\', "/");
        let parts: Vec<_> = s.split('/').collect();
        if parts.len() > 64 || parts.iter().any(|c| !safe_component(c, true)) {
            return fail(crate::i18n::t("msg.bc9f4f58ba"));
        }
        if r.processes.len() > 32
            || r.processes.iter().any(|s| {
                s.len() > 128
                    || s.contains(['/', '\\', ':', '\0'])
                    || !s.to_lowercase().ends_with(".exe")
            })
        {
            return fail(crate::i18n::t("msg.cc25be369c"));
        }
        if let Some(k) = &r.knowledge {
            if k.application.is_empty()
                || k.application.len() > 128
                || k.purpose.is_empty()
                || k.purpose.len() > 2048
                || ![
                    "cache",
                    "logs",
                    "history",
                    "models",
                    "extensions",
                    "toolchain",
                    "runtime",
                    "mixed",
                    "app",
                ]
                .contains(&k.kind.as_str())
                || !["documented", "local", "unverified"].contains(&k.confidence.as_str())
                || !["candidate", "review", "exclude"].contains(&k.migration.as_str())
                || (k.confidence == "unverified" && k.migration == "candidate")
                || !["home", "project"].contains(&k.scope.as_str())
                || k.markers.is_empty()
                || k.markers.len() > 16
                || k.sources.is_empty()
                || k.sources.len() > 16
                || k.sources.iter().any(|s| {
                    s.len() > 2048
                        || !(s.starts_with("https://") || s == "local-metadata:2026-10-24")
                })
                || k.relocation_env.len() > 8
                || k.relocation_env.iter().any(|v| {
                    v.is_empty()
                        || v.len() > 64
                        || !v
                            .bytes()
                            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
                })
                || (k.scope == "home" && k.owner_suffix.is_empty())
                || (!k.owner_suffix.is_empty() && !safe_relative(&k.owner_suffix, true))
                || k.markers.iter().any(|group| {
                    group.is_empty()
                        || group.len() > 16
                        || group.iter().any(|m| !safe_relative(m, false))
                })
            {
                return fail(crate::i18n::t("msg.e83a4cb32d"));
            }
            let owner = k.owner_suffix.replace('\\', "/").to_lowercase();
            let suffix = r.suffix.replace('\\', "/").to_lowercase();
            if (!owner.is_empty() && suffix != owner && !suffix.starts_with(&(owner + "/")))
                || suffix.split('/').last() == Some("*")
            {
                return fail(crate::i18n::t("msg.bc9f4f58ba"));
            }
        } else if pack.schema_version == 2 {
            return fail(crate::i18n::t("msg.e83a4cb32d"));
        }
    }
    Ok(())
}
pub fn load(path: &Path) -> Result<Pack> {
    if fs::metadata(path)?.len() > 1024 * 1024 {
        return fail(crate::i18n::t("msg.b83263baf4"));
    }
    let p: Pack = serde_json::from_slice(&fs::read(path)?)?;
    validate(&p)?;
    Ok(p)
}
pub fn active(store: &Path) -> Result<Pack> {
    let p = store.join("rules.json");
    if p.exists() {
        load(&p)
    } else {
        let p = builtin();
        validate(&p)?;
        Ok(p)
    }
}
pub fn import(path: &Path, store: &Path) -> Result<Pack> {
    let p = load(path)?;
    fs::create_dir_all(store)?;
    let _lock = platform::lock(&store.join("rules.lock"))?;
    let old = active(store)?;
    if p.revision <= old.revision {
        return fail(crate::i18n::t("msg.d7f22cd158"));
    }
    platform::atomic_json(&store.join("rules.json"), &p)?;
    Ok(p)
}
pub fn matches(path: &Path, rule: &Rule) -> bool {
    let suffix = rule.suffix.replace('/', "\\").to_lowercase();
    let p = platform::key(path);
    let actual: Vec<_> = p.split('\\').collect();
    let expected: Vec<_> = suffix.split('\\').collect();
    actual.len() > expected.len()
        && actual[actual.len() - expected.len()..]
            .iter()
            .zip(expected)
            .all(|(a, b)| b == "*" || *a == b)
}
fn safe_component(s: &str, wildcard: bool) -> bool {
    if wildcard && s == "*" {
        return true;
    }
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    !s.is_empty()
        && ![".", ".."].contains(&s)
        && !s.ends_with([' ', '.'])
        && !s
            .chars()
            .any(|c| c.is_control() || "*?:<>|/\\\"".contains(c))
        && !["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "CLOCK$"].contains(&stem.as_str())
        && !(stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}
fn safe_relative(s: &str, wildcard: bool) -> bool {
    s.len() <= 1024
        && s.replace('\\', "/").split('/').count() <= 64
        && s.replace('\\', "/")
            .split('/')
            .all(|c| safe_component(c, wildcard))
}
fn plain_chain(path: &Path, allow_link_leaf: bool) -> bool {
    for (i, ancestor) in path.ancestors().enumerate() {
        let Ok(meta) = fs::symlink_metadata(ancestor) else {
            return false;
        };
        if platform::reparse(&meta) && !(i == 0 && allow_link_leaf) {
            return false;
        }
    }
    true
}
fn owned(path: &Path, rule: &Rule, homes: &[std::path::PathBuf]) -> bool {
    let Some(k) = &rule.knowledge else {
        return false;
    };
    if k.migration == "exclude" || !matches(path, rule) {
        return false;
    }
    let owner_count = if k.owner_suffix.is_empty() {
        0
    } else {
        k.owner_suffix.replace('\\', "/").split('/').count()
    };
    let suffix_count = rule.suffix.replace('\\', "/").split('/').count();
    let Some(owner) = path.ancestors().nth(suffix_count - owner_count) else {
        return false;
    };
    if k.scope == "home" {
        let Some(home) = owner.ancestors().nth(owner_count) else {
            return false;
        };
        if !homes.iter().any(|h| platform::same(h, home)) {
            return false;
        }
    }
    if !plain_chain(owner, false) || !plain_chain(path, true) {
        return false;
    }
    // Test existence only: never inspect credentials, settings or transcripts.
    k.markers.iter().any(|group| {
        group
            .iter()
            .all(|marker| plain_chain(&owner.join(marker), false))
    })
}
pub fn discover(
    pack: &Pack,
    homes: &[std::path::PathBuf],
    paths: &[std::path::PathBuf],
) -> Vec<Candidate> {
    use std::{collections::HashMap, path::PathBuf};
    let mut by_leaf: HashMap<String, Vec<&Rule>> = HashMap::new();
    let mut probe = paths.to_vec();
    for rule in &pack.rules {
        if rule
            .knowledge
            .as_ref()
            .is_some_and(|k| k.migration != "exclude")
        {
            let suffix = rule.suffix.replace('\\', "/");
            by_leaf
                .entry(suffix.rsplit('/').next().unwrap().to_lowercase())
                .or_default()
                .push(rule);
            if rule.knowledge.as_ref().unwrap().scope == "home" && !suffix.contains('*') {
                for home in homes {
                    probe.push(home.join(PathBuf::from(suffix.clone())));
                }
            }
        }
    }
    let mut seen = HashSet::new();
    let mut found = vec![];
    for path in probe {
        if !seen.insert(platform::key(&path)) {
            continue;
        }
        let Some(leaf) = path.file_name().map(|n| n.to_string_lossy().to_lowercase()) else {
            continue;
        };
        if let Some(rules) = by_leaf.get(&leaf) {
            for rule in rules {
                if owned(&path, rule, homes)
                    && fs::symlink_metadata(&path)
                        .is_ok_and(|m| m.is_dir() || platform::reparse(&m))
                {
                    found.push(candidate(&path, rule));
                    break;
                }
            }
        }
    }
    found
}
pub fn candidate(path: &Path, rule: &Rule) -> Candidate {
    let key = format!("rule.{}", rule.id);
    let name = if crate::i18n::has_key(&key) {
        crate::i18n::t(&key)
    } else {
        &rule.name
    };
    let mut c = custom(path, name, &rule.category);
    c.note = crate::i18n::format("msg.d71de96a80", &[format!("{}", rule.id)]);
    c.processes = rule.processes.clone();
    c.knowledge = rule.knowledge.clone();
    c
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtin_is_valid() {
        validate(&builtin()).unwrap();
        let source: serde_json::Value =
            serde_json::from_str(include_str!("../rules/freeport-rules.json")).unwrap();
        assert_eq!(serde_json::to_value(builtin()).unwrap(), source);
    }
    #[test]
    fn owned_application_requires_scoped_evidence() {
        let root = std::env::temp_dir().join(format!("fp-knowledge-{}", std::process::id()));
        let home = root.join("home");
        let leaf = home.join(".claude/projects");
        fs::create_dir_all(&leaf).unwrap();
        let rule = builtin()
            .rules
            .into_iter()
            .find(|r| r.id == "claude-projects")
            .unwrap();
        assert!(!owned(&leaf, &rule, &[home.clone()]));
        fs::write(
            home.join(".claude/settings.json"),
            b"not parsed: credentials must never be read",
        )
        .unwrap();
        assert!(owned(&leaf, &rule, &[home.clone()]));
        assert!(!owned(&leaf, &rule, &[root.join("other")]));
        assert!(!owned(&leaf.join("child"), &rule, &[home.clone()]));
        fs::remove_file(home.join(".claude/settings.json")).unwrap();
        fs::remove_dir(&leaf).unwrap();
        fs::remove_dir(home.join(".claude")).unwrap();
        fs::remove_dir(&home).unwrap();
        fs::remove_dir(&root).unwrap();
    }
    #[test]
    fn review_and_application_roots_are_offered_but_exclusions_are_not() {
        let root = std::env::temp_dir().join(format!("fp-review-{}", std::process::id()));
        let home = root.join("home");
        let codex = home.join(".codex");
        fs::create_dir_all(codex.join("sessions")).unwrap();
        fs::create_dir_all(codex.join(".sandbox")).unwrap();
        fs::write(codex.join("config.toml"), b"not parsed").unwrap();
        let found = discover(&builtin(), &[home.clone()], &[codex.join(".sandbox")]);
        let ids: Vec<_> = found.iter().map(|c| c.note.clone()).collect();
        let migration = |path: &Path| {
            found
                .iter()
                .find(|c| platform::same(&c.source, path))
                .and_then(|c| c.knowledge.as_ref())
                .map(|k| k.migration.clone())
        };
        assert_eq!(migration(&codex).as_deref(), Some("review"), "{ids:?}");
        assert_eq!(
            migration(&codex.join("sessions")).as_deref(),
            Some("candidate")
        );
        assert_eq!(migration(&codex.join(".sandbox")), None);
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn legacy_name_only_rules_never_become_candidates() {
        let mut rule = builtin().rules.remove(0);
        rule.knowledge = None;
        assert!(!owned(
            Path::new(r"C:\Users\someone\.claude\projects"),
            &rule,
            &[]
        ));
    }
    #[test]
    fn profiles_match_exactly_one_component() {
        let rule = builtin()
            .rules
            .into_iter()
            .find(|r| r.id == "edge-cache")
            .unwrap();
        assert!(matches(
            Path::new(r"C:\Users\x\AppData\Local\Microsoft\Edge\User Data\Default\Cache"),
            &rule
        ));
        assert!(!matches(
            Path::new(r"C:\Users\x\AppData\Local\Microsoft\Edge\User Data\Default\nested\Cache"),
            &rule
        ));
    }
    #[test]
    fn ownership_and_policy_boundaries_are_strict() {
        for bad in [
            "../secret",
            "C:/secret",
            "a:stream",
            "a/CON.txt",
            "a/NUL",
            "a/..",
            "a/leaf.",
            "a/leaf ",
            "a/\nleaf",
            "a/*",
        ] {
            let mut pack = builtin();
            pack.rules[0].knowledge.as_mut().unwrap().markers = vec![vec![bad.into()]];
            assert!(validate(&pack).is_err(), "{bad:?}");
        }
        let mut pack = builtin();
        pack.rules[0].knowledge.as_mut().unwrap().confidence = "unverified".into();
        assert!(validate(&pack).is_err());
        let mut pack = builtin();
        pack.rules[0].knowledge.as_mut().unwrap().markers.clear();
        assert!(validate(&pack).is_err());
    }
    #[test]
    fn patterns_do_not_match_name_prefixes() {
        let r = builtin()
            .rules
            .into_iter()
            .find(|r| r.id == "claude-projects")
            .unwrap();
        assert!(matches(Path::new(r"E:\Users\other\.claude\projects"), &r));
        for n in [
            r"E:\Users\other\.claudex\projects",
            r"E:\Users\other\x.claude\projects",
            r"E:\Users\other\.claude\projects\child",
        ] {
            assert!(!matches(Path::new(n), &r));
        }
    }
    #[test]
    fn adversarial_rule_boundaries() {
        for bad in [
            "",
            "..",
            "a/../b",
            "/absolute",
            r"C:\data",
            r"\\server\data",
            "a//b",
            "a/pre*fix",
            "a/?",
            "a\0b",
            "./a",
        ] {
            let mut p = builtin();
            p.rules[0].suffix = bad.into();
            assert!(validate(&p).is_err(), "{bad:?}");
        }
    }
    #[test]
    fn unknown_fields_rejected() {
        assert!(serde_json::from_str::<Pack>(
            r#"{"schema_version":1,"revision":1,"rules":[],"command":"bad"}"#
        )
        .is_err());
    }
    #[test]
    fn duplicate_ids_rejected() {
        let mut p = builtin();
        p.rules.push(p.rules[0].clone());
        assert!(validate(&p).is_err());
    }
    #[test]
    fn five_thousand_path_boundaries() {
        let r = Rule {
            id: "test".into(),
            name: "test".into(),
            category: "test".into(),
            suffix: "cache/leaf".into(),
            processes: vec![],
            knowledge: None,
        };
        for i in 0..5000 {
            assert!(matches(Path::new(&format!(r"E:\root\{i}\cache\leaf")), &r));
            assert!(!matches(
                Path::new(&format!(r"E:\root\{i}\xcache\leaf")),
                &r
            ));
            assert!(!matches(
                Path::new(&format!(r"E:\root\{i}\cache\leaf-{i}")),
                &r
            ));
        }
    }
}
