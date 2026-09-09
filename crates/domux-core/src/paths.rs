//! Where domux keeps its files, derived from `names` and the environment.
//!
//! Every public function reads the process environment once. The `_in` variants take an
//! explicit `Env` so tests and the harness can pin every input.

use crate::names::{
    CONFIG_DIR_NAME, SOCKET_DIR_PREFIX, SOCKET_FILE_NAME, STATE_DIR_NAME, V1_SESSIONS_DIR_NAME,
    V1_STATE_DIR_NAME,
};
use std::path::PathBuf;

/// The inputs the paths depend on.
#[derive(Debug, Clone)]
pub struct Env {
    pub home: PathBuf,
    pub xdg_runtime_dir: Option<PathBuf>,
    pub uid: u32,
    /// `DOMUX_SOCKET`: set in every pane by the server, so a shell inside a pane reaches its
    /// own server; also how tests point the CLI at a harness server.
    pub socket_override: Option<PathBuf>,
    /// `DOMUX_STATE_DIR`: for running a scratch server beside the real one.
    pub state_dir_override: Option<PathBuf>,
    /// `DOMUX_CONFIG_FILE`: same purpose.
    pub config_file_override: Option<PathBuf>,
}

impl Env {
    pub fn from_process() -> Env {
        let var = |name: &str| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Env {
            home: var("HOME").unwrap_or_else(|| PathBuf::from("/")),
            xdg_runtime_dir: var("XDG_RUNTIME_DIR"),
            uid: uid(),
            socket_override: var("DOMUX_SOCKET"),
            state_dir_override: var("DOMUX_STATE_DIR"),
            config_file_override: var("DOMUX_CONFIG_FILE"),
        }
    }
}

fn uid() -> u32 {
    // Safe: getuid has no preconditions and cannot fail.
    unsafe { libc_getuid() }
}

extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

pub fn state_dir_in(env: &Env) -> PathBuf {
    match &env.state_dir_override {
        Some(p) => p.clone(),
        None => env.home.join(".local").join("share").join(STATE_DIR_NAME),
    }
}

pub fn state_file_in(env: &Env) -> PathBuf {
    state_dir_in(env).join("state.json")
}

pub fn pr_cache_file_in(env: &Env) -> PathBuf {
    state_dir_in(env).join("pr-cache.json")
}

pub fn log_file_in(env: &Env) -> PathBuf {
    state_dir_in(env).join("server.log")
}

pub fn config_file_in(env: &Env) -> PathBuf {
    match &env.config_file_override {
        Some(p) => p.clone(),
        None => env
            .home
            .join(".config")
            .join(CONFIG_DIR_NAME)
            .join("domux.toml"),
    }
}

/// Where V1 keeps its session files, which `import v1` reads and nothing writes.
///
/// `DOMUX_STATE_DIR` does not reach it: that variable moves V2's state directory, and V1's
/// is somewhere else by definition. `import v1 --from` is how a caller reads a copy.
pub fn v1_sessions_dir_in(env: &Env) -> PathBuf {
    env.home
        .join(".local")
        .join("share")
        .join(V1_STATE_DIR_NAME)
        .join(V1_SESSIONS_DIR_NAME)
}

pub fn socket_path_in(env: &Env) -> PathBuf {
    if let Some(p) = &env.socket_override {
        return p.clone();
    }
    match &env.xdg_runtime_dir {
        Some(dir) => dir.join(SOCKET_FILE_NAME),
        None => PathBuf::from("/tmp")
            .join(format!("{SOCKET_DIR_PREFIX}{}", env.uid))
            .join(SOCKET_FILE_NAME),
    }
}

pub fn state_dir() -> PathBuf {
    state_dir_in(&Env::from_process())
}
pub fn state_file() -> PathBuf {
    state_file_in(&Env::from_process())
}
pub fn pr_cache_file() -> PathBuf {
    pr_cache_file_in(&Env::from_process())
}
pub fn log_file() -> PathBuf {
    log_file_in(&Env::from_process())
}
pub fn config_file() -> PathBuf {
    config_file_in(&Env::from_process())
}
pub fn socket_path() -> PathBuf {
    socket_path_in(&Env::from_process())
}
pub fn v1_sessions_dir() -> PathBuf {
    v1_sessions_dir_in(&Env::from_process())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn socket_path_uses_xdg_runtime_dir_when_set() {
        let env = Env {
            home: "/home/u".into(),
            xdg_runtime_dir: Some("/run/user/501".into()),
            uid: 501,
            socket_override: None,
            state_dir_override: None,
            config_file_override: None,
        };
        assert_eq!(
            socket_path_in(&env),
            PathBuf::from("/run/user/501/domux2.sock")
        );
    }

    #[test]
    fn socket_path_falls_back_to_tmp_with_uid() {
        let env = Env {
            home: "/home/u".into(),
            xdg_runtime_dir: None,
            uid: 501,
            socket_override: None,
            state_dir_override: None,
            config_file_override: None,
        };
        assert_eq!(
            socket_path_in(&env),
            PathBuf::from("/tmp/domux2-501/domux2.sock")
        );
    }

    #[test]
    fn socket_override_wins_over_everything() {
        let env = Env {
            home: "/home/u".into(),
            xdg_runtime_dir: Some("/run/user/501".into()),
            uid: 501,
            socket_override: Some("/tmp/x/s.sock".into()),
            state_dir_override: None,
            config_file_override: None,
        };
        assert_eq!(socket_path_in(&env), PathBuf::from("/tmp/x/s.sock"));
    }

    /// V1's directory is not V2's, and `DOMUX_STATE_DIR` moves only V2's. A build that read
    /// the override here would point `import v1` at V2's own state directory, where there
    /// are no session files, and after the M3 cut-over it would point at the directory V2
    /// writes.
    #[test]
    fn the_v1_sessions_directory_is_v1s_own_and_ignores_the_state_dir_override() {
        let env = Env {
            home: "/home/u".into(),
            xdg_runtime_dir: None,
            uid: 501,
            socket_override: None,
            state_dir_override: Some("/scratch/state".into()),
            config_file_override: None,
        };
        assert_eq!(
            v1_sessions_dir_in(&env),
            PathBuf::from("/home/u/.local/share/domux/sessions")
        );
        assert_ne!(v1_sessions_dir_in(&env), state_dir_in(&env));
    }

    #[test]
    fn state_and_config_paths_use_the_domux2_names() {
        let env = Env {
            home: "/home/u".into(),
            xdg_runtime_dir: None,
            uid: 501,
            socket_override: None,
            state_dir_override: None,
            config_file_override: None,
        };
        assert_eq!(
            state_dir_in(&env),
            PathBuf::from("/home/u/.local/share/domux2")
        );
        assert_eq!(
            state_file_in(&env),
            PathBuf::from("/home/u/.local/share/domux2/state.json")
        );
        assert_eq!(
            pr_cache_file_in(&env),
            PathBuf::from("/home/u/.local/share/domux2/pr-cache.json")
        );
        assert_eq!(
            log_file_in(&env),
            PathBuf::from("/home/u/.local/share/domux2/server.log")
        );
        assert_eq!(
            config_file_in(&env),
            PathBuf::from("/home/u/.config/domux2/domux.toml")
        );
    }
}
