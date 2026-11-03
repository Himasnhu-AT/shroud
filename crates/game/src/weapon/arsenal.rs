//! What the player carries and how they handle it: primary + sidearm ([`Arsenal`]), the gun in
//! hand ([`Weapon`]), weapon keys ([`WeaponInput`]), switching with holster/draw times, aiming
//! down sights (FOV zoom, slower movement, sensitivity scaling), sprint pose and the spread cone.

use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use engine::fps::{FpsController, FpsInput, FpsMotor};
use engine::gfx::MainCamera;
use engine::input::{CursorCaptured, LookInput};
use engine::settings::CoreSettings;

use super::{Armed, GunEvent, WeaponKind, WeaponSet, approach, ease};
use crate::player::{Player, PlayerIntent, PlayerSet};
use crate::economy::{BuyMenu, RoundLoadout};
use crate::state::{GameState, has_visuals};

/// Holster time before the next gun starts coming up.
pub const HOLSTER_S: f32 = 0.16;
/// ADS move speed multiplier (on top of the gun's `move_mult`).
pub const ADS_MOVE_MULT: f32 = 0.6;
/// Inspect animation length.
pub const INSPECT_S: f32 = 2.2;

pub(super) fn build(app: &mut App) {
    app.init_resource::<WeaponInput>()
        .init_resource::<WeaponOverride>()
        .add_systems(PostStartup, equip_player)
        .add_systems(OnEnter(GameState::Playing), reset_loadout.after(crate::map::MapSet).after(crate::economy::start_round))
        .add_systems(
            Update,
            (
                read_input.in_set(PlayerSet::Input).run_if(has_visuals),
                apply_override.in_set(PlayerSet::Input).after(read_input),
                ads_sensitivity.before(PlayerSet::Input).run_if(in_state(GameState::Playing)),
                (switching, aim).chain().in_set(WeaponSet).before(super::fire::fire_and_reload),
                ads_fov.after(WeaponSet).run_if(has_visuals),
            ),
        );
}

/// Draw time for each gun (bigger guns come up slower).
pub fn draw_time(kind: WeaponKind) -> f32 {
    match kind {
        WeaponKind::Pistol => 0.32,
        WeaponKind::Smg => 0.42,
        WeaponKind::Rifle => 0.5,
        WeaponKind::Shotgun => 0.55,
        WeaponKind::Marksman => 0.65,
    }
}

/// An optic whose model would block the view when aimed (looking into the back of the scope).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Optic {
    /// Kestrel: wide low-power optic, red dot in a ring.
    Reflex,
    /// Longbow: magnified scope, black surround, hairline reticle.
    Scope,
}

impl Optic {
    pub fn of(kind: WeaponKind) -> Option<Optic> {
        match kind {
            WeaponKind::Rifle => Some(Optic::Reflex),
            WeaponKind::Marksman => Some(Optic::Scope),
            _ => None,
        }
    }
}

/// A gun's ammo while it is in a slot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GunState {
    pub kind: WeaponKind,
    pub ammo: u32,
    pub reserve: u32,
}

impl GunState {
    pub fn full(kind: WeaponKind) -> Self {
        let s = kind.spec();
        Self { kind, ammo: s.mag, reserve: s.reserve }
    }
}

/// Slot 0 = primary, slot 1 = sidearm.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct Arsenal {
    pub slots: [Option<GunState>; 2],
    pub active: usize,
    /// Slot to go back to with Q.
    pub last: usize,
}

impl Arsenal {
    #[cfg(test)]
    pub fn new(primary: WeaponKind) -> Self {
        Self { slots: [Some(GunState::full(primary)), Some(GunState::full(WeaponKind::Pistol))], active: 0, last: 1 }
    }

    /// The gun in the active slot (the pistol if that slot is somehow empty).
    pub fn active_gun(&self) -> GunState {
        self.slots.get(self.active).copied().flatten().or(self.slots.iter().flatten().next().copied()).unwrap_or(GunState::full(WeaponKind::Pistol))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Ready,
    /// Coming up; `left` seconds until it can fire.
    Drawing { left: f32, total: f32 },
    /// Going down; then slot `to` comes up.
    Holstering { left: f32, to: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReloadKind {
    /// Mag still has rounds: swap mags, keep the chambered round.
    Tactical,
    /// Dry: swap mags and chamber (slower).
    Empty,
    /// Shotgun: shell by shell, interruptible by the trigger.
    Shells,
}

/// The gun in the player's hands (live state of `Arsenal.slots[active]`).
#[derive(Component, Clone, Debug, PartialEq)]
pub struct Weapon {
    pub kind: WeaponKind,
    pub ammo: u32,
    /// Magazine size.
    pub mag: u32,
    pub reserve: u32,
    /// Seconds until the next shot is allowed.
    pub cooldown: f32,
    /// Seconds left on the current reload stage, while reloading (for a shotgun: this shell).
    pub reloading: Option<f32>,
    /// Length of the current reload stage.
    pub reload_total: f32,
    pub reload_kind: ReloadKind,
    /// Stage events already sent for this reload stage (bit flags, `fire.rs`).
    pub reload_marks: u8,
    pub phase: Phase,
    /// 0 = hip, 1 = fully aimed.
    pub ads: f32,
    /// ADS requested this frame (after all vetoes).
    pub ads_wanted: bool,
    /// 0..1 blend into the lowered sprint pose.
    pub sprint: f32,
    /// Current cone half-angle in radians (what the crosshair shows).
    pub spread: f32,
    /// Spread added by recent shots (decays at `spread_recover`).
    pub bloom: f32,
    /// Shots in the current string (indexes the recoil pattern; resets after a pause).
    pub burst: u32,
    pub since_shot: f32,
    /// Pump / bolt cycle animation time left after a `Single` shot.
    pub cycling: f32,
    pub trigger_was: bool,
    /// A semi-auto pull buffered during cooldown (seconds it stays valid).
    pub queued: f32,
    /// Seconds of inspect animation left.
    pub inspect: f32,
}

impl Weapon {
    pub fn new(g: GunState) -> Self {
        let mag = g.kind.spec().mag;
        Self {
            kind: g.kind,
            ammo: g.ammo.min(mag),
            mag,
            reserve: g.reserve,
            cooldown: 0.0,
            reloading: None,
            reload_total: 1.0,
            reload_kind: ReloadKind::Tactical,
            reload_marks: 0,
            phase: Phase::Drawing { left: draw_time(g.kind), total: draw_time(g.kind) },
            ads: 0.0,
            ads_wanted: false,
            sprint: 0.0,
            spread: g.kind.spec().spread_hip,
            bloom: 0.0,
            burst: 0,
            since_shot: 10.0,
            cycling: 0.0,
            trigger_was: false,
            queued: 0.0,
            inspect: 0.0,
        }
    }

    pub fn state(&self) -> GunState {
        GunState { kind: self.kind, ammo: self.ammo, reserve: self.reserve }
    }

    /// Reload progress 0..1 of the current stage, if reloading.
    pub fn reload_progress(&self) -> Option<f32> {
        self.reloading.map(|left| (1.0 - left / self.reload_total.max(1e-3)).clamp(0.0, 1.0))
    }

    /// Fully aimed through the Longbow's scope (breathing sway, black scope overlay).
    pub fn scoped(&self) -> bool {
        self.sighted() == Some(Optic::Scope)
    }

    /// Fully aimed through an optic: the viewmodel is hidden and the optic's overlay is the
    /// sight picture (the Kestrel's low-power optic, the Longbow's scope). Iron sights: None.
    pub fn sighted(&self) -> Option<Optic> {
        Optic::of(self.kind).filter(|_| self.ads > 0.92)
    }

    pub fn ready(&self) -> bool {
        self.phase == Phase::Ready
    }
}

/// Weapon keys this frame (the trigger and R are [`PlayerIntent`]).
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct WeaponInput {
    /// RMB held.
    pub ads: bool,
    /// 1 / 2 pressed: go to this slot.
    pub select: Option<usize>,
    /// Mouse wheel: +1 next, -1 previous.
    pub cycle: i32,
    /// Q: last weapon.
    pub last: bool,
    /// V: inspect.
    pub inspect: bool,
    /// E held (pickups).
    pub interact: bool,
}

/// Control-port overrides (`weapon.ads`, `weapon.switch`, `weapon.pickup`); sticky until changed.
#[derive(Resource, Clone, Debug, Default)]
pub struct WeaponOverride {
    pub ads: Option<bool>,
    pub interact: Option<bool>,
    /// One-shot slot request.
    pub select: Option<usize>,
    pub inspect: bool,
}

fn equip_player(mut commands: Commands, loadout: Res<RoundLoadout>, players: Query<Entity, (With<Player>, Without<Weapon>)>) {
    for e in &players {
        let a = loadout.arsenal();
        let mut w = Weapon::new(a.active_gun());
        w.phase = Phase::Ready;
        commands.entity(e).insert((Armed(w.kind), w, a));
    }
}

/// Every round starts with what the player kept (`economy::RoundLoadout`: the guns they survived
/// with, or the pistol alone), full ammo, the best gun drawn.
pub fn reset_loadout(mut commands: Commands, loadout: Res<RoundLoadout>, players: Query<Entity, With<Player>>) {
    for e in &players {
        let a = loadout.arsenal();
        let w = Weapon::new(a.active_gun());
        commands.entity(e).insert((Armed(w.kind), w, a));
    }
}

fn read_input(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    scroll: Option<Res<AccumulatedMouseScroll>>,
    captured: Res<CursorCaptured>,
    buy: Res<BuyMenu>,
    mut input: ResMut<WeaponInput>,
) {
    let (Some(keys), Some(mouse)) = (keys, mouse) else { return };
    let live = captured.0;
    // The buy menu owns the number keys while it is open.
    let slots = !buy.open;
    let wheel = scroll.map_or(0.0, |s| s.delta.y);
    let new = WeaponInput {
        ads: live && mouse.pressed(MouseButton::Right),
        select: if !slots {
            None
        } else if keys.just_pressed(KeyCode::Digit1) {
            Some(0)
        } else if keys.just_pressed(KeyCode::Digit2) {
            Some(1)
        } else {
            None
        },
        cycle: if !live || wheel.abs() < 0.01 { 0 } else if wheel > 0.0 { -1 } else { 1 },
        last: keys.just_pressed(KeyCode::KeyQ),
        inspect: keys.just_pressed(KeyCode::KeyV),
        interact: keys.pressed(KeyCode::KeyE),
    };
    if *input != new {
        *input = new;
    }
}

fn apply_override(mut ov: ResMut<WeaponOverride>, mut input: ResMut<WeaponInput>) {
    if ov.ads.is_none() && ov.interact.is_none() && ov.select.is_none() && !ov.inspect {
        return;
    }
    if let Some(a) = ov.ads {
        input.ads = a;
    }
    if let Some(i) = ov.interact {
        input.interact = i;
    }
    if let Some(s) = ov.select.take() {
        input.select = Some(s);
    }
    if std::mem::take(&mut ov.inspect) {
        input.inspect = true;
    }
}

/// Number keys, wheel and Q start a holster; then the other gun draws.
pub fn switching(
    time: Res<Time>,
    mut input: ResMut<WeaponInput>,
    mut q: Query<(&mut Weapon, &mut Arsenal, &mut Armed), With<Player>>,
    mut events: MessageWriter<GunEvent>,
) {
    let dt = time.delta_secs();
    let Ok((mut w, mut a, mut armed)) = q.single_mut() else { return };
    let filled = |a: &Arsenal, s: usize| a.slots.get(s).is_some_and(|g| g.is_some());
    // A request this frame.
    let mut want = input.select.take();
    if input.cycle != 0 {
        let n = a.slots.len() as i32;
        let mut s = a.active as i32;
        for _ in 0..n {
            s = (s + input.cycle).rem_euclid(n);
            if filled(&a, s as usize) {
                break;
            }
        }
        want = Some(s as usize);
        input.cycle = 0;
    }
    if std::mem::take(&mut input.last) {
        want = Some(a.last);
    }
    if let Some(to) = want {
        let current_target = match w.phase {
            Phase::Holstering { to, .. } => to,
            _ => a.active,
        };
        if to != current_target && filled(&a, to) {
            w.reloading = None;
            w.inspect = 0.0;
            w.ads_wanted = false;
            // Switching back mid-holster: start from how far down the gun already is.
            let left = match w.phase {
                Phase::Holstering { left, .. } => left,
                _ => HOLSTER_S,
            };
            if to == a.active {
                w.phase = Phase::Drawing { left: HOLSTER_S - left, total: HOLSTER_S };
            } else {
                w.phase = Phase::Holstering { left, to };
            }
        }
    }
    match w.phase {
        Phase::Ready => {}
        Phase::Drawing { left, total } => {
            let left = left - dt;
            w.phase = if left <= 0.0 { Phase::Ready } else { Phase::Drawing { left, total } };
        }
        Phase::Holstering { left, to } => {
            let left = left - dt;
            if left > 0.0 {
                w.phase = Phase::Holstering { left, to };
            } else if let Some(next) = a.slots[to] {
                let from = a.active;
                a.slots[from] = Some(w.state());
                a.active = to;
                a.last = from;
                let mut nw = Weapon::new(next);
                nw.sprint = w.sprint;
                *w = nw;
                if armed.0 != next.kind {
                    armed.0 = next.kind;
                }
                events.write(GunEvent::Draw { kind: next.kind });
            } else {
                w.phase = Phase::Ready;
            }
        }
    }
    if std::mem::take(&mut input.inspect) && w.ready() && w.reloading.is_none() && !w.ads_wanted {
        w.inspect = INSPECT_S;
        events.write(GunEvent::Inspect);
    }
}

/// ADS blend, sprint pose, movement speed and the spread cone.
pub fn aim(
    time: Res<Time>,
    input: Res<WeaponInput>,
    intent: Res<PlayerIntent>,
    mut q: Query<(&mut Weapon, &FpsMotor, &FpsInput, &mut FpsController), With<Player>>,
    mut events: MessageWriter<GunEvent>,
) {
    let dt = time.delta_secs();
    let Ok((mut w, motor, fin, mut ctl)) = q.single_mut() else { return };
    let spec = w.kind.spec();
    let base = crate::player::controller();
    let speed = Vec2::new(motor.velocity.x, motor.velocity.z).length();

    // Sprint pose: running forward on the ground, not shooting or aiming.
    let sprinting = fin.sprint && fin.move_axis.y > 0.1 && motor.grounded && !motor.crouched && speed > base.walk_speed + 0.5;
    let sprint_target = (sprinting && !intent.fire && !input.ads && w.reloading.is_none()) as u8 as f32;
    let sprint = w.sprint + (sprint_target - w.sprint) * approach(12.0, dt);
    if (w.sprint - sprint).abs() > 1e-5 {
        w.sprint = sprint;
    }

    // ADS: not while switching, sprinting or (except the shotgun) reloading.
    let reload_blocks = w.reloading.is_some() && w.kind != WeaponKind::Shotgun;
    let wanted = input.ads && !matches!(w.phase, Phase::Holstering { .. }) && !reload_blocks && w.sprint < 0.5;
    if wanted && !w.ads_wanted {
        events.write(GunEvent::AdsIn);
        w.inspect = 0.0;
    }
    if w.ads_wanted != wanted {
        w.ads_wanted = wanted;
    }
    let step = dt / spec.ads_time_s.max(0.05);
    let ads = if wanted { (w.ads + step).min(1.0) } else { (w.ads - step * 1.3).max(0.0) };
    if w.ads != ads {
        w.ads = ads;
    }

    // Movement speed: the gun's weight, slower aimed.
    let mult = spec.move_mult * (1.0 + (ADS_MOVE_MULT - 1.0) * ease(ads));
    let (walk, sprint_speed, crouch) = (base.walk_speed * mult, base.sprint_speed * spec.move_mult, base.crouch_speed * mult);
    if (ctl.walk_speed - walk).abs() > 1e-3 || (ctl.sprint_speed - sprint_speed).abs() > 1e-3 || (ctl.crouch_speed - crouch).abs() > 1e-3 {
        ctl.walk_speed = walk;
        ctl.sprint_speed = sprint_speed;
        ctl.crouch_speed = crouch;
    }

    // Spread cone.
    let a = ease(ads);
    let mut cone = spec.spread_hip + (spec.spread_ads - spec.spread_hip) * a;
    let run = (speed / base.walk_speed).min(1.6);
    cone += spec.spread_move * run * (1.0 - 0.65 * a);
    if !motor.grounded {
        cone += spec.spread_hip * 1.2 + 0.035;
    }
    if motor.crouched && motor.grounded {
        cone *= 0.72;
    }
    let bloom = (w.bloom - spec.spread_recover * dt).max(0.0);
    if w.bloom != bloom {
        w.bloom = bloom;
    }
    let spread = cone + bloom * (1.0 - 0.5 * a);
    if (w.spread - spread).abs() > 1e-6 {
        w.spread = spread;
    }
    if w.inspect > 0.0 {
        w.inspect = if intent.fire || intent.reload || w.reloading.is_some() { 0.0 } else { (w.inspect - dt).max(0.0) };
    }
}

/// FOV multiplier for the current ADS blend.
pub fn zoom_of(w: Option<&Weapon>) -> f32 {
    w.map_or(1.0, |w| 1.0 + (w.kind.spec().ads_zoom - 1.0) * ease(w.ads))
}

/// Mouse look slows with zoom (same screen distance per mouse count at any zoom).
fn ads_sensitivity(settings: Res<CoreSettings>, q: Query<&Weapon, With<Player>>, mut look: ResMut<LookInput>) {
    let Ok(w) = q.single() else { return };
    if w.ads <= 0.0 || (look.yaw == 0.0 && look.pitch == 0.0) {
        return;
    }
    let base = (settings.fov_deg.to_radians() * 0.5).tan();
    let zoomed = (settings.fov_deg.to_radians() * zoom_of(Some(w)) * 0.5).tan();
    let k = (zoomed / base).clamp(0.1, 1.0);
    look.yaw *= k;
    look.pitch *= k;
}

/// ADS zoom on the main camera (the engine sets the base FOV from settings).
fn ads_fov(
    settings: Res<CoreSettings>,
    state: Option<Res<State<GameState>>>,
    q: Query<&Weapon, With<Player>>,
    mut cams: Query<&mut Projection, With<MainCamera>>,
) {
    let playing = state.is_some_and(|s| *s.get() == GameState::Playing);
    let zoom = if playing { zoom_of(q.single().ok()) } else { 1.0 };
    let fov = settings.fov_deg.clamp(30.0, 150.0).to_radians() * zoom;
    for mut p in &mut cams {
        if let Projection::Perspective(pp) = p.as_ref()
            && (pp.fov - fov).abs() > 1e-4
            && let Projection::Perspective(pp) = p.as_mut()
        {
            pp.fov = fov;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arsenal_starts_with_primary_and_pistol() {
        let a = Arsenal::new(WeaponKind::Shotgun);
        assert_eq!(a.slots[0].map(|g| g.kind), Some(WeaponKind::Shotgun));
        assert_eq!(a.slots[1].map(|g| g.kind), Some(WeaponKind::Pistol));
        assert_eq!(a.slots[0].unwrap().ammo, WeaponKind::Shotgun.spec().mag);
    }

    #[test]
    fn zoom_follows_ads_blend() {
        let mut w = Weapon::new(GunState::full(WeaponKind::Marksman));
        assert_eq!(zoom_of(Some(&w)), 1.0);
        w.ads = 1.0;
        assert!((zoom_of(Some(&w)) - WeaponKind::Marksman.spec().ads_zoom).abs() < 1e-5);
        assert!(w.scoped());
    }

    #[test]
    fn scoped_guns_aim_through_their_optic() {
        // Guns whose optic would block the view (looking into the back of the scope model)
        // hide the viewmodel aimed and show the optic overlay; iron-sight guns keep the model.
        for kind in WeaponKind::ALL {
            let mut w = Weapon::new(GunState::full(kind));
            w.ads = 1.0;
            let want = match kind {
                WeaponKind::Rifle => Some(Optic::Reflex),
                WeaponKind::Marksman => Some(Optic::Scope),
                _ => None,
            };
            assert_eq!(w.sighted(), want, "{kind:?}");
            assert_eq!(w.scoped(), kind == WeaponKind::Marksman);
            w.ads = 0.5;
            assert_eq!(w.sighted(), None, "{kind:?} half aimed");
        }
    }
}
