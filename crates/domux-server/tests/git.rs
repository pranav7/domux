mod support;

use domux_server::git;
use support::{commit, git as run_git, repo_with_origin};

#[test]
fn default_branch_reads_origin_head_and_falls_back_to_main() {
    let (_tmp, repo) = repo_with_origin("develop");
    assert_eq!(git::default_branch(&repo), "develop");
    assert_eq!(git::base_ref(&repo, None), "origin/develop");
    assert_eq!(
        git::base_ref(&repo, Some("origin/release")),
        "origin/release",
        "the config wins"
    );
    let bare = tempfile::tempdir().unwrap();
    run_git(bare.path(), &["init", "-q"]);
    assert_eq!(
        git::default_branch(bare.path()),
        "main",
        "no origin/HEAD means main"
    );
}

#[test]
fn is_repo_tells_a_checkout_from_a_plain_folder() {
    let (_tmp, repo) = repo_with_origin("main");
    assert!(git::is_repo(&repo));
    let plain = tempfile::tempdir().unwrap();
    assert!(!git::is_repo(plain.path()));
}

#[test]
fn default_branch_answers_main_for_a_directory_that_is_not_a_repository() {
    // The one answer in this file that is a guess rather than a fact. It is V1's behaviour and
    // the brief's, so it is pinned here rather than removed: callers ask `is_repo` first, and a
    // change to this line has to change this test with it.
    let plain = tempfile::tempdir().unwrap();
    assert!(!git::is_repo(plain.path()));
    assert_eq!(git::default_branch(plain.path()), "main");
    // A directory that is not there at all is not a repository either.
    assert!(!git::is_repo(&plain.path().join("nowhere")));
}

#[test]
fn worktree_add_creates_the_slot_on_a_fresh_branch_from_the_base() {
    let (_tmp, repo) = repo_with_origin("main");
    let path = git::slot_path(&repo, 1);
    git::worktree_add(&repo, &path, "workspace-1", "origin/main").unwrap();
    assert!(path.join("README.md").is_file());
    assert_eq!(git::branch_of(&path).unwrap(), "workspace-1");
    assert_eq!(git::existing_slots(&repo).unwrap(), vec![1]);
}

/// A slot branch gets no upstream, and two slots can therefore be built at once.
///
/// Both halves are the same fact from two sides. Setting an upstream is a write to
/// `.git/config`, and git guards that with a lock file, so without `--no-track` one of two
/// concurrent adds fails with "could not lock config file" after it has already made its
/// branch: the slot is not built and a stray branch is left behind. The configuration
/// assertion is the deterministic half - the concurrency below is a race and could pass by
/// scheduling alone - and three rounds is what makes the race unlikely to be won three times.
///
/// `workspace.create` reaches this whenever two calls arrive together, which is one press of
/// the create key twice.
#[test]
fn a_slot_branch_has_no_upstream_so_two_slots_can_be_added_at_once() {
    let (_tmp, repo) = repo_with_origin("main");
    git::worktree_add(
        &repo,
        &git::slot_path(&repo, 1),
        "workspace-1",
        "origin/main",
    )
    .unwrap();
    let configured = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["config", "--get", "branch.workspace-1.remote"])
        .output()
        .unwrap();
    assert!(
        !configured.status.success() && configured.stdout.is_empty(),
        "a slot branch tracks nothing: {}",
        String::from_utf8_lossy(&configured.stdout)
    );

    for round in 0..3 {
        let (first, second) = (2 + round * 2, 3 + round * 2);
        let one = {
            let (repo, path) = (repo.clone(), git::slot_path(&repo, first));
            std::thread::spawn(move || {
                git::worktree_add(&repo, &path, &git::slot_branch(first), "origin/main")
            })
        };
        let two = {
            let (repo, path) = (repo.clone(), git::slot_path(&repo, second));
            std::thread::spawn(move || {
                git::worktree_add(&repo, &path, &git::slot_branch(second), "origin/main")
            })
        };
        one.join().unwrap().expect("the first add of the pair");
        two.join().unwrap().expect("the second add of the pair");
    }
    assert_eq!(
        git::existing_slots(&repo).unwrap(),
        vec![1, 2, 3, 4, 5, 6, 7]
    );
}

#[test]
fn worktree_add_resets_a_branch_that_already_exists() {
    let (_tmp, repo) = repo_with_origin("main");
    run_git(&repo, &["branch", "workspace-1"]);
    commit(&repo, "later.md", "later\n");
    run_git(&repo, &["push", "-q", "origin", "main"]);
    let path = git::slot_path(&repo, 1);
    git::worktree_add(&repo, &path, "workspace-1", "origin/main").unwrap();
    assert!(
        path.join("later.md").is_file(),
        "the stale branch was reset to the base"
    );
}

#[test]
fn a_dirty_or_unpushed_worktree_reports_dirty_and_a_clean_one_does_not() {
    let (_tmp, repo) = repo_with_origin("main");
    let path = git::slot_path(&repo, 1);
    git::worktree_add(&repo, &path, "workspace-1", "origin/main").unwrap();
    assert!(!git::is_dirty(&path, "workspace-1", "origin/main").unwrap());
    std::fs::write(path.join("scratch.txt"), "x").unwrap();
    assert!(
        git::is_dirty(&path, "workspace-1", "origin/main").unwrap(),
        "an untracked file is dirty"
    );
    run_git(&path, &["add", "scratch.txt"]);
    run_git(&path, &["commit", "-q", "-m", "Add scratch"]);
    assert!(
        git::is_dirty(&path, "workspace-1", "origin/main").unwrap(),
        "an unpushed commit is dirty"
    );
}

#[test]
fn reset_to_base_and_clean_return_the_slot_to_the_base_and_keep_ignored_files() {
    let (_tmp, repo) = repo_with_origin("main");
    // `.gitignore` belongs on the base, not on the slot's branch. A reset to `origin/main`
    // replaces the tree with what `origin/main` holds, so a `.gitignore` committed only on
    // `workspace-1` is gone by the time `clean` runs, and `.env` would then be an ordinary
    // untracked file that `git clean -fd` deletes. Committing and pushing it to `main`
    // first is what makes the ignore rule survive the reset, which is the behaviour this
    // test is about.
    std::fs::write(repo.join(".gitignore"), ".env\n").unwrap();
    run_git(&repo, &["add", ".gitignore"]);
    run_git(&repo, &["commit", "-q", "-m", "Ignore env"]);
    run_git(&repo, &["push", "-q", "origin", "main"]);
    let path = git::slot_path(&repo, 1);
    git::worktree_add(&repo, &path, "workspace-1", "origin/main").unwrap();
    std::fs::write(path.join(".env"), "SECRET=1").unwrap();
    std::fs::write(path.join("scratch.txt"), "x").unwrap();
    git::reset_to_base(&path, "workspace-1", "origin/main").unwrap();
    git::clean(&path).unwrap();
    assert!(!path.join("scratch.txt").exists(), "untracked files go");
    assert!(
        path.join(".gitignore").is_file(),
        "the base's own files come back"
    );
    assert!(
        path.join(".env").is_file(),
        "ignored files stay: the worktree.conf setup lives in them"
    );
    assert_eq!(
        git::branch_of(&path).unwrap(),
        "workspace-1",
        "the slot keeps its branch"
    );
    assert!(!git::is_dirty(&path, "workspace-1", "origin/main").unwrap());
}

#[test]
fn worktree_remove_takes_the_directory_and_the_branch_and_frees_the_number() {
    let (_tmp, repo) = repo_with_origin("main");
    let path = git::slot_path(&repo, 1);
    git::worktree_add(&repo, &path, "workspace-1", "origin/main").unwrap();
    git::worktree_remove(&repo, &path, "workspace-1", false).unwrap();
    assert!(!path.exists());
    assert!(git::existing_slots(&repo).unwrap().is_empty());
    assert!(!run_git(&repo, &["branch", "--list", "workspace-1"]).contains("workspace-1"));
}

#[test]
fn a_failed_git_command_names_the_command_and_git_s_own_message() {
    let plain = tempfile::tempdir().unwrap();
    let err = git::branch_of(plain.path()).unwrap_err();
    assert!(
        err.to_string()
            .starts_with("git rev-parse --abbrev-ref HEAD failed: "),
        "{err}"
    );
    assert!(err.to_string().contains("not a git repository"), "{err}");
}

// The tests above are the brief's. The ones below cover behaviour the brief describes but
// does not assert, each one motivated by a one-line change to `git.rs` that would otherwise
// leave the suite green.

#[test]
fn slot_path_and_slot_branch_name_the_slot_under_the_current_worktree_dir() {
    // Every caller reaches a worktree through these two, and every test above builds its
    // path with `slot_path` as well, so the literal answer is asserted once here: a wrong
    // join order or the legacy directory used for creation would read as correct anywhere
    // else.
    let root = std::path::Path::new("/tmp/audrey-app");
    assert_eq!(git::slot_branch(1), "workspace-1");
    assert_eq!(git::slot_branch(12), "workspace-12");
    assert_eq!(
        git::slot_path(root, 3),
        root.join(".domux").join("worktrees").join("workspace-3")
    );
    assert_eq!(git::WORKTREE_DIR, ".domux/worktrees");
    assert_eq!(git::LEGACY_WORKTREE_DIR, ".baag/worktrees");
}

#[test]
fn existing_slots_reads_both_worktree_dirs_and_sorts_by_number() {
    // Dropping `LEGACY_WORKTREE_DIR` from the scan, or sorting the names as strings, leaves
    // every other test green: nothing else here creates a legacy slot or a two-digit one.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let current = root.join(git::WORKTREE_DIR);
    let legacy = root.join(git::LEGACY_WORKTREE_DIR);
    for name in ["workspace-3", "workspace-10", "workspace-1"] {
        std::fs::create_dir_all(current.join(name)).unwrap();
    }
    // `workspace-1` in both directories is one slot, not two, and a legacy-only slot counts.
    std::fs::create_dir_all(legacy.join("workspace-1")).unwrap();
    std::fs::create_dir_all(legacy.join("workspace-2")).unwrap();
    // Neither a file nor a name that is not a slot number is a slot.
    std::fs::create_dir_all(current.join("workspace-abc")).unwrap();
    std::fs::create_dir_all(current.join("workspace-0")).unwrap();
    std::fs::create_dir_all(current.join("scratch")).unwrap();
    std::fs::write(current.join("workspace-7"), "a file, not a slot").unwrap();
    assert_eq!(git::existing_slots(root).unwrap(), vec![1, 2, 3, 10]);
}

#[test]
fn existing_slots_is_empty_when_no_worktree_dir_exists() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(git::existing_slots(tmp.path()).unwrap().is_empty());
}

#[test]
fn existing_slots_reports_a_worktree_dir_it_cannot_read() {
    // A directory that is missing is "no slots"; a directory that is there and unreadable is
    // not. Reporting the second as empty would hand Task 16 a slot number that is already
    // taken, and taking a slot that is taken is what removes someone's work.
    let tmp = tempfile::tempdir().unwrap();
    let current = tmp.path().join(git::WORKTREE_DIR);
    std::fs::create_dir_all(current.parent().unwrap()).unwrap();
    std::fs::write(&current, "not a directory").unwrap();
    let err = git::existing_slots(tmp.path()).unwrap_err();
    assert!(err.to_string().starts_with("read "), "{err}");
    assert!(err.to_string().contains(".domux/worktrees"), "{err}");
}

#[test]
fn worktree_add_recreates_a_slot_whose_directory_was_deleted_behind_git_s_back() {
    // git still holds the registration after an `rm -rf`, and both `git branch -f` and
    // `git worktree add` refuse while it stands. The prune inside `worktree_add` is what
    // clears it, and without this test dropping that line changes nothing.
    let (_tmp, repo) = repo_with_origin("main");
    let path = git::slot_path(&repo, 1);
    git::worktree_add(&repo, &path, "workspace-1", "origin/main").unwrap();
    std::fs::remove_dir_all(&path).unwrap();
    assert!(run_git(&repo, &["worktree", "list"]).contains("prunable"));
    git::worktree_add(&repo, &path, "workspace-1", "origin/main").unwrap();
    assert!(path.join("README.md").is_file());
    assert_eq!(git::branch_of(&path).unwrap(), "workspace-1");
}

#[test]
fn worktree_remove_refuses_a_slot_with_modified_or_untracked_files_unless_it_is_forced() {
    // This is the one call in V2 that deletes a directory the author was working in. An
    // unconditional `--force` would pass every other test in this file.
    let (_tmp, repo) = repo_with_origin("main");
    let path = git::slot_path(&repo, 1);
    git::worktree_add(&repo, &path, "workspace-1", "origin/main").unwrap();
    std::fs::write(path.join("scratch.txt"), "x").unwrap();
    let err = git::worktree_remove(&repo, &path, "workspace-1", false).unwrap_err();
    assert!(
        err.to_string().contains("modified or untracked files"),
        "{err}"
    );
    assert!(
        path.join("scratch.txt").is_file(),
        "the work is still there"
    );
    assert!(
        run_git(&repo, &["branch", "--list", "workspace-1"]).contains("workspace-1"),
        "a refused removal leaves the branch alone"
    );
    git::worktree_remove(&repo, &path, "workspace-1", true).unwrap();
    assert!(!path.exists());
    assert!(!run_git(&repo, &["branch", "--list", "workspace-1"]).contains("workspace-1"));
}

#[test]
fn worktree_remove_tolerates_a_branch_that_is_already_gone() {
    // A create that crashed halfway leaves the directory without the branch. Removing the
    // slot must still finish, and it must still finish for that reason only: any other
    // `git branch -D` failure is a real error.
    let (_tmp, repo) = repo_with_origin("main");
    let path = git::slot_path(&repo, 1);
    git::worktree_add(&repo, &path, "workspace-1", "origin/main").unwrap();
    run_git(&path, &["checkout", "-q", "--detach"]);
    run_git(&repo, &["branch", "-D", "workspace-1"]);
    git::worktree_remove(&repo, &path, "workspace-1", false).unwrap();
    assert!(!path.exists());
    assert!(git::existing_slots(&repo).unwrap().is_empty());
}

#[test]
fn worktree_remove_reports_a_branch_it_could_not_delete() {
    // "already gone" is the one `git branch -D` failure that is not a failure. Every other
    // one has to reach the caller: a branch still checked out somewhere else survives the
    // removal, and answering "deleted" would report an outcome that did not happen.
    let (_tmp, repo) = repo_with_origin("main");
    let path = git::slot_path(&repo, 1);
    let elsewhere = repo.join("elsewhere");
    git::worktree_add(&repo, &path, "workspace-1", "origin/main").unwrap();
    run_git(&path, &["checkout", "-q", "--detach"]);
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            elsewhere.to_str().unwrap(),
            "workspace-1",
        ],
    );
    let err = git::worktree_remove(&repo, &path, "workspace-1", false).unwrap_err();
    assert!(
        err.to_string()
            .starts_with("git branch -D workspace-1 failed: "),
        "{err}"
    );
    assert!(
        run_git(&repo, &["branch", "--list", "workspace-1"]).contains("workspace-1"),
        "the branch that could not be deleted is still there"
    );
}

#[test]
fn branch_of_reports_head_for_a_detached_slot_and_fails_before_the_first_commit() {
    let (_tmp, repo) = repo_with_origin("main");
    let path = git::slot_path(&repo, 1);
    git::worktree_add(&repo, &path, "workspace-1", "origin/main").unwrap();
    run_git(&path, &["checkout", "-q", "--detach"]);
    assert_eq!(
        git::branch_of(&path).unwrap(),
        "HEAD",
        "git spells 'no branch' as HEAD"
    );
    // Before the first commit git prints `HEAD` on stdout and fails at the same time.
    // Reading stdout and ignoring the exit code would report a branch named HEAD that the
    // repository does not have.
    let empty = tempfile::tempdir().unwrap();
    run_git(empty.path(), &["init", "-q"]);
    let err = git::branch_of(empty.path()).unwrap_err();
    assert!(err.to_string().contains("unknown revision"), "{err}");
}

#[test]
fn base_ref_ignores_a_configuration_that_is_blank() {
    let (_tmp, repo) = repo_with_origin("main");
    assert_eq!(git::base_ref(&repo, Some("")), "origin/main");
    assert_eq!(git::base_ref(&repo, Some("   ")), "origin/main");
    assert_eq!(
        git::base_ref(&repo, Some("  origin/release  ")),
        "origin/release",
        "a configured base is trimmed"
    );
}

#[test]
fn fetch_takes_the_remote_from_the_base_and_names_a_remote_that_is_not_there() {
    let (_tmp, repo) = repo_with_origin("main");
    git::fetch(&repo, "origin/main").unwrap();
    // A base with no remote in it is fetched from origin, which is what `git branch -f`
    // then resolves against.
    git::fetch(&repo, "main").unwrap();
    let err = git::fetch(&repo, "upstream/main").unwrap_err();
    assert!(
        err.to_string()
            .starts_with("git fetch -q -- upstream main failed: "),
        "{err}"
    );
}

#[test]
fn is_repo_is_false_for_a_bare_repository() {
    // A project root has to be a working tree: a bare repository has no files to open a
    // pane in. git answers "false" here with exit code zero, so an implementation that only
    // read the exit code would call it a checkout.
    let tmp = tempfile::tempdir().unwrap();
    let bare = tmp.path().join("origin.git");
    std::fs::create_dir_all(&bare).unwrap();
    run_git(&bare, &["init", "-q", "--bare"]);
    assert!(!git::is_repo(&bare));
}

#[test]
fn a_git_command_refuses_a_directory_that_is_not_absolute() {
    // `git -C ""` is a documented no-op: git runs in the process directory. An empty or
    // relative root would point every command in this file at whatever the server was started
    // in, `clean -fd` and `reset --hard` included.
    let err = git::clean(std::path::Path::new("")).unwrap_err();
    assert!(err.to_string().contains("is not an absolute path"), "{err}");
    assert!(err.to_string().contains("pass the project root"), "{err}");
    assert!(git::run(std::path::Path::new("some/relative/dir"), &["status"]).is_err());
    assert!(git::existing_slots(std::path::Path::new("")).is_err());
    // The slot path is a second directory, and `worktree_add` creates its parent with the
    // process's own working directory rather than git's, so it is refused separately.
    let (_tmp, repo) = repo_with_origin("main");
    let relative = std::path::Path::new(".domux/worktrees/workspace-1");
    assert!(git::worktree_add(&repo, relative, "workspace-1", "origin/main").is_err());
    assert!(git::worktree_remove(&repo, relative, "workspace-1", false).is_err());
    assert!(
        !std::path::Path::new(".domux").exists(),
        "a refused create makes no directory in the process's own directory"
    );
}

#[test]
fn reset_to_base_resets_the_slots_own_branch_when_another_one_is_checked_out() {
    // Clearing a slot resets the slot's branch. Without the checkout, `reset --hard` lands on
    // whatever HEAD is at, and a branch the author checked out inside the slot loses its
    // commits. This is the destructive operation with the fewest ways to notice it went wrong.
    let (_tmp, repo) = repo_with_origin("main");
    let path = git::slot_path(&repo, 1);
    git::worktree_add(&repo, &path, "workspace-1", "origin/main").unwrap();
    run_git(&path, &["checkout", "-q", "-b", "feature-x"]);
    commit(&path, "feature.md", "a week of work\n");
    let kept = run_git(&path, &["rev-parse", "feature-x"]);
    git::reset_to_base(&path, "workspace-1", "origin/main").unwrap();
    assert_eq!(
        git::branch_of(&path).unwrap(),
        "workspace-1",
        "the slot is back on its own branch"
    );
    assert_eq!(
        run_git(&path, &["rev-parse", "feature-x"]),
        kept,
        "the branch that is not the slot's is untouched"
    );
}

#[test]
fn worktree_add_refuses_a_slot_directory_that_is_already_there_before_it_moves_the_branch() {
    // The directory is present and its registration is gone, so pruning cannot clear it. git
    // would let `branch -f` succeed here and only then find the directory in the way, which
    // leaves the author's commit reachable from the reflog and nowhere else.
    let (_tmp, repo) = repo_with_origin("main");
    let path = git::slot_path(&repo, 1);
    git::worktree_add(&repo, &path, "workspace-1", "origin/main").unwrap();
    commit(&path, "feature.md", "a week of work\n");
    let kept = run_git(&repo, &["rev-parse", "workspace-1"]);
    std::fs::remove_dir_all(repo.join(".git/worktrees/workspace-1")).unwrap();
    let err = git::worktree_add(&repo, &path, "workspace-1", "origin/main").unwrap_err();
    assert!(err.to_string().contains("already there"), "{err}");
    assert_eq!(
        run_git(&repo, &["rev-parse", "workspace-1"]),
        kept,
        "the branch still points at the author's commit"
    );
    assert!(
        path.join("feature.md").is_file(),
        "and the files are still there"
    );
}

#[test]
fn worktree_add_accepts_a_slot_directory_that_is_empty() {
    // git accepts an empty directory, so the guard above must not be widened to `exists()`: a
    // leftover empty directory would then be a slot number nobody could ever create.
    let (_tmp, repo) = repo_with_origin("main");
    let path = git::slot_path(&repo, 1);
    std::fs::create_dir_all(&path).unwrap();
    git::worktree_add(&repo, &path, "workspace-1", "origin/main").unwrap();
    assert!(path.join("README.md").is_file());
}

#[test]
fn worktree_remove_finishes_when_the_directory_is_already_gone() {
    // Task 18 deletes a workspace whose directory the author may have removed by hand. git
    // exits zero here, and the branch still has to go.
    let (_tmp, repo) = repo_with_origin("main");
    let path = git::slot_path(&repo, 1);
    git::worktree_add(&repo, &path, "workspace-1", "origin/main").unwrap();
    std::fs::remove_dir_all(&path).unwrap();
    git::worktree_remove(&repo, &path, "workspace-1", false).unwrap();
    assert!(git::existing_slots(&repo).unwrap().is_empty());
    assert!(!run_git(&repo, &["branch", "--list", "workspace-1"]).contains("workspace-1"));
}

#[test]
fn is_dirty_compares_against_the_base_when_the_slot_has_no_upstream() {
    // The branch `is_dirty` takes for **every** slot `worktree_add` builds, since the add is
    // `--no-track` (decision record 0007). This is the normal path now, not the edge case it
    // was written as, and `is_dirty` is Task 18's gate in front of deleting a workspace. A
    // fixture here that set an upstream would take the main path back out of the suite.
    let (_tmp, repo) = repo_with_origin("main");
    let path = git::slot_path(&repo, 1);
    git::worktree_add(&repo, &path, "workspace-1", "origin/main").unwrap();
    assert!(!git::is_dirty(&path, "workspace-1", "origin/main").unwrap());
    commit(&path, "work.md", "a week of work\n");
    assert!(
        git::is_dirty(&path, "workspace-1", "origin/main").unwrap(),
        "an unpushed commit is dirty with no upstream too"
    );
}

/// A slot branched from a base that is not the default branch reads clean until it holds work
/// of its own, because `is_dirty` is told which base to measure against.
///
/// This is the case that made the base a parameter. Dropping the slot's upstream separated two
/// ranges that had silently coincided: while `worktree add -b` set an upstream, it pointed at
/// the **base**, so `@{u}..branch` and `origin/<default branch>..branch` were the same range
/// whenever the base was the default branch - and different the moment it was not. Guessing
/// `origin/main` here counts release's own commit as the slot's work, and a fresh workspace
/// answers dirty on the first create.
///
/// The second slot on the default base is the control: both must read clean, from different
/// bases, or the test says nothing about which base was used.
#[test]
fn a_slot_from_a_base_other_than_the_default_branch_reads_clean_until_it_holds_work() {
    let (_tmp, repo) = repo_with_origin("main");
    run_git(&repo, &["checkout", "-q", "-b", "release"]);
    commit(&repo, "release-only.md", "released\n");
    run_git(&repo, &["push", "-q", "origin", "release"]);
    run_git(&repo, &["checkout", "-q", "main"]);

    let path = git::slot_path(&repo, 1);
    git::worktree_add(&repo, &path, "workspace-1", "origin/release").unwrap();
    assert_eq!(
        run_git(&path, &["status", "--porcelain"]),
        "",
        "nothing has touched the slot"
    );
    assert!(
        !git::is_dirty(&path, "workspace-1", "origin/release").unwrap(),
        "a fresh slot from a base of its own is clean; measured against origin/main it would \
         hold release's commit and read dirty on the first create"
    );
    commit(&path, "work.md", "a week of work\n");
    assert!(
        git::is_dirty(&path, "workspace-1", "origin/release").unwrap(),
        "and its own commit is what makes it dirty"
    );

    // The same question on the default base, where the old guess and the base agree.
    let plain = git::slot_path(&repo, 2);
    git::worktree_add(&repo, &plain, "workspace-2", "origin/main").unwrap();
    assert!(!git::is_dirty(&plain, "workspace-2", "origin/main").unwrap());
}

/// The other branch of `is_dirty`: a slot whose upstream somebody set by hand is measured
/// against that upstream. Nothing `worktree_add` builds has one, so the fixture sets it.
#[test]
fn is_dirty_compares_against_the_upstream_when_the_slot_has_one() {
    let (_tmp, repo) = repo_with_origin("main");
    let path = git::slot_path(&repo, 1);
    git::worktree_add(&repo, &path, "workspace-1", "origin/main").unwrap();
    run_git(&path, &["push", "-q", "-u", "origin", "workspace-1"]);
    assert!(!git::is_dirty(&path, "workspace-1", "origin/main").unwrap());
    commit(&path, "work.md", "a week of work\n");
    assert!(
        git::is_dirty(&path, "workspace-1", "origin/main").unwrap(),
        "a commit the upstream does not have is dirty"
    );
    run_git(&path, &["push", "-q", "origin", "workspace-1"]);
    assert!(
        !git::is_dirty(&path, "workspace-1", "origin/main").unwrap(),
        "and pushing it makes the slot clean again, which the fallback range would not say"
    );
}

#[test]
fn a_base_or_branch_that_git_would_read_as_an_option_is_refused() {
    // `git fetch --upload-pack=<command> <remote> <branch>` runs that command. I confirmed it
    // against git 2.50 before writing this: the base reaches git positionally, and it comes
    // from `[worktrees] base` in the author's own configuration, so it is refused by name
    // rather than escaped.
    let (_tmp, repo) = repo_with_origin("main");
    let marker = repo.join("executed");
    let hostile = format!("--upload-pack=touch {}; git-upload-pack", marker.display());
    // Each call below has to fail for the right reason, not merely fail. Every one of these
    // arguments makes git itself error out a step or two later, so `is_err()` alone passes
    // with the guard deleted. The command the refusal names is what tells the guards apart,
    // and `fetch` guarding the base is not `worktree_add` guarding it.
    let refused = |err: git::GitError, command: &str| {
        let text = err.to_string();
        assert!(
            text.starts_with(&format!("{command} failed: ")),
            "expected {command} to refuse it: {text}"
        );
        assert!(text.contains("git reads as an option"), "{text}");
    };
    let err = git::fetch(&repo, &hostile).unwrap_err();
    assert!(err.to_string().contains("[worktrees] base"), "{err}");
    refused(err, "git fetch");
    assert!(!marker.exists(), "and nothing ran");
    // The halves of the base are checked, not only the whole string.
    refused(git::fetch(&repo, "origin/-x").unwrap_err(), "git fetch");
    refused(git::fetch(&repo, "-r/main").unwrap_err(), "git fetch");
    // Every operation that takes a base or a branch refuses one itself, rather than leaving it
    // to whichever command happens to run first. Each label names a command that actually runs
    // with the value it guards: `git worktree remove` takes the path and never the branch, so
    // that refusal is named after `git branch -D`, and `is_dirty` is named after `git log`
    // rather than the `rev-parse` probe whose failure it ignores.
    let path = git::slot_path(&repo, 1);
    let add = git::worktree_add(&repo, &path, "workspace-1", &hostile).unwrap_err();
    refused(add, "git worktree add");
    let add = git::worktree_add(&repo, &path, "-b", "origin/main").unwrap_err();
    refused(add, "git worktree add");
    let remove = git::worktree_remove(&repo, &path, "-D", false).unwrap_err();
    refused(remove, "git branch -D");
    refused(
        git::is_dirty(&repo, "--help", "origin/main").unwrap_err(),
        "git log",
    );
    refused(
        git::is_dirty(&repo, "workspace-1", &hostile).unwrap_err(),
        "git log",
    );
    let reset = git::reset_to_base(&repo, "-x", "origin/main").unwrap_err();
    refused(reset, "git checkout");
    let reset = git::reset_to_base(&repo, "workspace-1", "-x").unwrap_err();
    refused(reset, "git reset");
    assert!(!path.exists(), "and none of them made a slot");
}

#[test]
fn the_test_repositories_ignore_the_machines_own_git_configuration() {
    // A global `core.hooksPath` would run this machine's hooks inside these repositories and a
    // global `commit.gpgsign` would have them try to sign. Repository configuration wins, so
    // the isolation is checked here rather than assumed.
    let (_tmp, repo) = repo_with_origin("main");
    assert_eq!(run_git(&repo, &["config", "commit.gpgsign"]), "false");
    assert!(run_git(&repo, &["config", "core.hooksPath"]).ends_with("no-hooks"));
    assert!(
        !std::path::Path::new(&run_git(&repo, &["config", "core.hooksPath"])).exists(),
        "the hooks directory is deliberately not there"
    );
}
