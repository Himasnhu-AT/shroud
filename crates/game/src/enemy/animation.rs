//! Drives the enemy model's animations (Quaternius mannequin, see assets/ANIMATIONS.md).
//! Missing model or clips: nothing happens and enemies keep their pose / primitives.

use bevy::gltf::Gltf;
use bevy::prelude::*;
use std::time::Duration;

use super::{Enemy, EnemyCombat};
use crate::assets::{AssetsDir, ENEMY_MODEL};
use crate::state::has_visuals;

/// Clip per animation slot, in `Anim` order.
const CLIPS: [&str; 9] = [
    "Pistol_Idle_Loop",
    "Jog_Fwd_Loop",
    "Crouch_Idle_Loop",
    "Crouch_Fwd_Loop",
    "Pistol_Shoot",
    "Death01",
    "Pistol_Aim_Neutral",
    "Hit_Chest",
    "Hit_Head",
];
/// How long a hit reaction owns the body before blending back to locomotion.
const HIT_REACT_S: f32 = 0.4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Anim {
    Idle = 0,
    Run,
    CrouchIdle,
    CrouchMove,
    Shoot,
    Death,
    Aim,
    HitChest,
    HitHead,
}

#[derive(Resource)]
struct EnemyAnims {
    gltf: Handle<Gltf>,
    graph: Option<(Handle<AnimationGraph>, Vec<Option<AnimationNodeIndex>>)>,
}

/// On the enemy root: which entity holds its `AnimationPlayer`, and what's playing.
#[derive(Component)]
pub struct AnimLink {
    player: Entity,
    current: Option<Anim>,
    hits_seen: u32,
    hit_timer: f32,
}

pub struct EnemyAnimationPlugin;

impl Plugin for EnemyAnimationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load.run_if(has_visuals))
            .add_systems(Update, (build_graph, link, drive).chain().run_if(has_visuals).run_if(resource_exists::<EnemyAnims>));
    }
}

fn load(mut commands: Commands, dir: Res<AssetsDir>, server: Res<AssetServer>) {
    if dir.has(ENEMY_MODEL) {
        commands.insert_resource(EnemyAnims { gltf: server.load(ENEMY_MODEL), graph: None });
    }
}

fn build_graph(mut anims: ResMut<EnemyAnims>, gltfs: Res<Assets<Gltf>>, mut graphs: ResMut<Assets<AnimationGraph>>) {
    if anims.graph.is_some() {
        return;
    }
    let Some(gltf) = gltfs.get(&anims.gltf) else { return };
    let mut graph = AnimationGraph::new();
    let root = graph.root;
    let nodes = CLIPS
        .iter()
        .map(|name| gltf.named_animations.get(*name).map(|clip| graph.add_clip(clip.clone(), 1.0, root)))
        .collect::<Vec<_>>();
    if nodes.iter().any(Option::is_none) {
        warn!("enemy model is missing some animation clips: {:?}", CLIPS);
    }
    anims.graph = Some((graphs.add(graph), nodes));
}

/// When a model's `AnimationPlayer` appears, hook it to its enemy.
fn link(
    mut commands: Commands,
    anims: Res<EnemyAnims>,
    players: Query<Entity, Added<AnimationPlayer>>,
    parents: Query<&ChildOf>,
    enemies: Query<(), With<Enemy>>,
) {
    let Some((graph, _)) = &anims.graph else { return };
    for player in &players {
        let mut e = player;
        while let Ok(p) = parents.get(e) {
            e = p.parent();
            if enemies.contains(e) {
                commands.entity(player).insert((AnimationGraphHandle(graph.clone()), AnimationTransitions::new()));
                commands.entity(e).insert(AnimLink { player, current: None, hits_seen: 0, hit_timer: 0.0 });
                break;
            }
        }
    }
}

fn drive(
    time: Res<Time>,
    anims: Res<EnemyAnims>,
    mut enemies: Query<(&EnemyCombat, &mut AnimLink)>,
    mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
) {
    let Some((_, nodes)) = &anims.graph else { return };
    for (c, mut link) in &mut enemies {
        let moving = c.vel.len() > 0.5;
        link.hit_timer -= time.delta_secs();
        let new_hit = c.hits != link.hits_seen;
        link.hits_seen = c.hits;
        if new_hit {
            link.hit_timer = HIT_REACT_S;
        }
        let want = if c.dead {
            Anim::Death
        } else if link.hit_timer > 0.0 {
            if c.hit_head { Anim::HitHead } else { Anim::HitChest }
        } else if c.crouched {
            if moving { Anim::CrouchMove } else { Anim::CrouchIdle }
        } else if moving {
            Anim::Run
        } else if c.last_shot < 0.3 {
            Anim::Shoot
        } else if c.aim_at.is_some() {
            Anim::Aim
        } else {
            Anim::Idle
        };
        let restart = new_hit && !c.dead;
        if link.current == Some(want) && !restart {
            continue;
        }
        let Some(node) = nodes.get(want as usize).copied().flatten() else { continue };
        let Ok((mut player, mut transitions)) = players.get_mut(link.player) else { continue };
        let blend = match (want, link.current) {
            (Anim::HitChest | Anim::HitHead, _) => 60,
            (_, Some(Anim::HitChest | Anim::HitHead)) => 220,
            _ => 150,
        };
        let active = transitions.play(&mut player, node, Duration::from_millis(blend));
        if !matches!(want, Anim::Death | Anim::HitChest | Anim::HitHead) {
            active.repeat();
        }
        link.current = Some(want);
    }
}
