//! Menu screen stack and what the buttons do.
//!
//! `Menu` state: `[Main]`, Play pushes `Setup`, Settings pushes `Settings`.
//! `Playing`: empty until Esc pushes `Pause` (sets [`Paused`], pauses virtual + physics time,
//! frees the cursor). Settings can be pushed on top. Popping the last screen resumes.
//! The top screen is rebuilt whenever it (or the settings tab) changes.

use avian3d::prelude::{Physics, PhysicsTime};
use bevy::prelude::*;
use bevy::window::WindowFocused;

use super::widgets::{Focus, Focusable, NavBack, UiAction, UiActivated, WidgetSet, wrap_tab};
use crate::match_flow::MatchFlow;
use crate::save::Brain;
use crate::settings::Settings;
use crate::state::{GameState, Paused, has_visuals};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Main,
    Setup,
    Settings,
    Pause,
}

impl Screen {
    pub fn parse(s: &str) -> Option<Screen> {
        Some(match s {
            "main" => Screen::Main,
            "setup" => Screen::Setup,
            "settings" => Screen::Settings,
            "pause" => Screen::Pause,
            _ => return None,
        })
    }
}

pub const TABS: [&str; 4] = ["Gameplay", "Controls", "Video", "Audio"];

#[derive(Resource, Default, Debug)]
pub struct Screens {
    /// (screen, focus order to restore when it is on top again).
    pub stack: Vec<(Screen, u32)>,
    pub tab: usize,
    built: Option<(Screen, usize)>,
}

impl Screens {
    pub fn top(&self) -> Option<Screen> {
        self.stack.last().map(|s| s.0)
    }
    fn push(&mut self, s: Screen, cur_focus: Option<u32>) {
        if let (Some(last), Some(f)) = (self.stack.last_mut(), cur_focus) {
            last.1 = f;
        }
        if s == Screen::Settings {
            self.tab = 0;
        }
        self.stack.push((s, 0));
    }
}

/// Root of the screen currently built (despawned on rebuild).
#[derive(Component)]
pub struct ScreenRoot;

/// Settings changed since the last save (saved after a short delay, or on Back).
#[derive(Resource, Default)]
struct SaveDebounce(Option<f32>);

pub struct ScreensPlugin;

impl Plugin for ScreensPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Screens>()
            .init_resource::<SaveDebounce>()
            .add_systems(OnEnter(GameState::Menu), open_main_menu)
            .add_systems(OnExit(GameState::Menu), clear_screens)
            .add_systems(OnEnter(GameState::Playing), clear_screens)
            .add_systems(OnExit(GameState::Playing), clear_screens)
            .add_systems(
                Update,
                (handle_back, handle_actions, auto_pause_on_focus_loss, rebuild)
                    .chain()
                    .after(WidgetSet)
                    .run_if(has_visuals),
            )
            .add_systems(Update, save_settings_debounced);
    }
}

fn open_main_menu(mut screens: ResMut<Screens>, mut paused: ResMut<Paused>) {
    screens.stack = vec![(Screen::Main, 0)];
    paused.0 = false;
}

/// Leaving/entering a state closes every screen and un-pauses.
fn clear_screens(mut commands: Commands, mut screens: ResMut<Screens>, roots: Query<Entity, With<ScreenRoot>>, mut pause: PauseCtl) {
    screens.stack.clear();
    for e in &roots {
        commands.entity(e).despawn();
    }
    screens.built = None;
    pause.set(false);
}

/// Pause plumbing shared by Esc, buttons and the control port.
#[derive(bevy::ecs::system::SystemParam)]
pub struct PauseCtl<'w> {
    paused: ResMut<'w, Paused>,
    virt: ResMut<'w, Time<Virtual>>,
    physics: Option<ResMut<'w, Time<Physics>>>,
}

impl PauseCtl<'_> {
    pub fn set(&mut self, on: bool) {
        if self.paused.0 == on {
            return;
        }
        self.paused.0 = on;
        if on {
            self.virt.pause();
        } else {
            self.virt.unpause();
        }
        if let Some(p) = self.physics.as_mut() {
            if on {
                p.pause();
            } else {
                p.unpause();
            }
        }
    }
}

fn handle_back(
    mut backs: MessageReader<NavBack>,
    mut out: MessageWriter<UiActivated>,
    screens: Res<Screens>,
    state: Res<State<GameState>>,
) {
    for _ in backs.read() {
        match (screens.top(), state.get()) {
            (Some(Screen::Main), _) => {}
            (Some(Screen::Pause), _) => {
                out.write(UiActivated(UiAction::Resume));
            }
            (Some(_), _) => {
                out.write(UiActivated(UiAction::Back));
            }
            (None, GameState::Playing) => {
                // Opened by Esc: handled as an action so the control port can do the same.
                out.write(UiActivated(UiAction::Back));
            }
            (None, GameState::Debrief) => {
                out.write(UiActivated(UiAction::MainMenu));
            }
            _ => {}
        }
    }
}

/// Clicking away from the game (alt-tab) pauses a live round. Never in test-driven runs.
fn auto_pause_on_focus_loss(
    mut events: MessageReader<WindowFocused>,
    state: Res<State<GameState>>,
    mut screens: ResMut<Screens>,
    mut pause: PauseCtl,
) {
    let lost = events.read().any(|e| !e.focused);
    if lost && *state.get() == GameState::Playing && screens.stack.is_empty() && crate::player::not_test_driven() {
        screens.push(Screen::Pause, None);
        pause.set(true);
    }
}

fn handle_actions(
    mut actions: MessageReader<UiActivated>,
    mut screens: ResMut<Screens>,
    state: Res<State<GameState>>,
    mut next: ResMut<NextState<GameState>>,
    mut flow: ResMut<MatchFlow>,
    mut brain: ResMut<Brain>,
    mut settings: ResMut<Settings>,
    focus: Res<Focus>,
    focusables: Query<&Focusable>,
    tracer: Option<Res<crate::ops::trace::GameTracer>>,
    mut pause: PauseCtl,
    mut exit: MessageWriter<AppExit>,
) {
    let cur_focus = focus.0.and_then(|e| focusables.get(e).ok()).map(|f| f.order);
    for UiActivated(a) in actions.read().copied() {
        match a {
            UiAction::Play => screens.push(Screen::Setup, cur_focus),
            UiAction::Settings => screens.push(Screen::Settings, cur_focus),
            UiAction::Quit => {
                settings.save();
                exit.write(AppExit::Success);
            }
            UiAction::StartMatch => {
                settings.save();
                flow.abandon();
                next.set(GameState::Playing);
            }
            UiAction::Resume => {
                screens.stack.clear();
                pause.set(false);
            }
            UiAction::Restart => {
                flow.abandon();
                pause.set(false);
                next.set(GameState::Playing);
            }
            UiAction::MainMenu => {
                flow.abandon();
                pause.set(false);
                next.set(GameState::Menu);
            }
            UiAction::Back => {
                if screens.top() == Some(Screen::Settings) {
                    settings.save();
                }
                match screens.top() {
                    None if *state.get() == GameState::Playing => {
                        screens.push(Screen::Pause, None);
                        pause.set(true);
                    }
                    Some(Screen::Main) | None => {}
                    Some(Screen::Pause) => {
                        screens.stack.clear();
                        pause.set(false);
                    }
                    Some(_) => {
                        screens.stack.pop();
                    }
                }
            }
            UiAction::Tab(i) => {
                if screens.top() == Some(Screen::Settings) {
                    screens.tab = wrap_tab(screens.tab, i, TABS.len());
                }
            }
            UiAction::ResetTab => {
                let defaults = Settings::default();
                for f in super::settings_screen::tab_fields(screens.tab) {
                    f.set(&mut settings, f.get(&defaults));
                }
            }
            UiAction::NextMatch => next.set(GameState::Playing),
            UiAction::ResetAi => {
                brain.reset(tracer.as_deref());
                next.set(GameState::Menu);
            }
        }
    }
}

fn save_settings_debounced(time: Res<Time<Real>>, settings: Res<Settings>, mut pending: ResMut<SaveDebounce>) {
    if settings.is_changed() && !settings.is_added() {
        pending.0 = Some(0.6);
    }
    if let Some(t) = pending.0.as_mut() {
        *t -= time.delta_secs();
        if *t <= 0.0 {
            settings.save();
            pending.0 = None;
        }
    }
}

/// Spawn the top screen when it changed.
fn rebuild(
    mut commands: Commands,
    mut screens: ResMut<Screens>,
    roots: Query<Entity, With<ScreenRoot>>,
    mut focus: ResMut<Focus>,
    focusables: Query<(Entity, &Focusable)>,
    ctx: ScreenCtx,
) {
    let want = screens.top().map(|s| (s, screens.tab));
    if want == screens.built {
        // Restore remembered focus once the rebuilt screen's widgets exist.
        if let Some(order) = screens.stack.last().map(|s| s.1).filter(|o| *o > 0)
            && let Some((e, _)) = focusables.iter().find(|(_, f)| f.order == order)
        {
            focus.0 = Some(e);
            if let Some(last) = screens.stack.last_mut() {
                last.1 = 0;
            }
        }
        return;
    }
    for e in &roots {
        commands.entity(e).despawn();
    }
    focus.0 = None;
    screens.built = want;
    let Some((screen, tab)) = want else { return };
    let root = commands.spawn((ScreenRoot, super::theme::fullscreen(), GlobalZIndex(20))).id();
    match screen {
        Screen::Main => crate::menu::spawn_main(&mut commands, root, &ctx),
        Screen::Setup => crate::menu::spawn_setup(&mut commands, root, &ctx),
        Screen::Settings => super::settings_screen::spawn(&mut commands, root, tab, &ctx),
        Screen::Pause => super::pause::spawn(&mut commands, root, &ctx),
    }
}

/// Read-only data screens show.
#[derive(bevy::ecs::system::SystemParam)]
pub struct ScreenCtx<'w> {
    pub settings: Res<'w, Settings>,
    pub brain: Res<'w, Brain>,
    pub flow: Res<'w, MatchFlow>,
    pub state: Res<'w, State<GameState>>,
}

/// Control port: open a screen (`None` closes everything; in Playing that resumes).
pub fn open_screen(world: &mut World, screen: Option<Screen>, tab: Option<usize>) -> Result<(), String> {
    let state = *world.resource::<State<GameState>>().get();
    let mut screens = world.resource_mut::<Screens>();
    match screen {
        None => screens.stack.retain(|s| s.0 == Screen::Main),
        Some(Screen::Pause) => {
            if state != GameState::Playing {
                return Err("pause needs GameState::Playing".into());
            }
            screens.stack = vec![(Screen::Pause, 0)];
        }
        Some(Screen::Main) | Some(Screen::Setup) if state != GameState::Menu => {
            return Err("main/setup need GameState::Menu (game.menu first)".into());
        }
        Some(s) => {
            if screens.top() != Some(s) {
                screens.push(s, None);
            }
        }
    }
    if let Some(t) = tab {
        screens.tab = t.min(TABS.len() - 1);
    }
    let paused = state == GameState::Playing && !screens.stack.is_empty();
    let mut ctl = world.resource_mut::<Paused>();
    if ctl.0 != paused {
        ctl.0 = paused;
        if paused {
            world.resource_mut::<Time<Virtual>>().pause();
        } else {
            world.resource_mut::<Time<Virtual>>().unpause();
        }
        if let Some(mut p) = world.get_resource_mut::<Time<Physics>>() {
            if paused { p.pause() } else { p.unpause() }
        }
    }
    Ok(())
}
