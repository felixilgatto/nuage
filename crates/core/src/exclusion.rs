use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{AccountId, RemotePath};

/// A rule excluding remote items from synchronisation for one account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exclusion {
    pub account: AccountId,
    pub rule: ExclusionRule,
    pub created_at: DateTime<Utc>,
    /// One of the default patterns ([`DEFAULT_PATTERNS`]), which can be
    /// turned off per account.
    #[serde(default)]
    pub default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ExclusionRule {
    /// A specific remote file or folder.
    Path(RemotePath),
    /// A pattern applied to the whole account, e.g. `*.tmp` or `node_modules/`.
    Glob(String),
}

/// Patterns excluded unless the user turns them off: temporary and lock
/// files of office suites and of GNOME's atomic saves.
pub const DEFAULT_PATTERNS: [&str; 4] = ["~$*", ".~lock.*#", "*.tmp", ".goutputstream-*"];

/// One compiled rule: path components to match.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Compiled {
    components: Vec<String>,
    /// Matches from the root only (a path, or a pattern starting with `/`).
    anchored: bool,
    /// Literal components (a path): no wildcard.
    literal: bool,
}

/// The exclusions of one account, ready to match paths.
///
/// Semantics follow rclone's filters, so that what `nuage-fs` keeps local
/// is exactly what rclone hides:
///
/// - a pattern without `/` (`*.tmp`, `node_modules`) matches a file or
///   folder name at any depth;
/// - a pattern with `/` matches consecutive names, from the root if it
///   starts with `/`, else at any depth;
/// - a trailing `/` (`node_modules/`) is accepted and ignored;
/// - `*` matches within a name, `**` across names, `?` one character;
/// - everything inside an excluded folder is excluded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExclusionSet {
    rules: Vec<Compiled>,
}

fn split(path: &str) -> Vec<String> {
    path.split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .map(str::to_owned)
        .collect()
}

/// Wildcard match of one name: `*` (any run), `?` (one character).
fn match_name(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Whether `pattern` (with `**` components) matches a prefix of `path`.
fn match_prefix(pattern: &[String], path: &[String], literal: bool) -> bool {
    match pattern.split_first() {
        None => true,
        Some((first, rest)) if !literal && first == "**" => {
            (0..=path.len()).any(|skip| match_prefix(rest, &path[skip..], literal))
        }
        Some((first, rest)) => match path.split_first() {
            Some((name, tail)) => {
                let ok = if literal {
                    first == name
                } else {
                    match_name(first, name)
                };
                ok && match_prefix(rest, tail, literal)
            }
            None => false,
        },
    }
}

/// Escapes rclone's glob characters in a literal path.
fn escape(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '{' | '}' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

impl ExclusionSet {
    /// Compiles rules. Path rules are relative to the account root.
    pub fn new<'a>(rules: impl IntoIterator<Item = &'a ExclusionRule>) -> Self {
        let rules = rules
            .into_iter()
            .filter_map(|rule| {
                let compiled = match rule {
                    ExclusionRule::Path(p) => Compiled {
                        components: split(&p.path),
                        anchored: true,
                        literal: true,
                    },
                    ExclusionRule::Glob(g) => Compiled {
                        components: split(g),
                        anchored: g.starts_with('/'),
                        literal: false,
                    },
                };
                (!compiled.components.is_empty()).then_some(compiled)
            })
            .collect();
        Self { rules }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Whether `path` (relative to the account root) is excluded.
    pub fn is_excluded(&self, path: &str) -> bool {
        let names = split(path);
        self.rules.iter().any(|rule| {
            let starts = if rule.anchored { 0..1 } else { 0..names.len() };
            starts.into_iter().any(|s| {
                s < names.len() && match_prefix(&rule.components, &names[s..], rule.literal)
            })
        })
    }

    /// The equivalent rclone filter rules (`_filter.FilterRule`).
    pub fn rclone_filter_rules<'a>(
        rules: impl IntoIterator<Item = &'a ExclusionRule>,
    ) -> Vec<String> {
        let mut out = Vec::new();
        for rule in rules {
            let base = match rule {
                ExclusionRule::Path(p) => {
                    let path = split(&p.path).join("/");
                    if path.is_empty() {
                        continue;
                    }
                    format!("/{}", escape(&path))
                }
                ExclusionRule::Glob(g) => {
                    let g = g.trim_end_matches('/');
                    if split(g).is_empty() {
                        continue;
                    }
                    g.to_owned()
                }
            };
            // The item itself, and everything inside it if it is a folder.
            out.push(format!("- {base}"));
            out.push(format!("- {base}/**"));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(globs: &[&str], paths: &[&str]) -> ExclusionSet {
        let rules: Vec<ExclusionRule> = globs
            .iter()
            .map(|g| ExclusionRule::Glob((*g).to_owned()))
            .chain(
                paths
                    .iter()
                    .map(|p| ExclusionRule::Path(RemotePath::new(AccountId::new("a"), *p))),
            )
            .collect();
        ExclusionSet::new(&rules)
    }

    #[test]
    fn names() {
        assert!(match_name("*.tmp", "a.tmp"));
        assert!(match_name("*.tmp", ".tmp"));
        assert!(!match_name("*.tmp", "a.tmp.odt"));
        assert!(match_name("~$*", "~$report.docx"));
        assert!(match_name(".~lock.*#", ".~lock.report.odt#"));
        assert!(match_name("a?c", "abc"));
        assert!(!match_name("a?c", "ac"));
        assert!(match_name("*", ""));
    }

    #[test]
    fn patterns_at_any_depth() {
        let s = set(&["*.tmp", "node_modules/"], &[]);
        assert!(s.is_excluded("x.tmp"));
        assert!(s.is_excluded("a/b/x.tmp"));
        assert!(s.is_excluded("proj/node_modules"));
        assert!(s.is_excluded("proj/node_modules/lib/index.js"));
        assert!(!s.is_excluded("proj/main.js"));
        assert!(!s.is_excluded(""));
    }

    #[test]
    fn anchored_and_multi_component() {
        let s = set(&["/build", "docs/*.pdf", "a/**/z"], &[]);
        assert!(s.is_excluded("build/out"));
        assert!(!s.is_excluded("src/build"));
        assert!(s.is_excluded("docs/x.pdf"));
        assert!(s.is_excluded("p/docs/x.pdf"));
        assert!(!s.is_excluded("docs/sub/x.pdf"));
        assert!(s.is_excluded("a/z"));
        assert!(s.is_excluded("a/b/c/z/file"));
    }

    #[test]
    fn paths_are_literal_and_anchored() {
        let s = set(&[], &["Archives", "Photos/2019 [old]*"]);
        assert!(s.is_excluded("Archives"));
        assert!(s.is_excluded("Archives/2020/a.zip"));
        assert!(!s.is_excluded("Work/Archives"));
        assert!(!s.is_excluded("Archives2"));
        assert!(s.is_excluded("Photos/2019 [old]*/x.jpg"));
        assert!(!s.is_excluded("Photos/2019 [old]x"));
    }

    #[test]
    fn defaults() {
        let rules: Vec<_> = DEFAULT_PATTERNS
            .iter()
            .map(|p| ExclusionRule::Glob((*p).to_owned()))
            .collect();
        let s = ExclusionSet::new(&rules);
        for path in [
            "~$Report.docx",
            "dir/.~lock.notes.odt#",
            "a.tmp",
            ".goutputstream-ABC123",
        ] {
            assert!(s.is_excluded(path), "{path}");
        }
        assert!(!s.is_excluded("Report.docx"));
    }

    #[test]
    fn rclone_rules() {
        let rules = [
            ExclusionRule::Glob("*.tmp".into()),
            ExclusionRule::Glob("node_modules/".into()),
            ExclusionRule::Path(RemotePath::new(AccountId::new("a"), "/My [files]")),
        ];
        assert_eq!(
            ExclusionSet::rclone_filter_rules(&rules),
            vec![
                "- *.tmp",
                "- *.tmp/**",
                "- node_modules",
                "- node_modules/**",
                "- /My \\[files\\]",
                "- /My \\[files\\]/**",
            ]
        );
    }
}
