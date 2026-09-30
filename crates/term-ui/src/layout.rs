//! The tab's split layout: a binary tree of panes, independent of any host.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SplitDir {
    Right,
    Down,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_resize_and_close_preserve_the_remaining_panes() {
        let mut layout = Layout::leaf("a");
        assert!(layout.split("a", "b", SplitDir::Right));
        assert!(layout.split("b", "c", SplitDir::Down));
        let area = Rect {
            x: 10.0,
            y: 20.0,
            w: 1000.0,
            h: 600.0,
        };
        assert_eq!(layout.handles(area).len(), 2);
        layout.set_ratio(&[], 0.3);
        layout.set_ratio(&[true], 0.7);
        let rects = layout.rects(area);
        assert_eq!(
            rects
                .iter()
                .map(|(_, r)| (r.x, r.y, r.w, r.h))
                .collect::<Vec<_>>(),
            vec![
                (10.0, 20.0, 300.0, 600.0),
                (310.0, 20.0, 700.0, 420.0),
                (310.0, 440.0, 700.0, 180.0)
            ]
        );
        assert!(!layout.remove("b"));
        assert_eq!(layout.ids(), vec!["a", "c"]);
        assert_eq!(layout.handles(area).len(), 1);
        assert!(!layout.remove("a"));
        assert_eq!(layout.ids(), vec!["c"]);
        assert!(layout.handles(area).is_empty());
    }
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

/// A draggable divider between two panes.
#[derive(Clone)]
pub struct Handle {
    /// Path to the split node (`[]` = root; `true` = right/bottom child).
    pub path: Vec<bool>,
    pub dir: SplitDir,
    /// The area the split divides, for computing the new ratio.
    pub area: Rect,
    /// The hit band for the pointer.
    pub rect: Rect,
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

    /// Divider handles for dragging: a thin hit band at each split boundary,
    /// tagged with the path to that split and the area it divides.
    pub fn handles(&self, area: Rect) -> Vec<Handle> {
        let mut out = Vec::new();
        self.handles_into(area, &mut Vec::new(), &mut out);
        out
    }

    fn handles_into(&self, area: Rect, path: &mut Vec<bool>, out: &mut Vec<Handle>) {
        if let Layout::Split { dir, ratio, a, b } = self {
            let r = ratio.clamp(0.1, 0.9);
            match dir {
                SplitDir::Right => {
                    let w = area.w * r;
                    let x = area.x + w;
                    out.push(Handle {
                        path: path.clone(),
                        dir: *dir,
                        area,
                        rect: Rect {
                            x: x - 3.0,
                            y: area.y,
                            w: 6.0,
                            h: area.h,
                        },
                    });
                    path.push(false);
                    a.handles_into(Rect { w, ..area }, path, out);
                    *path.last_mut().unwrap() = true;
                    b.handles_into(
                        Rect {
                            x,
                            w: area.w - w,
                            ..area
                        },
                        path,
                        out,
                    );
                    path.pop();
                }
                SplitDir::Down => {
                    let h = area.h * r;
                    let y = area.y + h;
                    out.push(Handle {
                        path: path.clone(),
                        dir: *dir,
                        area,
                        rect: Rect {
                            x: area.x,
                            y: y - 3.0,
                            w: area.w,
                            h: 6.0,
                        },
                    });
                    path.push(false);
                    a.handles_into(Rect { h, ..area }, path, out);
                    *path.last_mut().unwrap() = true;
                    b.handles_into(
                        Rect {
                            y,
                            h: area.h - h,
                            ..area
                        },
                        path,
                        out,
                    );
                    path.pop();
                }
            }
        }
    }

    /// Set the ratio of the split at `path` (empty path = the root split).
    pub fn set_ratio(&mut self, path: &[bool], ratio: f32) {
        if let Layout::Split { ratio: r, a, b, .. } = self {
            if path.is_empty() {
                *r = ratio.clamp(0.1, 0.9);
            } else if path[0] {
                b.set_ratio(&path[1..], ratio);
            } else {
                a.set_ratio(&path[1..], ratio);
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
