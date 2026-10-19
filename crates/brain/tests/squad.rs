//! Squad layer: plays, position evaluation, coordinated vs solo, determinism.
use brain::arena::Arena;
use brain::evolve::BrainState;
use brain::ghost::GhostMode;
use brain::limits::Limits;
use brain::math::{V2, angle_diff};
use brain::mind::{AllyInfo, EnemyMind, MindCtx, Observation};
use brain::player_model::PlayerModel;
use brain::sim::{self, BODY_RADIUS, DT, SPRINT, WALK};
use brain::squad::{Callout, MemberView, Order, Play, Role, SquadCoordinator};

struct Frame {
    pos: Vec<V2>,
    orders: Vec<Option<Order>>,
}

/// Minimal kinematic world: a stationary, harmless player; the squad runs `play`.
fn run_play(play: Play, player: V2, secs: f32, seed: u64) -> (Vec<Frame>, Vec<Callout>) {
    let a = Arena::foundry();
    let model = PlayerModel::default();
    let cx = MindCtx { arena: &a, model: &model, limits: Limits::for_generation(0) };
    let mut minds: Vec<EnemyMind> = BrainState::new(seed).squad_minds(seed);
    let n = minds.len();
    let mut pos: Vec<V2> = a.enemy_spawns.iter().take(n).copied().collect();
    let mut looks = vec![V2::new(0.0, 1.0); n];
    let mut last_seen = vec![99.0f32; n];
    let mut last_known = vec![player; n];
    let mut squad = SquadCoordinator::new(&a, n);
    squad.force_play = Some(play);
    let (mut frames, mut calls) = (Vec::new(), Vec::new());
    let mut t = 0.0;
    while t < secs {
        t += DT;
        let views: Vec<MemberView> = (0..n)
            .map(|i| MemberView {
                pos: pos[i],
                alive: true,
                hp: 1.0,
                last_known: last_known[i],
                last_seen: last_seen[i],
                heard: None,
                look: looks[i],
                traits: minds[i].traits,
                bark: minds[i].take_bark(),
            })
            .collect();
        let orders = squad.tick(DT, &views, &a, &model).to_vec();
        calls.extend(squad.drain_callouts());
        for (m, o) in minds.iter_mut().zip(&orders) {
            m.set_order(*o);
        }
        let board = squad.board;
        let snap = pos.clone();
        for i in 0..n {
            let sees = a.line_of_sight(pos[i], false, player, false) && pos[i].dist(player) < sim::VISION_RANGE;
            if sees {
                last_seen[i] = 0.0;
                last_known[i] = player;
            } else {
                last_seen[i] += DT;
            }
            let shared = board.merge(last_known[i], last_seen[i]);
            let obs = Observation {
                dt: DT,
                pos: pos[i],
                hp: 1.0,
                ammo_frac: 1.0,
                player_visible: sees,
                player_pos: shared.0,
                player_hp: 1.0,
                player_facing: 0.0,
                time_since_seen: shared.1,
                allies: (0..n).filter(|&j| j != i).map(|j| AllyInfo { pos: snap[j], engaged: false, flanking: false }).collect(),
                ..Default::default()
            };
            let act = minds[i].think(&obs, &cx);
            looks[i] = act.look_dir;
            let speed = if act.crouch { sim::CROUCH_SPEED } else if act.sprint { SPRINT } else { WALK };
            pos[i] = a.resolve_collision(pos[i] + act.move_dir * speed * DT, BODY_RADIUS);
        }
        frames.push(Frame { pos: pos.clone(), orders });
    }
    (frames, calls)
}

fn role_idx(f: &Frame, role: Role) -> Option<usize> {
    f.orders.iter().position(|o| o.is_some_and(|o| o.role == role))
}

#[test]
fn crossfire_positions_are_separated() {
    let player = V2::new(0.0, -14.0);
    for seed in [1, 2, 3] {
        let (frames, _) = run_play(Play::Crossfire, player, 2.0, seed);
        let last = frames.last().unwrap();
        let dests: Vec<V2> = last.orders.iter().flatten().filter(|o| o.role == Role::Crossfire).filter_map(|o| o.dest).collect();
        assert_eq!(dests.len(), 2, "seed {seed}: crossfire needs a pair of positions");
        let (b0, b1) = ((dests[0] - player).angle(), (dests[1] - player).angle());
        let sep = angle_diff(b0, b1).abs().to_degrees();
        eprintln!("seed {seed}: crossfire bearing separation {sep:.0} deg");
        assert!(sep > 60.0, "seed {seed}: crossfire lines differ by only {sep:.0} deg");
    }
}

#[test]
fn trade_follower_stays_behind_entry() {
    let player = V2::new(0.0, 14.0);
    for seed in [1, 2] {
        let (frames, _) = run_play(Play::Trade, player, 10.0, seed);
        let mut ds = Vec::new();
        for f in frames.iter().skip((3.0 / DT) as usize) {
            let (Some(e), Some(tr)) = (role_idx(f, Role::Entry), role_idx(f, Role::Trader)) else { continue };
            let d = f.pos[e].dist(f.pos[tr]);
            let behind = f.pos[tr].dist(player) > f.pos[e].dist(player);
            ds.push((d, behind));
        }
        let ok = ds.iter().filter(|(d, b)| (4.0..=8.0).contains(d) && *b).count() as f32 / ds.len().max(1) as f32;
        let mean = ds.iter().map(|x| x.0).sum::<f32>() / ds.len().max(1) as f32;
        eprintln!("seed {seed}: trade follower mean gap {mean:.1} m, {:.0}% of samples 4-8 m behind", ok * 100.0);
        assert!(!ds.is_empty() && ok > 0.7, "seed {seed}: follower in 4-8 m only {:.0}% (mean {mean:.1})", ok * 100.0);
    }
}

#[test]
fn bounding_overwatch_holds_while_moving() {
    let player = V2::new(0.0, 29.0);
    let (frames, calls) = run_play(Play::Overwatch, player, 8.0, 4);
    let mut both = 0;
    for w in frames.windows(2) {
        let speed = |i: usize| w[1].pos[i].dist(w[0].pos[i]) / DT;
        let holding = (0..w[1].pos.len()).any(|i| w[1].orders[i].is_some_and(|o| o.role == Role::Overwatch) && speed(i) < 0.5);
        let moving = (0..w[1].pos.len()).any(|i| w[1].orders[i].is_some_and(|o| o.role == Role::Bound) && speed(i) > 2.0);
        both += (holding && moving) as u32;
    }
    eprintln!("overwatch: {both} ticks with a holder and a mover; callouts {:?}", calls.iter().map(|c| &c.text).collect::<Vec<_>>());
    assert!(both > 20, "need ticks where one holds while another moves, got {both}");
    assert!(calls.iter().any(|c| c.text == "Moving - cover me"));
    assert!(calls.iter().all(|c| c.text.is_ascii()));
}

#[test]
fn pin_and_flank_calls_a_sector() {
    let (frames, calls) = run_play(Play::PinFlank, V2::new(0.0, -14.0), 2.0, 5);
    assert!(frames.last().unwrap().orders.iter().flatten().any(|o| o.role == Role::Flanker));
    assert!(calls.iter().any(|c| c.text.starts_with("Flanking ")), "{calls:?}");
}

/// Coordinated squads beat the same squads playing solo, against the same ghost.
#[test]
fn coordinated_beats_uncoordinated() {
    let a = Arena::foundry();
    let model = PlayerModel::default();
    let limits = Limits::for_generation(0);
    let (mut on, mut off) = ((0.0, 0.0, 0.0), (0.0, 0.0, 0.0));
    let rounds: u64 = 480;
    for k in 0..rounds {
        let b = BrainState::new(100 + k / 8);
        for (coord, acc) in [(true, &mut on), (false, &mut off)] {
            let r = sim::run_round_squad(&a, &model, limits, b.squad_minds(k), 5000 + k, GhostMode::V2, coord);
            acc.0 += (!r.player_won) as u32 as f32;
            acc.1 += r.enemies.iter().map(|e| e.damage_dealt).sum::<f32>();
            acc.2 += (r.duration >= sim::ROUND_TIME - 0.1) as u32 as f32;
        }
    }
    let n = rounds as f32;
    eprintln!(
        "squad win rate: coordinated {:.3} (dmg {:.1}, timeouts {}) vs solo {:.3} (dmg {:.1}, timeouts {}) over {rounds} rounds",
        on.0 / n,
        on.1 / n,
        on.2,
        off.0 / n,
        off.1 / n,
        off.2
    );
    assert!(on.0 > off.0, "coordinated squads should win more often");
}

#[test]
fn squad_rounds_are_deterministic() {
    let a = Arena::foundry();
    let model = PlayerModel::default();
    let s = BrainState::new(9);
    for seed in [1, 2, 3] {
        let r1 = sim::run_round(&a, &model, Limits::for_generation(0), s.squad_minds(seed), seed);
        let r2 = sim::run_round(&a, &model, Limits::for_generation(0), s.squad_minds(seed), seed);
        assert_eq!(r1.player_won, r2.player_won);
        assert_eq!(r1.duration, r2.duration);
        for (x, y) in r1.enemies.iter().zip(&r2.enemies) {
            assert_eq!(x.damage_dealt, y.damage_dealt);
            assert_eq!(x.time_alive, y.time_alive);
        }
    }
    let (f1, c1) = run_play(Play::Crossfire, V2::new(0.0, -14.0), 3.0, 7);
    let (f2, c2) = run_play(Play::Crossfire, V2::new(0.0, -14.0), 3.0, 7);
    assert_eq!(c1, c2);
    assert_eq!(f1.last().unwrap().pos, f2.last().unwrap().pos);
}
