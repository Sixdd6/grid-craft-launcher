//! Rule evaluation for libraries and arguments: OS, architecture, and feature gates.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Whether a matching rule includes or excludes the item it guards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// A match includes the item.
    Allow,
    /// A match excludes the item.
    Disallow,
}

/// The `os` clause of a rule. Every present field must match.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct OsRule {
    /// `linux`, `windows`, or `osx`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Regex matched against the OS version string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// `x86`, `x86_64`, or `arm64`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
}

/// One entry of a `rules` array in a version JSON.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Rule {
    /// What a match does.
    pub action: Action,
    /// OS constraints, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<OsRule>,
    /// Launcher feature flags the rule requires, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<BTreeMap<String, bool>>,
}

/// The environment a rule set is evaluated against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleContext {
    /// `linux`, `windows`, or `osx`.
    pub os_name: &'static str,
    /// OS version string matched against `os.version` regexes. May be empty.
    pub os_version: String,
    /// `x86`, `x86_64`, or `arm64`.
    pub arch: &'static str,
    /// Launcher feature flags that are on. Absent names count as false.
    pub features: BTreeMap<String, bool>,
}

impl RuleContext {
    /// Builds a context for the machine this launcher runs on, with no features on.
    pub fn current() -> Self {
        RuleContext {
            os_name: current_os_name(),
            os_version: current_os_version(),
            arch: current_arch(),
            features: BTreeMap::new(),
        }
    }
}

/// The Mojang OS name for the target this binary was built for.
fn current_os_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

/// The Mojang architecture name for the target this binary was built for.
fn current_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "x86" => "x86",
        "aarch64" => "arm64",
        other => other,
    }
}

/// Best-effort OS version string. Empty when the platform does not expose one cheaply.
fn current_os_version() -> String {
    if cfg!(target_os = "linux") {
        std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    } else {
        String::new()
    }
}

/// Whether a rule set allows the item it guards: empty allows, otherwise the last match wins.
pub fn rules_allow(rules: &[Rule], ctx: &RuleContext) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for rule in rules {
        if rule_matches(rule, ctx) {
            allowed = matches!(rule.action, Action::Allow);
        }
    }
    allowed
}

/// Whether every clause a rule declares holds in this context.
fn rule_matches(rule: &Rule, ctx: &RuleContext) -> bool {
    if let Some(os) = &rule.os {
        if let Some(name) = &os.name
            && name != ctx.os_name
        {
            return false;
        }
        if let Some(arch) = &os.arch
            && arch != ctx.arch
        {
            return false;
        }
        if let Some(version) = &os.version
            && !version_matches(version, &ctx.os_version)
        {
            return false;
        }
    }
    if let Some(features) = &rule.features {
        for (name, wanted) in features {
            let have = ctx.features.get(name).copied().unwrap_or(false);
            if have != *wanted {
                return false;
            }
        }
    }
    true
}

/// Whether an `os.version` regex matches the context version. A bad regex never matches.
fn version_matches(pattern: &str, version: &str) -> bool {
    match regex::Regex::new(pattern) {
        Ok(re) => re.is_match(version),
        Err(err) => {
            tracing::warn!(pattern, %err, "ignoring invalid os.version regex");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(os: &'static str) -> RuleContext {
        RuleContext {
            os_name: os,
            os_version: "10.0".to_string(),
            arch: "x86_64",
            features: BTreeMap::new(),
        }
    }

    fn os_rule(action: Action, name: &str) -> Rule {
        Rule {
            action,
            os: Some(OsRule {
                name: Some(name.to_string()),
                ..OsRule::default()
            }),
            features: None,
        }
    }

    #[test]
    fn empty_rules_allow() {
        assert!(rules_allow(&[], &ctx("linux")));
    }

    #[test]
    fn allow_linux_matches_linux_only() {
        let rules = [os_rule(Action::Allow, "linux")];
        assert!(rules_allow(&rules, &ctx("linux")));
        assert!(!rules_allow(&rules, &ctx("windows")));
    }

    #[test]
    fn last_matching_rule_wins() {
        let rules = [
            Rule {
                action: Action::Allow,
                os: None,
                features: None,
            },
            os_rule(Action::Disallow, "osx"),
        ];
        assert!(!rules_allow(&rules, &ctx("osx")));
        assert!(rules_allow(&rules, &ctx("linux")));
    }

    #[test]
    fn unknown_feature_is_false() {
        let rules = [Rule {
            action: Action::Allow,
            os: None,
            features: Some(BTreeMap::from([("is_demo_user".to_string(), true)])),
        }];
        assert!(!rules_allow(&rules, &ctx("linux")));

        let mut on = ctx("linux");
        on.features.insert("is_demo_user".to_string(), true);
        assert!(rules_allow(&rules, &on));
    }

    #[test]
    fn arch_and_version_clauses_narrow_a_rule() {
        let rules = [Rule {
            action: Action::Allow,
            os: Some(OsRule {
                name: None,
                version: Some("^10\\.".to_string()),
                arch: Some("x86_64".to_string()),
            }),
            features: None,
        }];
        assert!(rules_allow(&rules, &ctx("windows")));

        let mut other = ctx("windows");
        other.os_version = "6.1".to_string();
        assert!(!rules_allow(&rules, &other));
    }

    #[test]
    fn current_context_names_a_known_os() {
        let c = RuleContext::current();
        assert!(["linux", "windows", "osx"].contains(&c.os_name));
    }
}
