//! What to review: the git arguments for each kind of diff revdiff offers.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum DiffSource {
    /// The working tree against the index… and the index against HEAD: what
    /// is not committed, staged or not.
    Uncommitted,
    /// What is staged.
    Staged,
    /// The working tree against a ref (`HEAD~3`, `main`).
    Against { base: String },
    /// One ref against another (`HEAD~1` `HEAD`).
    Between { from: String, to: String },
    /// What `head` has since it parted from `base` (`main...feature`).
    Since { base: String, head: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffRequest {
    pub source: DiffSource,
    /// Lines of context around a change; `None` is the whole file.
    #[serde(default)]
    pub context: Option<u32>,
    /// Files git does not track yet, shown as added.
    #[serde(default)]
    pub untracked: bool,
}

impl Default for DiffRequest {
    fn default() -> Self {
        Self {
            source: DiffSource::Uncommitted,
            context: None,
            untracked: true,
        }
    }
}

/// What a picker offers: branches, tags and recent commits.
///
/// Fields added later carry `#[serde(default)]`, so a client still reads
/// what an older server sends.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Refs {
    pub current_branch: Option<String>,
    /// `main` or `master`, whichever the repository has.
    pub default_branch: Option<String>,
    pub branches: Vec<String>,
    /// `(hash, subject)`, newest first.
    pub commits: Vec<(String, String)>,
    /// The remotes' branches as git names them (`origin/main`).
    #[serde(default)]
    pub remote_branches: Vec<String>,
    /// Newest first.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// The newer end of a comparison: a ref, or what is not committed yet.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Point {
    /// The files on disk, with what is staged.
    WorkingTree,
    /// What is staged.
    Staged,
    Ref {
        name: String,
    },
}

/// A comparison as a picker shows it: from `base` to `target`, and whether
/// only what `target` added since it parted from `base` counts (`base...target`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    pub base: String,
    pub target: Point,
    pub since: bool,
}

impl Comparison {
    /// The source that compares this. The index is only ever compared with
    /// HEAD, as `git diff --cached` does, so `base` does not count there.
    pub fn source(&self) -> DiffSource {
        match &self.target {
            Point::WorkingTree if self.base == "HEAD" => DiffSource::Uncommitted,
            Point::WorkingTree => DiffSource::Against {
                base: self.base.clone(),
            },
            Point::Staged => DiffSource::Staged,
            Point::Ref { name } if self.since => DiffSource::Since {
                base: self.base.clone(),
                head: name.clone(),
            },
            Point::Ref { name } => DiffSource::Between {
                from: self.base.clone(),
                to: name.clone(),
            },
        }
    }
}

impl From<&DiffSource> for Comparison {
    fn from(source: &DiffSource) -> Self {
        let at = |base: &str, target: Point, since: bool| Comparison {
            base: base.to_owned(),
            target,
            since,
        };
        match source {
            DiffSource::Uncommitted => at("HEAD", Point::WorkingTree, false),
            DiffSource::Staged => at("HEAD", Point::Staged, false),
            DiffSource::Against { base } => at(base, Point::WorkingTree, false),
            DiffSource::Between { from, to } => at(from, Point::Ref { name: to.clone() }, false),
            DiffSource::Since { base, head } => at(base, Point::Ref { name: head.clone() }, true),
        }
    }
}

impl DiffSource {
    /// A label for the interface.
    pub fn label(&self) -> String {
        match self {
            DiffSource::Uncommitted => "Uncommitted".into(),
            DiffSource::Staged => "Staged".into(),
            DiffSource::Against { base } => format!("{base} → working tree"),
            DiffSource::Between { from, to } => format!("{from}..{to}"),
            DiffSource::Since { base, head } => format!("{base}...{head}"),
        }
    }

    /// The refs named, refused if one could be read as an option.
    fn refs(&self) -> Vec<&str> {
        match self {
            DiffSource::Uncommitted | DiffSource::Staged => vec![],
            DiffSource::Against { base } => vec![base],
            DiffSource::Between { from, to } => vec![from, to],
            DiffSource::Since { base, head } => vec![base, head],
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        for name in self.refs() {
            if name.is_empty()
                || name.starts_with('-')
                || name.chars().any(|c| c.is_whitespace() || c.is_control())
            {
                return Err(format!("not a ref: {name:?}"));
            }
        }
        Ok(())
    }
}

impl DiffRequest {
    /// `git diff …` arguments, after `diff`.
    pub fn git_args(&self) -> Vec<String> {
        let mut args = vec![
            "--no-color".to_owned(),
            "--no-ext-diff".to_owned(),
            "-M".to_owned(),
        ];
        args.push(match self.context {
            Some(lines) => format!("-U{lines}"),
            // As much context as any file has: the whole file.
            None => "-U1000000".to_owned(),
        });
        match &self.source {
            DiffSource::Uncommitted => args.push("HEAD".into()),
            DiffSource::Staged => args.push("--cached".into()),
            DiffSource::Against { base } => args.push(base.clone()),
            DiffSource::Between { from, to } => {
                args.push(from.clone());
                args.push(to.clone());
            }
            DiffSource::Since { base, head } => args.push(format!("{base}...{head}")),
        }
        args.push("--".into());
        args
    }

    /// Whether untracked files belong in this diff: only one that ends at
    /// the working tree.
    pub fn includes_untracked(&self) -> bool {
        self.untracked
            && matches!(
                self.source,
                DiffSource::Uncommitted | DiffSource::Against { .. }
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_for_each_source() {
        let request = |source| DiffRequest {
            source,
            context: Some(3),
            untracked: false,
        };
        assert_eq!(
            request(DiffSource::Staged).git_args()[4..],
            ["--cached", "--"]
        );
        assert_eq!(
            request(DiffSource::Since {
                base: "main".into(),
                head: "HEAD".into()
            })
            .git_args()[4..],
            ["main...HEAD", "--"]
        );
        assert!(
            DiffSource::Against {
                base: "--output=/etc/x".into()
            }
            .validate()
            .is_err()
        );
        assert!(
            DiffSource::Between {
                from: "HEAD~1".into(),
                to: "HEAD".into()
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn a_comparison_is_a_source_and_back() {
        let sources = [
            DiffSource::Uncommitted,
            DiffSource::Staged,
            DiffSource::Against {
                base: "main".into(),
            },
            DiffSource::Between {
                from: "v1.0.0".into(),
                to: "HEAD".into(),
            },
            DiffSource::Since {
                base: "main".into(),
                head: "feature".into(),
            },
        ];
        for source in sources {
            assert_eq!(Comparison::from(&source).source(), source);
        }
        let staged = Comparison {
            base: "main".into(),
            target: Point::Staged,
            since: true,
        };
        assert_eq!(staged.source(), DiffSource::Staged);
    }

    #[test]
    fn refs_from_an_older_server_still_read() {
        let refs: Refs = serde_json::from_str(r#"{"currentBranch":"main","defaultBranch":"main","branches":["main"],"commits":[["abc","x"]]}"#).unwrap();
        assert!(refs.tags.is_empty() && refs.remote_branches.is_empty());
        assert_eq!(refs.commits.len(), 1);
    }
}
