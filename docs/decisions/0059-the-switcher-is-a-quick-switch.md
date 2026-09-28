# 0059: The switcher is a quick switch

**Date:** 2026-09-28
**Status:** Accepted. MUX-57. Changes what `leader s` opens into; the sidebar's Navigator box and
the agents overlay keep `/` as it was. **Amended by decision record 0060**, which makes the filter
fuzzy and moves the fill from the first match to the best one.
**Decision:** The switcher opens with its filter field open. The first letter typed narrows the
list, the fill lands on the first match, and Enter opens it. Acting on a row in any other way is
a step of its own, behind Tab, where the letters are the `[keys.list]` table again.

## Context

The author opens the switcher to go somewhere: a project, a workspace, an agent. Before this, the
switcher opened with the keys on the list, so reaching a row by name took `/`, the name, Enter to
leave the field, and Enter again to open the row. The letters were the list's keys until `/` was
pressed, so a name typed straight away ran `n`, `c` and `D` as commands instead. MUX-57 asks for
the Command K shape: open, type, go.

## The keys

In the switcher's field:

- A letter goes into the field. The cursor is cleared, so the fill lands on the first row the
  filter kept (`projects_box::rows_at`). With the field empty again, the cursor goes back to the
  workspace the client is in, where `switcher.open` puts it.
- Down and Up, and `C-n` and `C-p` (or `C-j` and `C-k`), move the fill among the matches without
  leaving the field. They run `list.down` and `list.up`.
- Enter runs `list.activate` on the row under the fill.
- Tab leaves the field and keeps the filter. The keys are then the list's, so `n`, `c`, `D`, `X`
  and `?` act on the row under the cursor, and `/` comes back to the field.
- Esc clears what was typed. On an empty field it closes the switcher.

The footer is the field and says those keys: `Filter › text  ⏎ open · tab actions · esc clear`,
with `esc close` once the field is empty. They are spelled in `input::quick_key` and in the footer
rather than read from `[keys.list]`, for the reason `input::filter_key` already gives: a text
field takes every letter, so a table that bound one would lose it.

## The fill without a cursor

The fill used to fall back to the workspace the client is in whenever the cursor was unset. Once
a filter is typed that row is usually gone, and the box drew no fill and Enter refused. So the
fallback now depends on the filter: the workspace the client is in while the filter is empty, the
first row the filter kept once it is not. One function, `projects_box::rows_at`, answers it for
the switcher, the sidebar and `api::list`, so the fill the reader sees is the row Enter acts on.
A key that moves the cursor sets it, and the next letter clears it again.

## The note

The start-up note (what the prune took away) shares the footer's row. The field used to outrank
it, which was harmless while the field opened only on `/`. A field that is always open would hide
the note for good, so an empty field in the switcher gives the row to the note. The first letter
is a key in a box, which clears the notes, so the field is on the screen by the time it holds
anything. A pill still outranks both. The sidebar's box keeps the old order, because its field
still opens only on `/`.

## What did not change

- The sidebar's Navigator box opens with the keys on the list, as before. The keys there are the
  reader's way around a column that is always on the screen, not a search.
- The agents overlay keeps `/` too. MUX-57 asked about the Navigator.
- `list.filter` still opens the field over the API, and `switcher.open` takes no new parameter.
- An overlay opened over the switcher, such as the keys or the name box, comes back to the list
  rather than to the field, because `ClientView::pop_overlay` closes the field. The reader
  reached it through Tab, so the list is where they were.
