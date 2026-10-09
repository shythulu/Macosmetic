//! Text forms of a path for the clipboard: "Copy path" and "Copy path as".

use std::path::{Path, PathBuf};

/// How a path is written when copied to the clipboard.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathVariant {
    /// The absolute path as the OS spells it, e.g. `/Users/me/a b.txt`.
    Posix,
    /// The path with the home directory written as `~`, e.g. `~/a b.txt`.
    Tilde,
    /// The path quoted for a POSIX shell, e.g. `'/Users/me/a b.txt'`.
    ShellQuoted,
    /// The path as a percent-encoded `file://` URL, e.g. `file:///Users/me/a%20b.txt`.
    FileUrl,
    /// The final path component only, e.g. `a b.txt`.
    Name,
}

impl PathVariant {
    /// Every variant, in menu order.
    pub const ALL: [PathVariant; 5] = [
        PathVariant::Posix,
        PathVariant::Tilde,
        PathVariant::ShellQuoted,
        PathVariant::FileUrl,
        PathVariant::Name,
    ];
}

/// Format `paths` as `variant`, one path per line.
///
/// `home` is the directory [`PathVariant::Tilde`] abbreviates. An empty `home` disables the
/// abbreviation.
pub fn format_paths(paths: &[PathBuf], variant: PathVariant, home: &Path) -> String {
    paths
        .iter()
        .map(|path| format_path(path, variant, home))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_path(path: &Path, variant: PathVariant, home: &Path) -> String {
    match variant {
        PathVariant::Posix => path.display().to_string(),
        PathVariant::Tilde => tilde(path, home),
        PathVariant::ShellQuoted => shlex::quote(&path.to_string_lossy()).into_owned(),
        // `from_file_path` only fails for relative paths; fall back to the plain path then.
        PathVariant::FileUrl => url::Url::from_file_path(path)
            .map(String::from)
            .unwrap_or_else(|()| path.display().to_string()),
        PathVariant::Name => path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
    }
}

fn tilde(path: &Path, home: &Path) -> String {
    if home.as_os_str().is_empty() {
        return path.display().to_string();
    }
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/Users/me";

    fn one(path: &str, variant: PathVariant) -> String {
        format_paths(&[PathBuf::from(path)], variant, Path::new(HOME))
    }

    #[test]
    fn posix_is_the_plain_path() {
        assert_eq!(
            one("/Users/me/a.txt", PathVariant::Posix),
            "/Users/me/a.txt"
        );
        assert_eq!(
            one("/Users/me/a b.txt", PathVariant::Posix),
            "/Users/me/a b.txt"
        );
        assert_eq!(
            one("/Users/me/é.txt", PathVariant::Posix),
            "/Users/me/é.txt"
        );
    }

    #[test]
    fn tilde_replaces_home_prefix() {
        assert_eq!(
            one("/Users/me/docs/a.txt", PathVariant::Tilde),
            "~/docs/a.txt"
        );
        assert_eq!(one("/Users/me/a b.txt", PathVariant::Tilde), "~/a b.txt");
        assert_eq!(one("/Users/me", PathVariant::Tilde), "~");
    }

    #[test]
    fn tilde_ignores_paths_outside_home() {
        // A sibling that merely shares a string prefix is not under home.
        assert_eq!(
            one("/Users/me2/a.txt", PathVariant::Tilde),
            "/Users/me2/a.txt"
        );
        assert_eq!(
            one("/Volumes/x/a.txt", PathVariant::Tilde),
            "/Volumes/x/a.txt"
        );
    }

    #[test]
    fn tilde_with_empty_home_is_plain_path() {
        let paths = [PathBuf::from("/Users/me/a.txt")];
        assert_eq!(
            format_paths(&paths, PathVariant::Tilde, Path::new("")),
            "/Users/me/a.txt"
        );
    }

    #[test]
    fn shell_quoted_leaves_safe_paths_bare() {
        assert_eq!(
            one("/Users/me/a.txt", PathVariant::ShellQuoted),
            "/Users/me/a.txt"
        );
    }

    #[test]
    fn shell_quoted_handles_spaces_and_quotes() {
        assert_eq!(
            one("/Users/me/a b.txt", PathVariant::ShellQuoted),
            "'/Users/me/a b.txt'"
        );
        assert_eq!(
            one("/Users/me/it's.txt", PathVariant::ShellQuoted),
            "\"/Users/me/it's.txt\""
        );
    }

    #[test]
    fn shell_quoted_handles_unicode() {
        assert_eq!(
            one("/Users/me/café menu.txt", PathVariant::ShellQuoted),
            "'/Users/me/café menu.txt'"
        );
    }

    #[test]
    fn file_url_percent_encodes() {
        assert_eq!(
            one("/Users/me/a.txt", PathVariant::FileUrl),
            "file:///Users/me/a.txt"
        );
        assert_eq!(
            one("/Users/me/a b.txt", PathVariant::FileUrl),
            "file:///Users/me/a%20b.txt"
        );
        assert_eq!(
            one("/Users/me/café.txt", PathVariant::FileUrl),
            "file:///Users/me/caf%C3%A9.txt"
        );
        assert_eq!(
            one("/Users/me/50%.txt", PathVariant::FileUrl),
            "file:///Users/me/50%25.txt"
        );
    }

    #[test]
    fn file_url_of_relative_path_falls_back_to_plain_path() {
        assert_eq!(
            one("relative/a.txt", PathVariant::FileUrl),
            "relative/a.txt"
        );
    }

    #[test]
    fn name_is_final_component() {
        assert_eq!(one("/Users/me/docs/a.txt", PathVariant::Name), "a.txt");
        assert_eq!(one("/Users/me/My Folder", PathVariant::Name), "My Folder");
        assert_eq!(one("/Users/me/日本語.md", PathVariant::Name), "日本語.md");
    }

    #[test]
    fn name_of_root_is_the_path() {
        assert_eq!(one("/", PathVariant::Name), "/");
    }

    #[test]
    fn multiple_paths_are_one_per_line() {
        let paths = [
            PathBuf::from("/Users/me/a b.txt"),
            PathBuf::from("/Users/me/docs/c.txt"),
        ];
        let home = Path::new(HOME);
        assert_eq!(
            format_paths(&paths, PathVariant::Posix, home),
            "/Users/me/a b.txt\n/Users/me/docs/c.txt"
        );
        assert_eq!(
            format_paths(&paths, PathVariant::Tilde, home),
            "~/a b.txt\n~/docs/c.txt"
        );
        assert_eq!(
            format_paths(&paths, PathVariant::ShellQuoted, home),
            "'/Users/me/a b.txt'\n/Users/me/docs/c.txt"
        );
        assert_eq!(
            format_paths(&paths, PathVariant::FileUrl, home),
            "file:///Users/me/a%20b.txt\nfile:///Users/me/docs/c.txt"
        );
        assert_eq!(
            format_paths(&paths, PathVariant::Name, home),
            "a b.txt\nc.txt"
        );
    }

    #[test]
    fn no_paths_is_empty() {
        assert_eq!(format_paths(&[], PathVariant::Posix, Path::new(HOME)), "");
    }
}
