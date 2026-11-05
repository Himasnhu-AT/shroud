//! Enemy animation on the engine's code-first animator (`engine::anim`, docs/ANIM.md), over the
//! Quaternius mannequin's clips (assets/ANIMATIONS.md).
//!
//! Graph `enemy` (built once from the glTF's named clips, shared by every enemy):
//! - layer `base` (full body): `idle` (Pistol_Idle_Loop), `move` (1D blend over ground speed:
//!   Walk_Loop 1.5 m/s, Jog_Fwd_Loop 4, Sprint_Loop 6; played backwards when backpedalling),
//!   `crouch` / `crouch_move`, and an any-state `death` (Death01, once).
//! - layer `upper` (override, masked to the `spine_01` subtree, weight param `upper`): `aim`
//!   (1D blend over aim pitch: Pistol_Aim_Down / Neutral / Up), `shoot` (Pistol_Shoot, fired by
//!   trigger `fire`), `reload` (Pistol_Reload while `reloading`), hit reacts `hit_chest` /
//!   `hit_head` (triggers, any state).
//!
//! Procedural on top: the pelvis twists toward the movement direction (strafing legs without
//! strafe clips) and `LookAt` on spine_02 / spine_03 turns the chest back to the aim point (aim
//! offset). Deaths turn the body to fall away from the shot, with a per-enemy random yaw and
//! playback speed so corpses don't all drop the same. The engine's distance LOD applies.
//!
//! Missing model or clips: nothing is attached and enemies keep their pose / primitives.

use std::sync::Arc;

use bevy::gltf::Gltf;
use bevy::prelude::*;
use engine::anim::{AnimGraphDef, AnimLayer, AnimState, AnimSystems, AnimateScene, Animator, BoneMask, Cond, LookAt, Motion, ParamValue, Skeleton, Transition};

use super::{Enemy, EnemyCombat, ModelRoot};
use crate::assets::{AssetsDir, enemy_model, load_enemy};
use crate::state::has_visuals;

/// Clip names per role; the first one found wins (later ones are fallbacks).
const IDLE: &[&str] = &["Pistol_Idle_Loop", "Idle_Loop"];
const WALK: &[&str] = &["Walk_Loop", "Jog_Fwd_Loop"];
const JOG: &[&str] = &["Jog_Fwd_Loop", "Walk_Loop"];
const SPRINT: &[&str] = &["Sprint_Loop", "Jog_Fwd_Loop"];
const CROUCH: &[&str] = &["Crouch_Idle_Loop", "Pistol_Idle_Loop"];
const CROUCH_MOVE: &[&str] = &["Crouch_Fwd_Loop", "Walk_Loop"];
const AIM: &[&str] = &["Pistol_Aim_Neutral", "Pistol_Idle_Loop"];
const AIM_UP: &[&str] = &["Pistol_Aim_Up", "Pistol_Aim_Neutral"];
const AIM_DOWN: &[&str] = &["Pistol_Aim_Down", "Pistol_Aim_Neutral"];
const SHOOT: &[&str] = &["Pistol_Shoot", "Pistol_Aim_Neutral"];
const RELOAD: &[&str] = &["Pistol_Reload", "Pistol_Aim_Neutral"];
const HIT_CHEST: &[&str] = &["Hit_Chest"];
const HIT_HEAD: &[&str] = &["Hit_Head", "Hit_Chest"];
const DEATH: &[&str] = &["Death01"];

/// Natural ground speeds (m/s) of the locomotion clips (blend-space sample points).
const WALK_SPEED: f32 = 1.5;
const JOG_SPEED: f32 = 4.0;
const SPRINT_SPEED: f32 = 6.0;
const CROUCH_SPEED: f32 = 1.8;
/// Pelvis twist toward the movement direction is clamped to this (rad); beyond ~110 degrees
/// from the facing the legs backpedal instead.
const MAX_TWIST: f32 = 1.2;
const BACKPEDAL: f32 = 1.95;
/// Aim pitch mapped to the Up/Down blend: +-this (rad) is the full clip.
const PITCH_RANGE: f32 = 0.75;
/// Upper-body layer weight at low ready (unaware, carrying) and while engaged.
const UPPER_CARRY: f32 = 0.3;
const UPPER_AIM: f32 = 1.0;
/// Minimum seconds between restarts of the shoot clip (automatic fire reads as recoil).
const FIRE_ANIM_GAP: f32 = 0.12;
/// How long a hit react holds the upper body at full weight.
const HIT_REACT_S: f32 = 0.45;

/// Parameter order in the graph (ids resolved once).
#[derive(Clone, Copy)]
struct Params {
    speed: u16,
    dir: u16,
    crouch_rate: u16,
    crouch: u16,
    dead: u16,
    pitch: u16,
    upper: u16,
    fire: u16,
    reloading: u16,
    hit_chest: u16,
    hit_head: u16,
    death_speed: u16,
}

#[derive(Resource)]
struct EnemyAnims {
    gltf: Handle<Gltf>,
    def: Option<(Arc<AnimGraphDef>, Params)>,
    failed: bool,
}

/// On the enemy root once its model's animator exists.
#[derive(Component)]
pub struct AnimLink {
    animator: Entity,
    hits_seen: u32,
    hit_timer: f32,
    fire_gap: f32,
    upper: f32,
    /// Bones resolved from the skeleton (after binding).
    bones: Option<Bones>,
    /// Death: target yaw of the root (falls away from the shot), set once.
    death_yaw: Option<f32>,
    /// Frames since binding (bones resolve once a pose has propagated).
    wait: u8,
}

#[derive(Clone, Copy)]
struct Bones {
    pelvis: Entity,
    spine: [Entity; 2],
}

/// Pelvis twist (procedural): yaw about the model's up axis on top of the animated pose.
#[derive(Component, Default)]
struct PelvisTwist {
    yaw: f32,
    base: Quat,
    wrote: Quat,
    valid: bool,
}

/// The ModelRoot already got its `AnimateScene`.
#[derive(Component)]
struct AnimRequested;

pub struct EnemyAnimationPlugin;

impl Plugin for EnemyAnimationPlugin {
    fn build(&self, app: &mut App) {
        // Few characters on screen: keep full-rate animation out to mid range (the engine's
        // default tiers, made for crowds, halved the rate of enemies 15-35 m away).
        app.insert_resource(engine::anim::AnimConfig {
            lod: engine::anim::AnimLod { distances: [40.0, 80.0, 150.0], ..default() },
            ..default()
        })
        .add_systems(Startup, load.run_if(has_visuals))
            .add_systems(
                Update,
                (build_def, request, link, resolve_bones, drive).chain().run_if(has_visuals).run_if(resource_exists::<EnemyAnims>),
            )
            .add_systems(
                PostUpdate,
                twist_pelvis
                    .after(AnimSystems::RootMotion)
                    .before(AnimSystems::Procedural)
                    .run_if(has_visuals)
                    .run_if(resource_exists::<EnemyAnims>),
            );
    }
}

fn load(mut commands: Commands, dir: Res<AssetsDir>, server: Res<AssetServer>) {
    let model = enemy_model(&dir);
    if dir.has(model) {
        commands.insert_resource(EnemyAnims { gltf: load_enemy(&server, model, None), def: None, failed: false });
    }
}

fn build_def(mut anims: ResMut<EnemyAnims>, gltfs: Res<Assets<Gltf>>) {
    if anims.def.is_some() || anims.failed {
        return;
    }
    let Some(gltf) = gltfs.get(&anims.gltf) else { return };
    let clip = |names: &[&str]| names.iter().find_map(|n| gltf.named_animations.get(*n).cloned());
    let (Some(idle), Some(jog)) = (clip(IDLE), clip(JOG)) else {
        warn!("enemy model has no idle/jog clips: enemies stay unanimated");
        anims.failed = true;
        return;
    };
    let or = |names: &[&str], fb: &Handle<AnimationClip>| clip(names).unwrap_or_else(|| fb.clone());
    let walk = or(WALK, &jog);
    let sprint = or(SPRINT, &jog);
    let crouch = or(CROUCH, &idle);
    let crouch_move = or(CROUCH_MOVE, &walk);
    let aim = or(AIM, &idle);
    let aim_up = or(AIM_UP, &aim);
    let aim_down = or(AIM_DOWN, &aim);
    let shoot = or(SHOOT, &aim);
    let reload = or(RELOAD, &aim);
    let hit_chest = or(HIT_CHEST, &aim);
    let hit_head = or(HIT_HEAD, &hit_chest);
    let death = or(DEATH, &idle);
    if [HIT_CHEST, DEATH, SHOOT, RELOAD].iter().any(|n| clip(&n[..]).is_none()) {
        warn!("enemy model is missing some animation clips; using fallbacks");
    }

    let base = AnimLayer::new("base")
        .state(
            AnimState::new("idle", Motion::clip(idle))
                .on(Transition::to("crouch").when(Cond::is("crouch", true)).fade(0.2))
                .on(Transition::to("move").when(Cond::gt("speed", 0.4)).fade(0.2)),
        )
        .state(
            AnimState::new("move", Motion::blend1d("speed", [(WALK_SPEED, walk), (JOG_SPEED, jog), (SPRINT_SPEED, sprint)]))
                .speed_param("dir")
                .on(Transition::to("crouch_move").when(Cond::is("crouch", true)).fade(0.2))
                .on(Transition::to("idle").when(Cond::lt("speed", 0.3)).fade(0.25)),
        )
        .state(
            AnimState::new("crouch", Motion::clip(crouch))
                .on(Transition::to("idle").when(Cond::is("crouch", false)).fade(0.2))
                .on(Transition::to("crouch_move").when(Cond::gt("speed", 0.4)).fade(0.2)),
        )
        .state(
            AnimState::new("crouch_move", Motion::clip(crouch_move))
                .speed_param("crouch_rate")
                .on(Transition::to("move").when(Cond::is("crouch", false)).fade(0.2))
                .on(Transition::to("crouch").when(Cond::lt("speed", 0.3)).fade(0.25)),
        )
        .state(AnimState::new("death", Motion::clip(death)).once().speed_param("death_speed"))
        .any(Transition::to("death").when(Cond::is("dead", true)).fade(0.12));

    let upper = AnimLayer::new("upper")
        .mask(BoneMask::subtree("spine_01"))
        .weight_param("upper")
        .state(AnimState::new("aim", Motion::blend1d("pitch", [(-1.0, aim_down), (0.0, aim), (1.0, aim_up)])))
        .state(AnimState::new("shoot", Motion::clip(shoot)).once().on(Transition::to("aim").exit_time(0.8).fade(0.12)))
        .state(AnimState::new("reload", Motion::clip(reload)).on(Transition::to("aim").when(Cond::is("reloading", false)).fade(0.2)))
        .state(AnimState::new("hit_chest", Motion::clip(hit_chest)).once().on(Transition::to("aim").exit_time(0.85).fade(0.15)))
        .state(AnimState::new("hit_head", Motion::clip(hit_head)).once().on(Transition::to("aim").exit_time(0.85).fade(0.15)))
        .any(Transition::to("hit_head").when(Cond::trigger("hit_head")).fade(0.05).can_self())
        .any(Transition::to("hit_chest").when(Cond::trigger("hit_chest")).fade(0.05).can_self())
        .any(Transition::to("reload").when(Cond::is("reloading", true)).fade(0.15))
        .any(Transition::to("shoot").when(Cond::trigger("fire")).fade(0.04).can_self());

    let def = AnimGraphDef::builder("enemy")
        .float("speed", 0.0)
        .float("dir", 1.0)
        .float("crouch_rate", 1.0)
        .bool("crouch", false)
        .bool("dead", false)
        .float("pitch", 0.0)
        .float("upper", UPPER_CARRY)
        .trigger("fire")
        .bool("reloading", false)
        .trigger("hit_chest")
        .trigger("hit_head")
        .float("death_speed", 1.0)
        .layer(base)
        .layer(upper)
        .build();
    let def = match def {
        Ok(d) => d,
        Err(e) => {
            warn!("enemy anim graph: {e}");
            anims.failed = true;
            return;
        }
    };
    let id = |n: &str| def.param_id(n).unwrap_or(0);
    let params = Params {
        speed: id("speed"),
        dir: id("dir"),
        crouch_rate: id("crouch_rate"),
        crouch: id("crouch"),
        dead: id("dead"),
        pitch: id("pitch"),
        upper: id("upper"),
        fire: id("fire"),
        reloading: id("reloading"),
        hit_chest: id("hit_chest"),
        hit_head: id("hit_head"),
        death_speed: id("death_speed"),
    };
    anims.def = Some((def, params));
}

/// Give each enemy model the animator (once the graph exists).
fn request(mut commands: Commands, anims: Res<EnemyAnims>, models: Query<Entity, (With<ModelRoot>, Without<AnimRequested>)>) {
    let Some((def, _)) = &anims.def else { return };
    for m in &models {
        commands.entity(m).insert((AnimateScene(def.clone()), AnimRequested));
    }
}

/// When a model's animator appears, hook it to its enemy.
fn link(mut commands: Commands, added: Query<Entity, Added<Animator>>, parents: Query<&ChildOf>, enemies: Query<(), With<Enemy>>) {
    for animator in &added {
        let mut e = animator;
        while let Ok(p) = parents.get(e) {
            e = p.parent();
            if enemies.contains(e) {
                commands.entity(e).insert(AnimLink {
                    animator,
                    hits_seen: 0,
                    hit_timer: 0.0,
                    fire_gap: 0.0,
                    upper: UPPER_CARRY,
                    bones: None,
                    death_yaw: None,
                    wait: 0,
                });
                break;
            }
        }
    }
}

/// After binding: find the pelvis and spine bones; put the aim look-at on the spine.
fn resolve_bones(
    mut commands: Commands,
    mut links: Query<(&mut AnimLink, &Transform), With<Enemy>>,
    skeletons: Query<&Skeleton>,
    animators: Query<&Animator>,
    globals: Query<&GlobalTransform>,
) {
    for (mut link, root) in &mut links {
        if link.bones.is_some() || !animators.get(link.animator).is_ok_and(Animator::is_bound) {
            continue;
        }
        // Two frames after binding the bones' global transforms hold a real pose.
        link.wait = link.wait.saturating_add(1);
        if link.wait < 3 {
            continue;
        }

        let Ok(skel) = skeletons.get(link.animator) else { continue };
        let (Some(pelvis), Some(s2), Some(s3)) = (skel.entity("pelvis"), skel.entity("spine_02"), skel.entity("spine_03")) else {
            // Not this rig: animate without the procedural layer.
            link.bones = Some(Bones { pelvis: Entity::PLACEHOLDER, spine: [Entity::PLACEHOLDER; 2] });
            continue;
        };
        // The chest's forward in bone space: the character's forward through the bone's
        // current world rotation (bind or first pose; the model faces the root's forward).
        let fwd = root.forward().as_vec3();
        for (bone, w) in [(s2, 0.45), (s3, 0.6)] {
            let Ok(g) = globals.get(bone) else { continue };
            let axis = g.rotation().inverse() * fwd;
            commands.entity(bone).insert(LookAt::new(Vec3::ZERO).axis(axis).weight(w).max_angle(0.9));
        }
        commands.entity(pelvis).insert(PelvisTwist::default());
        link.bones = Some(Bones { pelvis, spine: [s2, s3] });
    }
}

/// Set a float parameter only when it changed (no change ticks when steady).
fn set_f(a: &mut Mut<Animator>, id: u16, v: f32) {
    if a.param_values().get(id as usize).is_some_and(|p| (p.as_f32() - v).abs() > 1e-3) {
        a.set_id(id, ParamValue::Float(v));
    }
}

fn set_b(a: &mut Mut<Animator>, id: u16, v: bool) {
    if a.param_values().get(id as usize).is_some_and(|p| p.as_bool() != v) {
        a.set_id(id, ParamValue::Bool(v));
    }
}

fn drive(
    time: Res<Time>,
    anims: Res<EnemyAnims>,
    mut enemies: Query<(Entity, &EnemyCombat, &mut AnimLink, &mut Transform)>,
    mut animators: Query<&mut Animator>,
    mut looks: Query<&mut LookAt>,
    mut twists: Query<&mut PelvisTwist>,
) {
    let Some((_, p)) = &anims.def else { return };
    let dt = time.delta_secs();
    for (e, c, mut link, mut root) in &mut enemies {
        let Ok(mut a) = animators.get_mut(link.animator) else { continue };
        let fwd = root.forward().as_vec3();
        let fwd2 = Vec2::new(fwd.x, fwd.z).normalize_or(Vec2::NEG_Y);
        let vel = Vec2::new(c.vel.x, c.vel.y);
        let speed = vel.length();

        if c.dead {
            set_b(&mut a, p.dead, true);
            set_f(&mut a, p.upper, 0.0);
            // Fall away from the shot: face where the shooter was (Death01 falls backwards),
            // plus a per-enemy random twist and speed.
            let seed = e.index().index().wrapping_mul(2654435761);
            let jitter = ((seed >> 8) % 1000) as f32 / 1000.0 - 0.5;
            if link.death_yaw.is_none() {
                let toward = c.aim_at.map(|t| (t - root.translation).xz()).filter(|d| d.length() > 0.1).unwrap_or(fwd2);
                let yaw = (-toward.x).atan2(-toward.y) + jitter * 0.8;
                link.death_yaw = Some(yaw);
                set_f(&mut a, p.death_speed, 0.9 + 0.3 * (jitter + 0.5));
            }
            if let Some(yaw) = link.death_yaw {
                let want = Quat::from_rotation_y(yaw);
                if root.rotation.angle_between(want) > 0.01 {
                    root.rotation = root.rotation.slerp(want, 1.0 - (-10.0 * dt).exp());
                }
            }
            if let Some(b) = link.bones {
                for s in b.spine {
                    if let Ok(mut la) = looks.get_mut(s)
                        && la.weight != 0.0
                    {
                        la.weight = 0.0;
                    }
                }
                if let Ok(mut tw) = twists.get_mut(b.pelvis)
                    && tw.yaw != 0.0
                {
                    tw.yaw = 0.0;
                }
            }
            continue;
        }

        // --- Locomotion: speed, backpedal, pelvis twist toward the move direction.
        let (mut twist, mut dir) = (0.0, 1.0);
        if speed > 0.4 {
            let v = vel / speed;
            let ang = fwd2.perp_dot(v).atan2(fwd2.dot(v));
            // (A +angle in (x, z) is a -yaw about world +Y.)
            if ang.abs() > BACKPEDAL {
                dir = -1.0;
                let back = ang - std::f32::consts::PI * ang.signum();
                twist = -back.clamp(-MAX_TWIST, MAX_TWIST);
            } else {
                twist = -ang.clamp(-MAX_TWIST, MAX_TWIST);
            }
        }
        set_f(&mut a, p.speed, (speed * 4.0).round() / 4.0);
        set_f(&mut a, p.dir, dir);
        set_f(&mut a, p.crouch_rate, dir * (speed / CROUCH_SPEED).clamp(0.6, 1.6));
        set_b(&mut a, p.crouch, c.crouched);

        // --- Upper body: aim pitch, fire, reload, hit reacts.
        let pitch = c.aim_at.map_or(0.0, |t| {
            let eye = root.translation + Vec3::Y * if c.crouched { 1.0 } else { 1.45 };
            let d = t - eye;
            d.y.atan2(d.xz().length().max(0.1))
        });
        set_f(&mut a, p.pitch, ((pitch / PITCH_RANGE).clamp(-1.0, 1.0) * 10.0).round() / 10.0);
        let reloading = c.gun.reloading() || c.gun.switching();
        set_b(&mut a, p.reloading, reloading);
        link.fire_gap -= dt;
        if c.last_shot < dt + 1e-4 && link.fire_gap <= 0.0 {
            a.set_id(p.fire, ParamValue::Trigger(true));
            link.fire_gap = FIRE_ANIM_GAP;
        }
        link.hit_timer -= dt;
        if c.hits != link.hits_seen {
            link.hits_seen = c.hits;
            link.hit_timer = HIT_REACT_S;
            a.set_id(if c.hit_head { p.hit_head } else { p.hit_chest }, ParamValue::Trigger(true));
        }
        let want = if link.hit_timer > 0.0 || reloading || c.aim_at.is_some() { UPPER_AIM } else { UPPER_CARRY };
        let k = 1.0 - (-8.0 * dt).exp();
        link.upper += (want - link.upper) * k;
        set_f(&mut a, p.upper, (link.upper * 20.0).round() / 20.0);

        // --- Procedural: twist the hips, the chest looks back at the aim point (yaw only;
        // the Up/Down clips do pitch).
        let Some(b) = link.bones else { continue };
        if let Ok(mut tw) = twists.get_mut(b.pelvis) {
            let y = tw.yaw + (twist - tw.yaw) * (1.0 - (-10.0 * dt).exp());
            if (y - tw.yaw).abs() > 1e-3 {
                tw.yaw = y;
            }
        }
        let chest = root.translation + Vec3::Y * 1.3;
        let target = c.aim_at.map_or(chest + fwd * 10.0, |t| Vec3::new(t.x, chest.y, t.z));
        // Without an aim target the chest only undoes the hip twist (looks where it faces).
        let weight = |w: f32| if c.aim_at.is_some() || twist.abs() > 0.05 { w } else { 0.0 };
        for (s, w) in b.spine.into_iter().zip([0.45, 0.6]) {
            if let Ok(mut la) = looks.get_mut(s) {
                let w = weight(w);
                if la.weight != w {
                    la.weight = w;
                }
                if la.target.distance_squared(target) > 1e-4 {
                    la.target = target;
                }
            }
        }
    }
}

/// Twist the pelvis about the model's up axis on top of this frame's pose. Starts from the
/// animated rotation every frame (or the last base when LOD skipped the pose), so it never
/// accumulates.
fn twist_pelvis(mut q: Query<(&mut PelvisTwist, &mut Transform, &ChildOf)>, globals: Query<&GlobalTransform>) {
    for (mut tw, mut tf, parent) in &mut q {
        if !(tw.valid && tf.rotation == tw.wrote) {
            tw.base = tf.rotation;
        }
        if tw.yaw.abs() < 1e-3 {
            if tw.valid && tf.rotation == tw.wrote {
                tf.rotation = tw.base;
            }
            tw.valid = false;
            continue;
        }
        let up = globals.get(parent.parent()).map_or(Vec3::Y, |g| (g.rotation().inverse() * Vec3::Y).normalize_or(Vec3::Y));
        let r = Quat::from_axis_angle(up, tw.yaw) * tw.base;
        tf.rotation = r;
        tw.wrote = r;
        tw.valid = true;
    }
}
