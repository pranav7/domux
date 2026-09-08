# 0006: work that shells out runs on a job lane, and the reply travels with it

Status: accepted, 2026-09-08 (M2, task 16)

## The problem

One core task owns all mutable state. Every handler runs on it, one at a time, and while a
handler runs nothing else happens: no key is routed, no pane output is fed to an emulator,
no frame is composed for any client.

`project.add` is the first handler that has to ask the world a question before it can
answer. Deciding what a path is means `git rev-parse --is-inside-work-tree`,
`git symbolic-ref refs/remotes/origin/HEAD` and a read of the worktree directory: three
forks and a handful of syscalls. `workspace.create` is worse, because `git worktree add`
fetches from a remote and can take seconds, and M3's agent resume and hook installation are
worse again.

Running any of that on the core task freezes every pane in every client for as long as it
takes. That the git calls in `project.add` are usually fast is not an exception: a fork is
slow on a loaded machine, a network fetch is slow whenever the network is, and the rule is
about who runs the work rather than how long it takes today.

## The choice

A job lane. A handler that has to shell out queues a `CoreJob` and sets `Ctx::defer_reply`,
and does nothing else. The core runs the job on a blocking task and the answer comes back as
`CoreMsg::JobFinished`, like any other message. The `JobFinished` arm, back on the core task,
makes every model change and answers the caller. One writer, as before.

The waiting caller's reply travels with the job, as a `JobReply` holding the request id and
the oneshot sender. So:

- The caller waits exactly as long as the work takes, and gets the real answer.
- The core does not wait at all: frames keep being drawn and keys keep being routed while
  the job runs.
- Nothing has to invent a status field or a second call to ask whether the work finished.

A job a key started carries no reply, because nobody is waiting on a keystroke. Its failure
goes to that client's hint row instead, which is where a failed key's message already goes
(`Core::answer`, principle 8).

The model change is in the `JobFinished` arm rather than in the job, which is what keeps the
one-writer rule: the job reads the world and reports what it found, and everything that
touches the model happens on the core task afterwards. It is also why idempotence lives
there. `project.add` is idempotent on the canonical path, and the canonical path is only
known once the job has resolved it, so the handler cannot decide it and the job must not.

## What was set aside

**Running "fast" subprocesses inline.** It needs a rule for which subprocesses are fast, and
there is none: the same `git rev-parse` that takes a millisecond on a warm repository takes
much longer on a cold one, on a network filesystem, or behind a filesystem event watcher.
A rule that depends on the machine is not a rule.

**Answering before the work is done, with a status to poll.** `project.add` would answer
`{"ok": true}` and the caller would then have to ask whether the project appeared. That
makes every caller carry a loop, makes failure arrive somewhere other than where the call
was made, and puts a state on the wire ("in progress") that the domain model does not have.
The reply travelling with the job gets the same non-blocking core with none of that.

**A second task that owns a job queue.** The core already owns a message channel and already
knows how to be told things; another owner would be another place to reason about ordering.
`tokio::task::spawn_blocking` plus a `CoreMsg` is the smaller mechanism.

## What the lane does and does not serialise

Every `JobFinished` arm runs on the core task, one after another, like every other message.
So a job that only **reads** the world and lets its arm decide is safe against a second call
arriving while it runs: two `project.add` calls for one unregistered path both queue a job,
both jobs find no project, and then the first arm registers it and the second arm's
`project_at` sees what the first one wrote. One project, and both callers get it back.
`two_project_add_calls_in_flight_at_once_register_one_project` pins this, with both requests
on their own sockets so neither waits on the other; it was checked against a `read_project`
slowed by 300 ms, which guarantees both jobs are genuinely in flight, and against the
idempotence check being removed, which turns it red.

**A job that chooses something does not get that for free.** The decision is made inside the
job, off the core task, so two jobs can choose the same thing before either arm runs.
Task 17's `workspace.create` is exactly that shape: the slot number is the lowest free one,
two concurrent creates would both choose it, and both would run `git worktree add` for
`workspace-1`. The second one fails, loudly and without corrupting anything, but not where a
reader would expect: `git::worktree_add`'s occupied-directory check does **not** catch it,
because `is_occupied` answers false for a directory that is missing or empty and it runs
before `fetch`, `prune` and `create_dir_all`. Both calls pass that check, and the loser fails
later inside `git worktree add` itself, in git's own words rather than domux's.

Whoever adds such a job owns the answer, and the shape to reach for is a set of claims on the
core, taken in the handler where one runs at a time and released when the job finishes. It is
deliberately not built here: `project.add` cannot reach the defect it would prevent, and a
claim on it would answer `busy` to a second call that today gets the right answer. A
mechanism with nothing in the milestone that can exercise it is one no test can tell from its
opposite.

## What follows from it

- Task 17's `workspace.create` and Task 18's `workspace.clear` and `workspace.delete` add
  variants to `CoreJob` and arms to `JobFinished`. They shell out for longer than
  `project.add` does, which is the point.
- M3's agent resume and hook installation take the same road.
- A handler that defers must queue a job, or its caller waits for ever. `Core::api` answers
  such a handler with an internal error rather than leaving the connection open.
- A job that panics still sends `JobFinished`, for the same reason a fact provider that
  panics still sends `FactFetched`: a key left in flight for the life of the server is worse
  than an error.
