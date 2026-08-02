//! Town Talk ticker — bottom-left living feed with click-to-locate.

use bevy::prelude::*;
use rail_map::tile_to_world;
use rail_sim::{ComplaintFeed, StationService, TalkKind};

use crate::inspect::{Selectable, Selection};
use crate::map::CameraFocusRequest;
use crate::palette::{BALLAST_L, BG1, HI, OK, OUTLINE, RAIL_L, WARN};
use crate::ui::kit::{body_font, micro_font, SPACE_2, SPACE_3};

/// How many ticker rows are visible at once (design: up to four).
pub const TOWN_TALK_VISIBLE: usize = 4;

#[derive(Component)]
pub struct TownTalkRoot;

#[derive(Component)]
pub(crate) struct TownTalkRow {
    index: usize,
}

#[derive(Component)]
pub(crate) struct TownTalkLineText;

#[derive(Component)]
pub(crate) struct TownTalkAgeText;

#[derive(Component)]
pub(crate) struct TownTalkMoodIcon;

/// Last signature painted — skip rewrite when unchanged.
#[derive(Resource, Debug, Default)]
pub(crate) struct TownTalkCache {
    signature: String,
}

pub fn setup_town_talk_ui(mut commands: Commands) {
    commands.insert_resource(TownTalkCache::default());
    commands
        .spawn((
            TownTalkRoot,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(SPACE_3 + 52.0),
                left: Val::Px(SPACE_3),
                width: Val::Px(340.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                padding: UiRect::all(Val::Px(SPACE_2)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::ZERO,
                ..default()
            },
            BackgroundColor(BG1),
            BorderColor::all(OUTLINE),
            ZIndex(5),
        ))
        .with_children(|parent| {
            parent.spawn((Text::new("Town Talk"), body_font(), TextColor(HI)));
            for i in 0..TOWN_TALK_VISIBLE {
                parent
                    .spawn((
                        Button,
                        TownTalkRow { index: i },
                        Node {
                            width: Val::Percent(100.0),
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(6.0),
                            align_items: AlignItems::Center,
                            padding: UiRect::axes(Val::Px(4.0), Val::Px(2.0)),
                            border: UiRect::all(Val::Px(1.0)),
                            border_radius: BorderRadius::ZERO,
                            ..default()
                        },
                        BackgroundColor(BG1),
                        BorderColor::all(OUTLINE),
                    ))
                    .with_children(|row| {
                        row.spawn((
                            TownTalkMoodIcon,
                            Text::new("-"),
                            micro_font(),
                            TextColor(BALLAST_L),
                        ));
                        row.spawn((
                            TownTalkLineText,
                            Text::new(if i == 0 { "Town is quiet…" } else { "" }),
                            micro_font(),
                            TextColor(RAIL_L),
                            Node {
                                flex_grow: 1.0,
                                ..default()
                            },
                        ));
                        row.spawn((
                            TownTalkAgeText,
                            Text::new(""),
                            micro_font(),
                            TextColor(BALLAST_L),
                        ));
                    });
            }
        });
}

pub fn refresh_town_talk_rows(
    feed: Res<ComplaintFeed>,
    service: Res<StationService>,
    mut cache: ResMut<TownTalkCache>,
    mut rows: Query<(Entity, &TownTalkRow, &Children, &mut Visibility, &mut BorderColor)>,
    mut text_q: Query<(
        Option<&TownTalkLineText>,
        Option<&TownTalkAgeText>,
        Option<&TownTalkMoodIcon>,
        &mut Text,
        &mut TextColor,
    )>,
) {
    let now = service.tick;
    let signature: String = feed
        .iter()
        .take(TOWN_TALK_VISIBLE)
        .map(|e| {
            format!(
                "{}#{}@{}:{}",
                e.display_line(),
                e.count,
                e.sim_tick,
                e.age_label(now)
            )
        })
        .collect::<Vec<_>>()
        .join("|");
    if signature == cache.signature {
        return;
    }
    cache.signature = signature;

    let mut row_list: Vec<_> = rows.iter_mut().collect();
    row_list.sort_by_key(|(_, r, _, _, _)| r.index);

    for (_entity, row, children, vis, border) in &mut row_list {
        let entry = feed.get(row.index);
        match entry {
            None if row.index == 0 && feed.is_empty() => {
                **vis = Visibility::Visible;
                **border = BorderColor::all(OUTLINE);
                for child in children.iter() {
                    write_row_child(
                        &mut text_q,
                        child,
                        Some(("Town is quiet…", BALLAST_L)),
                        Some(""),
                        Some(("-", BALLAST_L)),
                    );
                }
            }
            None => {
                **vis = Visibility::Hidden;
            }
            Some(entry) => {
                **vis = Visibility::Visible;
                let (icon_ch, accent) = match entry.kind {
                    TalkKind::Complaint => ("x", WARN),
                    TalkKind::Praise => ("+", OK),
                    TalkKind::Opportunity => ("*", HI),
                    TalkKind::Warning => ("!", WARN),
                };
                **border = BorderColor::all(OUTLINE);
                let line = entry.display_line();
                let age = entry.age_label(now);
                for child in children.iter() {
                    write_row_child(
                        &mut text_q,
                        child,
                        Some((line.as_str(), RAIL_L)),
                        Some(age.as_str()),
                        Some((icon_ch, accent)),
                    );
                }
            }
        }
    }
}

fn write_row_child(
    text_q: &mut Query<(
        Option<&TownTalkLineText>,
        Option<&TownTalkAgeText>,
        Option<&TownTalkMoodIcon>,
        &mut Text,
        &mut TextColor,
    )>,
    child: Entity,
    line: Option<(&str, Color)>,
    age: Option<&str>,
    icon: Option<(&str, Color)>,
) {
    let Ok((is_line, is_age, is_icon, mut text, mut color)) = text_q.get_mut(child) else {
        return;
    };
    if is_line.is_some() {
        if let Some((s, c)) = line {
            *text = Text::new(s);
            *color = TextColor(c);
        }
    } else if is_age.is_some() {
        if let Some(s) = age {
            *text = Text::new(s);
            *color = TextColor(BALLAST_L);
        }
    } else if is_icon.is_some() {
        if let Some((s, c)) = icon {
            *text = Text::new(s);
            *color = TextColor(c);
        }
    }
}

pub fn town_talk_clicks(
    feed: Res<ComplaintFeed>,
    interactions: Query<(&Interaction, &TownTalkRow), Changed<Interaction>>,
    mut selection: ResMut<Selection>,
    mut focus: ResMut<CameraFocusRequest>,
) {
    for (interaction, row) in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Some(entry) = feed.get(row.index) else {
            continue;
        };

        if let Some(peep_id) = entry.peep_id {
            selection.set(Selectable::Peep(peep_id));
        } else if let Some(station_id) = entry.station_id {
            selection.set(Selectable::Station(station_id));
        }

        if let Some(tile) = entry.tile {
            let (x, y) = tile_to_world(tile);
            focus.0 = Some(Vec2::new(x, y));
        }
    }
}
