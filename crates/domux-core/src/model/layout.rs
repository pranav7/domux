//! The tab's layout: binary splits with a direction and a ratio, panes as leaves, and the
//! solver that turns the tree into box rectangles with a one-cell gap between boxes.

use crate::ids::PaneId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A rectangle in screen cells. The renderer converts it to ratatui's `Rect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub fn right(&self) -> u16 {
        self.x + self.width
    }
    pub fn bottom(&self) -> u16 {
        self.y + self.height
    }
}

/// Cells between neighbouring boxes (interface spec assumption 18).
pub const GAP: u16 = 1;
/// The smallest box: one border row above and below one content row, or the columns for it.
pub const MIN_BOX: u16 = 3;

/// Which way the second child sits relative to the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SplitDir {
    /// Side by side: `second` is to the right of `first`.
    Right,
    /// Stacked: `second` is below `first`.
    Down,
}

/// A direction on the screen, for splits, focus moves and resizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl std::str::FromStr for Direction {
    type Err = String;
    fn from_str(s: &str) -> Result<Direction, String> {
        match s {
            "left" => Ok(Direction::Left),
            "right" => Ok(Direction::Right),
            "up" => Ok(Direction::Up),
            "down" => Ok(Direction::Down),
            other => Err(format!(
                "unknown direction {other:?}; expected left, right, up or down"
            )),
        }
    }
}

/// One terminal. `command`, `title` and `pid` are facts and are not persisted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Pane {
    pub id: PaneId,
    /// The last known working directory: from OSC 7 when the shell reports it, otherwise
    /// from the process inspector. Resume spawns the shell here.
    pub cwd: PathBuf,
    /// The foreground command's name, from the process inspector.
    #[serde(skip)]
    pub command: Option<String>,
    /// The title from OSC 0 or 2.
    #[serde(skip)]
    pub title: Option<String>,
    #[serde(skip)]
    pub pid: Option<u32>,
    #[serde(skip)]
    pub copy_mode: bool,
}

/// The leaf content. One variant on purpose: a non-terminal pane later is an added variant
/// (architecture spec section 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PaneContent {
    Terminal(Pane),
}

impl PaneContent {
    pub fn pane(&self) -> &Pane {
        match self {
            PaneContent::Terminal(p) => p,
        }
    }
    pub fn pane_mut(&mut self) -> &mut Pane {
        match self {
            PaneContent::Terminal(p) => p,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "node", rename_all = "snake_case")]
pub enum LayoutNode {
    Split {
        dir: SplitDir,
        ratio: f32,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
    Leaf(PaneContent),
}

impl LayoutNode {
    pub fn leaf(pane: Pane) -> LayoutNode {
        LayoutNode::Leaf(PaneContent::Terminal(pane))
    }

    pub fn pane_ids(&self) -> Vec<PaneId> {
        self.panes().into_iter().map(|p| p.id.clone()).collect()
    }

    /// Every pane in reading order: first before second, recursively.
    pub fn panes(&self) -> Vec<&Pane> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect<'a>(&'a self, out: &mut Vec<&'a Pane>) {
        match self {
            LayoutNode::Leaf(c) => out.push(c.pane()),
            LayoutNode::Split { first, second, .. } => {
                first.collect(out);
                second.collect(out);
            }
        }
    }

    pub fn pane(&self, id: &PaneId) -> Option<&Pane> {
        self.panes().into_iter().find(|p| &p.id == id)
    }

    pub fn pane_mut(&mut self, id: &PaneId) -> Option<&mut Pane> {
        match self {
            LayoutNode::Leaf(c) => {
                if &c.pane().id == id {
                    Some(c.pane_mut())
                } else {
                    None
                }
            }
            LayoutNode::Split { first, second, .. } => {
                first.pane_mut(id).or_else(|| second.pane_mut(id))
            }
        }
    }

    pub fn contains(&self, id: &PaneId) -> bool {
        self.pane(id).is_some()
    }

    /// Replaces the leaf `target` with a split holding `target` and `new` at ratio 0.5.
    /// `Right` and `Down` put `new` second; `Left` and `Up` put it first. Returns false when
    /// `target` is not in the tree.
    pub fn split_leaf(&mut self, target: &PaneId, dir: Direction, new: Pane) -> bool {
        match self {
            LayoutNode::Leaf(c) if &c.pane().id == target => {
                let existing = std::mem::replace(self, LayoutNode::leaf(new.clone()));
                let fresh = LayoutNode::leaf(new);
                let (split_dir, first, second) = match dir {
                    Direction::Right => (SplitDir::Right, existing, fresh),
                    Direction::Left => (SplitDir::Right, fresh, existing),
                    Direction::Down => (SplitDir::Down, existing, fresh),
                    Direction::Up => (SplitDir::Down, fresh, existing),
                };
                *self = LayoutNode::Split {
                    dir: split_dir,
                    ratio: 0.5,
                    first: Box::new(first),
                    second: Box::new(second),
                };
                true
            }
            LayoutNode::Leaf(_) => false,
            LayoutNode::Split { first, second, .. } => {
                first.split_leaf(target, dir, new.clone()) || second.split_leaf(target, dir, new)
            }
        }
    }

    /// Removes the leaf `target`; its sibling takes the split's place. The last leaf cannot
    /// be removed (a tab always has a pane); the caller closes the tab instead.
    pub fn remove_leaf(&mut self, target: &PaneId) -> Option<Pane> {
        match self {
            LayoutNode::Leaf(_) => None,
            LayoutNode::Split { first, second, .. } => {
                if let LayoutNode::Leaf(c) = &**first {
                    if &c.pane().id == target {
                        let removed = c.pane().clone();
                        let keep =
                            std::mem::replace(&mut **second, LayoutNode::leaf(removed.clone()));
                        *self = keep;
                        return Some(removed);
                    }
                }
                if let LayoutNode::Leaf(c) = &**second {
                    if &c.pane().id == target {
                        let removed = c.pane().clone();
                        let keep =
                            std::mem::replace(&mut **first, LayoutNode::leaf(removed.clone()));
                        *self = keep;
                        return Some(removed);
                    }
                }
                first
                    .remove_leaf(target)
                    .or_else(|| second.remove_leaf(target))
            }
        }
    }

    /// Moves the boundary nearest to `target` in the axis of `dir` by `cells`, so that the
    /// boundary moves towards `dir`. Returns false when no ancestor split has that axis.
    pub fn resize(&mut self, target: &PaneId, dir: Direction, cells: u16, area: Rect) -> bool {
        self.resize_in(target, dir, cells, area) == Some(true)
    }

    fn resize_in(
        &mut self,
        target: &PaneId,
        dir: Direction,
        cells: u16,
        area: Rect,
    ) -> Option<bool> {
        match self {
            LayoutNode::Leaf(c) => {
                if &c.pane().id == target {
                    Some(false)
                } else {
                    None
                }
            }
            LayoutNode::Split {
                dir: split,
                ratio,
                first,
                second,
            } => {
                let (first_area, second_area) = split_areas(*split, *ratio, area);
                let found_first = first.resize_in(target, dir, cells, first_area);
                let found_second = if found_first.is_none() {
                    second.resize_in(target, dir, cells, second_area)
                } else {
                    None
                };
                let handled = found_first.or(found_second)?;
                if handled {
                    return Some(true);
                }
                let axis_matches = matches!(
                    (*split, dir),
                    (SplitDir::Right, Direction::Left | Direction::Right)
                        | (SplitDir::Down, Direction::Up | Direction::Down)
                );
                if !axis_matches {
                    return Some(false);
                }
                let total = match split {
                    SplitDir::Right => area.width,
                    SplitDir::Down => area.height,
                }
                .saturating_sub(GAP) as f32;
                // Both children need MIN_BOX out of `total`, so below 2 * MIN_BOX there is no
                // legal ratio and the boundary genuinely cannot move. Refusing is honest;
                // widening the clamp would place a box below MIN_BOX.
                if total < 2.0 * MIN_BOX as f32 {
                    return Some(false);
                }
                let towards_second = matches!(dir, Direction::Right | Direction::Down);
                let delta = cells as f32 / total;
                let mut next = if towards_second {
                    *ratio + delta
                } else {
                    *ratio - delta
                };
                let min = MIN_BOX as f32 / total;
                next = next.clamp(min, 1.0 - min);
                *ratio = next;
                Some(true)
            }
        }
    }
}

/// Splits `area` into the two child areas with a `GAP` between them.
fn split_areas(dir: SplitDir, ratio: f32, area: Rect) -> (Rect, Rect) {
    match dir {
        SplitDir::Right => {
            let usable = area.width.saturating_sub(GAP);
            let first_w = ((usable as f32) * ratio).round().clamp(0.0, usable as f32) as u16;
            let second_w = usable - first_w;
            (
                Rect {
                    x: area.x,
                    y: area.y,
                    width: first_w,
                    height: area.height,
                },
                Rect {
                    x: area.x + first_w + GAP,
                    y: area.y,
                    width: second_w,
                    height: area.height,
                },
            )
        }
        SplitDir::Down => {
            let usable = area.height.saturating_sub(GAP);
            let first_h = ((usable as f32) * ratio).round().clamp(0.0, usable as f32) as u16;
            let second_h = usable - first_h;
            (
                Rect {
                    x: area.x,
                    y: area.y,
                    width: area.width,
                    height: first_h,
                },
                Rect {
                    x: area.x,
                    y: area.y + first_h + GAP,
                    width: area.width,
                    height: second_h,
                },
            )
        }
    }
}

/// The box rectangle of every pane, in reading order. With `zoomed` set, only that pane is
/// returned and it takes the whole area.
pub fn solve(node: &LayoutNode, area: Rect, zoomed: Option<&PaneId>) -> Vec<(PaneId, Rect)> {
    if let Some(z) = zoomed {
        if node.contains(z) {
            return vec![(z.clone(), area)];
        }
    }
    let mut out = Vec::new();
    solve_into(node, area, &mut out);
    out
}

fn solve_into(node: &LayoutNode, area: Rect, out: &mut Vec<(PaneId, Rect)>) {
    match node {
        LayoutNode::Leaf(c) => out.push((c.pane().id.clone(), area)),
        LayoutNode::Split {
            dir,
            ratio,
            first,
            second,
        } => {
            let (a, b) = split_areas(*dir, *ratio, area);
            solve_into(first, a, out);
            solve_into(second, b, out);
        }
    }
}

/// The pane next to `from` in `dir`: among panes whose edge faces `from` across the gap, the
/// nearest one. Distance ranks first so focus never skips a column or a row; among panes at
/// the same distance the one whose extent on the other axis overlaps `from` most wins, and
/// among panes tied on both the earliest in reading order wins.
pub fn neighbour_by_geometry(
    rects: &[(PaneId, Rect)],
    from: &PaneId,
    dir: Direction,
) -> Option<PaneId> {
    let (_, me) = rects.iter().find(|(id, _)| id == from)?;
    let mut best: Option<(&PaneId, i32, i32)> = None; // (id, overlap, distance)
    for (id, r) in rects {
        if id == from {
            continue;
        }
        let (facing, distance, overlap) = match dir {
            Direction::Left => (
                r.right() <= me.x,
                me.x as i32 - r.right() as i32,
                overlap_1d(me.y, me.bottom(), r.y, r.bottom()),
            ),
            Direction::Right => (
                r.x >= me.right(),
                r.x as i32 - me.right() as i32,
                overlap_1d(me.y, me.bottom(), r.y, r.bottom()),
            ),
            Direction::Up => (
                r.bottom() <= me.y,
                me.y as i32 - r.bottom() as i32,
                overlap_1d(me.x, me.right(), r.x, r.right()),
            ),
            Direction::Down => (
                r.y >= me.bottom(),
                r.y as i32 - me.bottom() as i32,
                overlap_1d(me.x, me.right(), r.x, r.right()),
            ),
        };
        if !facing || overlap <= 0 {
            continue;
        }
        let better = match best {
            None => true,
            Some((_, o, d)) => distance < d || (distance == d && overlap > o),
        };
        if better {
            best = Some((id, overlap, distance));
        }
    }
    best.map(|(id, _, _)| id.clone())
}

fn overlap_1d(a0: u16, a1: u16, b0: u16, b1: u16) -> i32 {
    (a1.min(b1) as i32) - (a0.max(b0) as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn pane(id: &str) -> Pane {
        Pane {
            id: PaneId(id.to_string()),
            cwd: PathBuf::from("/tmp"),
            command: None,
            title: None,
            pid: None,
            copy_mode: false,
        }
    }

    fn area() -> Rect {
        Rect {
            x: 0,
            y: 1,
            width: 40,
            height: 9,
        }
    }

    #[test]
    fn one_leaf_fills_the_area() {
        let tree = LayoutNode::leaf(pane("p_0001"));
        let rects = solve(&tree, area(), None);
        assert_eq!(
            rects,
            vec![(
                PaneId("p_0001".into()),
                Rect {
                    x: 0,
                    y: 1,
                    width: 40,
                    height: 9
                }
            )]
        );
    }

    #[test]
    fn split_right_places_the_new_pane_on_the_right_with_a_one_cell_gap() {
        let mut tree = LayoutNode::leaf(pane("p_0001"));
        assert!(tree.split_leaf(&PaneId("p_0001".into()), Direction::Right, pane("p_0002")));
        let rects = solve(&tree, area(), None);
        // 40 columns minus a gap of 1 is 39; half rounds to 20 for the first pane.
        assert_eq!(
            rects[0],
            (
                PaneId("p_0001".into()),
                Rect {
                    x: 0,
                    y: 1,
                    width: 20,
                    height: 9
                }
            )
        );
        assert_eq!(
            rects[1],
            (
                PaneId("p_0002".into()),
                Rect {
                    x: 21,
                    y: 1,
                    width: 19,
                    height: 9
                }
            )
        );
    }

    #[test]
    fn split_down_stacks_and_split_left_inserts_before() {
        let mut tree = LayoutNode::leaf(pane("p_0001"));
        tree.split_leaf(&PaneId("p_0001".into()), Direction::Down, pane("p_0002"));
        let rects = solve(&tree, area(), None);
        assert_eq!(
            rects[0].1,
            Rect {
                x: 0,
                y: 1,
                width: 40,
                height: 4
            }
        );
        assert_eq!(
            rects[1].1,
            Rect {
                x: 0,
                y: 6,
                width: 40,
                height: 4
            }
        );
        tree.split_leaf(&PaneId("p_0001".into()), Direction::Left, pane("p_0003"));
        assert_eq!(
            tree.pane_ids(),
            vec![
                PaneId("p_0003".into()),
                PaneId("p_0001".into()),
                PaneId("p_0002".into())
            ]
        );
    }

    #[test]
    fn zoomed_pane_takes_the_whole_area() {
        let mut tree = LayoutNode::leaf(pane("p_0001"));
        tree.split_leaf(&PaneId("p_0001".into()), Direction::Right, pane("p_0002"));
        let rects = solve(&tree, area(), Some(&PaneId("p_0002".into())));
        assert_eq!(rects, vec![(PaneId("p_0002".into()), area())]);
    }

    #[test]
    fn remove_leaf_collapses_the_split_and_gives_the_sibling_the_space() {
        let mut tree = LayoutNode::leaf(pane("p_0001"));
        tree.split_leaf(&PaneId("p_0001".into()), Direction::Right, pane("p_0002"));
        let removed = tree.remove_leaf(&PaneId("p_0001".into())).expect("removed");
        assert_eq!(removed.id, PaneId("p_0001".into()));
        assert_eq!(
            solve(&tree, area(), None),
            vec![(PaneId("p_0002".into()), area())]
        );
        assert!(
            tree.remove_leaf(&PaneId("p_0002".into())).is_none(),
            "the last leaf cannot be removed"
        );
    }

    #[test]
    fn resize_moves_the_nearest_boundary_by_cells() {
        let mut tree = LayoutNode::leaf(pane("p_0001"));
        tree.split_leaf(&PaneId("p_0001".into()), Direction::Right, pane("p_0002"));
        assert!(tree.resize(&PaneId("p_0001".into()), Direction::Right, 2, area()));
        let rects = solve(&tree, area(), None);
        assert_eq!(rects[0].1.width, 22);
        assert_eq!(rects[1].1.width, 17);
        assert!(tree.resize(&PaneId("p_0002".into()), Direction::Left, 4, area()));
        let rects = solve(&tree, area(), None);
        assert_eq!(rects[0].1.width, 18);
        // A vertical resize with no stacked ancestor does nothing and says so.
        assert!(!tree.resize(&PaneId("p_0001".into()), Direction::Up, 1, area()));
    }

    #[test]
    fn resize_never_shrinks_a_pane_below_three_cells() {
        let mut tree = LayoutNode::leaf(pane("p_0001"));
        tree.split_leaf(&PaneId("p_0001".into()), Direction::Right, pane("p_0002"));
        for _ in 0..30 {
            tree.resize(&PaneId("p_0001".into()), Direction::Right, 2, area());
        }
        let rects = solve(&tree, area(), None);
        assert!(rects[1].1.width >= 3, "{rects:?}");
    }

    #[test]
    fn resize_refuses_when_the_split_is_too_small_for_two_boxes() {
        // The split's own axis minus GAP is what the two children share, and each needs
        // MIN_BOX, so below 2 * MIN_BOX no ratio is legal and the resize is refused.
        for height in 0..=6u16 {
            let mut tree = LayoutNode::leaf(pane("p_0001"));
            tree.split_leaf(&PaneId("p_0001".into()), Direction::Down, pane("p_0002"));
            let small = Rect {
                x: 0,
                y: 0,
                width: 40,
                height,
            };
            assert!(
                !tree.resize(&PaneId("p_0001".into()), Direction::Down, 1, small),
                "a height of {height} has no room for two boxes"
            );
        }
        let mut wide = LayoutNode::leaf(pane("p_0001"));
        wide.split_leaf(&PaneId("p_0001".into()), Direction::Right, pane("p_0002"));
        assert!(!wide.resize(
            &PaneId("p_0001".into()),
            Direction::Right,
            1,
            Rect {
                x: 0,
                y: 0,
                width: 6,
                height: 9
            }
        ));

        // 7 is the first size that fits two 3-cell boxes and the gap between them.
        let mut tree = LayoutNode::leaf(pane("p_0001"));
        tree.split_leaf(&PaneId("p_0001".into()), Direction::Down, pane("p_0002"));
        let fits = Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 7,
        };
        assert!(tree.resize(&PaneId("p_0001".into()), Direction::Down, 1, fits));
        let rects = solve(&tree, fits, None);
        assert_eq!(rects[0].1.height, 3);
        assert_eq!(rects[1].1.height, 3);
    }

    #[test]
    fn neighbour_by_geometry_prefers_the_nearest_pane_over_a_larger_overlap() {
        // Split(Right, p1, Split(Right, Split(Down, p2, p4), p3)) over {0, 1, 40, 9} gives
        // p1 {0,1,20,9}, p2 {21,1,9,4}, p4 {21,6,9,4}, p3 {31,1,9,9}, in that reading order.
        // Every pane in the right two columns faces p1, so p3 is the largest overlap from p1
        // (9 rows against 4) and the first candidate in order from p3's own left. Both times
        // the right answer is the middle column, one cell away, not p3's tall column.
        let mut tree = LayoutNode::leaf(pane("p_0001"));
        tree.split_leaf(&PaneId("p_0001".into()), Direction::Right, pane("p_0002"));
        tree.split_leaf(&PaneId("p_0002".into()), Direction::Right, pane("p_0003"));
        tree.split_leaf(&PaneId("p_0002".into()), Direction::Down, pane("p_0004"));
        let rects = solve(&tree, area(), None);
        let left = PaneId("p_0001".into());

        // Ranking by overlap first answers p3 here, two columns away.
        assert_eq!(
            neighbour_by_geometry(&rects, &left, Direction::Right),
            Some(PaneId("p_0002".into()))
        );
        // Ranking by overlap first answers p1; taking the first facing candidate also
        // answers p1, since p1 comes before p2 in reading order.
        assert_eq!(
            neighbour_by_geometry(&rects, &PaneId("p_0003".into()), Direction::Left),
            Some(PaneId("p_0002".into()))
        );
        assert_eq!(
            neighbour_by_geometry(&rects, &PaneId("p_0002".into()), Direction::Down),
            Some(PaneId("p_0004".into()))
        );
        assert_eq!(neighbour_by_geometry(&rects, &left, Direction::Left), None);
    }

    #[test]
    fn neighbour_by_geometry_prefers_the_largest_overlap_at_the_same_distance() {
        // Split(Right, Split(Down, Split(Down, p2, p4), p1), p3) over {0, 1, 40, 9} gives
        // p2 {0,1,20,2}, p4 {0,4,20,1}, p1 {0,6,20,4}, p3 {21,1,19,9}, in that reading order.
        // All three left panes face p3 one cell away, so distance cannot separate them and
        // the tie-break decides: p1 overlaps 4 rows against p2's 2 and p4's 1, even though
        // p2 comes first in reading order.
        let mut tree = LayoutNode::leaf(pane("p_0001"));
        tree.split_leaf(&PaneId("p_0001".into()), Direction::Right, pane("p_0003"));
        tree.split_leaf(&PaneId("p_0001".into()), Direction::Up, pane("p_0002"));
        tree.split_leaf(&PaneId("p_0002".into()), Direction::Down, pane("p_0004"));
        let rects = solve(&tree, area(), None);
        assert_eq!(
            neighbour_by_geometry(&rects, &PaneId("p_0003".into()), Direction::Left),
            Some(PaneId("p_0001".into()))
        );
    }
}
