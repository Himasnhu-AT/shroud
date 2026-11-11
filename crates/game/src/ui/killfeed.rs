//! Kill feed (top right): who killed whom with which gun, plus a headshot badge.
//!
//! Kills are detected here, without touching gameplay code: [`detect_kills`] remembers the last
//! [`Damage`] each target took and publishes a [`KillEvent`] when a [`Health`] drops to zero
//! (enemies and the player). The scoreboard and the round summary read those events too, so it
//! runs headless as well; only the feed lines need visuals.

use std::collections::HashMap;

use bevy::prelude::*;
use brain::squad::callsign;
use brain::weapons::WeaponKind;

use super::hud::KillFeed;
use super::theme::*;
use crate::enemy::Enemy;
use crate::player::Player;
use crate::state::{GameState, has_visuals};
use crate::weapon::{Damage, DamageCause, Health, WeaponSet};

/// Lines kept in the feed.
const MAX_LINES: usize = 5;
const LIFE_S: f32 = 5.0;
const FADE_S: f32 = 0.6;
const BG_ALPHA: f32 = 0.7;

/// Someone died. Labels are callsigns ("Alpha"), or "You" for the player.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct KillEvent {
    pub killer: &'static str,
    pub victim: &'static str,
    /// The gun that did it; `None` for a grenade or the bomb (the killing hit's
    /// [`DamageCause`]).
    pub weapon: Option<WeaponKind>,
    pub headshot: bool,
    /// The player got the kill / was killed.
    pub by_player: bool,
    pub on_player: bool,
    /// Enemy slots of the killer / victim, when they are enemies.
    pub killer_slot: Option<usize>,
    pub victim_slot: Option<usize>,
    /// The killing hit was a gun's (not a frag; the bomb is never "by_player"). The career's
    /// kill objectives only count these.
    pub gun: bool,
    /// What dealt the killing hit.
    pub cause: DamageCause,
}

impl KillEvent {
    /// "Longbow", "Frag", "Bomb".
    pub fn weapon_label(&self) -> &'static str {
        match self.cause {
            DamageCause::Gun(k) => k.label(),
            DamageCause::Grenade => "Frag",
            DamageCause::Bomb => "Bomb",
        }
    }
}

pub struct KillFeedPlugin;

impl Plugin for KillFeedPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<KillEvent>()
            .add_systems(Update, detect_kills.after(WeaponSet).run_if(in_state(GameState::Playing)))
            .add_systems(Update, (show_kills.after(detect_kills), fade_lines).chain().run_if(has_visuals));
    }
}

/// Last hit on each target: (source, headshot, what dealt it).
#[derive(Default)]
pub(crate) struct LastHits {
    hits: HashMap<Entity, (Entity, bool, DamageCause)>,
    dead: HashMap<Entity, bool>,
}

pub(crate) fn detect_kills(
    mut dmg: MessageReader<Damage>,
    healths: Query<(Entity, &Health, Option<&Enemy>, Has<Player>)>,
    players: Query<(), With<Player>>,
    enemies: Query<&Enemy>,
    mut out: MessageWriter<KillEvent>,
    mut st: Local<LastHits>,
) {
    let st = &mut *st;
    for d in dmg.read() {
        st.hits.insert(d.target, (d.source, d.headshot, d.cause));
    }
    for (e, hp, enemy, is_player) in &healths {
        let dead = hp.is_dead();
        let was = st.dead.insert(e, dead).unwrap_or(false);
        if !dead || was || (enemy.is_none() && !is_player) {
            continue;
        }
        // No hit on record (a scripted / test kill): nothing to report.
        let Some(&(src, headshot, cause)) = st.hits.get(&e) else { continue };
        let by_player = players.contains(src);
        let weapon = cause.gun();
        let shot = weapon.is_some();
        let (killer, killer_slot) = if by_player {
            ("You", None)
        } else if let Ok(en) = enemies.get(src) {
            (callsign(en.slot), Some(en.slot))
        } else {
            ("Squad", None)
        };
        let (victim, victim_slot) = match enemy {
            Some(en) => (callsign(en.slot), Some(en.slot)),
            None => ("You", None),
        };
        out.write(KillEvent { killer, victim, weapon, headshot, by_player, on_player: is_player, killer_slot, victim_slot, gun: shot, cause });
    }
    // Forget entities that are gone (corpses despawn, rounds respawn the squad).
    if st.dead.len() > healths.iter().count() {
        st.dead.retain(|e, _| healths.contains(*e));
        st.hits.retain(|e, _| healths.contains(*e));
    }
}

#[derive(Component)]
struct FeedLine(f32);

fn show_kills(
    mut commands: Commands,
    mut kills: MessageReader<KillEvent>,
    feed: Query<Entity, With<KillFeed>>,
    lines: Query<(Entity, &FeedLine)>,
) {
    let Ok(feed) = feed.single() else {
        kills.clear();
        return;
    };
    let mut count = lines.iter().count();
    for k in kills.read() {
        if count >= MAX_LINES
            && let Some((old, _)) = lines.iter().max_by(|a, b| a.1.0.total_cmp(&b.1.0))
        {
            commands.entity(old).despawn();
            count -= 1;
        }
        count += 1;
        let (kc, vc) = if k.on_player { (ENEMY, FRIENDLY) } else if k.by_player { (FRIENDLY, ENEMY) } else { (ENEMY, ENEMY) };
        commands
            .spawn((
                FeedLine(0.0),
                Node { align_items: AlignItems::Center, column_gap: px(10), padding: UiRect::axes(px(12), px(5)), ..default() },
                BackgroundColor(Color::srgba(0.03, 0.04, 0.06, BG_ALPHA)),
                Pickable::IGNORE,
                ChildOf(feed),
            ))
            .with_children(|l| {
                l.spawn(text(k.killer.to_uppercase(), label(SIZE_LABEL + 2.0), kc));
                l.spawn((
                    Node { padding: UiRect::axes(px(6), px(1)), border: UiRect::all(px(1)), ..default() },
                    BorderColor::all(TEXT_MUTED.with_alpha(0.5)),
                    Pickable::IGNORE,
                    children![text(k.weapon_label().to_uppercase(), label(SIZE_TINY), TEXT_MUTED)],
                ));
                if k.headshot {
                    l.spawn((
                        Node { padding: UiRect::axes(px(5), px(0)), ..default() },
                        BackgroundColor(ACCENT.with_alpha(0.9)),
                        Pickable::IGNORE,
                        children![text("HS", label(SIZE_TINY), BG)],
                    ));
                }
                l.spawn(text(k.victim.to_uppercase(), label(SIZE_LABEL + 2.0), vc));
            });
    }
}

fn fade_lines(
    mut commands: Commands,
    time: Res<Time>,
    mut lines: Query<(Entity, &mut FeedLine, &mut BackgroundColor, &Children)>,
    children: Query<&Children>,
    mut texts: Query<&mut TextColor>,
    mut borders: Query<&mut BorderColor>,
) {
    for (e, mut l, mut bg, kids) in &mut lines {
        l.0 += time.delta_secs();
        if l.0 >= LIFE_S {
            commands.entity(e).despawn();
            continue;
        }
        // Only the fade-out touches colours (no writes for most of the line's life).
        let a = ((LIFE_S - l.0) / FADE_S).min(1.0);
        if a >= 1.0 {
            continue;
        }
        bg.0.set_alpha(a * BG_ALPHA);
        for k in kids.iter() {
            for n in std::iter::once(k).chain(children.iter_descendants(k)) {
                if let Ok(mut t) = texts.get_mut(n) {
                    t.0.set_alpha(a);
                }
                if let Ok(mut b) = borders.get_mut(n) {
                    *b = BorderColor::all(TEXT_MUTED.with_alpha(0.5 * a));
                }
            }
        }
    }
}
