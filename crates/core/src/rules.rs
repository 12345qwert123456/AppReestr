//! Curated rules from `data/rules/*.toml`, extendable by the user.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

const CLASSIFY: &str = include_str!("../../../data/rules/classify.toml");
const REPOS: &str = include_str!("../../../data/rules/repos.toml");
const PROTECT: &str = include_str!("../../../data/rules/protect.toml");

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct ClassifyRules {
    pub app_packages: Vec<String>,
    pub system_packages: Vec<String>,
    pub system_desktop_ids: Vec<String>,
    pub system_name_patterns: Vec<String>,
    pub system_name_patterns_weak: Vec<String>,
    pub system_groups: Vec<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct RepoRules {
    pub arch_official: Vec<String>,
    pub distro: BTreeMap<String, String>,
    pub same_name_repos: BTreeMap<String, String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct ProtectRules {
    pub critical: Vec<String>,
    pub session_processes: Vec<String>,
    pub desktop_components: Vec<String>,
    pub polkit_agents: Vec<String>,
}

#[derive(Debug, Default, Clone)]
pub struct Rules {
    pub classify: ClassifyRules,
    pub repos: RepoRules,
    pub protect: ProtectRules,
}

/// User additions: the same keys as the bundled files, in one file.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct UserRules {
    #[serde(flatten)]
    classify: ClassifyRules,
    critical: Vec<String>,
}

impl Rules {
    pub fn bundled() -> Self {
        Self {
            classify: toml::from_str(CLASSIFY).expect("bundled classify.toml"),
            repos: toml::from_str(REPOS).expect("bundled repos.toml"),
            protect: toml::from_str(PROTECT).expect("bundled protect.toml"),
        }
    }

    /// Bundled rules extended by `user_file` when it exists.
    pub fn load(user_file: &Path) -> Self {
        let mut rules = Self::bundled();
        let Ok(text) = std::fs::read_to_string(user_file) else { return rules };
        match toml::from_str::<UserRules>(&text) {
            Ok(user) => {
                let c = &mut rules.classify;
                c.app_packages.extend(user.classify.app_packages);
                c.system_packages.extend(user.classify.system_packages);
                c.system_desktop_ids.extend(user.classify.system_desktop_ids);
                c.system_name_patterns.extend(user.classify.system_name_patterns);
                c.system_name_patterns_weak.extend(user.classify.system_name_patterns_weak);
                c.system_groups.extend(user.classify.system_groups);
                rules.protect.critical.extend(user.critical);
            }
            Err(e) => tracing::warn!("ignoring {}: {e}", user_file.display()),
        }
        rules
    }
}

/// Glob match supporting only `*`.
pub fn glob(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !text.starts_with(first) || !text[first.len()..].ends_with(last) || text.len() < first.len() + last.len() {
        return false;
    }
    let mut rest = &text[first.len()..text.len() - last.len()];
    for mid in &parts[1..parts.len() - 1] {
        match rest.find(mid) {
            Some(i) => rest = &rest[i + mid.len()..],
            None => return false,
        }
    }
    true
}

pub fn any_glob<S: AsRef<str>>(patterns: &[S], text: &str) -> Option<String> {
    patterns.iter().find(|p| glob(p.as_ref(), text)).map(|p| p.as_ref().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globbing() {
        assert!(glob("lib*", "libreoffice"));
        assert!(glob("*-fonts-*", "noto-fonts-cjk"));
        assert!(glob("cachyos*", "cachyos-extra-v3"));
        assert!(!glob("*-headers", "linux-headers-x"));
        assert!(glob("a*b*c", "aXXbYYc"));
        assert!(!glob("ab*ba", "aba"));
        assert!(glob("exact", "exact"));
    }

    #[test]
    fn bundled_rules_parse() {
        let r = Rules::bundled();
        assert!(r.repos.arch_official.contains(&"extra".to_string()));
        assert!(!r.protect.session_processes.is_empty());
    }
}
