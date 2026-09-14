//! Where the client runs, for `auto`: the one check that tells Omarchy from everything else.
//!
//! The client makes the check rather than the server. The server can be started by a service
//! or by an earlier session whose environment is not the reader's, two clients can attach from
//! two places, and the server never reads a client's environment.

use crate::omarchy;
use domux_core::theme::Desktop;
use std::path::Path;

/// The three signals the desktop is read from, as values rather than as `std::env` calls, so
/// `detect` is a pure function a test can drive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesktopEnv {
    /// `OMARCHY_PATH`, which Omarchy exports into every login shell.
    pub omarchy_path: Option<String>,
    /// Whether `$HOME/.local/state/omarchy/current/theme.name` is a file. Omarchy writes it on
    /// every theme change, so it holds for a terminal started without the session environment.
    pub omarchy_theme_file: bool,
    /// `SSH_CONNECTION` or `SSH_TTY` is set and not empty.
    pub remote: bool,
}

impl DesktopEnv {
    pub fn from_process() -> DesktopEnv {
        DesktopEnv::read(|name| std::env::var(name).ok())
    }

    /// The signals from `var`, which answers an environment variable by name. `HOME` comes
    /// from it too, so a test can point the theme file check at a directory of its own.
    fn read(var: impl Fn(&str) -> Option<String>) -> DesktopEnv {
        let set = |name: &str| var(name).filter(|v| !v.is_empty());
        DesktopEnv {
            omarchy_path: set("OMARCHY_PATH"),
            omarchy_theme_file: set("HOME")
                .is_some_and(|home| Path::new(&home).join(omarchy::THEME_NAME_FILE).is_file()),
            remote: set("SSH_CONNECTION").is_some() || set("SSH_TTY").is_some(),
        }
    }
}

/// A remote session is `Unknown` whatever else holds: the reader's desktop is on the other end
/// of the connection, and nothing on this host describes it. Otherwise Omarchy is found by its
/// path variable or by its theme file.
pub fn detect(env: &DesktopEnv) -> Desktop {
    if env.remote {
        Desktop::Unknown
    } else if env.omarchy_path.is_some() || env.omarchy_theme_file {
        Desktop::Omarchy
    } else {
        Desktop::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(omarchy_path: Option<&str>, omarchy_theme_file: bool, remote: bool) -> DesktopEnv {
        DesktopEnv {
            omarchy_path: omarchy_path.map(str::to_string),
            omarchy_theme_file,
            remote,
        }
    }

    #[test]
    fn desktop_detect_finds_omarchy_by_its_path_variable_or_its_theme_file() {
        let cases = [
            (None, false, Desktop::Unknown),
            (
                Some("/home/me/.local/share/omarchy"),
                false,
                Desktop::Omarchy,
            ),
            (None, true, Desktop::Omarchy),
            (
                Some("/home/me/.local/share/omarchy"),
                true,
                Desktop::Omarchy,
            ),
        ];
        for (path, file, want) in cases {
            assert_eq!(
                detect(&env(path, file, false)),
                want,
                "OMARCHY_PATH {path:?}, theme file {file}"
            );
        }
    }

    #[test]
    fn desktop_detect_answers_unknown_over_ssh() {
        for (path, file) in [
            (None, false),
            (Some("/home/me/.local/share/omarchy"), false),
            (None, true),
            (Some("/home/me/.local/share/omarchy"), true),
        ] {
            assert_eq!(
                detect(&env(path, file, true)),
                Desktop::Unknown,
                "OMARCHY_PATH {path:?}, theme file {file}"
            );
        }
    }

    /// An empty variable is not set, the theme file is looked for under `HOME`, and either ssh
    /// variable makes the session remote.
    #[test]
    fn desktop_env_reads_the_theme_file_under_home_and_either_ssh_variable() {
        let home = tempfile::tempdir().unwrap();
        let home_str = home.path().to_str().unwrap().to_string();
        let read = |vars: &[(&str, &str)]| {
            let vars: HashMap<String, String> = vars
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            DesktopEnv::read(|name| vars.get(name).cloned())
        };
        assert_eq!(read(&[("HOME", &home_str)]), DesktopEnv::default());
        assert_eq!(
            read(&[("HOME", &home_str), ("OMARCHY_PATH", ""), ("SSH_TTY", "")]),
            DesktopEnv::default(),
            "an empty variable is not set"
        );
        assert_eq!(
            read(&[("OMARCHY_PATH", "/o")]).omarchy_path.as_deref(),
            Some("/o")
        );
        assert!(read(&[("SSH_TTY", "/dev/pts/3")]).remote);
        assert!(read(&[("SSH_CONNECTION", "10.0.0.1 22 10.0.0.2 5000")]).remote);

        let name = home.path().join(omarchy::THEME_NAME_FILE);
        std::fs::create_dir_all(name.parent().unwrap()).unwrap();
        assert!(
            !read(&[("HOME", &home_str)]).omarchy_theme_file,
            "the directory alone is not the file"
        );
        std::fs::write(&name, "ristretto\n").unwrap();
        assert!(read(&[("HOME", &home_str)]).omarchy_theme_file);
        assert!(
            !read(&[]).omarchy_theme_file,
            "no HOME, nowhere to look for it"
        );
    }
}
