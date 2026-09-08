use domux_server::worktree_conf::{apply, parse, run_lines, Summary, Verb};
use std::path::Path;

#[test]
fn parse_reads_the_three_verbs_and_warns_about_everything_else() {
    let text =
        "# setup\n\nlink .env\ncopy config/local.yml\nrun bin/setup --fast\nlink\nteleport foo\n";
    let (directives, warnings) = parse(text);
    assert_eq!(directives.len(), 3);
    assert_eq!(directives[0].verb, Verb::Link);
    assert_eq!(directives[0].arg, ".env");
    assert_eq!(directives[1].verb, Verb::Copy);
    assert_eq!(directives[2].verb, Verb::Run);
    assert_eq!(
        directives[2].arg, "bin/setup --fast",
        "run keeps the rest of the line"
    );
    assert_eq!(
        warnings,
        vec![
            "link: missing argument".to_string(),
            "unknown directive \"teleport foo\"".to_string()
        ]
    );
}

#[test]
fn link_points_at_the_main_checkout_and_copy_makes_an_independent_file() {
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main");
    let slot = tmp.path().join("workspace-1");
    std::fs::create_dir_all(main.join("config")).unwrap();
    std::fs::create_dir_all(&slot).unwrap();
    std::fs::write(main.join(".env"), "SECRET=1").unwrap();
    std::fs::write(main.join("config/local.yml"), "port: 3000").unwrap();
    let (directives, _) = parse("link .env\ncopy config/local.yml\nrun bin/setup\n");
    let applied = apply(&main, &slot, &directives);
    assert!(slot.join(".env").is_symlink());
    assert_eq!(
        std::fs::read_link(slot.join(".env")).unwrap(),
        main.join(".env")
    );
    assert!(!slot.join("config/local.yml").is_symlink());
    assert_eq!(
        std::fs::read_to_string(slot.join("config/local.yml")).unwrap(),
        "port: 3000"
    );
    std::fs::write(slot.join("config/local.yml"), "port: 4000").unwrap();
    assert_eq!(
        std::fs::read_to_string(main.join("config/local.yml")).unwrap(),
        "port: 3000",
        "the copy is independent"
    );
    assert_eq!(applied.summary().to_string(), "linked 1, copied 1");
    assert_eq!(
        run_lines(&directives),
        vec!["bin/setup".to_string()],
        "run is left for the pane"
    );
}

#[test]
fn a_missing_source_is_skipped_and_counted_and_the_rest_still_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main");
    let slot = tmp.path().join("workspace-1");
    std::fs::create_dir_all(&main).unwrap();
    std::fs::create_dir_all(&slot).unwrap();
    std::fs::write(main.join("kept.txt"), "yes").unwrap();
    std::os::unix::fs::symlink(main.join("nowhere"), main.join("dangling")).unwrap();
    let (directives, _) = parse("link gone.txt\ncopy kept.txt\nlink dangling\ncopy gone.txt\n");
    let applied = apply(&main, &slot, &directives);
    assert!(slot.join("kept.txt").is_file());
    assert!(
        slot.join("dangling").is_symlink(),
        "a link asks whether the name is taken, not whether it leads anywhere"
    );
    assert_eq!(
        applied.summary().to_string(),
        "linked 1, copied 1, 2 skipped"
    );
    assert_eq!(
        applied.failures(),
        vec![
            "link gone.txt: source missing: gone.txt".to_string(),
            "copy gone.txt: source missing: gone.txt".to_string()
        ]
    );
}

#[test]
fn applying_twice_leaves_the_same_result_and_applying_to_the_checkout_itself_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main");
    let slot = tmp.path().join("workspace-1");
    std::fs::create_dir_all(&main).unwrap();
    std::fs::create_dir_all(&slot).unwrap();
    std::fs::write(main.join(".env"), "SECRET=1").unwrap();
    let (directives, _) = parse("link .env\n");
    apply(&main, &slot, &directives);
    apply(&main, &slot, &directives);
    assert_eq!(
        std::fs::read_link(slot.join(".env")).unwrap(),
        main.join(".env"),
        "a second apply is a no-op"
    );
    let applied = apply(&main, &main, &directives);
    assert_eq!(
        applied.failures(),
        vec!["link .env: refusing to apply the setup to the main checkout itself".to_string()],
        "a link onto itself would replace the real file with a loop"
    );
    assert_eq!(
        std::fs::read_to_string(main.join(".env")).unwrap(),
        "SECRET=1"
    );
}

#[test]
fn parse_drops_an_argument_that_carries_a_control_character() {
    // A run line is typed into the workspace's first pane. A carriage return in it submits
    // early, so the pane runs a second command the reader of the file never saw on that line.
    let (directives, warnings) = parse("run make\rrm -rf ~\nlink .env\rx\ncopy ok.txt\n");
    assert_eq!(directives.len(), 1);
    assert_eq!(directives[0].verb, Verb::Copy);
    assert_eq!(directives[0].arg, "ok.txt");
    assert_eq!(
        warnings,
        vec![
            "run: the argument contains a control character".to_string(),
            "link: the argument contains a control character".to_string()
        ]
    );
}

#[test]
fn parse_keeps_the_order_and_skips_blank_lines_comments_and_indentation() {
    let text = "  \n\t# a comment\n  copy second.txt  \nrun  first thing  \n#link ignored\n";
    let (directives, warnings) = parse(text);
    assert_eq!(directives.len(), 2);
    assert_eq!(directives[0].verb, Verb::Copy);
    assert_eq!(directives[0].arg, "second.txt", "the argument is trimmed");
    assert_eq!(directives[1].verb, Verb::Run);
    assert_eq!(
        directives[1].arg, "first thing",
        "run keeps the inner spacing and loses the outer"
    );
    assert!(warnings.is_empty());
}

#[test]
fn apply_refuses_a_main_checkout_or_a_slot_that_is_not_an_absolute_path() {
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main");
    let slot = tmp.path().join("workspace-1");
    std::fs::create_dir_all(&main).unwrap();
    std::fs::create_dir_all(&slot).unwrap();
    std::fs::write(main.join(".env"), "SECRET=1").unwrap();
    let (directives, _) = parse("link .env\n");

    let relative_main = Path::new("not-absolute-main");
    let applied = apply(relative_main, &slot, &directives);
    assert_eq!(
        applied.failures(),
        vec![
            "link .env: the main checkout \"not-absolute-main\" is not an absolute path; \
              pass the project root"
                .to_string()
        ]
    );
    assert!(!slot.join(".env").exists(), "nothing was written");

    let relative_slot = Path::new("not-absolute-slot");
    let applied = apply(&main, relative_slot, &directives);
    assert_eq!(
        applied.failures(),
        vec![
            "link .env: the slot \"not-absolute-slot\" is not an absolute path; pass the slot \
              directory under .domux/worktrees"
                .to_string()
        ]
    );
    assert!(
        !relative_slot.exists(),
        "a relative slot must not be created wherever the server was started"
    );
    // The empty path is the one git reads as \"here\", so it is named on its own.
    let applied = apply(&main, Path::new(""), &directives);
    assert_eq!(
        applied.failures(),
        vec![
            "link .env: the slot \"\" is not an absolute path; pass the slot directory under \
              .domux/worktrees"
                .to_string()
        ]
    );
}

#[test]
fn apply_refuses_a_main_checkout_or_a_slot_that_is_not_a_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main");
    let slot = tmp.path().join("workspace-1");
    std::fs::create_dir_all(&main).unwrap();
    std::fs::create_dir_all(&slot).unwrap();
    std::fs::write(main.join(".env"), "SECRET=1").unwrap();
    let (directives, _) = parse("link .env\n");

    let gone = tmp.path().join("gone");
    let applied = apply(&gone, &slot, &directives);
    assert_eq!(
        applied.failures(),
        vec![format!(
            "link .env: the main checkout \"{}\" is not a directory; check the project root",
            gone.display()
        )]
    );

    let applied = apply(&main, &gone, &directives);
    assert_eq!(
        applied.failures(),
        vec![format!(
            "link .env: the slot \"{}\" is not a directory; create the workspace before applying \
             its setup",
            gone.display()
        )]
    );
    assert!(!gone.exists(), "a slot that is not there is not created");
}

#[test]
fn apply_refuses_a_path_that_leaves_the_main_checkout() {
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main");
    let slot = tmp.path().join("workspace-1");
    std::fs::create_dir_all(&main).unwrap();
    std::fs::create_dir_all(&slot).unwrap();
    std::fs::write(tmp.path().join("outside.txt"), "not yours").unwrap();
    std::fs::write(main.join("ok.txt"), "yes").unwrap();
    let (directives, _) = parse(
        "link ../outside.txt\ncopy ../outside.txt\nlink /etc/hosts\ncopy .\nlink ./\ncopy ok.txt\n",
    );
    let applied = apply(&main, &slot, &directives);
    assert_eq!(
        applied.failures(),
        vec![
            "link ../outside.txt: the path \"../outside.txt\" leaves the main checkout with \
             \"..\"; name a path inside it"
                .to_string(),
            "copy ../outside.txt: the path \"../outside.txt\" leaves the main checkout with \
             \"..\"; name a path inside it"
                .to_string(),
            "link /etc/hosts: the path \"/etc/hosts\" is absolute; name a path relative to the \
             main checkout"
                .to_string(),
            "copy .: the path \".\" names no file; name a path relative to the main checkout"
                .to_string(),
            "link ./: the path \"./\" names no file; name a path relative to the main checkout"
                .to_string(),
        ]
    );
    assert_eq!(applied.summary().to_string(), "copied 1, 5 skipped");
    assert!(
        !slot.join("../outside.txt").is_symlink(),
        "nothing was written beside the slot"
    );
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("outside.txt")).unwrap(),
        "not yours"
    );
    assert!(
        std::fs::read_dir(&slot).unwrap().count() == 1,
        "only the one good copy landed in the slot"
    );
}

#[test]
fn link_replaces_a_file_or_a_folder_that_is_in_the_way_in_the_slot() {
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main");
    let slot = tmp.path().join("workspace-1");
    std::fs::create_dir_all(main.join("vendor")).unwrap();
    std::fs::create_dir_all(slot.join("vendor")).unwrap();
    std::fs::write(main.join("vendor/lib.txt"), "the real one").unwrap();
    std::fs::write(slot.join("vendor/stale.txt"), "stale").unwrap();
    std::fs::write(main.join(".env"), "SECRET=1").unwrap();
    std::fs::write(slot.join(".env"), "an older file").unwrap();
    // A link left by an earlier setup whose target is gone, and a path whose folders the slot
    // does not have yet.
    std::fs::write(main.join("stale-link"), "the current one").unwrap();
    std::os::unix::fs::symlink(main.join("removed-earlier"), slot.join("stale-link")).unwrap();
    std::fs::create_dir_all(main.join("tool/cache")).unwrap();
    std::fs::write(main.join("tool/cache/db"), "warm").unwrap();
    let (directives, _) = parse("link vendor\nlink .env\nlink stale-link\nlink tool/cache/db\n");
    let applied = apply(&main, &slot, &directives);
    assert_eq!(applied.failures(), Vec::<String>::new());
    assert_eq!(applied.summary().to_string(), "linked 4");
    assert_eq!(
        std::fs::read_to_string(slot.join("stale-link")).unwrap(),
        "the current one",
        "a link whose target is gone is replaced"
    );
    assert_eq!(
        std::fs::read_link(slot.join("tool/cache/db")).unwrap(),
        main.join("tool/cache/db"),
        "the slot's folders are created for it"
    );
    assert!(slot.join("vendor").is_symlink());
    assert_eq!(
        std::fs::read_link(slot.join("vendor")).unwrap(),
        main.join("vendor")
    );
    assert!(slot.join("vendor/lib.txt").is_file());
    assert!(slot.join(".env").is_symlink());
    assert_eq!(
        std::fs::read_to_string(slot.join(".env")).unwrap(),
        "SECRET=1"
    );
    assert!(
        !main.join("vendor/stale.txt").exists(),
        "replacing the slot's folder must not write into the main checkout"
    );
    // A second apply removes the link, not what it points at.
    let applied = apply(&main, &slot, &directives);
    assert_eq!(applied.failures(), Vec::<String>::new());
    assert_eq!(
        std::fs::read_link(slot.join("vendor")).unwrap(),
        main.join("vendor")
    );
    assert_eq!(
        std::fs::read_to_string(main.join("vendor/lib.txt")).unwrap(),
        "the real one",
        "the main checkout's folder survives a second link"
    );
}

#[test]
fn copy_refuses_a_folder_and_leaves_a_sibling_of_the_same_stem_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main");
    let slot = tmp.path().join("workspace-1");
    std::fs::create_dir_all(main.join("config")).unwrap();
    std::fs::create_dir_all(&slot).unwrap();
    std::fs::write(main.join("config/local.yml"), "port: 3000").unwrap();
    std::fs::write(main.join("keep.tmp"), "mine").unwrap();
    std::fs::write(main.join("keep.yml"), "also mine").unwrap();
    std::fs::write(slot.join("keep.tmp"), "mine").unwrap();
    // The scratch name is ours, so a symlink sitting on it is removed rather than written
    // through. `git clean -fd` leaves what worktree.conf put in a slot, so a link left there
    // by an earlier setup outlives the workspace it was made for.
    std::fs::write(main.join("victim.txt"), "untouched").unwrap();
    std::os::unix::fs::symlink(main.join("victim.txt"), slot.join("keep.yml.tmp")).unwrap();
    std::os::unix::fs::symlink(main.join("config"), main.join("linked-config")).unwrap();
    let (directives, _) = parse("copy config\ncopy linked-config\ncopy keep.yml\n");
    let applied = apply(&main, &slot, &directives);
    assert_eq!(
        applied.failures(),
        vec![
            "copy config: copy is for files; use link for the folder config".to_string(),
            "copy linked-config: copy is for files; use link for the folder linked-config"
                .to_string()
        ],
        "a name that leads to a folder is a folder"
    );
    assert_eq!(applied.summary().to_string(), "copied 1, 2 skipped");
    assert_eq!(
        std::fs::read_to_string(slot.join("keep.tmp")).unwrap(),
        "mine",
        "the scratch name is keep.yml.tmp, so a real keep.tmp is untouched"
    );
    assert!(
        !slot.join("keep.yml.tmp").exists(),
        "the scratch file is renamed, not left behind"
    );
    assert_eq!(
        std::fs::read_to_string(slot.join("keep.yml")).unwrap(),
        "also mine"
    );
    assert_eq!(
        std::fs::read_to_string(main.join("victim.txt")).unwrap(),
        "untouched",
        "the copy did not write through a symlink on the scratch name"
    );
}

#[test]
fn run_lines_keeps_every_run_directive_in_order_and_apply_never_runs_one() {
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main");
    let slot = tmp.path().join("workspace-1");
    std::fs::create_dir_all(&main).unwrap();
    std::fs::create_dir_all(&slot).unwrap();
    let (directives, _) =
        parse("run touch ran-first.txt\nlink .env\nrun bin/setup --fast\ncopy x\nrun echo done\n");
    assert_eq!(
        run_lines(&directives),
        vec![
            "touch ran-first.txt".to_string(),
            "bin/setup --fast".to_string(),
            "echo done".to_string()
        ]
    );
    let applied = apply(&main, &slot, &directives);
    assert!(
        !slot.join("ran-first.txt").exists() && !main.join("ran-first.txt").exists(),
        "apply spawns no process; the pane runs the run lines"
    );
    assert_eq!(
        applied.summary().to_string(),
        "2 skipped",
        "only the two missing sources are counted, not the run lines"
    );
}

#[test]
fn the_summary_names_only_the_counts_that_happened() {
    assert_eq!(Summary::default().to_string(), "");
    assert_eq!(
        Summary {
            linked: 2,
            copied: 1,
            ran: 1,
            skipped: 3
        }
        .to_string(),
        "linked 2, copied 1, ran 1, 3 skipped"
    );
    assert_eq!(
        Summary {
            linked: 0,
            copied: 0,
            ran: 4,
            skipped: 0
        }
        .to_string(),
        "ran 4",
        "the caller fills ran in once it has typed the run lines into the pane"
    );
}

#[test]
fn apply_refuses_a_slot_that_is_the_main_checkout_under_another_name() {
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main");
    std::fs::create_dir_all(&main).unwrap();
    std::fs::write(main.join(".env"), "SECRET=1").unwrap();
    let (directives, _) = parse("link .env\n");

    let alias = tmp.path().join("alias");
    std::os::unix::fs::symlink(&main, &alias).unwrap();
    let applied = apply(&main, &alias, &directives);
    assert_eq!(
        applied.failures(),
        vec!["link .env: refusing to apply the setup to the main checkout itself".to_string()],
        "a slot reached through a symlink is still the main checkout"
    );

    let roundabout = main.join("..").join("main");
    let applied = apply(&main, &roundabout, &directives);
    assert_eq!(
        applied.failures(),
        vec!["link .env: refusing to apply the setup to the main checkout itself".to_string()],
        "so is one reached through .."
    );
    assert_eq!(
        std::fs::read_to_string(main.join(".env")).unwrap(),
        "SECRET=1"
    );
    assert!(
        main.join(".env").is_file(),
        "the real file is not a link now"
    );
}

#[test]
fn link_refuses_a_destination_reached_through_a_symlink_out_of_the_slot() {
    // The reviewer's reproduction, with no attacker in it: a conf that links a folder and then
    // names something inside it. Before the destination was bounded, the second directive
    // removed the main checkout's own file and left a loop where it was.
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main");
    let slot = tmp.path().join("workspace-1");
    std::fs::create_dir_all(main.join("vendor")).unwrap();
    std::fs::create_dir_all(main.join("target/debug")).unwrap();
    std::fs::create_dir_all(&slot).unwrap();
    std::fs::write(
        main.join("vendor/lib.txt"),
        "THE REAL FILE IN THE MAIN CHECKOUT",
    )
    .unwrap();
    std::fs::write(main.join("target/debug/big.bin"), "EXPENSIVE BUILD OUTPUT").unwrap();
    let (directives, _) = parse(
        "link vendor\nlink vendor/lib.txt\nlink target\nlink target/debug\ncopy vendor/lib.txt\n",
    );
    let applied = apply(&main, &slot, &directives);
    assert_eq!(
        applied.failures(),
        vec![
            "link vendor/lib.txt: the slot's \"vendor\" is a link, so this would land outside \
             the slot; link either a folder or what is inside it, not both"
                .to_string(),
            "link target/debug: the slot's \"target\" is a link, so this would land outside the \
             slot; link either a folder or what is inside it, not both"
                .to_string(),
            "copy vendor/lib.txt: the slot's \"vendor\" is a link, so this would land outside \
             the slot; link either a folder or what is inside it, not both"
                .to_string(),
        ]
    );
    assert_eq!(applied.summary().to_string(), "linked 2, 3 skipped");
    assert!(
        main.join("vendor/lib.txt").is_file(),
        "the main checkout's file is a regular file, not a loop"
    );
    assert_eq!(
        std::fs::read_to_string(main.join("vendor/lib.txt")).unwrap(),
        "THE REAL FILE IN THE MAIN CHECKOUT"
    );
    assert_eq!(
        std::fs::read_to_string(main.join("target/debug/big.bin")).unwrap(),
        "EXPENSIVE BUILD OUTPUT",
        "remove_dir_all did not run in the main checkout"
    );
}

#[test]
fn link_and_copy_refuse_a_slot_folder_that_leads_out_of_the_slot() {
    // The same guard against a symlink somebody else put in the slot, which is the reviewer's
    // attacker case: nothing outside the slot is read, written or removed.
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main");
    let slot = tmp.path().join("workspace-1");
    let elsewhere = tmp.path().join("elsewhere");
    std::fs::create_dir_all(main.join("vendor")).unwrap();
    std::fs::create_dir_all(&slot).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(main.join("vendor/precious.txt"), "from main").unwrap();
    std::fs::write(elsewhere.join("precious.txt"), "PRECIOUS, OUTSIDE THE SLOT").unwrap();
    std::os::unix::fs::symlink(&elsewhere, slot.join("vendor")).unwrap();
    let (directives, _) = parse("link vendor/precious.txt\ncopy vendor/precious.txt\n");
    let applied = apply(&main, &slot, &directives);
    assert_eq!(applied.failures().len(), 2);
    for failure in applied.failures() {
        assert!(
            failure.ends_with(
                "the slot's \"vendor\" is a link, so this would land outside the slot; link \
                 either a folder or what is inside it, not both"
            ),
            "unexpected failure: {failure}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(elsewhere.join("precious.txt")).unwrap(),
        "PRECIOUS, OUTSIDE THE SLOT"
    );
    assert_eq!(
        std::fs::read_dir(&elsewhere).unwrap().count(),
        1,
        "nothing was created outside the slot, the scratch file included"
    );
}

#[test]
fn a_slot_path_that_goes_through_a_file_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main");
    let slot = tmp.path().join("workspace-1");
    std::fs::create_dir_all(main.join("config")).unwrap();
    std::fs::create_dir_all(&slot).unwrap();
    std::fs::write(main.join("config/local.yml"), "port: 3000").unwrap();
    std::fs::write(slot.join("config"), "a file where a folder should be").unwrap();
    let (directives, _) = parse("copy config/local.yml\n");
    let applied = apply(&main, &slot, &directives);
    assert_eq!(
        applied.failures(),
        vec![
            "copy config/local.yml: the slot's \"config\" is a file, not a folder; name a path \
              that does not go through it"
                .to_string()
        ]
    );
    assert_eq!(
        std::fs::read_to_string(slot.join("config")).unwrap(),
        "a file where a folder should be"
    );
}

#[test]
fn link_normalises_a_dot_component_and_a_trailing_slash_in_the_path() {
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main");
    let slot = tmp.path().join("workspace-1");
    std::fs::create_dir_all(main.join("config")).unwrap();
    std::fs::create_dir_all(main.join("vendor")).unwrap();
    std::fs::create_dir_all(&slot).unwrap();
    std::fs::write(main.join("config/local.yml"), "port: 3000").unwrap();
    std::fs::write(main.join("keep.txt"), "kept").unwrap();
    let (directives, _) =
        parse("link ./config/./local.yml\nlink vendor/\ncopy ./keep.txt\ncopy keep.txt/\n");
    let applied = apply(&main, &slot, &directives);
    assert_eq!(applied.failures(), Vec::<String>::new());
    assert_eq!(applied.summary().to_string(), "linked 2, copied 2");
    // Compared as text, not as paths: `Path` equality skips a `.` component and a trailing
    // slash, so it cannot see what was actually stored in the link.
    assert_eq!(
        std::fs::read_link(slot.join("config/local.yml"))
            .unwrap()
            .to_str()
            .unwrap(),
        main.join("config/local.yml").to_str().unwrap(),
        "the stored target holds no . component"
    );
    assert_eq!(
        std::fs::read_link(slot.join("vendor"))
            .unwrap()
            .to_str()
            .unwrap(),
        main.join("vendor").to_str().unwrap(),
        "a trailing slash names the folder itself, and is not stored"
    );
    assert_eq!(
        std::fs::read_to_string(slot.join("keep.txt")).unwrap(),
        "kept"
    );
}

#[test]
fn a_source_that_cannot_be_read_is_not_reported_as_a_missing_source() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main");
    let slot = tmp.path().join("workspace-1");
    std::fs::create_dir_all(main.join("locked")).unwrap();
    std::fs::create_dir_all(&slot).unwrap();
    std::fs::write(main.join("locked/secret.txt"), "there all along").unwrap();
    std::fs::set_permissions(main.join("locked"), std::fs::Permissions::from_mode(0o000)).unwrap();
    let readable = std::fs::metadata(main.join("locked/secret.txt")).is_ok();
    let (directives, _) = parse("link locked/secret.txt\ncopy locked/secret.txt\n");
    let applied = apply(&main, &slot, &directives);
    std::fs::set_permissions(main.join("locked"), std::fs::Permissions::from_mode(0o755)).unwrap();
    if readable {
        // Running as root, where mode 000 denies nothing. There is no unreadable file to test.
        return;
    }
    for (failure, verb) in applied.failures().iter().zip(["link", "copy"]) {
        assert!(
            failure.starts_with(&format!(
                "{verb} locked/secret.txt: could not read locked/secret.txt in the main checkout: "
            )),
            "a file that is there but cannot be read is not a missing file: {failure}"
        );
    }
    assert_eq!(applied.failures().len(), 2);
}

#[test]
fn a_copy_that_cannot_be_renamed_into_place_leaves_no_scratch_file() {
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main");
    let slot = tmp.path().join("workspace-1");
    std::fs::create_dir_all(&main).unwrap();
    std::fs::create_dir_all(slot.join("a.txt/in-the-way")).unwrap();
    std::fs::write(main.join("a.txt"), "the file").unwrap();
    let (directives, _) = parse("copy a.txt\n");
    let applied = apply(&main, &slot, &directives);
    assert_eq!(applied.failures().len(), 1, "the rename failed");
    assert!(
        !slot.join("a.txt.tmp").exists(),
        "the scratch file is taken back when the rename fails"
    );
    assert!(
        slot.join("a.txt/in-the-way").is_dir(),
        "the folder survives"
    );
}

#[test]
fn parse_drops_an_argument_that_reorders_how_it_reads() {
    // A right-to-left override shows the reader one command and hands the pane another, and it
    // is not a control character in Unicode's sense, so the C0 rule does not cover it.
    let (directives, warnings) = parse("run make\u{202e}txt.esrever\nrun ok\u{200d}fine\n");
    assert_eq!(
        warnings,
        vec!["run: the argument contains a text direction control".to_string()]
    );
    assert_eq!(directives.len(), 1);
    assert_eq!(
        directives[0].arg, "ok\u{200d}fine",
        "a zero-width joiner drives nothing and is left alone"
    );
    for c in [
        '\u{200e}', '\u{200f}', '\u{202a}', '\u{202d}', '\u{2066}', '\u{2069}',
    ] {
        let (directives, warnings) = parse(&format!("link a{c}b\n"));
        assert!(directives.is_empty(), "{c:?} was accepted");
        assert_eq!(
            warnings,
            vec!["link: the argument contains a text direction control".to_string()]
        );
    }
}
