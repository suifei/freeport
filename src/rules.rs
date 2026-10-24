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
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pack {
    pub schema_version: u32,
    pub revision: u64,
    pub rules: Vec<Rule>,
}
pub fn builtin() -> Pack {
    serde_json::from_str(include_str!("../rules/freeport-rules.json"))
        .expect("validated bundled rules")
}
pub fn validate(pack: &Pack) -> Result<()> {
    if pack.schema_version != 1
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
        if parts.len() > 64
            || parts.iter().any(|c| {
                c.is_empty() || *c == "." || *c == ".." || c.contains(['*', '?', '<', '>', '|'])
            })
        {
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
    p.ends_with(&("\\".to_owned() + &suffix))
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
    c
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtin_is_valid() {
        validate(&builtin()).unwrap();
    }
    #[test]
    fn patterns_do_not_match_name_prefixes() {
        let r = builtin()
            .rules
            .into_iter()
            .find(|r| r.suffix == ".claude")
            .unwrap();
        assert!(matches(Path::new(r"E:\Users\other\.claude"), &r));
        for n in [
            r"E:\Users\other\.claudex",
            r"E:\Users\other\x.claude",
            r"E:\Users\other\.claude\child",
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
            "a/*",
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
