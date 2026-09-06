//! Argument expansion: rule filtering and `${placeholder}` substitution.

use std::collections::{BTreeMap, HashSet};

use crate::events::{Event, EventSink};

use super::rules::RuleContext;
use super::version::Argument;

/// Values substituted into `${name}` placeholders.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArgContext {
    /// Placeholder name (without `${}`) to value.
    pub vars: BTreeMap<String, String>,
}

/// Expands modern argument entries: drops the ones rules reject, substitutes placeholders.
pub fn expand_arguments(
    args: &[Argument],
    rules: &RuleContext,
    ctx: &ArgContext,
    sink: Option<&EventSink>,
) -> Vec<String> {
    let mut warned = HashSet::new();
    let mut out = Vec::new();
    for arg in args {
        match arg {
            Argument::Plain(value) => out.push(substitute(value, ctx, &mut warned, sink)),
            Argument::Conditional {
                rules: guards,
                value,
            } => {
                if super::rules::rules_allow(guards, rules) {
                    for value in value.as_slice() {
                        out.push(substitute(value, ctx, &mut warned, sink));
                    }
                }
            }
        }
    }
    out
}

/// Splits a pre-1.13 `minecraftArguments` string and substitutes placeholders.
pub fn expand_legacy(minecraft_arguments: &str, ctx: &ArgContext) -> Vec<String> {
    let mut warned = HashSet::new();
    minecraft_arguments
        .split_whitespace()
        .map(|value| substitute(value, ctx, &mut warned, None))
        .collect()
}

/// The JVM arguments old versions imply, since they carry no `arguments.jvm`.
pub fn default_legacy_jvm_args() -> Vec<String> {
    vec![
        "-Djava.library.path=${natives_directory}".to_string(),
        "-cp".to_string(),
        "${classpath}".to_string(),
    ]
}

/// Replaces every `${name}`. An unknown name stays in place and warns once per call.
fn substitute(
    value: &str,
    ctx: &ArgContext,
    warned: &mut HashSet<String>,
    sink: Option<&EventSink>,
) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        let (head, tail) = rest.split_at(start);
        out.push_str(head);
        let Some(end) = tail.find('}') else {
            out.push_str(tail);
            return out;
        };
        let name = &tail[2..end];
        match ctx.vars.get(name) {
            Some(v) => out.push_str(v),
            None => {
                out.push_str(&tail[..=end]);
                if warned.insert(name.to_string())
                    && let Some(sink) = sink
                {
                    let _ = sink.send(Event::Warning(format!(
                        "unknown launch argument placeholder ${{{name}}}, left as written"
                    )));
                }
            }
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mojang::version::VersionJson;

    const V1_20_1: &str = include_str!("../../../../tests/fixtures/mojang/1.20.1.json");

    fn linux_ctx() -> RuleContext {
        RuleContext {
            os_name: "linux",
            os_version: "6.1.0".to_string(),
            arch: "x86_64",
            features: BTreeMap::new(),
        }
    }

    fn vars(pairs: &[(&str, &str)]) -> ArgContext {
        ArgContext {
            vars: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    fn fixture() -> VersionJson {
        serde_json::from_str(V1_20_1).expect("fixture parses")
    }

    fn full_game_vars() -> ArgContext {
        vars(&[
            ("auth_player_name", "alice"),
            ("version_name", "1.20.1"),
            ("game_directory", "/games/mc"),
            ("assets_root", "/cache/assets"),
            ("assets_index_name", "5"),
            ("auth_uuid", "0123"),
            ("auth_access_token", "tok"),
            ("clientid", "cid"),
            ("auth_xuid", "xuid"),
            ("user_type", "msa"),
            ("version_type", "release"),
        ])
    }

    #[test]
    fn expands_game_arguments_and_leaves_no_placeholders() {
        let v = fixture();
        let args = v.arguments.expect("modern arguments");
        let out = expand_arguments(&args.game, &linux_ctx(), &full_game_vars(), None);
        let i = out
            .iter()
            .position(|a| a == "--username")
            .expect("--username present");
        assert_eq!(out[i + 1], "alice");
        assert!(!out.iter().any(|a| a.contains("${")), "{out:?}");
    }

    #[test]
    fn feature_gated_game_arguments_stay_out_by_default() {
        let v = fixture();
        let args = v.arguments.expect("modern arguments");
        let out = expand_arguments(&args.game, &linux_ctx(), &full_game_vars(), None);
        assert!(!out.iter().any(|a| a == "--demo"));
        assert!(!out.iter().any(|a| a == "--width"));
    }

    #[test]
    fn osx_only_jvm_argument_is_absent_on_linux() {
        let v = fixture();
        let args = v.arguments.expect("modern arguments");
        let ctx = vars(&[
            ("natives_directory", "/n"),
            ("launcher_name", "gcl"),
            ("launcher_version", "0.1.0"),
            ("classpath", "a.jar:b.jar"),
        ]);
        let out = expand_arguments(&args.jvm, &linux_ctx(), &ctx, None);
        assert!(!out.iter().any(|a| a == "-XstartOnFirstThread"));
        assert!(out.iter().any(|a| a == "-Djava.library.path=/n"));
        assert!(out.iter().any(|a| a == "a.jar:b.jar"));
    }

    #[tokio::test]
    async fn unknown_placeholder_stays_and_warns_once() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let args = vec![
            Argument::Plain("--a=${missing}".to_string()),
            Argument::Plain("--b=${missing}".to_string()),
            Argument::Plain("--c=${known}".to_string()),
        ];
        let out = expand_arguments(&args, &linux_ctx(), &vars(&[("known", "yes")]), Some(&tx));
        assert_eq!(out, vec!["--a=${missing}", "--b=${missing}", "--c=yes"]);
        let first = rx.recv().await.expect("one warning");
        assert!(
            matches!(&first, Event::Warning(m) if m.contains("missing")),
            "{first:?}"
        );
        drop(tx);
        assert!(rx.recv().await.is_none(), "only one warning per name");
    }

    #[test]
    fn legacy_arguments_split_and_substitute() {
        let out = expand_legacy(
            "--username ${auth_player_name} --version ${version_name}",
            &vars(&[("auth_player_name", "alice"), ("version_name", "1.8.9")]),
        );
        assert_eq!(out, vec!["--username", "alice", "--version", "1.8.9"]);
    }

    #[test]
    fn default_legacy_jvm_args_carry_natives_and_classpath() {
        assert_eq!(
            default_legacy_jvm_args(),
            vec![
                "-Djava.library.path=${natives_directory}",
                "-cp",
                "${classpath}"
            ]
        );
    }
}
