//! The real path of a folder picked from inside a sandbox.
//!
//! In a flatpak the folder picker answers through the document portal: a
//! folder comes back as `/run/user/UID/doc/<id>/<name>`, its copy in the
//! portal's own mount, not as the folder itself. git there is not in the
//! repository (and the path dies with the session), so it is turned back into
//! the folder's own path, which the sandbox reads (`--filesystem=host:ro`).

use std::path::{Component, Path, PathBuf};

/// `path` itself when it is not in the document portal, or when the portal
/// does not say where it really is.
pub fn resolve(path: PathBuf) -> PathBuf {
    let Some(document) = Document::parse(&path) else {
        return path;
    };
    match host_path(&document.id) {
        // Joining nothing would leave a trailing `/`, and the folder would
        // not be the same project as when added by its path.
        Some(host) if document.rest.as_os_str().is_empty() => visible(host),
        Some(host) => visible(host.join(document.rest)),
        None => path,
    }
}

/// A path in the document portal's mount.
#[derive(Debug, PartialEq, Eq)]
struct Document {
    id: String,
    /// What follows the exported file or folder itself.
    rest: PathBuf,
}

impl Document {
    /// `/run/user/UID/doc/<id>/<name>/…`, or the same under
    /// `doc/by-app/<app id>/`.
    fn parse(path: &Path) -> Option<Self> {
        let mut parts = path.components();
        if parts.next() != Some(Component::RootDir) {
            return None;
        }
        let mut next = || match parts.next() {
            Some(Component::Normal(part)) => part.to_str(),
            _ => None,
        };
        if (next()?, next()?) != ("run", "user") {
            return None;
        }
        next()?.parse::<u32>().ok()?;
        if next()? != "doc" {
            return None;
        }
        let mut id = next()?;
        if id == "by-app" {
            next()?;
            id = next()?;
        }
        let id = id.to_owned();
        // The exported file or folder's name.
        next()?;
        Some(Self {
            id,
            rest: parts.as_path().to_owned(),
        })
    }
}

#[cfg(target_os = "linux")]
fn host_path(id: &str) -> Option<PathBuf> {
    use ashpd::documents::{DocumentID, Documents};

    let id = DocumentID::from(id);
    let paths = futures_lite::future::block_on(async {
        Documents::new()
            .await?
            .host_paths(std::slice::from_ref(&id))
            .await
    });
    match paths {
        Ok(paths) => paths.get(&id).map(|path| path.as_ref().to_owned()),
        Err(error) => {
            eprintln!("backseating: asking the document portal for a folder's path: {error}");
            None
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn host_path(_id: &str) -> Option<PathBuf> {
    None
}

/// The host's path as the sandbox sees it: the host's own `/usr`, `/etc` and
/// the like are under `/run/host`.
fn visible(path: PathBuf) -> PathBuf {
    if path.exists() {
        return path;
    }
    let under_host = Path::new("/run/host").join(path.strip_prefix("/").unwrap_or(&path));
    if under_host.exists() {
        under_host
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_paths_are_recognised() {
        assert_eq!(
            Document::parse(Path::new("/run/user/1000/doc/a41d895a/backseating")),
            Some(Document {
                id: "a41d895a".into(),
                rest: PathBuf::new(),
            })
        );
        assert_eq!(
            Document::parse(Path::new(
                "/run/user/1000/doc/by-app/app.akergez.Backseating/a41d895a/repo/src"
            )),
            Some(Document {
                id: "a41d895a".into(),
                rest: "src".into(),
            })
        );
    }

    #[test]
    fn other_paths_are_left_alone() {
        for path in [
            "/var/home/user/repo",
            "/run/user/1000/doc",
            "/run/user/1000/doc/a41d895a",
            "/run/user/1000/gvfs/repo",
            "run/user/1000/doc/a41d895a/repo",
        ] {
            assert_eq!(Document::parse(Path::new(path)), None, "{path}");
            assert_eq!(resolve(path.into()), PathBuf::from(path));
        }
    }
}
