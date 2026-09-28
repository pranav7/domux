# 0060: The filter is fuzzy

**Date:** 2026-09-28
**Status:** Accepted. Follows 0059. Changes what `/` keeps in the switcher, the sidebar's
Navigator box, the Agents box and the agents overlay.
**Decision:** A term of the filter matches a field when its letters appear there in order, with
any letters between them. The filter's terms are split on spaces and each must match in one field
of the row. Where the fill goes by itself, it goes to the row the filter matches best, and the
rows keep their order.

## Context

0059 made the switcher a quick switch: open, type, go. The filter still kept a row only when the
typed text was a substring of it, so `tod` found `tod-agent-perf` and `todagent` found nothing,
because the name has a hyphen the reader did not type. A quick switch is typed fast and from
memory, and the reader does not remember where the hyphens are.

## The match

`domux_core::fuzzy::score` answers for every list, because `list_box::filter_rows` is still the one
function that filters them.

- The filter is split on spaces into terms. A row stays when every term matches.
- A term matches a field when its letters appear in the field in order, without case. `todagent`
  and `tap` both match `tod-agent-perf`; `pat` does not.
- A term matches within one field, never across two. A row's fields are parted by
  `fuzzy::FIELD_SEPARATOR`, a control character no name can hold. Before this the fields were
  parted by a space and a filter that ran across two matched by accident; now it cannot, and two
  terms do the same job by design: `audrey tod` finds the `tod` workspace of `audrey-app`.

The fields are the ones `/` already searched: for a workspace the project name, the handle, the
name, the branch and the pull request number; for an agent the session name, the kind, the place
and the pane's name.

## The fill

A subsequence match keeps more rows than a substring did, and the first of them in the list is
often not the one the reader meant. So `score` also says how well a row matched, and with no
cursor the switcher's fill goes to the best match (`list_box::best_match`), the first of them on a
tie. `projects_box::rows_at` still answers it for the switcher, the sidebar and `api::list`, so
the fill the reader sees is the row Enter opens.

The score rewards what a reader types when they mean a row: letters that run together, a letter at
the start of the field, and a letter at the start of a word inside it, after a space, a hyphen or
any other mark. Each letter skipped between two letters of a term costs a little. So `auth` typed
at `auth` beats the same four letters spread over `a user that helps`, and a name typed whole
beats the same letters scattered through a longer one. The numbers are in `fuzzy.rs` and are not a
contract; what the tests hold is the order they give.

## What did not change

- **Nothing reorders the Navigator.** The rows stay in the order the list builds them in; only
  the fill moves to the best match. Sorting by score would move rows under the reader with every
  letter, which is what the Navigator promises not to do.
- The sidebar's boxes and the agents overlay put the fill on the cursor, as before. They match
  fuzzily because they share `filter_rows`; where the fill goes is 0059's question and was only
  asked of the switcher.
- The matched letters are not marked in the row. The rows are built before the filter runs, and
  the fill already says which row Enter opens.
