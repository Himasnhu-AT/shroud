//! Individual bot tactics: shoot on sight (tokens no longer hold fire), the isolated bot's
//! peek-shoot-hide loop at cover posts, reloading behind cover, toughness against the ghost.
use brain::arena::{Arena, MapId};
use brain::difficulty::Difficulty;
use brain::evolve::BrainState;
use brain::ghost::GhostMode;
use brain::limits::Limits;
use brain::math::V2;
use brain::mind::{AllyInfo, EnemyMind, MindCtx, Observation, PostPhase};
use brain::perception::Alert;
use brain::player_model::PlayerModel;
use brain::posts;
use brain::sim::{self, BODY_RADIUS, DT, FIRE_INTERVAL, MAG, RELOAD_S};
use brain::squad::UtilityBudget;

fn mind(seed: u64) -> EnemyMind {
    BrainState::new(seed).squad_minds(seed).remove(0)
}

/// One simulated enemy body against a player standing still at `player` (who doesn't shoot).
struct Bot {
    pos: V2,
    crouched: bool,
    ammo: u32,
    reload_t: f32,
    cooldown: f32,
    last_seen: f32,
}

struct Tick {
    t: f32,
    pos: V2,
    fired: bool,
    reload_started: bool,
    phase: Option<PostPhase>,
    visible: bool,
    lean: f32,
}

#[allow(clippy::too_many_arguments)]
/// `allies`: offsets from the bot (they keep up with it).
fn run(m: &mut EnemyMind, a: &Arena, start: V2, player: V2, secs: f32, ammo: u32, allies: &[V2], token_denied: bool) -> Vec<Tick> {
    let model = PlayerModel::default();
    let cx = MindCtx { arena: a, model: &model, limits: Limits::for_generation(0) };
    let mut b = Bot { pos: start, crouched: false, ammo, reload_t: 0.0, cooldown: 0.0, last_seen: 0.5 };
    let mut out = Vec::new();
    let mut t = 0.0;
    while t < secs {
        t += DT;
        let visible = a.line_of_sight(b.pos, b.crouched, player, false);
        b.last_seen = if visible { 0.0 } else { b.last_seen + DT };
        let o = Observation {
            dt: DT,
            pos: b.pos,
            hp: 1.0,
            ammo_frac: if b.reload_t > 0.0 { 0.0 } else { b.ammo as f32 / MAG as f32 },
            crouched: b.crouched,
            player_visible: visible,
            player_pos: player,
            player_hp: 1.0,
            player_facing: (b.pos - player).angle(),
            time_since_seen: b.last_seen,
            allies: allies.iter().map(|&off| AllyInfo { pos: b.pos + off, engaged: true, flanking: false }).collect(),
            squad_alert: Alert::Combat,
            token_denied,
            ..Default::default()
        };
        let act = m.think(&o, &cx);
        b.crouched = act.crouch;
        let speed = if b.crouched { sim::CROUCH_SPEED } else if act.sprint { sim::SPRINT } else { sim::WALK };
        b.pos = a.resolve_collision(b.pos + act.move_dir * speed * DT, BODY_RADIUS);
        let mut reload_started = false;
        let mut fired = false;
        b.cooldown -= DT;
        if b.reload_t > 0.0 {
            b.reload_t -= DT;
            if b.reload_t <= 0.0 {
                b.ammo = MAG;
            }
        } else if act.reload && b.ammo < MAG {
            b.reload_t = RELOAD_S;
            reload_started = true;
        } else if act.fire && b.cooldown <= 0.0 && b.ammo > 0 {
            b.cooldown = FIRE_INTERVAL;
            b.ammo -= 1;
            fired = true;
            if b.ammo == 0 {
                // The weapon reloads itself on an empty magazine, wherever he is.
                b.reload_t = RELOAD_S;
                reload_started = true;
            }
        }
        out.push(Tick { t, pos: b.pos, fired, reload_started, phase: m.post_phase(), visible, lean: act.lean });
    }
    out
}

/// A test scene: a player in the open and a cover point near him with at least two posts.
fn scene(a: &Arena, player: V2) -> Option<V2> {
    a.cover.iter().map(|c| c.pos).find(|&c| {
        let d = c.dist(player);
        (9.0..15.0).contains(&d)
            && !a.line_of_sight(player, false, c, true)
            && posts::pick(a, c, player, &[], None).is_some_and(|p| {
                posts::pick(a, p.pos, player, &[(p.peek, 0.0)], Some(p.pos)).is_some_and(|q| q.pos.dist(p.pos) >= posts::NEXT_POST_M)
            })
    })
}

/// Two walkable spots 15 m apart (north-south) with a clear line between them.
fn open_pair(a: &Arena) -> (V2, V2) {
    for x in (-26..=26).step_by(2) {
        for z in (-26..=10).step_by(2) {
            let me = V2::new(x as f32, z as f32 + 15.0);
            let p = V2::new(x as f32, z as f32);
            if a.nav.walkable_at(me) && a.nav.walkable_at(p) && a.line_of_sight(me, true, p, true) {
                return (me, p);
            }
        }
    }
    panic!("no open pair")
}

#[test]
fn shoots_on_sight_without_a_token() {
    let a = Arena::foundry();
    let model = PlayerModel::default();
    let limits = Limits::for_generation(0);
    let cx = MindCtx { arena: &a, model: &model, limits };
    let (me, p) = open_pair(&a);
    for seed in 0..8u64 {
        for denied in [false, true] {
            let mut m = mind(seed);
            let base = Observation {
                dt: DT,
                pos: me,
                hp: 1.0,
                ammo_frac: 1.0,
                player_pos: p,
                player_hp: 1.0,
                player_facing: 0.0,
                squad_alert: Alert::Combat,
                token_denied: denied,
                time_since_seen: 2.0,
                ..Default::default()
            };
            // Already in the fight (radio), then he appears.
            for _ in 0..10 {
                m.think(&base, &cx);
            }
            let first = (1..=60).find(|_| m.think(&Observation { player_visible: true, time_since_seen: 0.0, ..base.clone() }, &cx).fire).map(|k| k as f32 * DT);
            let first = first.unwrap_or_else(|| panic!("seed {seed} denied {denied}: never fired"));
            let react = (m.traits.reaction_time * brain::mind::COMBAT_REACTION).max(limits.min_reaction_s);
            assert!(first >= limits.min_reaction_s - DT, "seed {seed}: fired at {first:.2}s, under the fairness floor");
            assert!(first <= react + 0.25, "seed {seed} denied {denied}: first shot at {first:.2}s (reaction {react:.2}s)");
        }
    }
}

#[test]
fn caught_unaware_reacts_slower() {
    let a = Arena::foundry();
    let model = PlayerModel::default();
    let limits = Limits::for_generation(0);
    let cx = MindCtx { arena: &a, model: &model, limits };
    let (me, p) = open_pair(&a);
    for seed in 0..4u64 {
        let mut m = mind(seed);
        let o = Observation { dt: DT, pos: me, hp: 1.0, ammo_frac: 1.0, player_pos: p, player_hp: 1.0, player_visible: true, ..Default::default() };
        let first = (1..=80).find(|_| m.think(&o, &cx).fire).map(|k| k as f32 * DT).unwrap_or(99.0);
        assert!(first > 0.75, "seed {seed}: unaware enemy fired after {first:.2}s");
    }
}

#[test]
fn isolated_bot_peeks_shoots_hides_and_moves_posts() {
    let a = Arena::foundry();
    let player = V2::new(0.0, 22.0);
    let start = scene(&a, player).expect("a cover point with two posts near the player");
    let mut cycles_total = 0;
    for seed in 0..4u64 {
        let mut m = mind(seed);
        let trace = run(&mut m, &a, start, player, 14.0, MAG, &[], false);
        assert!(m.isolated(), "no allies: isolated");
        let mut last = None;
        for k in &trace {
            if k.phase != last {
                eprintln!("  {:5.2}s {:?} at ({:.1},{:.1}) vis {}", k.t, k.phase, k.pos.x, k.pos.y, k.visible);
                last = k.phase;
            }
        }
        // Cycles: Peek with shots, then Hide.
        let mut cycles = 0;
        let mut peek_spots: Vec<V2> = Vec::new();
        let mut fired_in_peek = false;
        for w in trace.windows(2) {
            if w[1].phase == Some(PostPhase::Peek) && w[1].fired {
                fired_in_peek = true;
                if !peek_spots.iter().any(|s| s.dist(w[1].pos) < 1.5) {
                    peek_spots.push(w[1].pos);
                }
            }
            if w[0].phase == Some(PostPhase::Peek) && w[1].phase == Some(PostPhase::Hide) && fired_in_peek {
                cycles += 1;
                fired_in_peek = false;
            }
        }
        // Hiding means hidden: whenever crouched behind the post he can't see it.
        let hidden_while_hiding = trace.iter().filter(|k| k.phase == Some(PostPhase::Hide) && !k.fired).filter(|k| k.visible).count();
        let hide_ticks = trace.iter().filter(|k| k.phase == Some(PostPhase::Hide)).count();
        eprintln!("seed {seed}: {cycles} cycles, peek spots {peek_spots:?}, seen while hiding {hidden_while_hiding}/{hide_ticks}");
        assert!(cycles >= 2, "seed {seed}: {cycles} peek-fire-hide cycles in 14 s");
        assert!(peek_spots.len() >= 2, "seed {seed}: re-peeks from the same spot {peek_spots:?}");
        assert!((hidden_while_hiding as f32) < 0.2 * hide_ticks as f32, "seed {seed}: exposed while hiding");
        cycles_total += cycles;
    }
    assert!(cycles_total >= 8);
}

#[test]
fn peeks_lean_out_at_tall_cover() {
    // Somewhere a peek steps sideways out of tall cover: the action leans that way.
    let a = Arena::foundry();
    let player = V2::new(0.0, 22.0);
    let start = scene(&a, player).unwrap();
    let mut leaned = false;
    for seed in 0..4u64 {
        let mut m = mind(seed);
        leaned |= run(&mut m, &a, start, player, 14.0, MAG, &[], false).iter().any(|k| k.phase == Some(PostPhase::Peek) && k.lean != 0.0);
    }
    assert!(leaned, "no lean on any peek");
}

#[test]
fn bot_with_squad_nearby_fights_in_the_open_with_a_token() {
    let a = Arena::foundry();
    let player = V2::new(0.0, 22.0);
    let start = scene(&a, player).unwrap();
    let mut m = mind(1);
    let trace = run(&mut m, &a, start, player, 6.0, MAG, &[V2::new(3.0, 0.0)], false);
    assert!(!m.isolated());
    // A token holder with support close by keeps up the pressure instead of working posts.
    let posting = trace.iter().filter(|k| k.phase.is_some()).count();
    assert!((posting as f32) < 0.3 * trace.len() as f32, "posting {posting}/{}", trace.len());
    assert!(trace.iter().filter(|k| k.fired).count() >= 10, "fights");
}

#[test]
fn reloads_behind_cover_not_in_the_open() {
    let a = Arena::foundry();
    let player = V2::new(0.0, 22.0);
    // An open spot with hidden cover within ~3.5 m, the player in clear view.
    let spot = a
        .cover
        .iter()
        .map(|c| c.pos)
        .filter(|c| !a.line_of_sight(player, false, *c, true) && (9.0..16.0).contains(&c.dist(player)))
        .find_map(|c| {
            (0..16).map(|k| c + V2::from_angle(k as f32 * std::f32::consts::TAU / 16.0) * 3.0).find(|&p| {
                a.nav.walkable_at(p) && a.line_of_sight(p, true, player, false) && p.dist(player) > 9.0 && posts::reload_spot(&a, p, player, 4.0).is_some()
            })
        })
        .expect("setup: an open spot next to cover");
    let mut in_open = 0;
    let mut total = 0;
    for seed in 0..6u64 {
        let mut m = mind(seed);
        // A token holder, squad around (not isolated), five rounds left.
        let trace = run(&mut m, &a, spot, player, 6.0, 5, &[V2::new(0.0, -6.0)], false);
        if let Some(k) = trace.iter().find(|k| k.reload_started) {
            total += 1;
            let hidden = !a.line_of_sight(player, false, k.pos, true);
            eprintln!("seed {seed}: reload at {:.1}s at {:?} hidden {hidden}", k.t, k.pos);
            if !hidden {
                in_open += 1;
            }
        }
    }
    assert!(total >= 4, "reloaded in {total}/6 runs");
    assert!(in_open <= 1, "{in_open}/{total} reloads started in the open");
}

#[test]
fn radio_updates_reach_members_at_different_times() {
    // The squad's belief jumps; members turn to it at their own (seeded) delays, not in unison.
    let a = Arena::foundry();
    let model = PlayerModel::default();
    let cx = MindCtx { arena: &a, model: &model, limits: Limits::for_generation(0) };
    let me = V2::new(-20.0, -20.0);
    let old = V2::new(-20.0, -5.0);
    let new = V2::new(-5.0, -20.0);
    let mut turn_times = Vec::new();
    for seed in 0..6u64 {
        let mut m = mind(seed);
        let base = Observation { dt: DT, pos: me, hp: 1.0, ammo_frac: 1.0, player_hp: 1.0, squad_alert: Alert::Combat, time_since_seen: 0.5, ..Default::default() };
        for _ in 0..20 {
            m.think(&Observation { player_pos: old, ..base.clone() }, &cx);
        }
        let k = (1..=40).find(|_| {
            let act = m.think(&Observation { player_pos: new, ..base.clone() }, &cx);
            act.look_dir.dot((new - me).norm()) > 0.2
        });
        turn_times.push(k.unwrap_or(99) as f32 * DT);
    }
    let lo = turn_times.iter().copied().fold(f32::MAX, f32::min);
    let hi = turn_times.iter().copied().fold(f32::MIN, f32::max);
    eprintln!("turn times {turn_times:?}");
    assert!(lo >= 0.15, "someone turned before the radio could reach him: {lo}");
    assert!(hi - lo > 0.1, "they all turned together: {turn_times:?}");
}

#[test]
fn squads_stay_tough_against_the_ghost() {
    // Elite gen-0 squads must beat ghost v2 most of the time on every map (grenades off).
    for id in MapId::ALL {
        let a = Arena::build(id);
        let d = Difficulty::Elite;
        let limits = d.limits(0);
        let mut squad_wins = 0;
        let n = 24;
        for k in 0..n as u64 {
            let s = BrainState::new(1 + k % 4);
            let minds = s.squad_minds_for(500 + k, d.params().squad_size, &limits);
            let r = sim::run_round_opts(&a, &s.player_model, limits, minds, 500 + k, GhostMode::V2, true, UtilityBudget::NONE);
            squad_wins += !r.player_won as u32;
        }
        eprintln!("{}: Elite squad won {squad_wins}/{n}", id.label());
        assert!(squad_wins as f32 >= 0.6 * n as f32, "{}: Elite squad won only {squad_wins}/{n}", id.label());
    }
}

#[test]
fn members_without_a_token_fire_suppressively() {
    // Same enemy, same shot: without a token (and not defending itself) the spread is wider.
    let a = Arena::foundry();
    let model = PlayerModel::default();
    let cx = MindCtx { arena: &a, model: &model, limits: Limits::for_generation(0) };
    let (me, p) = open_pair(&a);
    let spread = |denied: bool| {
        let mut m = mind(3);
        let o = Observation {
            dt: DT,
            pos: me,
            hp: 1.0,
            ammo_frac: 1.0,
            player_pos: p,
            player_hp: 1.0,
            player_visible: true,
            squad_alert: Alert::Combat,
            token_denied: denied,
            allies: vec![AllyInfo { pos: me + V2::new(4.0, 0.0), engaged: true, flanking: false }],
            ..Default::default()
        };
        let errs: Vec<f32> = (0..400)
            .map(|_| m.think(&o, &cx))
            .filter(|act| act.fire)
            .map(|act| brain::math::angle_diff(act.shot_dir.angle(), act.look_dir.angle()))
            .collect();
        assert!(errs.len() > 50, "fired {} times", errs.len());
        (errs.iter().map(|e| e * e).sum::<f32>() / errs.len() as f32).sqrt()
    };
    let (with, without) = (spread(false), spread(true));
    eprintln!("rms aim error: token {:.2} deg, no token {:.2} deg", with.to_degrees(), without.to_degrees());
    assert!(without > 1.3 * with, "token {with} vs no token {without}");
}
