//! Perception, alert states, attack tokens, aggression and stuck recovery.
use brain::arena::{Arena, BlockKind};
use brain::evolve::BrainState;
use brain::genome::Traits;
use brain::limits::Limits;
use brain::math::V2;
use brain::mind::{EnemyMind, MindCtx, Observation};
use brain::perception::{self, Alert, GUNSHOT_RANGE};
use brain::player_model::PlayerModel;
use brain::sim::{self, BODY_RADIUS, DT};
use brain::squad::{MemberView, SquadCoordinator, token_budget};
use brain::utility::{self, Goal, UtilityInput};

fn member(pos: V2, last_known: V2, last_seen: f32) -> MemberView {
    MemberView { pos, alive: true, hp: 1.0, last_known, last_seen, heard: None, look: V2::new(0.0, 1.0), traits: Traits::default(), bark: None }
}

fn obs(pos: V2, player: V2, age: f32, alert: Alert) -> Observation {
    Observation { dt: DT, pos, hp: 1.0, ammo_frac: 1.0, player_pos: player, player_hp: 1.0, time_since_seen: age, squad_alert: alert, ..Default::default() }
}

#[test]
fn shooting_one_enemy_alerts_whole_squad_within_one_tick() {
    let a = Arena::foundry();
    let model = PlayerModel::default();
    let cx = MindCtx { arena: &a, model: &model, limits: Limits::for_generation(0) };
    let mut minds: Vec<EnemyMind> = BrainState::new(3).squad_minds(3);
    let n = minds.len();
    let mut coord = SquadCoordinator::new(&a, n);
    let spawns: Vec<V2> = a.enemy_spawns.iter().take(n).copied().collect();
    let views: Vec<MemberView> = spawns.iter().map(|&p| member(p, a.player_spawn, 99.0)).collect();
    // A quiet start: nobody knows anything.
    coord.tick(DT, &views, &a, &model);
    for (i, m) in minds.iter_mut().enumerate() {
        m.think(&obs(spawns[i], a.player_spawn, 99.0, coord.board.alert), &cx);
        assert_ne!(m.alert(), Alert::Combat);
    }
    // The player shoots member 0 from here.
    let shooter = V2::new(-20.0, 8.0);
    coord.reveal(0, shooter, &a);
    coord.tick(DT, &views, &a, &model);
    let board = coord.board;
    assert_eq!(board.alert, Alert::Combat);
    assert!(board.pos.dist(shooter) < 1e-4, "shooter position on the board");
    assert!(board.age <= DT + 1e-4, "no radio delay for being shot (age {})", board.age);
    for (i, m) in minds.iter_mut().enumerate() {
        let (pos, age) = board.merge(a.player_spawn, 99.0);
        m.think(&obs(spawns[i], pos, age, board.alert), &cx);
        assert_eq!(m.alert(), Alert::Combat, "member {i}");
    }
}

#[test]
fn fresh_sighting_relays_instantly_in_combat() {
    let a = Arena::foundry();
    let model = PlayerModel::default();
    let mut coord = SquadCoordinator::new(&a, 3);
    let p = V2::new(0.0, 20.0);
    let mut views: Vec<MemberView> = a.enemy_spawns.iter().take(3).map(|&s| member(s, a.player_spawn, 99.0)).collect();
    coord.tick(DT, &views, &a, &model);
    views[1].last_seen = 0.0;
    views[1].last_known = p;
    coord.tick(DT, &views, &a, &model);
    assert_eq!(coord.board.alert, Alert::Combat);
    assert!(coord.board.pos.dist(p) < 1e-4 && coord.board.age < 0.1);
}

#[test]
fn gunshot_occluded_by_tall_block_and_heard_at_40m_in_open() {
    let a = Arena::foundry();
    let b = a.blocks.iter().find(|b| b.kind == BlockKind::Tall).expect("a tall block");
    let src = b.center - V2::new(b.half.x + 3.0, 0.0);
    let lis = b.center + V2::new(b.half.x + 3.0, 0.0);
    let d = src.dist(lis);
    let open = 1.0 - d / GUNSHOT_RANGE;
    let behind = perception::loudness(&a, src, lis, GUNSHOT_RANGE);
    assert!(behind < open * 0.5, "occluded {behind} vs open {open}");

    // A clear 40 m line somewhere in the arena.
    let line = (-28..=28)
        .map(|y| (V2::new(-20.0, y as f32), V2::new(20.0, y as f32)))
        .find(|(s, l)| !a.blocks.iter().any(|b| b.kind != BlockKind::Low && b.intersects_segment(*s, *l)))
        .expect("a clear 40 m line");
    assert!(perception::hear(&a, line.0, line.1, GUNSHOT_RANGE).is_some(), "40 m open gunshot is heard");
    // Footsteps: running carries, sneaking is silent.
    assert!(perception::footstep_range(6.0, false) >= 12.0);
    assert!(perception::footstep_range(4.0, false) < perception::footstep_range(6.0, false));
    assert_eq!(perception::footstep_range(1.0, true), 0.0);
    assert!(perception::hear(&a, line.0, line.0 + V2::new(15.0, 0.0), perception::footstep_range(6.0, false)).is_none());
}

#[test]
fn stuck_against_wall_corner_reaches_goal() {
    let a = Arena::foundry();
    let model = PlayerModel::default();
    let cx = MindCtx { arena: &a, model: &model, limits: Limits::for_generation(0) };
    let mut reached = 0;
    let blocks: Vec<_> = a.blocks.iter().filter(|b| b.kind == BlockKind::Tall).take(4).collect();
    for (k, b) in blocks.iter().enumerate() {
        let mut mind = BrainState::new(k as u64).squad_minds(k as u64).remove(0);
        // Wedged into the block's corner, goal on the far side.
        let mut pos = a.resolve_collision(b.center + b.half + V2::new(0.1, 0.1), BODY_RADIUS);
        let goal = a.nav.nearest_walkable(b.center - b.half - V2::new(2.0, 2.0));
        let mut t = 0.0;
        while t < 25.0 && pos.dist(goal) > 1.5 {
            t += DT;
            let act = mind.think(&obs(pos, goal, 0.5 + t, Alert::Suspicious), &cx);
            let speed = if act.crouch { sim::CROUCH_SPEED } else if act.sprint { sim::SPRINT } else { sim::WALK };
            pos = a.resolve_collision(pos + act.move_dir * speed * DT, BODY_RADIUS);
        }
        if pos.dist(goal) <= 1.5 {
            reached += 1;
        } else {
            eprintln!("block {k}: stuck at {pos:?}, goal {goal:?}");
        }
    }
    assert_eq!(reached, blocks.len());
}

#[test]
fn attack_tokens_limit_shooters() {
    let a = Arena::foundry();
    let model = PlayerModel::default();
    for n in 1..=6 {
        let mut coord = SquadCoordinator::new(&a, n);
        let p = V2::new(0.0, 20.0);
        let views: Vec<MemberView> = (0..n).map(|i| member(V2::new(-10.0 + 4.0 * i as f32, 10.0), p, 0.0)).collect();
        for _ in 0..40 {
            coord.tick(DT, &views, &a, &model);
            let k = coord.tokens.iter().filter(|t| **t).count();
            assert_eq!(k, token_budget(n), "n {n}");
            assert!(k <= 3);
        }
    }
    // Tokens no longer hold fire: a distant member without one shoots on sight too (with a
    // wider, suppressive spread; see bot_tactics.rs).
    let cx = MindCtx { arena: &a, model: &model, limits: Limits::for_generation(0) };
    let shots = |denied: bool| {
        let mut m = BrainState::new(5).squad_minds(5).remove(0);
        let me = V2::new(0.0, -10.0);
        let p = V2::new(0.0, 10.0);
        (0..60)
            .filter(|_| {
                let o = Observation { player_visible: true, time_since_seen: 0.0, token_denied: denied, player_facing: 0.0, ..obs(me, p, 0.0, Alert::Combat) };
                m.think(&o, &cx).fire
            })
            .count()
    };
    assert!(shots(false) > 0, "token holder shoots");
    assert!(shots(true) > 0, "no token: still shoots on sight");
}

#[test]
fn aggressive_choices() {
    let t = Traits { aggression: 0.5, cover_affinity: 0.5, retreat_health: 0.3, flank_bias: 0.3, preferred_range: 15.0, ..Traits::default() };
    let base = UtilityInput {
        in_contact: true,
        visible: true,
        dist: 15.0,
        hp: 0.7,
        player_hp: 1.0,
        taking_fire: false,
        time_since_seen: 0.0,
        allies_engaged: 1,
        flankers: 0,
        model_confidence: 0.0,
        player_aiming_at_me: 1.0,
        player_reloading: false,
        player_hurt: false,
        combat: true,
        token: true,
    };
    // Seen and being aimed at: fight, don't hide.
    let g = utility::choose(Goal::Hunt, &base, &t, None, false);
    assert!(matches!(g, Goal::Attack | Goal::Push), "{g:?}");
    // He's reloading: push.
    let g = utility::choose(Goal::Attack, &UtilityInput { player_reloading: true, ..base }, &t, None, false);
    assert_eq!(g, Goal::Push);
    // Hurt to 25% (above the scaled retreat threshold): still fighting.
    let g = utility::choose(Goal::Attack, &UtilityInput { hp: 0.25, ..base }, &t, None, false);
    assert_ne!(g, Goal::Retreat);
    // No token: reposition (flank) instead of trading shots.
    let g = utility::choose(Goal::Hunt, &UtilityInput { token: false, ..base }, &t, None, false);
    assert_eq!(g, Goal::Flank);
}

#[test]
fn enemies_keep_standoff_from_player() {
    let a = Arena::foundry();
    let model = PlayerModel::default();
    let cx = MindCtx { arena: &a, model: &model, limits: Limits::for_generation(0) };
    let p = V2::new(0.0, 20.0);
    let mut closest = f32::MAX;
    for seed in 0..4u64 {
        let mut m = BrainState::new(seed).squad_minds(seed).remove(0);
        let mut pos = V2::new(0.0, 12.0);
        for _ in 0..(10.0 / DT) as usize {
            // He's visible and reloading: the most tempting moment to rush him.
            let o = Observation { player_visible: true, player_reloading: true, ..obs(pos, p, 0.0, Alert::Combat) };
            let act = m.think(&o, &cx);
            let speed = if act.sprint { sim::SPRINT } else { sim::WALK };
            pos = a.resolve_collision(pos + act.move_dir * speed * DT, BODY_RADIUS);
            closest = closest.min(pos.dist(p));
        }
    }
    assert!(closest > brain::mind::STANDOFF_M - 0.3, "came within {closest:.2} m");
}

#[test]
fn contact_callout_names_true_zone() {
    let a = Arena::foundry();
    let model = PlayerModel::default();
    let mut coord = SquadCoordinator::new(&a, 4);
    let mut views: Vec<MemberView> = a.enemy_spawns.iter().take(4).map(|&s| member(s, a.player_spawn, 99.0)).collect();
    for _ in 0..10 {
        coord.tick(DT, &views, &a, &model);
    }
    coord.drain_callouts();
    for (p, zone) in [(V2::new(-20.0, 8.0), "west lane"), (V2::new(20.0, -3.0), "east lane"), (V2::new(0.0, 20.0), "south yard")] {
        views[2].last_seen = 0.0;
        views[2].last_known = p;
        coord.tick(DT, &views, &a, &model);
        let calls = coord.drain_callouts();
        assert!(calls.iter().any(|c| c.text.contains(zone)), "{p:?}: {calls:?}");
        // Lose him for a while so the next sighting is a new contact.
        views[2].last_seen = 99.0;
        for _ in 0..200 {
            coord.tick(DT, &views, &a, &model);
        }
        coord.drain_callouts();
    }
}

/// No wallhack: out of sight, his last-seen aim stops counting as "aiming at me" within
/// `AIM_MEMORY_S`; in sight it counts in full.
#[test]
fn stale_aim_fades_out_of_sight() {
    let me = V2::new(0.0, 0.0);
    let p = V2::new(0.0, 10.0);
    let dir_p = (p - me).norm();
    // Facing straight at me (from p toward me).
    let facing = (me - p).angle();
    let at = |visible: bool, age: f32| brain::mind::aiming_at_me(&Observation { player_visible: visible, player_facing: facing, ..obs(me, p, age, Alert::Combat) }, dir_p);
    assert!((at(true, 0.0) - 1.0).abs() < 1e-4);
    assert!(at(false, 0.5) > 0.5 && at(false, 0.5) < 1.0);
    assert_eq!(at(false, brain::mind::AIM_MEMORY_S), 0.0);
    assert_eq!(at(false, 30.0), 0.0);
}
