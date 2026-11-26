//! Ghostty's own keys, for cmd and a number (decision record 0066).
//!
//! Ghostty binds cmd and a number to its own tabs twice over: once by the character the key types
//! (`super+1`) and once by the key itself (`super+digit_1`). While either stands, the chord never
//! reaches domux. `domux install ghostty --apply` adds the lines that unbind both, and at attach
//! the client asks Ghostty whether any still stand before it reads cmd at all.
//!
//! Ghostty answers for itself: `ghostty +list-keybinds` prints the bindings its config files
//! leave, so the answer covers every file it reads, in the order it reads them.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The numbers `D-1` to `D-9` open.
const DIGITS: std::ops::RangeInclusive<u8> = 1..=9;

/// The line above the ones an install adds, so a reader of the file knows what they are for.
pub const COMMENT: &str =
    "# cmd and a number open the domux agent with that number (domux install ghostty)";

/// The hint an attach in a Ghostty that keeps the chord leaves, until the reader's next key.
pub const KEEPS_HINT: &str = "Ghostty keeps cmd+1-9: run domux install ghostty --apply";

/// The lines that unbind cmd and a number in Ghostty, both forms of each number.
pub fn unbind_lines() -> Vec<String> {
    DIGITS
        .flat_map(|n| {
            [
                format!("keybind = cmd+{n}=unbind"),
                format!("keybind = cmd+digit_{n}=unbind"),
            ]
        })
        .collect()
}

/// The numbers Ghostty keeps with cmd and no other modifier, read from what `ghostty
/// +list-keybinds` printed. A chord that starts a sequence counts, because Ghostty takes it.
pub fn kept_digits(list: &str) -> Vec<u8> {
    let mut kept: Vec<u8> = list.lines().filter_map(kept_digit).collect();
    kept.sort_unstable();
    kept.dedup();
    kept
}

/// The number one `keybind = [prefix:]trigger[>next]=action` line keeps, if it keeps one.
fn kept_digit(line: &str) -> Option<u8> {
    let binding = line
        .trim()
        .strip_prefix("keybind")?
        .trim_start()
        .strip_prefix('=')?
        .trim();
    let (trigger, _action) = binding.split_once('=')?;
    let first = trigger.split('>').next()?;
    let chord = first.rsplit(':').next()?;
    let (mods, key) = chord.rsplit_once('+')?;
    if !matches!(mods, "super" | "cmd" | "command") {
        return None;
    }
    let n: u8 = key.strip_prefix("digit_").unwrap_or(key).parse().ok()?;
    DIGITS.contains(&n).then_some(n)
}

/// `cmd+1 to cmd+8` for a run, `cmd+1, cmd+3` otherwise: the chords a message names.
pub fn chords(kept: &[u8]) -> String {
    match kept {
        [] => String::new(),
        [n] => format!("cmd+{n}"),
        [first, .., last] if usize::from(last - first) + 1 == kept.len() => {
            format!("cmd+{first} to cmd+{last}")
        }
        _ => kept
            .iter()
            .map(|n| format!("cmd+{n}"))
            .collect::<Vec<_>>()
            .join(", "),
    }
}

/// The files Ghostty reads its config from, in the order it reads them, so a later one wins.
/// `config` is the name before Ghostty 1.3 and `config.ghostty` the name since, and Ghostty
/// reads both when both are there. macOS adds the two under Application Support, after the rest.
pub fn config_files(home: &Path, xdg_config_home: Option<&Path>, macos: bool) -> Vec<PathBuf> {
    let xdg = xdg_config_home
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(".config"))
        .join("ghostty");
    let mut files = vec![xdg.join("config"), xdg.join("config.ghostty")];
    if macos {
        let support = home.join("Library/Application Support/com.mitchellh.ghostty");
        files.push(support.join("config"));
        files.push(support.join("config.ghostty"));
    }
    files
}

/// The file an install writes: the last of `files` that is there, so the lines come after
/// everything Ghostty read before them, and the last of them, Ghostty's own choice for a new
/// file, when none is.
pub fn target(files: &[PathBuf]) -> Option<PathBuf> {
    files
        .iter()
        .rev()
        .find(|f| f.is_file())
        .or(files.last())
        .cloned()
}

/// Where Ghostty's binary is: in `GHOSTTY_BIN_DIR`, which Ghostty sets in the shells it starts,
/// then on `PATH`, then in the app on macOS.
pub fn binary(bin_dir: Option<&OsStr>, path: Option<&OsStr>) -> Option<PathBuf> {
    let in_bin_dir = bin_dir
        .filter(|d| !d.is_empty())
        .map(|d| Path::new(d).join("ghostty"));
    let on_path = path
        .map(|p| {
            std::env::split_paths(p)
                .map(|d| d.join("ghostty"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let in_app = cfg!(target_os = "macos")
        .then(|| PathBuf::from("/Applications/Ghostty.app/Contents/MacOS/ghostty"));
    in_bin_dir
        .into_iter()
        .chain(on_path)
        .chain(in_app)
        .find(|p| p.is_file())
}

/// What Ghostty's config files leave bound, as `ghostty +list-keybinds` prints it. `None` when
/// Ghostty could not be run or failed, which says nothing about the keys.
pub fn list_keybinds(binary: &Path) -> Option<String> {
    let out = Command::new(binary)
        .arg("+list-keybinds")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// What an install would write: the file with the unbind lines it does not have yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub path: PathBuf,
    /// The file as it was, or `None` when there was none.
    pub before: Option<String>,
    pub after: String,
    /// The lines `after` adds, the comment first. Empty when the file has every one.
    pub added: Vec<String>,
}

/// The plan for `path` holding `before`. A line counts as there whatever its spacing, so a
/// second install adds nothing.
pub fn plan(path: PathBuf, before: Option<String>) -> Plan {
    let squeeze = |l: &str| l.split_whitespace().collect::<String>();
    let text = before.clone().unwrap_or_default();
    let have: HashSet<String> = text.lines().map(squeeze).collect();
    let missing: Vec<String> = unbind_lines()
        .into_iter()
        .filter(|l| !have.contains(&squeeze(l)))
        .collect();
    if missing.is_empty() {
        return Plan {
            path,
            before,
            after: text,
            added: Vec::new(),
        };
    }
    let mut added = vec![COMMENT.to_string()];
    added.extend(missing);
    let mut after = text;
    if !after.is_empty() {
        if !after.ends_with('\n') {
            after.push('\n');
        }
        after.push('\n');
    }
    after.push_str(&added.join("\n"));
    after.push('\n');
    Plan {
        path,
        before,
        after,
        added,
    }
}

/// Writes `plan`: a copy of the old file beside it, then `path.tmp` renamed over `path`.
/// Answers where the old file was kept, or `None` when there was no file to keep.
pub fn apply(plan: &Plan, stamp: &str) -> std::io::Result<Option<PathBuf>> {
    if let Some(parent) = plan.path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let backup = match plan.before {
        Some(_) => {
            let backup = suffixed(&plan.path, &format!(".domux-backup-{stamp}"));
            std::fs::copy(&plan.path, &backup)?;
            Some(backup)
        }
        None => None,
    };
    let tmp = suffixed(&plan.path, ".tmp");
    std::fs::write(&tmp, &plan.after)?;
    std::fs::rename(&tmp, &plan.path)?;
    Ok(backup)
}

/// The path with `suffix` after the whole file name.
fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `ghostty +list-keybinds --default` prints for the numbers on macOS, with a few lines
    /// around it that keep no number.
    const DEFAULTS: &str = "keybind = super+digit_1=goto_tab:1
keybind = super+1=goto_tab:1
keybind = super+digit_8=goto_tab:8
keybind = super+8=goto_tab:8
keybind = super+9=last_tab
keybind = super+shift+digit_1=new_tab
keybind = ctrl+1=goto_tab:1
keybind = super+kp_1=goto_tab:1
keybind = super+t=new_tab
";

    #[test]
    fn kept_digits_reads_both_forms_and_only_cmd_alone() {
        assert_eq!(kept_digits(DEFAULTS), vec![1, 8, 9]);
        assert_eq!(kept_digits(""), Vec::<u8>::new());
    }

    /// A prefix such as `global:` and a sequence that starts with the chord still keep it.
    #[test]
    fn kept_digits_counts_prefixed_and_sequence_bindings() {
        let list = "keybind = global:super+2=goto_tab:2\nkeybind = super+digit_3>x=goto_tab:3\n";
        assert_eq!(kept_digits(list), vec![2, 3]);
    }

    #[test]
    fn chords_names_a_run_by_its_ends_and_a_gap_one_by_one() {
        assert_eq!(chords(&[1, 2, 3, 4, 5, 6, 7, 8]), "cmd+1 to cmd+8");
        assert_eq!(chords(&[9]), "cmd+9");
        assert_eq!(chords(&[1, 3]), "cmd+1, cmd+3");
    }

    #[test]
    fn config_files_are_in_ghosttys_order_with_application_support_last_on_macos() {
        let home = Path::new("/h");
        assert_eq!(
            config_files(home, None, true),
            vec![
                PathBuf::from("/h/.config/ghostty/config"),
                PathBuf::from("/h/.config/ghostty/config.ghostty"),
                PathBuf::from("/h/Library/Application Support/com.mitchellh.ghostty/config"),
                PathBuf::from(
                    "/h/Library/Application Support/com.mitchellh.ghostty/config.ghostty"
                ),
            ]
        );
        assert_eq!(
            config_files(home, Some(Path::new("/x")), false),
            vec![
                PathBuf::from("/x/ghostty/config"),
                PathBuf::from("/x/ghostty/config.ghostty"),
            ]
        );
    }

    /// The last file Ghostty reads wins, so an install writes the last one there; with none, the
    /// last of the list, which is Ghostty's own choice for a new file.
    #[test]
    fn target_is_the_last_file_there_or_the_last_of_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let files = config_files(dir.path(), None, true);
        assert_eq!(target(&files), Some(files[3].clone()));
        std::fs::create_dir_all(files[0].parent().unwrap()).unwrap();
        std::fs::write(&files[0], "font-size = 11\n").unwrap();
        assert_eq!(target(&files), Some(files[0].clone()));
        std::fs::create_dir_all(files[2].parent().unwrap()).unwrap();
        std::fs::write(&files[2], "").unwrap();
        assert_eq!(target(&files), Some(files[2].clone()));
    }

    /// The lines go after what the file held, under the comment, and a file that has some of them
    /// already, in any spacing, gets only the rest. A second plan adds nothing.
    #[test]
    fn plan_adds_only_the_missing_lines_once() {
        let before = "font-size = 11\nkeybind=cmd+1=unbind\n".to_string();
        let p = plan(PathBuf::from("/c"), Some(before.clone()));
        assert_eq!(p.added[0], COMMENT);
        assert_eq!(
            p.added.len(),
            1 + 17,
            "every line but the one already there"
        );
        assert!(p.after.starts_with(&format!("{before}\n{COMMENT}\n")));
        assert!(p.after.contains("keybind = cmd+digit_9=unbind\n"));
        let again = plan(PathBuf::from("/c"), Some(p.after.clone()));
        assert!(again.added.is_empty());
        assert_eq!(again.after, p.after);
        let fresh = plan(PathBuf::from("/c"), None);
        assert!(fresh.after.starts_with(COMMENT));
    }

    /// The write goes through a link to the file it names, so a config kept in a dotfiles
    /// repository stays there and the link stays a link.
    #[test]
    fn apply_writes_through_a_link_and_keeps_the_old_file() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("dotfiles-config");
        std::fs::write(&real, "font-size = 11\n").unwrap();
        let link = dir.path().join("config");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let path = std::fs::canonicalize(&link).unwrap();
        let p = plan(path, Some(std::fs::read_to_string(&link).unwrap()));
        let backup = apply(&p, "20261126120000")
            .unwrap()
            .expect("a file was there");
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read_to_string(&link).unwrap(), p.after);
        assert_eq!(std::fs::read_to_string(backup).unwrap(), "font-size = 11\n");
    }
}
