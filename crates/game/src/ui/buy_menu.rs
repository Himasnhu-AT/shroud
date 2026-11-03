//! The buy menu (B) and the money HUD. Items come from `economy::catalog()` (every
//! `WeaponKind`, armor and grenades) in five columns: Pistols, SMGs & Shotguns, Rifles & Snipers,
//! Gear, Grenades. Number keys 1–9, 0 buy the first ten items; every item is clickable.
//! Grenades show how many are carried of the most allowed (frag 1, flashbang 2, smoke 1). Owned items are marked,
//! unaffordable ones greyed. The menu frees the cursor (`state::rounds_only`) and closes on B,
//! Esc, or when the buy window ends.

use std::sync::OnceLock;

use bevy::prelude::*;
use engine::ui::menus::{MenuAction, MenuEvent};
use engine::ui::widgets::{ActionButton, KeepColor, Look, UiAction};

use super::hud::HideWhenPaused;
use super::theme::*;
use crate::economy::{self, BuyItem, BuyMenu, PlayerGrenades, PlayerWallet, RoundPhase, can_buy, catalog, player_kit, quote};
use crate::match_flow::MatchFlow;
use crate::state::{GameState, Paused, has_visuals};

pub struct BuyMenuPlugin;

impl Plugin for BuyMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BuyNotice>()
            .add_systems(OnEnter(GameState::Playing), spawn_money_hud.run_if(has_visuals).after(economy::start_round))
            .add_systems(
                Update,
                (toggle_key, buy_keys, buy_clicks, auto_close, (rebuild_menu, update_texts, money_hud).chain().run_if(has_visuals))
                    .chain()
                    .run_if(in_state(GameState::Playing)),
            )
            // After the engine's screen logic has seen this frame's Esc with pause disallowed.
            .add_systems(PostUpdate, esc_closes.run_if(in_state(GameState::Playing)));
    }
}

/// A short line in the menu after a failed buy ("Can't afford…").
#[derive(Resource, Default)]
struct BuyNotice(Option<(String, f32)>);

/// Stable `'static` ids for the item buttons ("buy.0", "buy.1", …).
fn item_id(i: usize) -> &'static str {
    static IDS: OnceLock<Vec<&'static str>> = OnceLock::new();
    let ids = IDS.get_or_init(|| (0..64).map(|i| &*Box::leak(format!("buy.{i}").into_boxed_str())).collect());
    ids.get(i).copied().unwrap_or("buy.none")
}

const KEYS: [KeyCode; 10] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
    KeyCode::Digit0,
];

fn key_label(i: usize) -> Option<&'static str> {
    ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"].get(i).copied()
}

fn toggle_key(mut commands: Commands, keys: Option<Res<ButtonInput<KeyCode>>>, paused: Res<Paused>) {
    if paused.0 || !keys.is_some_and(|k| k.just_pressed(KeyCode::KeyB)) {
        return;
    }
    commands.queue(|world: &mut World| {
        let open = world.resource::<BuyMenu>().open;
        let next = !open && can_buy(world).is_ok();
        if !open && !next {
            let why = can_buy(world).err().unwrap_or("");
            info!("buy menu: closed ({why})");
        }
        world.resource_mut::<BuyMenu>().open = next;
    });
}

fn request_buy(commands: &mut Commands, item: BuyItem) {
    commands.queue(move |world: &mut World| {
        let notice = match economy::buy(world, item, false) {
            Ok(_) => None,
            Err(e) => Some((e, 2.5)),
        };
        world.resource_mut::<BuyNotice>().0 = notice;
    });
}

fn buy_keys(mut commands: Commands, menu: Res<BuyMenu>, keys: Option<Res<ButtonInput<KeyCode>>>) {
    let Some(keys) = keys else { return };
    if !menu.open {
        return;
    }
    let items = catalog();
    for (i, k) in KEYS.iter().enumerate() {
        if keys.just_pressed(*k)
            && let Some(item) = items.get(i)
        {
            request_buy(&mut commands, *item);
        }
    }
}

fn buy_clicks(mut commands: Commands, mut events: MessageReader<MenuEvent>, menu: Res<BuyMenu>) {
    let items = catalog();
    for MenuEvent(id) in events.read() {
        let Some(i) = id.strip_prefix("buy.").and_then(|s| s.parse::<usize>().ok()) else { continue };
        if menu.open
            && let Some(item) = items.get(i)
        {
            request_buy(&mut commands, *item);
        }
    }
}

/// The buy window closed (left the zone, time up, died, round over): close the menu.
fn auto_close(world: &mut World) {
    if world.resource::<BuyMenu>().open && can_buy(world).is_err() {
        world.resource_mut::<BuyMenu>().open = false;
    }
    let dt = world.resource::<Time>().delta_secs();
    let mut n = world.resource_mut::<BuyNotice>();
    if let Some((_, t)) = n.0.as_mut() {
        *t -= dt;
        if *t <= 0.0 {
            n.0 = None;
        }
    }
}

fn esc_closes(keys: Option<Res<ButtonInput<KeyCode>>>, mut menu: ResMut<BuyMenu>) {
    if menu.open && keys.is_some_and(|k| k.just_pressed(KeyCode::Escape)) {
        menu.open = false;
    }
}

// ---- menu ----

#[derive(Component)]
struct MenuRoot;
#[derive(Component)]
struct MenuMoney;
#[derive(Component)]
struct MenuTimer;
#[derive(Component)]
struct MenuNotice;

/// What the menu shows; rebuilt when it changes.
#[derive(Clone, PartialEq)]
struct MenuSig {
    money: u32,
    carried: Vec<crate::weapon::WeaponKind>,
    armor: (u32, bool),
    grenades: brain::economy::GrenadeBag,
}

fn rebuild_menu(world: &mut World) {
    let open = world.resource::<BuyMenu>().open;
    let roots: Vec<Entity> = world.query_filtered::<Entity, With<MenuRoot>>().iter(world).collect();
    if !open {
        for e in roots {
            world.despawn(e);
        }
        return;
    }
    let Some((mut carried, armor)) = player_kit(world) else { return };
    carried.sort();
    carried.dedup();
    let money = world.resource::<PlayerWallet>().0.money;
    let bag = world.resource::<PlayerGrenades>().0;
    let sig = MenuSig { money, carried: carried.clone(), armor: armor.map_or((0, false), |a| (a.points.ceil() as u32, a.helmet && a.points > 0.0)), grenades: bag };
    let old = world.get_resource::<LastSig>().map(|s| s.0.clone());
    if !roots.is_empty() && old.as_ref() == Some(&sig) {
        return;
    }
    for e in roots {
        world.despawn(e);
    }
    world.insert_resource(LastSig(sig));
    let t = world.resource::<Theme>().clone();
    let items = catalog();
    world.commands().spawn((
        MenuRoot,
        Name::new("Buy menu"),
        DespawnOnExit(GameState::Playing),
        HideWhenPaused,
        fullscreen(),
        GlobalZIndex(15),
        BackgroundColor(t.bg.with_alpha(0.55)),
        Pickable::IGNORE,
    ))
    .with_children(|root| {
        root.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                right: px(0),
                top: px(0),
                bottom: px(0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|center| {
            center
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: px(GAP_M),
                        padding: UiRect::all(px(GAP_L)),
                        border: UiRect::top(px(3)),
                        border_radius: BorderRadius::all(px(RADIUS)),
                        width: px(1340),
                        ..default()
                    },
                    BackgroundColor(t.panel.with_alpha(0.96)),
                    BorderColor::all(t.accent),
                    Pickable::IGNORE,
                ))
                .with_children(|panel| {
                    header(panel, &t, money);
                    panel.spawn(rule(t.line));
                    panel.spawn((Node { column_gap: px(GAP_M), align_items: AlignItems::FlexStart, ..default() }, Pickable::IGNORE)).with_children(|cols| {
                        for cat in brain::economy::BuyCategory::ALL {
                            cols.spawn((Node { flex_direction: FlexDirection::Column, row_gap: px(GAP_S), width: px(244), ..default() }, Pickable::IGNORE))
                                .with_children(|col| {
                                    col.spawn(eyebrow(cat.label(), t.text_muted));
                                    for (i, item) in items.iter().enumerate().filter(|(_, it)| it.category() == cat) {
                                        item_card(col, &t, i, *item, quote(*item, &carried, armor, bag), money, bag);
                                    }
                                });
                        }
                    });
                    panel.spawn(rule(t.line));
                    panel.spawn((Node { column_gap: px(GAP_L), align_items: AlignItems::Center, ..default() }, Pickable::IGNORE)).with_children(|f| {
                        key_hint(f, &t, "1-9, 0", "Buy");
                        key_hint(f, &t, "Click", "Buy");
                        key_hint(f, &t, "B / Esc", "Close");
                        f.spawn((MenuNotice, text("", body(SIZE_SMALL), t.bad), KeepColor));
                    });
                });
        });
    });
}

#[derive(Resource)]
struct LastSig(MenuSig);

fn header(p: &mut ChildSpawnerCommands, t: &Theme, money: u32) {
    p.spawn((Node { justify_content: JustifyContent::SpaceBetween, align_items: AlignItems::FlexEnd, ..default() }, Pickable::IGNORE)).with_children(|h| {
        h.spawn((Node { flex_direction: FlexDirection::Column, row_gap: px(GAP_XS), ..default() }, Pickable::IGNORE)).with_children(|l| {
            eyebrow_bar(l, "Buy menu", t.accent);
            l.spawn(title("Loadout", SIZE_H2 + 8.0, t.text));
        });
        h.spawn((Node { flex_direction: FlexDirection::Column, align_items: AlignItems::FlexEnd, row_gap: px(GAP_XS), ..default() }, Pickable::IGNORE))
            .with_children(|r| {
                r.spawn((MenuMoney, text(fmt_money(money), display(52.0), t.good)));
                r.spawn((MenuTimer, text("", label(SIZE_LABEL), t.text_muted)));
            });
    });
}

/// "$16,000".
pub fn fmt_money(m: u32) -> String {
    let s = m.to_string();
    let mut out = String::from("$");
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn item_card(p: &mut ChildSpawnerCommands, t: &Theme, i: usize, item: BuyItem, quote: Result<u32, &'static str>, money: u32, bag: brain::economy::GrenadeBag) {
    let owned = quote.is_err();
    let afford = quote.is_ok_and(|q| q <= money);
    let clear = Color::NONE;
    let look = if afford {
        Look::new([Color::srgba(1.0, 1.0, 1.0, 0.035), t.accent.with_alpha(0.16), clear], [t.line, t.accent.with_alpha(0.85), clear], [t.text, t.text, t.text])
    } else {
        Look::new([Color::srgba(1.0, 1.0, 1.0, 0.015), Color::srgba(1.0, 1.0, 1.0, 0.03), clear], [t.line, t.line, clear], [t.text_dim, t.text_dim, t.text_dim])
    };
    let sub = match item {
        BuyItem::Gun(k) => format!("{}  \u{b7}  KILL +${}", crate::weapon::class_label(k), k.kill_reward()),
        BuyItem::Gear(brain::economy::Gear::Kevlar) => "100 ARMOR  \u{b7}  SOAKS HALF".into(),
        BuyItem::Gear(brain::economy::Gear::KevlarHelmet) => "ARMOR + HEAD COVER".into(),
        BuyItem::Grenade(k) => {
            use brain::grenade::GrenadeKind as G;
            let what = match k {
                G::Frag => "BLAST 6.5 M",
                G::Flash => "BLINDS",
                G::Smoke => "15 S COVER",
            };
            format!("{what}  \u{b7}  {}/{}", bag.count(k), brain::economy::grenade_max(k))
        }
    };
    let (price, price_color) = match quote {
        Err("full") => ("FULL".to_string(), t.good),
        Err(_) => ("OWNED".to_string(), t.good),
        Ok(q) if q <= money => (fmt_money(q), t.accent),
        Ok(q) => (fmt_money(q), t.bad.with_alpha(0.7)),
    };
    let mut e = p.spawn((
        Name::new(format!("Buy {}", item.label())),
        Node {
            column_gap: px(GAP_S + 4.0),
            align_items: AlignItems::Center,
            padding: UiRect::axes(px(12), px(10)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(RADIUS)),
            ..default()
        },
        BackgroundColor(clear),
        BorderColor::all(clear),
        look,
    ));
    if afford {
        e.insert(ActionButton(UiAction::Menu(MenuAction::Custom(item_id(i)))));
    } else {
        e.insert(Pickable::IGNORE);
    }
    e.with_children(|c| {
        let key = key_label(i).unwrap_or(" ");
        c.spawn((
            Node {
                width: px(26),
                height: px(26),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(3)),
                flex_shrink: 0.0,
                ..default()
            },
            BorderColor::all(if afford { t.line_strong } else { t.line }),
            Pickable::IGNORE,
        ))
        .with_child((text(key, label(SIZE_TINY), if afford { t.text } else { t.text_dim }), KeepColor));
        c.spawn((Node { flex_direction: FlexDirection::Column, row_gap: px(2), flex_grow: 1.0, ..default() }, Pickable::IGNORE)).with_children(|m| {
            m.spawn(text(item.label().to_uppercase(), label(22.0), t.text));
            m.spawn((text(sub, label(SIZE_TINY - 2.0), if owned || !afford { t.text_dim } else { t.text_muted }), KeepColor));
        });
        c.spawn((text(price, label(SIZE_LABEL + 2.0), price_color), KeepColor));
    });
}

fn update_texts(
    phase: Res<RoundPhase>,
    notice: Res<BuyNotice>,
    mut timers: Query<&mut Text, (With<MenuTimer>, Without<MenuNotice>)>,
    mut notices: Query<&mut Text, (With<MenuNotice>, Without<MenuTimer>)>,
) {
    let s = if phase.frozen() {
        format!("FREEZE TIME  {}", clock(phase.freeze_left))
    } else {
        format!("BUY ZONE  {}", clock(phase.buy_left))
    };
    for mut t in &mut timers {
        if t.0 != s {
            t.0 = s.clone();
        }
    }
    let n = notice.0.as_ref().map_or(String::new(), |(s, _)| s.clone());
    for mut t in &mut notices {
        if t.0 != n {
            t.0 = n.clone();
        }
    }
}

fn clock(s: f32) -> String {
    let s = s.max(0.0).ceil() as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

// ---- money HUD ----

#[derive(Component)]
struct MoneyText;
#[derive(Component)]
struct MoneyDelta;
#[derive(Component)]
struct BuyHint;

fn spawn_money_hud(mut commands: Commands) {
    commands
        .spawn((
            Name::new("Money"),
            HideWhenPaused,
            DespawnOnExit(GameState::Playing),
            Node {
                position_type: PositionType::Absolute,
                // Top-left under the squad line (bottom-left belongs to callouts and armor).
                left: px(40),
                top: px(66),
                align_items: AlignItems::Baseline,
                column_gap: px(GAP_S + 2.0),
                padding: UiRect::axes(px(10), px(2)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.03, 0.04, 0.06, 0.6)),
            Pickable::IGNORE,
        ))
        .with_children(|b| {
            b.spawn((MoneyText, text("$0", display(34.0), FRIENDLY)));
            b.spawn((MoneyDelta, text("", label(SIZE_LABEL), ACCENT)));
        });
    // Bottom centre: how to buy, while buying is possible.
    commands
        .spawn((
            BuyHint,
            HideWhenPaused,
            DespawnOnExit(GameState::Playing),
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                right: px(0),
                bottom: px(150),
                justify_content: JustifyContent::Center,
                display: Display::None,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|r| {
            r.spawn((
                Node { column_gap: px(GAP_S), align_items: AlignItems::Center, padding: UiRect::axes(px(14), px(6)), ..default() },
                BackgroundColor(Color::srgba(0.03, 0.04, 0.06, 0.62)),
                Pickable::IGNORE,
            ))
            .with_children(|row| {
                row.spawn((
                    Node { padding: UiRect::axes(px(7), px(2)), border: UiRect::all(px(1)), border_radius: BorderRadius::all(px(3)), ..default() },
                    BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.18)),
                    Pickable::IGNORE,
                    children![text("B", label(SIZE_TINY), TEXT)],
                ));
                row.spawn(text("BUY MENU", label(SIZE_LABEL), TEXT_MUTED));
            });
        });
}

fn money_hud(
    time: Res<Time>,
    wallet: Res<PlayerWallet>,
    menu: Res<BuyMenu>,
    phase: Res<RoundPhase>,
    flow: Res<MatchFlow>,
    mut money: Query<&mut Text, (With<MoneyText>, Without<MoneyDelta>)>,
    mut delta: Query<(&mut Text, &mut TextColor), (With<MoneyDelta>, Without<MoneyText>)>,
    mut hint: Query<&mut Node, With<BuyHint>>,
    mut last: Local<Option<(u32, u32, u32)>>,
    mut delta_t: Local<f32>,
) {
    let m = wallet.0.money;
    let key = (flow.match_no, flow.round);
    match *last {
        Some((lm, mn, r)) if (mn, r) == key && lm != m => {
            if let Ok((mut t, mut c)) = delta.single_mut() {
                let (s, col) = if m > lm { (format!("+{}", fmt_money(m - lm)), ACCENT) } else { (format!("-{}", fmt_money(lm - m)), TEXT_MUTED) };
                t.0 = s;
                c.0 = col;
            }
            *delta_t = 2.5;
        }
        _ => {}
    }
    *last = Some((m, key.0, key.1));
    if *delta_t > 0.0 {
        *delta_t -= time.delta_secs();
        if *delta_t <= 0.0
            && let Ok((mut t, _)) = delta.single_mut()
        {
            t.0.clear();
        }
    }
    let s = fmt_money(m);
    for mut t in &mut money {
        if t.0 != s {
            t.0 = s.clone();
        }
    }
    // The hint shows while buying is possible in principle (freeze or the buy window).
    let show = !menu.open && flow.round_over.is_none() && (phase.frozen() || phase.buy_left > 0.0);
    let d = if show { Display::Flex } else { Display::None };
    for mut n in &mut hint {
        if n.display != d {
            n.display = d;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn money_format() {
        assert_eq!(super::fmt_money(0), "$0");
        assert_eq!(super::fmt_money(800), "$800");
        assert_eq!(super::fmt_money(3250), "$3,250");
        assert_eq!(super::fmt_money(16000), "$16,000");
    }

    #[test]
    fn item_ids_are_stable() {
        assert_eq!(super::item_id(3), "buy.3");
        assert!(std::ptr::eq(super::item_id(3), super::item_id(3)));
    }
}
