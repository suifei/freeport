//! Application-owned candidates. Unknown dot directories are never auto-candidates.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub name: String,
    pub category: String,
    pub source: PathBuf,
    pub recommended: bool,
    pub note: String,
    pub processes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge: Option<crate::rules::Knowledge>,
}
pub fn custom(path: &Path, name: &str, category: &str) -> Candidate {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(crate::platform::key(path).as_bytes());
    Candidate {
        id: format!("{:x}", digest)[..12].into(),
        name: name.into(),
        category: category.into(),
        source: path.to_path_buf(),
        recommended: false,
        note: crate::i18n::t("msg.b99bd3a311").into(),
        processes: vec![],
        knowledge: None,
    }
}
pub fn discover(home: &Path, extra: &[PathBuf]) -> Vec<Candidate> {
    // Compatibility entry point; whole-volume discovery uses its existing index.
    crate::rules::discover(&crate::rules::builtin(), &[home.to_path_buf()], extra)
}
