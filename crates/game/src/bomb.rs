//! Bomb defusal, the game side of [`brain::objective`] (its doc comment is the rule set).
//!
//! The player always starts a round carrying the bomb. Holding E inside a site (standing, round
//! live; inside a site E plants, gun pickups there don't take the focus) plants it in [`PLANT_S`]; the gun lowers
//! and the player can't move or fire meanwhile, and letting go resets. The planted bomb sits at
//! the player's feet: a primitive charge with a keypad and a red LED that blinks with each beep,
//! beeping faster as the fuse runs. After [`FUSE_S`] it explodes (fireball, flash, smoke, camera
//! shake, [`blast_damage`] by distance, halved behind cover past the lethal radius).
//!
//! Defusing: every frame the squad's coordinator gets the bomb (`SquadCoordinator::set_bomb`);
//! while its `defuser` is alive and within [`DEFUSE_REACH`] the defuse ticks, and it resets when
//! he stops, dies or steps off. Round results are `match_flow::round_outcome`.
//!
//! Each site gets a thin painted outline and its letter on the floor ([`ArenaPiece`], so they go
//! with the map). Control port (dev): `bomb.state`, `bomb.plant`, `bomb.defuse`.

use bevy::asset::RenderAssetUsages;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use brain::math::V2;
use brain::objective::{self, BombSite, DEFUSE_REACH, DEFUSE_S, FUSE_S, PLANT_S, PlantedBomb, blast_damage, site_at};
use engine::fps::{FpsController, FpsInput, FpsMotor};
use engine::render_budget::Actor;

use crate::dev::{Dev, Prop};
use crate::economy::{PlayerWallet, RoundPhase};
use crate::enemy::{Enemy, EnemyCombat, Squad};
use crate::map::{ArenaPiece, ArenaRes, to_ground};
use crate::match_flow::MatchFlow;
use crate::player::{Player, PlayerIntent, PlayerSet};
use crate::state::{GameState, has_visuals, not_paused};
use crate::telemetry::Telemetry;
use crate::weapon::arsenal::WeaponInput;
use crate::weapon::{Damage, DamageCause, Health, WeaponSet};

/// Where the bomb is in its life this round.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BombPhase {
    /// On the player (planting or not).
    #[default]
    Carried,
    Planted,
    Exploded,
    Defused,
}

#[cfg_attr(not(feature = "dev"), allow(dead_code))]
impl BombPhase {
    pub fn label(self) -> &'static str {
        match self {
            BombPhase::Carried => "carried",
            BombPhase::Planted => "planted",
            BombPhase::Exploded => "exploded",
            BombPhase::Defused => "defused",
        }
    }
}

/// The round's bomb. Reset on every round start.
#[derive(Resource, Clone, Debug)]
pub struct Bomb {
    pub phase: BombPhase,
    /// 0..1 plant progress (E held in a site).
    pub plant: f32,
    /// The site (index into `Arena::sites`) the player stands in while carrying the bomb.
    pub in_site: Option<usize>,
    /// Holding E would plant right now.
    pub can_plant: bool,
    /// Planted: the bomb's position (on the floor) and site.
    pub pos: Vec3,
    pub site: usize,
    /// Seconds until it goes off.
    pub fuse_left: f32,
    /// 0..1 defuse progress, and the squad slot defusing.
    pub defuse: f32,
    pub defuser: Option<usize>,
    /// Dev: force this squad slot as the defuser (until the AI lands its own choice).
    pub forced_defuser: Option<usize>,
    /// The planted bomb entity (also the blast's damage source).
    pub entity: Option<Entity>,
    /// 0..1: how far the gun is lowered (planting, or dead).
    pub lower: f32,
    /// Seconds to the next beep, and how long the LED stays lit.
    next_beep: f32,
    pub led: f32,
    /// Dev: plant instantly on the next frame, with this fuse (s).
    pub(crate) request: Option<f32>,
    /// Dev: move this enemy here on the next fixed tick (before the AI moves it).
    pub(crate) teleport: Option<(Entity, Vec3)>,
}

impl Default for Bomb {
    fn default() -> Self {
        Bomb {
            phase: BombPhase::Carried,
            plant: 0.0,
            in_site: None,
            can_plant: false,
            pos: Vec3::ZERO,
            site: 0,
            fuse_left: FUSE_S,
            defuse: 0.0,
            defuser: None,
            forced_defuser: None,
            entity: None,
            lower: 0.0,
            next_beep: 0.0,
            led: 0.0,
            request: None,
            teleport: None,
        }
    }
}

impl Bomb {
    /// Planted, defused or exploded this round.
    pub fn was_planted(&self) -> bool {
        self.phase != BombPhase::Carried
    }
    /// What the AI sees.
    pub fn planted(&self) -> Option<PlantedBomb> {
        (self.phase == BombPhase::Planted).then(|| PlantedBomb { pos: to_ground(self.pos), site: self.site, fuse_left: self.fuse_left, defuse_progress: self.defuse })
    }
}

/// Bomb moments, for sound and the HUD.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub enum BombEvent {
    PlantStart,
    Planted { at: Vec3, site: char },
    /// One beep; `urgency` 0..1 as the fuse runs out.
    Beep { at: Vec3, urgency: f32 },
    DefuseStart { at: Vec3 },
    Defused { at: Vec3 },
    Exploded { at: Vec3 },
}

/// Seconds between beeps with `left` seconds of fuse: ~1 s at the plant down to 0.12 s at the end.
pub fn beep_interval(left: f32) -> f32 {
    let k = (left / FUSE_S).clamp(0.0, 1.0);
    0.12 + 0.88 * k.powf(1.4)
}

/// Defuse progress after `dt` seconds: grows while a defender is on it, else resets.
pub fn defuse_step(progress: f32, on_it: bool, dt: f32) -> f32 {
    if on_it { (progress + dt / DEFUSE_S).min(1.0) } else { 0.0 }
}

/// Blast damage at `d` metres when the line to the target is `clear`: lethal inside the kill
/// radius whatever is in the way, halved behind cover past it.
pub fn blast_at(d: f32, clear: bool) -> f32 {
    let dmg = blast_damage(d);
    if clear || d <= objective::BLAST_KILL_M { dmg } else { dmg * 0.5 }
}

pub struct BombPlugin;

impl Plugin for BombPlugin {
    fn build(&self, app: &mut App) {
        let plant = plant_input.after(PlayerSet::Input).before(PlayerSet::Move);
        // After the control port's scripted input, so a scripted move is stopped too.
        #[cfg(feature = "dev")]
        let plant = plant.after(crate::dev::ControlDrive);
        app.init_resource::<Bomb>()
            .add_message::<BombEvent>()
            .add_systems(Startup, init_assets.run_if(has_visuals))
            .add_systems(FixedUpdate, apply_teleport.before(crate::enemy::EnemySet))
            .add_systems(OnEnter(GameState::Playing), reset_bomb.after(crate::match_flow::begin_round).after(crate::enemy::spawn_squad))
            .add_systems(
                Update,
                (
                    plant.run_if(in_state(GameState::Playing)),
                    tick_bomb
                        .after(WeaponSet)
                        .before(crate::match_flow::check_round_end)
                        .run_if(in_state(GameState::Playing).and_then(not_paused)),
                    (blink_led, tick_smoke, site_markers).run_if(has_visuals),
                ),
            );
        #[cfg(feature = "dev")]
        commands::register(app);
    }
}

/// Dev teleports of an enemy (`bomb.defuse`): done on the fixed tick so the AI and the
/// transform easing see the new spot as where it stands.
fn apply_teleport(mut bomb: ResMut<Bomb>, mut q: Query<&mut Transform, With<Enemy>>) {
    let Some((e, at)) = bomb.teleport else { return };
    bomb.teleport = None;
    if let Ok(mut tf) = q.get_mut(e) {
        tf.translation.x = at.x;
        tf.translation.z = at.z;
    }
}

fn reset_bomb(mut bomb: ResMut<Bomb>, mut squad: ResMut<Squad>) {
    *bomb = Bomb::default();
    if let Some(c) = squad.coord.as_mut() {
        c.set_bomb(None);
    }
}

/// Planting (E in a site), and holding a dead player still. Runs between the input and the move.
fn plant_input(
    mut commands: Commands,
    time: Res<Time>,
    arena: Res<ArenaRes>,
    flow: Res<MatchFlow>,
    phase: Res<RoundPhase>,
    mut input: ResMut<WeaponInput>,
    mut bomb: ResMut<Bomb>,
    mut intent: ResMut<PlayerIntent>,
    mut players: Query<(&Transform, &FpsMotor, &FpsController, &Health, &mut FpsInput), With<Player>>,
    mut plant_out: PlantOut,
) {
    let dt = time.delta_secs();
    let Ok((tf, motor, ctl, hp, mut fin)) = players.single_mut() else { return };
    let ground = to_ground(tf.translation);
    let dead = hp.is_dead();
    bomb.in_site = if bomb.phase == BombPhase::Carried && !dead { site_at(&arena.0.sites, ground) } else { None };
    bomb.can_plant = bomb.in_site.is_some() && !phase.frozen() && flow.round_over.is_none() && motor.grounded;
    let holding = bomb.can_plant && input.interact;
    if let Some(fuse) = bomb.request.take()
        && bomb.phase == BombPhase::Carried
        && let Some(site) = site_at(&arena.0.sites, ground)
    {
        let feet = feet_of(tf, motor, ctl);
        plant_out.plant(&mut commands, &mut bomb, &arena.0.sites, feet, site, fuse);
    } else if holding {
        if bomb.plant == 0.0 {
            plant_out.events.write(BombEvent::PlantStart);
        }
        bomb.plant += dt / PLANT_S;
        if bomb.plant >= 1.0 {
            let feet = feet_of(tf, motor, ctl);
            let site = bomb.in_site.unwrap_or(0);
            plant_out.plant(&mut commands, &mut bomb, &arena.0.sites, feet, site, FUSE_S);
        }
    } else if bomb.plant != 0.0 {
        bomb.plant = 0.0;
    }
    // Planting (or dead): no moving, jumping or firing; the gun goes down. Dead also crouches
    // (the camera sinks) and stays there until the round ends.
    let still = holding || dead;
    if still {
        if intent.fire {
            intent.fire = false;
        }
        if fin.move_axis != Vec2::ZERO || fin.jump || fin.sprint {
            fin.move_axis = Vec2::ZERO;
            fin.jump = false;
            fin.sprint = false;
        }
        if dead && !fin.crouch {
            fin.crouch = true;
        }
    }
    // Dead (after a plant the round goes on without him): no reloading, switching or aiming
    // either. Grenades are held in `weapon::throwing`.
    if dead {
        if intent.reload {
            intent.reload = false;
        }
        if input.select.is_some() || input.cycle != 0 || input.last || input.inspect || input.ads {
            input.select = None;
            input.cycle = 0;
            input.last = false;
            input.inspect = false;
            input.ads = false;
        }
    }
    let target = if still { 1.0 } else { 0.0 };
    let lower = bomb.lower + (target - bomb.lower) * (1.0 - (-12.0 * dt).exp());
    if (lower - bomb.lower).abs() > 1e-4 {
        bomb.lower = lower;
    }
}

/// The floor under the player's capsule.
fn feet_of(tf: &Transform, motor: &FpsMotor, ctl: &FpsController) -> Vec3 {
    let h = if motor.crouched { ctl.crouch_height } else { ctl.height };
    tf.translation - Vec3::Y * (h * 0.5)
}

#[derive(bevy::ecs::system::SystemParam)]
struct PlantOut<'w> {
    wallet: ResMut<'w, PlayerWallet>,
    telemetry: ResMut<'w, Telemetry>,
    events: MessageWriter<'w, BombEvent>,
    assets: Option<Res<'w, BombAssets>>,
    dev: Dev<'w>,
}

impl PlantOut<'_> {
    fn plant(&mut self, commands: &mut Commands, bomb: &mut Bomb, sites: &[BombSite], feet: Vec3, site: usize, fuse: f32) {
        let pos = Vec3::new(feet.x, feet.y.max(0.0), feet.z);
        let letter = sites.get(site).map_or('?', |s| s.letter);
        let e = spawn_bomb(commands, self.assets.as_deref(), pos);
        *bomb = Bomb {
            phase: BombPhase::Planted,
            pos,
            site,
            fuse_left: fuse.max(0.1),
            entity: Some(e),
            lower: bomb.lower,
            next_beep: 0.0,
            ..Bomb::default()
        };
        self.wallet.0.earn(brain::economy::PLANT_BONUS);
        // The squad learns which site he favours from this when the match is over
        // (`PlayerModel::ingest`), never mid-match: abandoned matches teach it nothing.
        self.telemetry.0.plant(site);
        self.events.write(BombEvent::Planted { at: pos, site: letter });
        self.dev.trace("bomb.planted", &[("site", Prop::Int(site as i64))]);
        info!("bomb planted on {letter} at ({:.1}, {:.1})", pos.x, pos.z);
    }
}

/// Fuse, beeps, the defuse and the explosion.
fn tick_bomb(
    mut commands: Commands,
    time: Res<Time>,
    arena: Res<ArenaRes>,
    flow: Res<MatchFlow>,
    mut bomb: ResMut<Bomb>,
    mut squad: ResMut<Squad>,
    players: Query<(Entity, &Transform, &FpsMotor), With<Player>>,
    enemies: Query<(Entity, &Enemy, &Transform, &EnemyCombat)>,
    mut events: MessageWriter<BombEvent>,
    mut damage: MessageWriter<Damage>,
    mut fx: BlastFx,
    dev: Dev,
) {
    let dt = time.delta_secs();
    bomb.led = (bomb.led - dt).max(0.0);
    if bomb.phase != BombPhase::Planted || flow.round_over.is_some() {
        // Every frame, even with no bomb: it also tells the squad the round is defusal.
        if let Some(c) = squad.coord.as_mut() {
            c.set_bomb(None);
        }
        // Nobody is defusing a bomb that isn't planted (or once the round is over).
        if bomb.defuser.is_some() {
            bomb.defuser = None;
        }
        return;
    }
    let at = bomb.pos;
    // Beeps.
    bomb.fuse_left -= dt;
    bomb.next_beep -= dt;
    if bomb.next_beep <= 0.0 && bomb.fuse_left > 0.0 {
        bomb.next_beep += beep_interval(bomb.fuse_left).max(0.05);
        bomb.next_beep = bomb.next_beep.max(0.05);
        bomb.led = 0.09;
        events.write(BombEvent::Beep { at, urgency: 1.0 - (bomb.fuse_left / FUSE_S).clamp(0.0, 1.0) });
    }
    // Defuse: the coordinator's defuser (or the dev's), alive and on the bomb.
    // The dev's forced defuser goes on even while the squad would save.
    let forced = bomb.forced_defuser;
    let wants = forced.or_else(|| squad.coord.as_ref().and_then(|c| c.defuser.filter(|&i| c.wants_defuse(i))));
    let on_it = wants.filter(|&slot| {
        enemies.iter().any(|(_, e, tf, c)| e.slot == slot && !c.dead && to_ground(tf.translation).dist(to_ground(at)) <= DEFUSE_REACH)
    });
    if on_it.is_some() && on_it != bomb.defuser {
        bomb.defuse = 0.0;
        events.write(BombEvent::DefuseStart { at });
    }
    bomb.defuser = on_it;
    bomb.defuse = defuse_step(bomb.defuse, on_it.is_some(), dt);
    if bomb.defuse >= 1.0 {
        bomb.phase = BombPhase::Defused;
        bomb.defuser = None;
        events.write(BombEvent::Defused { at });
        dev.trace("bomb.defused", &[("slot", Prop::Int(on_it.unwrap_or(0) as i64))]);
        info!("bomb defused by squad slot {:?}", on_it);
    } else if bomb.fuse_left <= 0.0 {
        bomb.fuse_left = 0.0;
        bomb.phase = BombPhase::Exploded;
        bomb.defuser = None;
        events.write(BombEvent::Exploded { at });
        dev.trace("bomb.exploded", &[("site", Prop::Int(bomb.site as i64))]);
        info!("bomb exploded");
        let source = bomb.entity.unwrap_or(Entity::PLACEHOLDER);
        // Damage: the player and every live enemy, by distance, cover past the lethal radius.
        let core = to_ground(at);
        let clear = |p: V2, h: f32| arena.0.clear_line_3d(core, 0.6, p, h);
        if let Ok((pe, ptf, motor)) = players.single() {
            let p = to_ground(ptf.translation);
            let d = core.dist(p);
            let head = ptf.translation.y + if motor.crouched { 0.25 } else { 0.55 };
            let amount = blast_at(d, clear(p, ptf.translation.y) || clear(p, head));
            if amount > 0.5 {
                damage.write(Damage { target: pe, amount, source, headshot: false, cause: DamageCause::Bomb });
            }
            fx.shake_and_flash(d, clear(p, head));
        }
        for (ee, _, etf, c) in &enemies {
            if c.dead {
                continue;
            }
            let p = to_ground(etf.translation);
            let amount = blast_at(core.dist(p), clear(p, 0.95) || clear(p, if c.crouched { 1.15 } else { 1.6 }));
            if amount > 0.5 {
                damage.write(Damage { target: ee, amount, source, headshot: false, cause: DamageCause::Bomb });
            }
        }
        fx.explode(&mut commands, at);
        if let Some(e) = bomb.entity.take() {
            commands.entity(e).despawn();
        }
    }
    let planted = bomb.planted();
    if let Some(c) = squad.coord.as_mut() {
        c.set_bomb(planted);
        if forced.is_some() && planted.is_some() {
            c.defuser = forced;
        }
    }
}

// ---------------------------------------------------------------- visuals

/// Shared meshes and materials for the bomb, its blast smoke and the site paint.
#[derive(Resource)]
pub struct BombAssets {
    cube: Handle<Mesh>,
    sphere: Handle<Mesh>,
    cylinder: Handle<Mesh>,
    body: Handle<StandardMaterial>,
    stick: Handle<StandardMaterial>,
    keypad: Handle<StandardMaterial>,
    screen: Handle<StandardMaterial>,
    led_off: Handle<StandardMaterial>,
    led_on: Handle<StandardMaterial>,
    tape: Handle<StandardMaterial>,
    paint: Handle<StandardMaterial>,
}

fn init_assets(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    let mut m = |c: Color, rough: f32| mats.add(StandardMaterial { base_color: c, perceptual_roughness: rough, ..default() });
    let body = m(Color::srgb(0.16, 0.17, 0.13), 0.75);
    let stick = m(Color::srgb(0.62, 0.52, 0.36), 0.85);
    let keypad = m(Color::srgb(0.08, 0.085, 0.09), 0.5);
    let tape = m(Color::srgb(0.12, 0.12, 0.13), 0.6);
    let led_off = m(Color::srgb(0.25, 0.02, 0.02), 0.3);
    let screen = mats.add(StandardMaterial { base_color: Color::srgb(0.1, 0.2, 0.1), emissive: LinearRgba::rgb(0.25, 0.9, 0.3), ..default() });
    let led_on = mats.add(StandardMaterial { base_color: Color::srgb(1.0, 0.1, 0.05), emissive: LinearRgba::rgb(30.0, 1.0, 0.5), unlit: true, ..default() });
    // Worn yellow floor paint; unlit-ish so it reads in dark rooms but doesn't glow.
    let paint = mats.add(StandardMaterial {
        base_color: Color::srgba(0.95, 0.72, 0.12, 0.55),
        perceptual_roughness: 0.95,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    commands.insert_resource(BombAssets {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        sphere: meshes.add(Sphere::new(1.0)),
        cylinder: meshes.add(Cylinder::new(1.0, 1.0)),
        body,
        stick,
        keypad,
        screen,
        led_off,
        led_on,
        tape,
        paint,
    });
}

#[derive(Component)]
struct PlantedBombModel;
/// The lit LED (shown on each beep) and its light.
#[derive(Component)]
struct LedGlow;

/// The planted bomb: an entity at `pos` (on the floor); with `assets`, its model.
fn spawn_bomb(commands: &mut Commands, assets: Option<&BombAssets>, pos: Vec3) -> Entity {
    let mut e = commands.spawn((
        Name::new("Planted bomb"),
        PlantedBombModel,
        Actor,
        Transform::from_translation(pos).with_rotation(Quat::from_rotation_y(0.35)),
        Visibility::default(),
        DespawnOnExit(GameState::Playing),
    ));
    let Some(a) = assets else { return e.id() };
    e.with_children(|b| {
        let part = |mesh: &Handle<Mesh>, mat: &Handle<StandardMaterial>, t: Vec3, s: Vec3| {
            (Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone()), Transform::from_translation(t).with_scale(s), NotShadowCaster)
        };
        // Base plate, four charge sticks, two tape bands, the keypad box with a screen and keys.
        b.spawn(part(&a.cube, &a.body, Vec3::new(0.0, 0.025, 0.0), Vec3::new(0.34, 0.05, 0.24)));
        for i in 0..4 {
            let z = -0.075 + i as f32 * 0.05;
            b.spawn((
                Mesh3d(a.cylinder.clone()),
                MeshMaterial3d(a.stick.clone()),
                Transform::from_xyz(0.0, 0.075, z).with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)).with_scale(Vec3::new(0.024, 0.3, 0.024)),
                NotShadowCaster,
            ));
        }
        for x in [-0.09, 0.09] {
            b.spawn(part(&a.cube, &a.tape, Vec3::new(x, 0.076, 0.0), Vec3::new(0.035, 0.056, 0.215)));
        }
        b.spawn(part(&a.cube, &a.keypad, Vec3::new(0.0, 0.118, 0.0), Vec3::new(0.15, 0.04, 0.12)));
        b.spawn(part(&a.cube, &a.screen, Vec3::new(0.0, 0.139, -0.03), Vec3::new(0.1, 0.004, 0.035)));
        for r in 0..3 {
            for c in 0..3 {
                let p = Vec3::new(-0.035 + c as f32 * 0.035, 0.139, 0.008 + r as f32 * 0.016);
                b.spawn(part(&a.cube, &a.tape, p, Vec3::new(0.022, 0.006, 0.011)));
            }
        }
        // LED: a dark lens, and a bright one with a small light that flash on each beep.
        let led = Vec3::new(0.06, 0.142, -0.03);
        b.spawn(part(&a.sphere, &a.led_off, led, Vec3::splat(0.008)));
        b.spawn((LedGlow, Visibility::Hidden, Transform::from_translation(led)))
            .with_child((Mesh3d(a.sphere.clone()), MeshMaterial3d(a.led_on.clone()), Transform::from_scale(Vec3::splat(0.011)), NotShadowCaster))
            .with_child((PointLight { color: Color::srgb(1.0, 0.12, 0.08), intensity: 6_000.0, range: 2.5, ..default() }, Transform::from_xyz(0.0, 0.03, 0.0)));
    });
    e.id()
}

fn blink_led(bomb: Res<Bomb>, mut leds: Query<&mut Visibility, With<LedGlow>>) {
    let want = if bomb.led > 0.0 && bomb.phase == BombPhase::Planted { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in &mut leds {
        if *v != want {
            *v = want;
        }
    }
}

/// Explosion visuals and the player's shake / flash.
#[derive(bevy::ecs::system::SystemParam)]
struct BlastFx<'w> {
    meshes: Option<ResMut<'w, Assets<Mesh>>>,
    mats: Option<ResMut<'w, Assets<StandardMaterial>>>,
    assets: Option<Res<'w, BombAssets>>,
    feedback: Option<ResMut<'w, crate::ui::feedback::Feedback>>,
    flashed: Option<ResMut<'w, crate::enemy::grenade::Flashed>>,
}

#[derive(Component)]
struct BlastSmoke {
    t: f32,
    life: f32,
    vel: Vec3,
    size: f32,
    alpha: f32,
}

impl BlastFx<'_> {
    fn shake_and_flash(&mut self, d: f32, sees: bool) {
        if let Some(fb) = self.feedback.as_mut() {
            fb.shake((1.2 - d / 40.0).clamp(0.15, 1.0));
        }
        if sees
            && d < 30.0
            && let Some(f) = self.flashed.as_mut()
        {
            let peak = (1.0 - d / 30.0).clamp(0.0, 0.85);
            if peak > f.alpha() {
                **f = crate::enemy::grenade::Flashed { peak, t: 0.0, dur: 0.4 + peak };
            }
        }
    }

    fn explode(&mut self, commands: &mut Commands, at: Vec3) {
        let (Some(meshes), Some(mats)) = (self.meshes.as_mut(), self.mats.as_mut()) else { return };
        let c = at + Vec3::Y * 0.8;
        // White core flash, then the big fireball.
        crate::enemy::grenade::spawn_blast(commands, meshes, mats, c, LinearRgba::rgb(30.0, 26.0, 20.0), 4.0, 0.22, Color::WHITE, 8_000_000.0);
        crate::enemy::grenade::spawn_blast(commands, meshes, mats, c + Vec3::Y * 0.6, LinearRgba::rgb(5.0, 1.6, 0.3), 9.0, 1.3, Color::srgb(1.0, 0.55, 0.2), 5_000_000.0);
        let Some(a) = self.assets.as_ref() else { return };
        // A column of dark smoke rolling up and out.
        for i in 0..14 {
            let f = i as f32;
            let ang = f * 2.39;
            let r = 0.6 + (f * 0.37).fract() * 1.8;
            let vel = Vec3::new(ang.cos() * r * 0.8, 1.2 + (f * 0.53).fract() * 2.2, ang.sin() * r * 0.8);
            let alpha = 0.55 + (f * 0.71).fract() * 0.3;
            let mat = mats.add(StandardMaterial {
                base_color: Color::srgba(0.1, 0.095, 0.09, alpha),
                perceptual_roughness: 1.0,
                alpha_mode: AlphaMode::Blend,
                ..default()
            });
            commands.spawn((
                Name::new("Blast smoke"),
                BlastSmoke { t: 0.0, life: 4.5 + (f * 0.29).fract() * 2.0, vel, size: 1.4 + r * 0.8, alpha },
                Mesh3d(a.sphere.clone()),
                MeshMaterial3d(mat),
                Transform::from_translation(at + Vec3::new(ang.cos() * r, 0.6 + f * 0.12, ang.sin() * r)).with_scale(Vec3::splat(0.3)),
                NotShadowCaster,
                NotShadowReceiver,
                Actor,
                DespawnOnExit(GameState::Playing),
            ));
        }
    }
}

fn tick_smoke(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut BlastSmoke, &mut Transform, &MeshMaterial3d<StandardMaterial>)>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    let dt = time.delta_secs();
    for (e, mut s, mut tf, mat) in &mut q {
        s.t += dt;
        if s.t >= s.life {
            mats.remove(&mat.0);
            commands.entity(e).despawn();
            continue;
        }
        let k = s.t / s.life;
        tf.translation += s.vel * dt * (1.0 - k).powi(2);
        tf.scale = Vec3::splat(s.size * (0.35 + 0.65 * (k * 4.0).min(1.0).sqrt()) * (1.0 + 0.5 * k));
        if let Some(mut m) = mats.get_mut(&mat.0) {
            m.base_color.set_alpha(s.alpha * (1.0 - k).powf(1.5));
        }
    }
}

// ---------------------------------------------------------------- site markers

#[derive(Component)]
struct SiteMarker;

/// One merged, flat, shadowless mesh per map: each site's outline (dashed at the corners) and
/// its letter, painted on the floor. Rebuilt when the map changes.
fn site_markers(
    mut commands: Commands,
    arena: Res<ArenaRes>,
    assets: Option<Res<BombAssets>>,
    mut meshes: ResMut<Assets<Mesh>>,
    markers: Query<Entity, With<SiteMarker>>,
    mut built: Local<Option<brain::arena::MapId>>,
) {
    let Some(a) = assets else { return };
    if *built == Some(arena.0.id) && !markers.is_empty() {
        return;
    }
    for e in &markers {
        commands.entity(e).despawn();
    }
    *built = Some(arena.0.id);
    if arena.0.sites.is_empty() {
        return;
    }
    let mesh = site_paint_mesh(&arena.0.sites);
    commands.spawn((
        Name::new("Bomb site paint"),
        SiteMarker,
        ArenaPiece,
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(a.paint.clone()),
        Transform::from_xyz(0.0, 0.012, 0.0),
        NotShadowCaster,
        NotShadowReceiver,
    ));
}

/// Paint strokes (ground-plane segments `a`→`b`) for one site: the outline, inset a little, and
/// a 2.4 m letter in the middle.
pub fn site_strokes(s: &BombSite) -> Vec<(V2, V2)> {
    let inset = 0.25;
    let (x0, z0, x1, z1) = (s.min.x + inset, s.min.y + inset, s.max.x - inset, s.max.y - inset);
    let mut v = vec![
        (V2::new(x0, z0), V2::new(x1, z0)),
        (V2::new(x1, z0), V2::new(x1, z1)),
        (V2::new(x1, z1), V2::new(x0, z1)),
        (V2::new(x0, z1), V2::new(x0, z0)),
    ];
    let c = s.center();
    let h = 1.2;
    let w = 0.8;
    // Letters read from the south (the player's side): -z is "up".
    let p = |x: f32, y: f32| V2::new(c.x + x * w, c.y - y * h);
    match s.letter {
        'A' => {
            v.push((p(-1.0, -1.0), p(0.0, 1.0)));
            v.push((p(0.0, 1.0), p(1.0, -1.0)));
            v.push((p(-0.5, 0.0), p(0.5, 0.0)));
        }
        _ => {
            // B: the stem and two blocky bowls.
            v.push((p(-1.0, -1.0), p(-1.0, 1.0)));
            for (top, bot) in [(1.0, 0.0), (0.0, -1.0)] {
                v.push((p(-1.0, top), p(0.6, top)));
                v.push((p(0.6, top), p(1.0, top - 0.2)));
                v.push((p(1.0, top - 0.2), p(1.0, bot + 0.2)));
                v.push((p(1.0, bot + 0.2), p(0.6, bot)));
                v.push((p(0.6, bot), p(-1.0, bot)));
            }
        }
    }
    v
}

fn site_paint_mesh(sites: &[BombSite]) -> Mesh {
    let mut pos: Vec<[f32; 3]> = Vec::new();
    let mut idx: Vec<u32> = Vec::new();
    for s in sites {
        for (i, (a, b)) in site_strokes(s).into_iter().enumerate() {
            // Outline strokes are thin, letter strokes bolder.
            let half = if i < 4 { 0.07 } else { 0.11 };
            let d = (b - a).norm();
            let n = V2::new(-d.y, d.x) * half;
            // Extend each stroke by its half-width so the joints close.
            let (a, b) = (a - d * half, b + d * half);
            let base = pos.len() as u32;
            for p in [a + n, a - n, b - n, b + n] {
                pos.push([p.x, 0.0, p.y]);
            }
            idx.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
        }
    }
    let n = pos.len();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; n])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; n])
        .with_inserted_indices(Indices::U32(idx))
}

// ---------------------------------------------------------------- control port

#[cfg(feature = "dev")]
mod commands {
    use bevy::prelude::*;
    use engine::devtools::{AddControlCommand, params};
    use serde_json::{Value, json};

    use super::{Bomb, BombPhase};
    use crate::enemy::{Enemy, EnemyCombat};
    use crate::map::{ArenaRes, to_world};
    use crate::player::Player;

    pub(super) fn register(app: &mut App) {
        app.add_control_command("bomb.state", "The bomb: carried / planted (fuse, defuse) / exploded / defused", params(&[]), state)
            .add_control_command(
                "bomb.plant",
                "Teleport the player into a site and plant the bomb at once (next frame); plant=false only teleports",
                params(&[
                    ("site", "string", "A or B (default A)"),
                    ("fuse", "number", "fuse seconds (default 40)"),
                    ("plant", "boolean", "plant at once (default true); false = just stand in the site"),
                ]),
                plant,
            )
            .add_control_command(
                "bomb.defuse",
                "Force a squad member to defuse (teleported onto the bomb unless teleport=false); member=-1 clears",
                params(&[("member", "integer", "squad slot (default: the first alive)"), ("teleport", "boolean", "default true")]),
                defuse,
            );
    }

    fn state(world: &mut World, _: Value) -> Result<Value, String> {
        let b = world.resource::<Bomb>().clone();
        let sites = &world.resource::<ArenaRes>().0.sites;
        let letter = |i: usize| sites.get(i).map(|s| s.letter.to_string());
        Ok(json!({
            "phase": b.phase.label(),
            "plant": b.plant,
            "can_plant": b.can_plant,
            "in_site": b.in_site.and_then(letter),
            "site": (b.phase != BombPhase::Carried).then(|| letter(b.site)).flatten(),
            "pos": (b.phase != BombPhase::Carried).then(|| json!([b.pos.x, b.pos.y, b.pos.z])),
            "fuse_left": (b.phase == BombPhase::Planted).then_some(b.fuse_left),
            "defuse": b.defuse,
            "defuser": b.defuser,
            "sites": sites.iter().map(|s| json!({"letter": s.letter.to_string(), "min": [s.min.x, s.min.y], "max": [s.max.x, s.max.y]})).collect::<Vec<_>>(),
        }))
    }

    fn plant(world: &mut World, p: Value) -> Result<Value, String> {
        let letter = p.get("site").and_then(Value::as_str).unwrap_or("A").trim().to_ascii_uppercase();
        let fuse = p.get("fuse").and_then(Value::as_f64).map_or(brain::objective::FUSE_S, |f| f as f32);
        if world.resource::<Bomb>().phase != BombPhase::Carried {
            return Err("the bomb is not carried (already planted this round)".into());
        }
        let arena = &world.resource::<ArenaRes>().0;
        let site = arena.sites.iter().find(|s| s.letter.to_string() == letter).ok_or_else(|| format!("no site `{letter}`"))?;
        // Somewhere standable near the middle of the site.
        let mut at = arena.nav.nearest_walkable(site.center());
        if !site.contains(at) {
            at = site.center();
        }
        let player = world.query_filtered::<Entity, With<Player>>().iter(world).next().ok_or("no player")?;
        if let Some(mut tf) = world.get_mut::<Transform>(player) {
            tf.translation.x = at.x;
            tf.translation.z = at.y;
            tf.translation.y = tf.translation.y.max(0.9);
        }
        if let Some(mut m) = world.get_mut::<engine::fps::FpsMotor>(player) {
            m.velocity = Vec3::ZERO;
        }
        let now = p.get("plant").and_then(Value::as_bool).unwrap_or(true);
        if now {
            world.resource_mut::<Bomb>().request = Some(fuse);
        }
        Ok(json!({"site": letter, "pos": [at.x, at.y], "fuse": fuse, "planted": now}))
    }

    fn defuse(world: &mut World, p: Value) -> Result<Value, String> {
        let member = p.get("member").and_then(Value::as_i64);
        if member == Some(-1) {
            world.resource_mut::<Bomb>().forced_defuser = None;
            return Ok(json!({"member": null}));
        }
        let b = world.resource::<Bomb>().clone();
        if b.phase != BombPhase::Planted {
            return Err("no planted bomb".into());
        }
        let mut q = world.query::<(Entity, &Enemy, &EnemyCombat)>();
        let alive: Vec<(Entity, usize)> = q.iter(world).filter(|(_, _, c)| !c.dead).map(|(e, en, _)| (e, en.slot)).collect();
        let (e, slot) = match member {
            Some(m) => *alive.iter().find(|(_, s)| *s as i64 == m).ok_or_else(|| format!("no live squad member {m}"))?,
            None => *alive.iter().min_by_key(|(_, s)| *s).ok_or("no live squad member")?,
        };
        if p.get("teleport").and_then(Value::as_bool).unwrap_or(true) {
            let ground = brain::math::V2::new(b.pos.x + 0.7, b.pos.z);
            let at = to_world(ground, 0.0);
            world.resource_mut::<Bomb>().teleport = Some((e, at));
        }
        world.resource_mut::<Bomb>().forced_defuser = Some(slot);
        Ok(json!({"member": slot}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beeps_speed_up() {
        assert!((beep_interval(FUSE_S) - 1.0).abs() < 1e-4);
        assert!(beep_interval(FUSE_S * 0.5) < beep_interval(FUSE_S));
        assert!(beep_interval(0.0) <= 0.13);
    }

    #[test]
    fn defuse_resets_when_interrupted() {
        let mut p = 0.0;
        for _ in 0..10 {
            p = defuse_step(p, true, 0.5);
        }
        assert!((p - 5.0 / DEFUSE_S).abs() < 1e-4);
        assert_eq!(defuse_step(p, false, 0.016), 0.0);
        let full = (0..20).fold(0.0, |p, _| defuse_step(p, true, 0.5));
        assert_eq!(full, 1.0);
    }

    #[test]
    fn cover_only_helps_past_the_kill_radius() {
        assert_eq!(blast_at(3.0, false), blast_at(3.0, true));
        assert!(blast_at(15.0, false) < blast_at(15.0, true));
        assert_eq!(blast_at(40.0, true), 0.0);
    }

    #[test]
    fn site_paint_has_outline_and_letter() {
        for letter in ['A', 'B'] {
            let s = BombSite::new(letter, -5.0, -5.0, 5.0, 5.0);
            let strokes = site_strokes(&s);
            assert!(strokes.len() > 5, "{letter}: {}", strokes.len());
            // Everything stays inside the site.
            assert!(strokes.iter().all(|(a, b)| s.contains(*a) && s.contains(*b)));
        }
    }
}
