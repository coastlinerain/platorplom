#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// Overlap test matching the client renderer's rect semantics: strict `<`,
    /// so edge-only touching is not an overlap.
    pub fn overlaps(&self, other: &Rect) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }
}

#[cfg(test)]
mod tests {
    use super::Rect;

    #[test]
    fn rect_new_sets_fields_in_order() {
        let r = Rect::new(1.0, 2.0, 3.0, 4.0);
        assert_eq!(r.x, 1.0);
        assert_eq!(r.y, 2.0);
        assert_eq!(r.w, 3.0);
        assert_eq!(r.h, 4.0);
    }

    #[test]
    fn rect_overlaps_true_when_overlapping() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        assert!(a.overlaps(&b));
    }

    #[test]
    fn rect_overlaps_false_when_apart() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(20.0, 20.0, 5.0, 5.0);
        assert!(!a.overlaps(&b));
    }

    #[test]
    fn rect_overlaps_false_when_edges_touch() {
        // X-axis edge touch (right edge of a == left edge of b)
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(10.0, 0.0, 10.0, 10.0);
        assert!(!a.overlaps(&b));

        // Y-axis edge touch (bottom edge of a == top edge of b)
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(0.0, 10.0, 10.0, 10.0);
        assert!(!a.overlaps(&b));
    }
}
