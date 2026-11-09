//! Enemy loadouts: guns win in their own band in the sim, squads get their guns from the
//! weapon genes, old saves load, and room clearing finds doors to breach.

use brain::arena::{Arena, MapId};
use brain::difficulty::Difficulty;
use brain::evolve::BrainState;
use brain::genome::{TraitGenome, UNSET_LOADOUT};
use brain::limits::Limits;
use brain::loadout::{self, GunState, Role};
use brain::math::{V2, rng_from_seed};
use brain::sim;
use brain::squad::{Play, SquadCoordinator};
use brain::weapons::WeaponKind;

#[test]
fn shotgunner_wins_close_marksman_wins_far() {
    let l = Limits::for_generation(0);
    let close = sim::duel(WeaponKind::Shotgun, WeaponKind::Marksman, 5.0, 5.0, &l, 200, 1);
    let far = sim::duel(WeaponKind::Marksman, WeaponKind::Shotgun, 45.0, 5.0, &l, 200, 2);
    assert!(close > 0.7, "shotgun at 5 m wins {close}");
    assert!(far > 0.7, "marksman at 45 m wins {far}");
    // And the all-rounder: the rifle beats the shotgun at 25 m, loses to it at 4 m.
    let mid = sim::duel(WeaponKind::Rifle, WeaponKind::Shotgun, 25.0, 5.0, &l, 200, 3);
    let point_blank = sim::duel(WeaponKind::Shotgun, WeaponKind::Rifle, 4.0, 5.0, &l, 200, 4);
    assert!(mid > 0.6, "rifle at 25 m wins {mid}");
    assert!(point_blank > 0.5, "shotgun at 4 m wins {point_blank}");
}

#[test]
fn squads_are_equipped_from_their_weapon_genes() {
    let b = BrainState::new(7);
    let limits = Limits::for_generation(0);
    let elite = b.squad_minds_on(1, 6, &limits, Difficulty::Elite);
    let recruit = b.squad_minds_on(1, 3, &limits, Difficulty::Recruit);
    let ek: Vec<WeaponKind> = elite.iter().map(|m| m.weapon()).collect();
    let rk: Vec<WeaponKind> = recruit.iter().map(|m| m.weapon()).collect();
    let caps = loadout::caps(Difficulty::Elite, 6);
    for (i, c) in loadout::counts(&ek).iter().enumerate() {
        assert!(*c as usize <= caps[i], "{ek:?}");
    }
    assert!(rk.iter().all(|k| matches!(k, WeaponKind::Rifle | WeaponKind::Smg)), "{rk:?}");
    // The preferred range follows the gun.
    for m in &elite {
        let (lo, hi) = m.weapon().spec().sweet_spot();
        if m.weapon() != WeaponKind::Rifle {
            assert!(m.traits.preferred_range >= lo.min(3.5) && m.traits.preferred_range <= hi.max(8.0), "{:?} {}", m.weapon(), m.traits.preferred_range);
        }
        assert_eq!(m.role(), Role::of(m.weapon()));
    }
}

#[test]
fn old_genomes_without_weapon_genes_load() {
    let mut rng = rng_from_seed(3);
    let g = TraitGenome::random(&mut rng);
    let mut v = serde_json::to_value(&g).unwrap();
    v.as_object_mut().unwrap().remove("loadout");
    let mut old: TraitGenome = serde_json::from_value(v).unwrap();
    assert_eq!(old.loadout, UNSET_LOADOUT);
    // Decoding works before migration, and migration gives the seeded genes back.
    let _ = old.decode(&Limits::for_generation(0));
    old.fill_loadout();
    assert_eq!(old.loadout, g.loadout);
    // A whole brain without weapon genes round-trips through `load_or_new`.
    let b = BrainState::new(9);
    let mut j = serde_json::to_value(&b).unwrap();
    for g in j["traits"]["genomes"].as_array_mut().unwrap() {
        g.as_object_mut().unwrap().remove("loadout");
    }
    let dir = std::env::temp_dir().join(format!("shroud-loadout-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("brain.json");
    std::fs::write(&path, serde_json::to_vec(&j).unwrap()).unwrap();
    let loaded = BrainState::load_or_new(&path, 1);
    assert_eq!(loaded.seed, 9, "the old brain was kept");
    assert!(loaded.traits.genomes.iter().all(|g| g.loadout.iter().all(|x| (0.0..=1.0).contains(x))));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn sidearm_takes_over_a_dry_primary_mid_fight() {
    let a = Arena::yard();
    let b = BrainState::new(5);
    let limits = Limits::for_generation(5);
    let mut m = b.squad_minds_for(1, 1, &limits).remove(0);
    m.set_weapon(WeaponKind::Marksman);
    let model = brain::player_model::PlayerModel::default();
    let cx = brain::mind::MindCtx { arena: &a, model: &model, limits };
    let me = a.nav.nearest_walkable(V2::new(0.0, 10.0));
    let p = a.nav.nearest_walkable(V2::new(0.0, 0.0));
    let mut gun = GunState::new(WeaponKind::Marksman);
    gun.mags[0] = 0;
    gun.start_reload();
    let mut switched = false;
    for _ in 0..40 {
        let o = brain::mind::Observation {
            dt: sim::DT,
            pos: me,
            hp: 1.0,
            ammo_frac: gun.ammo_frac(),
            player_pos: p,
            player_hp: 1.0,
            player_visible: true,
            squad_alert: brain::perception::Alert::Combat,
            weapon: gun.held,
            primary_ammo: gun.primary_frac(),
            busy_s: gun.busy_s(),
            ..Default::default()
        };
        let act = m.think(&o, &cx);
        if let Some(k) = act.weapon {
            switched |= k == WeaponKind::Pistol;
            gun.switch_to(k);
        }
        gun.tick(sim::DT, act.fire);
    }
    assert!(switched, "drew the pistol instead of a 3 s reload in his face");
    assert_eq!(gun.held, WeaponKind::Pistol);
}

#[test]
fn trade_stacks_and_breaches_a_room() {
    let a = Arena::build(MapId::Office);
    let model = brain::player_model::PlayerModel::default();
    let mut sq = SquadCoordinator::new(&a, 3);
    sq.force_play = Some(Play::Trade);
    let player = a.nav.nearest_walkable(V2::new(-21.0, -27.0));
    let pos = [V2::new(-21.0, -12.0), V2::new(-23.0, -10.0), V2::new(-25.0, -8.0)].map(|p| a.nav.nearest_walkable(p));
    let t = brain::genome::Traits { weapon: WeaponKind::Shotgun, ..Default::default() };
    let views: Vec<brain::squad::MemberView> = pos
        .iter()
        .enumerate()
        .map(|(i, &p)| brain::squad::MemberView {
            pos: p,
            alive: true,
            hp: 1.0,
            last_known: player,
            last_seen: if i == 0 { 0.0 } else { 0.5 },
            heard: None,
            look: (player - p).norm(),
            traits: if i == 1 { t } else { brain::genome::Traits::default() },
            bark: None,
        })
        .collect();
    let mut stacked = false;
    for _ in 0..40 {
        let orders = sq.tick(0.05, &views, &a, &model).to_vec();
        if let Some(b) = sq.breach {
            stacked |= orders.iter().flatten().any(|o| o.dest.is_some_and(|d| d.dist(b.door.pos) < 3.5) && o.watch == Some(b.door.pos));
        }
    }
    let b = sq.breach.expect("the trade goes through the door as a breach");
    assert!(stacked, "entry and trader stack beside the door");
    assert_ne!(b.door.side(pos[0]), b.door.side(player));
}

/// What a breach run saw: the breach planned, whether someone stacked on its door (an order
/// beside the frame, watching it), and whether someone was then sent through it.
struct BreachRun {
    breach: Option<brain::squad::Breach>,
    stacked: bool,
    went: bool,
    calls: Vec<String>,
}

/// Members stand at `pos` (static) with `weapons`; he is at `player`, last seen `last_seen` s
/// ago by all of them. `play`: forced, or None to let the coordinator choose. 12 s of ticks.
fn breach_run(id: MapId, play: Option<Play>, player: V2, pos: &[V2], weapons: &[WeaponKind], last_seen: f32) -> BreachRun {
    let a = Arena::build(id);
    let model = brain::player_model::PlayerModel::default();
    let mut sq = SquadCoordinator::new(&a, pos.len());
    sq.force_play = play;
    let player = a.nav.nearest_walkable(player);
    let pos: Vec<V2> = pos.iter().map(|&p| a.nav.nearest_walkable(p)).collect();
    let views: Vec<brain::squad::MemberView> = pos
        .iter()
        .enumerate()
        .map(|(i, &p)| brain::squad::MemberView {
            pos: p,
            alive: true,
            hp: 1.0,
            last_known: player,
            last_seen,
            heard: None,
            // Looking away from his room (a search must not tick it off from outside).
            look: (p - player).norm(),
            traits: brain::genome::Traits { weapon: weapons[i % weapons.len()], ..Default::default() },
            bark: None,
        })
        .collect();
    let mut run = BreachRun { breach: None, stacked: false, went: false, calls: Vec::new() };
    for _ in 0..240 {
        let orders = sq.tick(0.05, &views, &a, &model).to_vec();
        run.calls.extend(sq.drain_callouts().into_iter().map(|c| c.text));
        if let Some(b) = sq.breach {
            run.breach = Some(b);
            run.stacked |= orders.iter().flatten().any(|o| o.dest.is_some_and(|d| d.dist(b.door.pos) < 3.5) && o.watch == Some(b.door.pos));
            run.went |= orders.iter().flatten().any(|o| o.via == Some(b.inside));
        }
    }
    run
}

fn assert_breached(what: &str, r: &BreachRun, player: V2, from: V2) {
    let b = r.breach.unwrap_or_else(|| panic!("{what}: no breach planned (calls {:?})", r.calls));
    eprintln!("{what}: door {:?} calls {:?}", b.door.pos, r.calls);
    assert!(r.stacked, "{what}: nobody stacked on the door");
    assert!(r.went, "{what}: nobody went through the door");
    assert_ne!(b.door.side(from), b.door.side(player), "{what}: the door is between them and him");
    assert!(r.calls.iter().any(|c| c.contains("Stacked on")), "{what}: no stack call ({:?})", r.calls);
}

#[test]
fn other_plays_breach_his_room_too() {
    // Office: he holds a back office north of the z = -22 partition; the squad is on the floor.
    let office_player = V2::new(-21.0, -27.0);
    let office = [V2::new(-21.0, -12.0), V2::new(-23.0, -10.0), V2::new(-25.0, -8.0), V2::new(-19.0, -9.0)];
    let guns = [WeaponKind::Shotgun, WeaponKind::Smg, WeaponKind::Rifle, WeaponKind::Rifle];
    for play in [Play::Trade, Play::PinFlank, Play::Collapse, Play::Sweep] {
        let r = breach_run(MapId::Office, Some(play), office_player, &office, &guns, 0.0);
        assert_breached(&format!("Office {play:?}"), &r, office_player, office[0]);
    }
    // Lost him in that office: the search's investigators stack and breach it.
    let r = breach_run(MapId::Office, None, office_player, &office, &guns, 5.0);
    assert_breached("Office search", &r, office_player, office[0]);
}

#[test]
fn squads_breach_township_houses() {
    // The red house hall (x 9-21, z 24-34; doors north at x 12 and east at z 31); the squad is
    // out on the street between it and the corner shop.
    let player = V2::new(12.0, 29.0);
    let street = [V2::new(6.0, 22.0), V2::new(4.0, 21.5), V2::new(8.0, 21.5), V2::new(3.0, 22.5)];
    let guns = [WeaponKind::Shotgun, WeaponKind::Smg, WeaponKind::Rifle, WeaponKind::Marksman];
    let a = Arena::build(MapId::Township);
    let doors = brain::rooms::doorways(&a);
    assert!(doors.len() >= 20, "Township doors: {}", doors.len());
    for play in [Play::Trade, Play::PinFlank, Play::Collapse] {
        let r = breach_run(MapId::Township, Some(play), player, &street, &guns, 0.0);
        assert_breached(&format!("Township {play:?}"), &r, player, street[0]);
    }
    let r = breach_run(MapId::Township, None, player, &street, &guns, 5.0);
    assert_breached("Township search", &r, player, street[0]);
}

#[test]
fn four_gene_saves_load_and_keep_their_genes() {
    // Saves from before the Shrike and Osprey carry four weapon genes and five-gun telemetry.
    let mut rng = rng_from_seed(5);
    let g = TraitGenome::random(&mut rng);
    let mut v = serde_json::to_value(&g).unwrap();
    v["loadout"] = serde_json::json!([0.1, 0.9, 0.2, 0.3]);
    let mut old: TraitGenome = serde_json::from_value(v).unwrap();
    assert_eq!(&old.loadout[..4], &[0.1, 0.9, 0.2, 0.3]);
    assert!(old.loadout[4] < 0.0 && old.loadout[5] < 0.0, "new genes load unset");
    old.fill_loadout();
    assert_eq!(&old.loadout[..4], &[0.1, 0.9, 0.2, 0.3], "old genes kept");
    assert_eq!(&old.loadout[4..], &g.loadout[4..], "new genes seeded like a fresh genome");
    // Five-gun weapon shares from an old player model load padded.
    let b = BrainState::new(4);
    let mut j = serde_json::to_value(&b).unwrap();
    j["player_model"]["weapon_use"] = serde_json::json!([0.1, 0.2, 0.5, 0.1, 0.1]);
    let back: BrainState = serde_json::from_value(j).unwrap();
    assert_eq!(back.player_model.favourite_weapon(), WeaponKind::Rifle);
    assert_eq!(back.player_model.weapon_use[7], 0.0);
}

#[test]
fn osprey_wins_long_lanes() {
    let l = Limits::for_generation(0);
    let far = sim::duel(WeaponKind::Sniper, WeaponKind::Smg, 45.0, 5.0, &l, 200, 6);
    assert!(far > 0.7, "Osprey at 45 m wins {far}");
    let close = sim::duel(WeaponKind::Shotgun, WeaponKind::Sniper, 5.0, 5.0, &l, 200, 7);
    assert!(close > 0.6, "shotgun at 5 m beats the Osprey {close}");
    let mid = sim::duel(WeaponKind::BattleRifle, WeaponKind::Smg, 25.0, 5.0, &l, 200, 8);
    assert!(mid > 0.55, "Shrike at 25 m beats the SMG {mid}");
}
