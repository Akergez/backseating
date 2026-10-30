//! git, run in a working tree on this machine: what the standalone tool uses
//! directly and what Aiddiction's server runs in a project for its clients.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

use crate::source::{DiffRequest, Refs};

/// A diff larger than this is refused rather than sent to an interface that
/// would have to draw it.
pub const MAX_DIFF_BYTES: usize = 32 << 20;

pub struct LocalRepo {
    root: PathBuf,
}

impl LocalRepo {
    /// The repository `dir` is in. A leading `~` is the home folder, as a
    /// path typed by hand means it.
    pub fn open(dir: &Path) -> Result<Self, String> {
        let dir = &expand_home(dir);
        if !dir.is_dir() {
            return Err(format!("{} is not a folder", dir.display()));
        }
        let output = git(dir, &["rev-parse", "--show-toplevel"])?;
        let root = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !output.status.success() || root.is_empty() {
            // git's own word for it: "dubious ownership" is not the same
            // trouble as no repository at all.
            let stderr = String::from_utf8_lossy(&output.stderr);
            let reason = stderr.trim();
            return Err(if reason.is_empty() {
                format!("{} is not in a git repository", dir.display())
            } else {
                format!("{} is not in a git repository: {reason}", dir.display())
            });
        }
        Ok(Self {
            root: PathBuf::from(root),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The unified diff `request` asks for.
    pub fn diff(&self, request: &DiffRequest) -> Result<String, String> {
        request.source.validate()?;
        let mut args = vec!["diff".to_owned()];
        args.extend(request.git_args());
        let output = git(
            &self.root,
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
        )?;
        if !output.status.success() {
            // A repository with no commit yet has no HEAD to diff against:
            // everything there is is new.
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !(stderr.contains("bad revision 'HEAD'")
                || stderr.contains("ambiguous argument 'HEAD'"))
            {
                return Err(stderr.trim().to_owned());
            }
        }
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        if request.includes_untracked() {
            for path in self.untracked()? {
                text.push_str(&self.new_file_diff(&path, request.context)?);
                if text.len() > MAX_DIFF_BYTES {
                    break;
                }
            }
        }
        if text.len() > MAX_DIFF_BYTES {
            return Err(format!(
                "the diff is larger than {} MB",
                MAX_DIFF_BYTES >> 20
            ));
        }
        Ok(text)
    }

    fn untracked(&self) -> Result<Vec<String>, String> {
        let output = git(
            &self.root,
            &["ls-files", "--others", "--exclude-standard", "-z"],
        )?;
        Ok(output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| String::from_utf8_lossy(path).into_owned())
            .collect())
    }

    /// An untracked file as a diff adding it.
    fn new_file_diff(&self, path: &str, context: Option<u32>) -> Result<String, String> {
        let unified = context.map_or("-U1000000".to_owned(), |lines| format!("-U{lines}"));
        let output = git(
            &self.root,
            &[
                "diff",
                "--no-color",
                "--no-ext-diff",
                &unified,
                "--no-index",
                "--",
                "/dev/null",
                path,
            ],
        )?;
        // `--no-index` exits 1 when the files differ, which they always do.
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Branches and the last commits, for a picker.
    pub fn refs(&self) -> Result<Refs, String> {
        let lines = |args: &[&str]| -> Result<Vec<String>, String> {
            let output = git(&self.root, args)?;
            Ok(String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::to_owned)
                .filter(|line| !line.is_empty())
                .collect())
        };
        let branches = lines(&["for-each-ref", "--format=%(refname:short)", "refs/heads"])?;
        let current_branch = lines(&["symbolic-ref", "--short", "-q", "HEAD"])?
            .into_iter()
            .next();
        let default_branch = ["main", "master"]
            .into_iter()
            .find(|name| branches.iter().any(|branch| branch == name))
            .map(str::to_owned);
        let commits = lines(&["log", "-n", "100", "--format=%h%x09%s"])
            .unwrap_or_default()
            .into_iter()
            .filter_map(|line| {
                line.split_once('\t')
                    .map(|(hash, subject)| (hash.to_owned(), subject.to_owned()))
            })
            .collect();
        // `origin/HEAD` names another branch of the list; a remote's bare
        // name is what git shortens it to.
        let remote_branches = lines(&["for-each-ref", "--format=%(refname:short)", "refs/remotes"])
            .unwrap_or_default()
            .into_iter()
            .filter(|name| name.contains('/') && !name.ends_with("/HEAD"))
            .collect();
        let tags = lines(&[
            "for-each-ref",
            "--sort=-creatordate",
            "--count=100",
            "--format=%(refname:short)",
            "refs/tags",
        ])
        .unwrap_or_default();
        Ok(Refs {
            current_branch,
            default_branch,
            branches,
            commits,
            remote_branches,
            tags,
        })
    }
}

/// `~` and `~/…` as the home folder; any other path as it is.
fn expand_home(path: &Path) -> PathBuf {
    let home = || std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    match path.strip_prefix("~") {
        Ok(rest) => match home() {
            Some(home) => PathBuf::from(home).join(rest),
            None => path.to_owned(),
        },
        Err(_) => path.to_owned(),
    }
}

/// The variable that names the git to run, when it is nowhere it is looked for.
pub const GIT_VARIABLE: &str = "DIFFREVIEW_GIT";

/// The git to run, found once.
///
/// `git` on the `PATH` is not always there for a program started from a
/// desktop rather than a terminal: an application opened from the Finder has
/// the bare system `PATH`, without Homebrew's, and Git for Windows may not
/// have put itself on it. So when `git` does not run, the login shell is asked
/// where its `git` is, and then the places git is usually installed.
pub fn program() -> Option<&'static Path> {
    static PROGRAM: OnceLock<Option<PathBuf>> = OnceLock::new();
    PROGRAM.get_or_init(find_program).as_deref()
}

fn find_program() -> Option<PathBuf> {
    if let Some(named) = std::env::var_os(GIT_VARIABLE).filter(|named| !named.is_empty()) {
        return Some(named.into());
    }
    let runs = |program: &Path| {
        command(program)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
    };
    let plain = PathBuf::from("git");
    if runs(&plain) {
        return Some(plain);
    }
    login_shell_git()
        .into_iter()
        .chain(usual_places())
        .find(|candidate| candidate.is_file() && runs(candidate))
}

/// Where the login shell finds `git`: the `PATH` a terminal would have.
#[cfg(unix)]
fn login_shell_git() -> Option<PathBuf> {
    let shell = std::env::var_os("SHELL")?;
    let output = Command::new(shell)
        .args(["-l", "-c", "command -v git"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let found = text
        .lines()
        .map(str::trim)
        .rfind(|line| line.starts_with('/'))?;
    Some(PathBuf::from(found))
}

#[cfg(not(unix))]
fn login_shell_git() -> Option<PathBuf> {
    None
}

fn usual_places() -> Vec<PathBuf> {
    let mut places: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for (variable, under) in [
            ("ProgramFiles", r"Git\cmd\git.exe"),
            ("ProgramFiles(x86)", r"Git\cmd\git.exe"),
            ("LOCALAPPDATA", r"Programs\Git\cmd\git.exe"),
            ("USERPROFILE", r"scoop\shims\git.exe"),
        ] {
            if let Some(root) = std::env::var_os(variable) {
                places.push(PathBuf::from(root).join(under));
            }
        }
    } else {
        places.extend(
            [
                "/opt/homebrew/bin/git",
                "/usr/local/bin/git",
                "/opt/local/bin/git",
                "/usr/bin/git",
                "/run/current-system/sw/bin/git",
            ]
            .map(PathBuf::from),
        );
        if let Some(home) = std::env::var_os("HOME") {
            places.push(PathBuf::from(home).join(".nix-profile/bin/git"));
        }
    }
    places
}

fn command(program: &Path) -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut command = Command::new(program);
    // A windowed program on Windows would flash a console for every git.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Where the user's own git configuration is, when this runs in a flatpak.
///
/// A flatpak's `XDG_CONFIG_HOME` is the application's own folder, so git
/// would miss `~/.config/git`: the user's config, global ignore file and
/// attributes.
fn host_config_home() -> Option<PathBuf> {
    if !Path::new("/.flatpak-info").exists() {
        return None;
    }
    std::env::var_os("HOST_XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

fn git(dir: &Path, args: &[&str]) -> Result<Output, String> {
    let program = program().ok_or_else(|| {
        format!("git was not found: install it, or set {GIT_VARIABLE} to the git to run")
    })?;
    let mut command = command(program);
    if let Some(config_home) = host_config_home() {
        command.env("XDG_CONFIG_HOME", config_home);
    }
    command
        .args(args)
        .current_dir(dir)
        // Never wait for a person, never page, never translate messages that
        // are matched on above.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat")
        .env("LC_ALL", "C")
        .output()
        .map_err(|error| format!("running git: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{FileStatus, parse};
    use crate::source::DiffSource;

    fn run(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn uncommitted_staged_untracked_and_commits() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        run(root, &["init", "-q", "-b", "main"]);
        std::fs::write(root.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        run(root, &["add", "."]);
        run(root, &["commit", "-q", "-m", "first"]);
        std::fs::write(root.join("a.txt"), "one\n2\nthree\n").unwrap();
        std::fs::write(root.join("new.txt"), "fresh\n").unwrap();

        let repo = LocalRepo::open(root).unwrap();
        let files = parse(&repo.diff(&DiffRequest::default()).unwrap());
        let paths: Vec<_> = files
            .iter()
            .map(|file| (file.path.as_str(), file.status))
            .collect();
        assert_eq!(
            paths,
            [
                ("a.txt", FileStatus::Modified),
                ("new.txt", FileStatus::Added)
            ]
        );
        // The whole file is there by default.
        assert_eq!(files[0].lines.len(), 1 + 4);

        let staged = DiffRequest {
            source: DiffSource::Staged,
            ..DiffRequest::default()
        };
        assert!(parse(&repo.diff(&staged).unwrap()).is_empty());

        run(root, &["add", "a.txt"]);
        run(root, &["commit", "-q", "-m", "second"]);
        let last = DiffRequest {
            source: DiffSource::Between {
                from: "HEAD~1".into(),
                to: "HEAD".into(),
            },
            context: Some(0),
            untracked: false,
        };
        let files = parse(&repo.diff(&last).unwrap());
        assert_eq!(files.len(), 1);
        assert_eq!((files[0].added, files[0].removed), (1, 1));

        run(root, &["tag", "v1"]);
        let refs = repo.refs().unwrap();
        assert_eq!(refs.current_branch.as_deref(), Some("main"));
        assert_eq!(refs.default_branch.as_deref(), Some("main"));
        assert_eq!(refs.commits.len(), 2);
        assert_eq!(refs.tags, ["v1"]);
        assert!(refs.remote_branches.is_empty());
    }

    #[test]
    fn a_repository_without_commits_shows_its_files_as_new() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-q"]);
        std::fs::write(dir.path().join("x"), "x\n").unwrap();
        let repo = LocalRepo::open(dir.path()).unwrap();
        let files = parse(&repo.diff(&DiffRequest::default()).unwrap());
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn a_folder_outside_git_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        assert!(LocalRepo::open(dir.path()).is_err());
    }

    #[test]
    fn a_missing_folder_is_named_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone");
        let error = LocalRepo::open(&missing).err().unwrap();
        assert_eq!(error, format!("{} is not a folder", missing.display()));
    }

    #[test]
    fn a_tilde_is_the_home_folder() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let home = PathBuf::from(home);
        assert_eq!(expand_home(Path::new("~")), home);
        assert_eq!(expand_home(Path::new("~/src/x")), home.join("src/x"));
        assert_eq!(expand_home(Path::new("/srv/~x")), Path::new("/srv/~x"));
    }

    #[test]
    fn git_is_found() {
        assert!(program().is_some());
    }
}
