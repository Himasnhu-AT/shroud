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
    let a = Arena::build(MapId::Foundry);
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
