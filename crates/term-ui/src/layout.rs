//! The tab's split layout: a binary tree of panes, independent of any host.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SplitDir {
    Right,
    Down,
}

/// A rectangle in logical points.
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// A split layout tree. Leaves hold pane ids.
#[derive(Clone)]
pub enum Layout {
    Leaf(String),
    Split {
        dir: SplitDir,
        ratio: f32,
        a: Box<Layout>,
        b: Box<Layout>,
    },
}

impl Layout {
    pub fn leaf(id: impl Into<String>) -> Self {
        Layout::Leaf(id.into())
    }

    /// Pane ids in order.
    pub fn ids(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect(&self, out: &mut Vec<String>) {
        match self {
            Layout::Leaf(id) => out.push(id.clone()),
            Layout::Split { a, b, .. } => {
                a.collect(out);
                b.collect(out);
            }
        }
    }

    /// The rectangle of every pane within `area` (leaves with a 6px gutter).
    pub fn rects(&self, area: Rect) -> Vec<(String, Rect)> {
        let mut out = Vec::new();
        self.rects_into(area, &mut out);
        out
    }

    fn rects_into(&self, area: Rect, out: &mut Vec<(String, Rect)>) {
        match self {
            Layout::Leaf(id) => out.push((id.clone(), area)),
            Layout::Split { dir, ratio, a, b } => {
                let r = ratio.clamp(0.1, 0.9);
                match dir {
                    SplitDir::Right => {
                        let w = area.w * r;
                        a.rects_into(Rect { w, ..area }, out);
                        b.rects_into(
                            Rect {
                                x: area.x + w,
                                w: area.w - w,
                                ..area
                            },
                            out,
                        );
                    }
                    SplitDir::Down => {
                        let h = area.h * r;
                        a.rects_into(Rect { h, ..area }, out);
                        b.rects_into(
                            Rect {
                                y: area.y + h,
                                h: area.h - h,
                                ..area
                            },
                            out,
                        );
                    }
                }
            }
        }
    }

    /// Split `target` into itself and a new leaf.
    pub fn split(&mut self, target: &str, new_id: &str, dir: SplitDir) -> bool {
        match self {
            Layout::Leaf(id) if id == target => {
                let old = Layout::Leaf(id.clone());
                *self = Layout::Split {
                    dir,
                    ratio: 0.5,
                    a: Box::new(old),
                    b: Box::new(Layout::leaf(new_id)),
                };
                true
            }
            Layout::Leaf(_) => false,
            Layout::Split { a, b, .. } => {
                a.split(target, new_id, dir) || b.split(target, new_id, dir)
            }
        }
    }

    /// Remove `target`; returns true if the tree collapsed to a single leaf.
    pub fn remove(&mut self, target: &str) -> bool {
        match self {
            Layout::Leaf(id) => id == target,
            Layout::Split { a, b, .. } => {
                if a.remove(target) {
                    *self = (**b).clone();
                    return false;
                }
                if b.remove(target) {
                    *self = (**a).clone();
                    return false;
                }
                false
            }
        }
    }
}
