# Verifying `import v1` against the author's own V1 state

Run on 2026-09-09 against `m2` at `c3c2a2e`, with the release binary.

Nothing here read or wrote `~/.local/share/domux` except one `cp -R`. Every import ran
against `/tmp/m2-import/sessions`, a copy. The server ran with
`DOMUX_STATE_DIR=/tmp/m2-import/state` and `DOMUX_SOCKET=/tmp/m2-import/domux2.sock`, so it
never touched `~/.local/share/domux2`, which the author's own `domux2` was writing to
throughout from a different process.

## What was there, measured before importing

    ls ~/.local/share/domux/sessions/*.json | wc -l                                    4
    jq -s 'map(select(.root == null or .root == "")) | length' *.json                  1
    jq -s '[.[] | .root // ""] | map(select(test("/(\.domux|\.baag)/worktrees/")))
       | length' *.json                                                                0

Four sessions: `domux`, `dotfiles`, `notes`, `richie-rich`. One has no root
(`richie-rich`, which also has no windows). None is a slot under a worktrees path, so the
half-migrated `.baag` case that follow-up 3 describes does not arise here and is still
untested against real data.

Four windows across the three sessions with roots. Two of them, both in `domux`, run the
agent `claude`. Two are named `2.1.263`, which is a version string rather than a number, so
the tab-number shadow of follow-up 10 does not arise either.

## The dry run and the real run agree

Both printed the same plan and the same skips, differing only in the verb: `Would import 3
projects, 3 workspaces, 4 tabs.` against `Imported 3 projects, 3 workspaces, 4 tabs.` That
is the property Task 22 and Task 23 built, and it holds against real data.

`project list` afterwards holds four projects, not three: `domux-m2` is the folder the
server was started in, seeded by `Core::new` before the import ran. That is the documented
start-up behaviour and not an import artefact.

## The finding: two windows are reported as skipped and are not

The report says:

    Skipped 3: richie-rich (no root recorded),
      domux (its window claude runs the agent claude, which V2 has no home for until M3),
      domux (its window 2.1.263 runs the agent claude, which V2 has no home for until M3).

Both `domux` windows were imported. Measured after the real run:

    domux workspace w_c985 tabs:
      index 1: name=null      cwd=/Users/pranav/projects/domux
      index 2: name=claude    cwd=/Users/pranav/projects/domux
      index 3: name=2.1.263   cwd=/Users/pranav/projects/domux

Index 1 is the tab every workspace gets. Indexes 2 and 3 are the two windows the report
says it skipped, at the right directory and under the right names. The planner intends
this: `an_agent_in_a_window_is_skipped_with_its_reason` asserts the tab **is** planned and
the reason recorded beside it.

So the behaviour is right and the sentence is wrong. **What V2 has no home for is the
agent, not the window.** The window came across; the agent did not. The reader is told
they lost two tabs they have, which sends them looking for work that is not missing.

`Skipped 3` also counts one whole session that really was dropped alongside two windows
that were imported, so the number names one kind of loss and totals two. That is the
defect Task 23 fixed in the failure line - naming each noun rather than summing them - and
it survived in the skip line.

This is the one thing the fixtures could not have found. Every shape in the author's
directory was one the fixtures already covered; what real data exposed was a sentence
about those shapes that the fixtures asserted without questioning.

## The second run, against the author's own V2 state

Run later on 2026-09-09, against `~/.local/share/domux2` rather than a scratch state
directory. Three workspaces did not arrive:

    /Users/pranav/projects/audrey/audrey-app/.domux/worktrees/workspace-1 is not registered
    under /Users/pranav/projects/audrey/audrey-app, so its tabs were not created.

with the same line for `workspace-2` and `workspace-3`, then `Imported 5 projects, 5
workspaces, 5 tabs.` and `Not imported: 3 workspaces. Read the messages above, then run this
again.`

Two causes, neither of them the import's.

The binary was stale. `~/bin/domux2` pointed at `projects/domux-v2/target/release/domux2`, a
detached worktree of this repository at `5288590`, built before decision record 0010 landed at
`bc3c18f`. That build's seed registered the directory the server was started in without asking
git, so `audrey-app` was held as `{"kind": "folder"}` with `main` and none of the four
`workspace-N` worktrees beside it. `dotfiles` in the same file was a git project with its slot
adopted, because `project.add` registered that one.

And the record could not be put right. `project.add` answered for a registered path as it
stood, so `workspace.list` held no slot for the import to rename and the same three lines would
print for ever. Decision record 0015 is the fix and has the reasoning; the sentence telling the
reader to run it again is true once a run can change the record.

Measured against a copy of the real state file with the fix in: `audrey-app` goes from
`folder` with one workspace to `git` on `main` with five, `workspace-1` arrives with the name
`Atcore | COS Testing`, `workspace-4` is adopted although no V1 session names it, and the run
ends `Imported 5 projects, 8 workspaces, 8 tabs.` with status 0.

What the two runs together say about this feature is that both of its field failures were
about state written by something else - a V1 session file in the first, a V2 state file in the
second - and neither was reachable from a fixture that builds its own inputs.
