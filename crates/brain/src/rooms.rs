//! Rooms and doorways for room clearing (stack and breach, corner checks). Map-agnostic: a
//! doorway is a lintel (`Decor::headers`) over walkable floor; a lintel over a low block is a
//! window, not a way in.

use crate::arena::{Arena, BlockKind};
use crate::math::V2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Doorway {
    /// Centre of the gap on the ground.
    pub pos: V2,
    /// Unit normal through the doorway (either way).
    pub normal: V2,
    pub width: f32,
}

impl Doorway {
    /// Which side of the doorway `p` is on (+1 along `normal`, -1 against).
    pub fn side(&self, p: V2) -> f32 {
        if (p - self.pos).dot(self.normal) >= 0.0 { 1.0 } else { -1.0 }
    }

    /// A point `depth` m through the door on the side opposite `from`.
    pub fn inside(&self, from: V2, depth: f32) -> V2 {
        self.pos - self.normal * (self.side(from) * depth)
    }

    /// Where to stack before breaching from `from`'s side: beside the frame (off the line of
    /// fire through the door), `back` m out. `slot` 0/1 picks the frame side.
    pub fn stack(&self, arena: &Arena, from: V2, back: f32, slot: usize) -> V2 {
        let out = self.normal * self.side(from);
        let lateral = self.normal.perp();
        let sides = if slot.is_multiple_of(2) { [1.0, -1.0] } else { [-1.0, 1.0] };
        for s in sides {
            let p = self.pos + out * back + lateral * (s * (self.width * 0.5 + 0.4));
            if arena.nav.walkable_at(p) && arena.nav.clear_line(p, self.pos + out * 0.6) {
                return p;
            }
        }
        arena.nav.nearest_walkable(self.pos + out * back)
    }
}

/// Every doorway on the map (lintels over walkable ground, no low block underneath).
pub fn doorways(arena: &Arena) -> Vec<Doorway> {
    let mut out = Vec::new();
    for h in &arena.decor.headers {
        let pos = (h.min + h.max) * 0.5;
        let size = h.max - h.min;
        if !arena.nav.walkable_at(pos) {
            continue;
        }
        let windowed = arena
            .blocks
            .iter()
            .any(|b| b.kind == BlockKind::Low && (pos.x - b.center.x).abs() < b.half.x + 0.2 && (pos.y - b.center.y).abs() < b.half.y + 0.2);
        if windowed {
            continue;
        }
        let (normal, width) = if size.x < size.y { (V2::new(1.0, 0.0), size.y) } else { (V2::new(0.0, 1.0), size.x) };
        out.push(Doorway { pos, normal, width });
    }
    out
}

/// Window sills: low blocks with a lintel above them.
pub fn is_window_sill(arena: &Arena, block: usize) -> bool {
    let Some(b) = arena.blocks.get(block) else { return false };
    b.kind == BlockKind::Low
        && arena.decor.headers.iter().any(|h| {
            let c = (h.min + h.max) * 0.5;
            (c.x - b.center.x).abs() < b.half.x + 0.3 && (c.y - b.center.y).abs() < b.half.y + 0.3
        })
}

/// Distance from `p` to the segment `a`-`b`.
fn seg_dist(p: V2, a: V2, b: V2) -> f32 {
    let ab = b - a;
    let l2 = ab.dot(ab);
    let t = if l2 > 1e-6 { ((p - a).dot(ab) / l2).clamp(0.0, 1.0) } else { 0.0 };
    p.dist(a + ab * t)
}

/// Doorways a walk `from` along `path` (smoothed waypoints) passes through, in order, with the
/// index of the waypoint ending that leg.
fn doors_along<'a>(doors: &'a [Doorway], from: V2, path: &'a [V2]) -> impl Iterator<Item = (Doorway, usize)> + 'a {
    let mut prev = from;
    path.iter().enumerate().flat_map(move |(i, &p)| {
        let a = prev;
        prev = p;
        let mut hits: Vec<(f32, Doorway)> =
            doors.iter().filter(|d| seg_dist(d.pos, a, p) < 0.9 && d.side(a) != d.side(p)).map(|d| (d.pos.dist(a), *d)).collect();
        hits.sort_by(|x, y| x.0.total_cmp(&y.0));
        hits.into_iter().map(move |(_, d)| (d, i))
    })
}

/// The first doorway a walk from `from` along `path` goes through, with the index of the
/// waypoint ending that leg.
pub fn door_on_path(doors: &[Doorway], from: V2, path: &[V2]) -> Option<(Doorway, usize)> {
    doors_along(doors, from, path).next()
}

/// Breaching `target` from `from`: the last doorway on the walk there, if the target is within
/// `reach` m past it (he holds the room behind that door).
pub fn breach_door(arena: &Arena, doors: &[Doorway], from: V2, target: V2, reach: f32) -> Option<Doorway> {
    let path = arena.nav.find_path(from, target)?;
    let d = doors_along(doors, from, &path).last()?.0;

    let past = d.side(target) != d.side(from);
    (past && d.pos.dist(target) <= reach && d.pos.dist(from) > 2.0).then_some(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::MapId;

    #[test]
    fn office_has_doors_and_a_breach_route() {
        let a = Arena::build(MapId::Office);
        let doors = doorways(&a);
        assert!(doors.len() >= 20, "{} doors", doors.len());
        // A player in a back office (north of the z = -22 separator), an enemy on the floor.
        let player = a.nav.nearest_walkable(V2::new(-21.0, -27.0));
        let me = a.nav.nearest_walkable(V2::new(-21.0, -12.0));
        let d = breach_door(&a, &doors, me, player, 14.0).expect("a door to breach");
        assert!(d.side(me) != d.side(player));
        let s = d.stack(&a, me, 2.0, 0);
        assert_eq!(d.side(s), d.side(me), "stacks on our side");
        assert!(a.nav.walkable_at(s));
        let inside = d.inside(me, 2.5);
        assert_eq!(d.side(inside), d.side(player));
    }
}
