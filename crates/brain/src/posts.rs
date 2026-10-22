//! Cover posts for the peek-shoot-hide loop (The Last of Us "posts", GAIP2 ch.34; Killzone 3
//! position picking, GAIP1 ch.29). A post is a cover point hidden from the player with a peek
//! cell beside it (or above it, for low cover) that sees him. Map-agnostic: works from
//! `Arena::cover` and line of sight only, never the player's true position beyond what the
//! caller believes.

use crate::arena::Arena;
use crate::math::V2;

/// Posts are searched within this radius (m) of the enemy.
pub const POST_RADIUS: f32 = 8.0;
/// Peek cells sit this far (m) sideways from the post.
pub const PEEK_OFFSETS: [f32; 3] = [1.0, 0.8, 1.2];
/// A post this close (m) to a recent peek is penalised, fading over `RECENT_PEEK_S`.
pub const RECENT_PEEK_M: f32 = 3.0;
pub const RECENT_PEEK_S: f32 = 10.0;
/// Moving on from a post means going at least this far (m): a re-peek from a new angle.
pub const NEXT_POST_M: f32 = 2.5;
/// Never pick a post closer than this to the player (m).
pub const MIN_PLAYER_DIST: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Post {
    /// Where to hide (crouched).
    pub pos: V2,
    /// Where to step to fire from (equals `pos` for low cover: stand up to peek).
    pub peek: V2,
    /// Hidden even standing (tall cover).
    pub tall: bool,
    pub score: f32,
}

/// A spot within ~1 m sideways of `post` that sees `player` standing, or `post` itself when
/// low cover hides it crouched but not standing (pop up over it).
pub fn peek_cell(arena: &Arena, post: V2, player: V2) -> Option<V2> {
    let fwd = (player - post).norm();
    let right = fwd.perp();
    for off in PEEK_OFFSETS {
        for s in [1.0, -1.0] {
            let p = post + right * (off * s);
            if arena.nav.walkable_at(p) && arena.nav.clear_line(post, p) && arena.line_of_sight(p, false, player, false) {
                return Some(p);
            }
        }
    }
    (arena.line_of_sight(post, false, player, false)).then_some(post)
}

/// How much a spot overlaps recent peeks (0 = fresh, 1 = just peeked from here).
pub fn recency(p: V2, recent: &[(V2, f32)]) -> f32 {
    recent
        .iter()
        .map(|&(q, age)| (1.0 - p.dist(q) / RECENT_PEEK_M).max(0.0) * (1.0 - age / RECENT_PEEK_S).max(0.0))
        .fold(0.0, f32::max)
}

/// The best post for an enemy at `me` against a player believed at `player`. `recent`: peek
/// spots used lately (pos, seconds ago); `current`: the post it is leaving (the next one is
/// at least `NEXT_POST_M` away).
pub fn pick(arena: &Arena, me: V2, player: V2, recent: &[(V2, f32)], current: Option<V2>) -> Option<Post> {
    let d_now = me.dist(player);
    let mut best: Option<Post> = None;
    for c in &arena.cover {
        let d_me = c.pos.dist(me);
        let d_pl = c.pos.dist(player);
        if d_me > POST_RADIUS || d_pl < MIN_PLAYER_DIST || current.is_some_and(|p| p.dist(c.pos) < NEXT_POST_M) {
            continue;
        }
        // Must hide at least crouched.
        if arena.line_of_sight(player, false, c.pos, true) {
            continue;
        }
        let Some(peek) = peek_cell(arena, c.pos, player) else { continue };
        let tall = !arena.line_of_sight(player, false, c.pos, false);
        let covered = if tall { 1.0 } else { 0.75 };
        // Getting there shouldn't mean walking at him.
        let toward = ((d_now - d_pl).max(0.0) / 6.0).min(1.0);
        // Nor crossing his view on the way (sample the midpoint).
        let mid = (me + c.pos) * 0.5;
        let crossing = if d_me > 2.0 && arena.line_of_sight(player, false, mid, false) { 0.8 } else { 1.0 };
        let fresh = 1.0 - recency(peek, recent).max(recency(c.pos, recent));
        let score = covered * (1.0 - 0.5 * toward) * crossing * (0.15 + 0.85 * fresh) - 0.02 * d_me;
        if best.is_none_or(|b| score > b.score) {
            best = Some(Post { pos: c.pos, peek, tall, score });
        }
    }
    best
}

/// Nearest cover point within `radius` of `me` hidden (crouched) from `player`: somewhere to
/// reload. None if the enemy is already hidden there or nothing is close.
pub fn reload_spot(arena: &Arena, me: V2, player: V2, radius: f32) -> Option<V2> {
    arena
        .cover
        .iter()
        .map(|c| c.pos)
        .filter(|c| c.dist(me) <= radius && c.dist(player) >= MIN_PLAYER_DIST - 1.0 && !arena.line_of_sight(player, false, *c, true))
        .min_by(|a, b| a.dist(me).total_cmp(&b.dist(me)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posts_hide_and_peek() {
        let a = Arena::foundry();
        let player = V2::new(0.0, 20.0);
        let me = V2::new(7.0, 13.0);
        let p = pick(&a, me, player, &[], None).expect("a post near the crates");
        assert!(!a.line_of_sight(player, false, p.pos, true), "hidden crouched");
        assert!(a.line_of_sight(p.peek, false, player, false), "peek sees him");
        // Having just peeked there, another post wins.
        let q = pick(&a, me, player, &[(p.peek, 0.0)], Some(p.pos)).expect("another post");
        assert!(q.pos.dist(p.pos) >= NEXT_POST_M, "{q:?} vs {p:?}");
    }
}
