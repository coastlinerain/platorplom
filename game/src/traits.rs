use crate::rect::Rect;

pub trait Colisionable {
    fn get_rect(&self) -> Rect;
}

pub fn detect_collision<A: Colisionable, B: Colisionable>(obj_a: &A, obj_b: &B) -> bool {
    obj_a.get_rect().overlaps(&obj_b.get_rect())
}
