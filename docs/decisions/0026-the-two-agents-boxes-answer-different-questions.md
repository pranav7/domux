# 0026: The two Agents boxes answer different questions

**Date:** 2026-09-10
**Status:** Accepted. Amends interface spec 6.2, 6.3, 6.4 and 6.8.
**Decision:** The sidebar's Agents box lists the running sessions, flat. The agents overlay
lists every record, grouped under a header per project. A red dot means the agent is waiting on
you and nothing else. The top bar draws no agent count. An exited record with no session id is
removed.

## Context

Four reports, all about the same box, and they only make sense together.

- **MUX-22**, "agent list has agents that don't exist or have ended the session": a sidebar
  holding six exited rows, two of them from two hours earlier, over two running ones. Pressing
  Enter on one of them answered "claude is running in pane p_… , so the line would go into its
  prompt instead of a shell".
- **MUX-21**: the overlay repeating `audrey-app › agent-harness › agent-harness` on every row.
- **MUX-23**: "no need to show number of agents here, and notification dot -> REMOVE".
- **The red dot**, reported in the same sitting: "it's always showing. but it should only show
  when the agent is waiting for input on something".

M3 drew one row grammar on two surfaces and gave both surfaces the same list. The grammar is
still one grammar and still lives in `render::agents_box`. What changed is that the two surfaces
now hold different rows, because they are read for different reasons.

## The sidebar's box is what is running

It is on the screen all day beside the panes, and it is glanced at, not studied. Every session
that ends left a row on it, so an afternoon buried the two agents that were working under an
hour of ones that were not. `RowForm::shows` is the whole rule, and it is asked in
`agents_box::rows` rather than at each surface, so `api::list`'s cursor walks the same rows the
sidebar draws: a cursor that could rest on a row nobody can see is worse than no cursor.

Nothing is lost. `leader a` opens the agents overlay, which keeps the exited records, and both
resume and dismiss are there. Three things follow from the sidebar having no exited rows and are
worth naming, because each one was a line of code that had to go somewhere:

- The sidebar's hint row says `open` for every row it can rest on. `resume` is the agents
  overlay's word now, and the overlay's own exited row carries it.
- Its empty text is "Nothing running", not "No agents yet". The second would be a lie told to a
  reader with a list of records one key away.
- `focus::enter_sidebar_box` keeps a cursor only on a live record. A record that exits leaves
  this box, and a cursor left on it would mark a row that is not on this screen.

## The overlay groups by project

Every row carried `project › workspace › tab`, and a reader with three sessions in one project
read the project name three times. The project is a header now - `projects_box::header`, the
same one the Projects box draws - and the row's second line is `workspace › tab` under it. The
groups come in the order their first agent does, which is `sorted_agents` order, so the project
holding the agent that most wants you is the first group in the box. Sorting the headers by name
would put a waiting agent below two idle projects.

The sidebar does not group. Thirty-eight columns have no room for a header every few rows, and
each row's own place line names the project there.

## A red dot means waiting, and only waiting

`Agent::needs_you` was waiting **or** unseen, and `agents_box::dot_color` let unseen win over the
state. `attention` turns unseen on when an agent starts waiting, when it goes from working to
idle, and when it exits - so in a list of a dozen sessions almost every row was red, and the
mark stopped saying anything. It is red for waiting now: the agent has asked you something and
is stopped until you answer.

`unseen` keeps its other two jobs, and they are the ones it is good at. It lifts an idle row
above the quiet ones in `sorted_agents`, and it brightens the row's recap until you have looked.
Neither of those claims the agent is blocked on you.

`Model::red_dot_count` follows, so `agent.list`'s `red_dots` counts what the dots count.

## The top bar draws no count

It was the agent list folded into one cell for a client with no sidebar. The number said nothing
the Agents box does not say, and the dot beside it was a second attention marker with no row
behind it: a reader who saw it still had to open the box to find out which agent. It is gone,
and with it `AgentsView.red_dots`, which nothing on a screen read any more.

`top_bar::bar_tab_hit` now measures only the location label to find where the tab row starts.

## An exited record with no session id is removed

One property, and it is permanent: the record was made by the observer and no hook ever reported
on it, so there is no session for `agent.resume` to name, and no hook can find it again either -
`Model::report_agent` matches a returning session by its id, and a record without one is
unreachable by every route there is. It can be dismissed and nothing else, which is a row asking
the reader to tidy up after domux. `observer::prune_unresumable` takes it, on the tick and on a
pane going, so it goes as it exits rather than on a schedule of its own.

**Three near misses are deliberately left alone.** Each was considered and each keeps its row.

- **A pane that is gone, or one another agent has taken over.** This is the refusal in MUX-22's
  screenshot, so it is the tempting one. Both conditions are temporary: the pane frees up, or the
  reader starts the session again themselves and its `SessionStart` brings this record back with
  its recap and its name. Removing the record would throw away the only thing that makes that
  return possible.
- **A kind that does not resume.** Codex and OpenCode carry no resume command in V2.0, and that
  is a gap in this release rather than a fact about the record - Codex and OpenCode resume is
  V2.x. Pruning on it would delete rows for a reason that is due to stop being true, and no
  reader would ever meet `resume::RESUME_UNAVAILABLE`, which is how they learn the other kinds
  are coming.
- **Age.** An exited record is not less useful for being old; the old ones are what a reader goes
  looking for. What made the list unreadable was the sidebar carrying every one of them, and that
  is answered above.

`plan_resume` keeps its refusal for a record with no session id. It is a guard on a state the
prune now reaches first, and the two read the same property from the same field.
