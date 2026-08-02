//! Bottom-left complaint feed panel.

use bevy::prelude::*;
use rail_sim::ComplaintFeed;

use crate::palette::{BG1, OUTLINE};
use crate::ui::kit::{body_font, text_primary, text_warn, SPACE_2, SPACE_3};

#[derive(Component)]
pub struct ComplaintFeedRoot;

#[derive(Component)]
pub struct ComplaintFeedText;

/// Last body string painted — skip rewrite when unchanged.
#[derive(Resource, Debug, Default)]
pub(crate) struct ComplaintFeedCache {
    body: String,
}

pub fn setup_complaint_feed_ui(mut commands: Commands) {
    commands.insert_resource(ComplaintFeedCache {
        body: "Town is quiet…".into(),
    });
    commands
        .spawn((
            ComplaintFeedRoot,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(SPACE_3 + 52.0),
                left: Val::Px(SPACE_3),
                width: Val::Px(320.0),
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
            parent.spawn((Text::new("Complaints"), body_font(), text_warn()));
            parent.spawn((
                ComplaintFeedText,
                Text::new("Town is quiet…"),
                body_font(),
                text_primary(),
            ));
        });
}

pub fn update_complaint_feed_ui(
    feed: Res<ComplaintFeed>,
    mut cache: ResMut<ComplaintFeedCache>,
    mut text_q: Query<&mut Text, With<ComplaintFeedText>>,
) {
    let body = if feed.is_empty() {
        "Town is quiet…".to_string()
    } else {
        feed.iter()
            .take(5)
            .map(|e| e.display_line())
            .collect::<Vec<_>>()
            .join("\n")
    };
    if body == cache.body {
        return;
    }
    cache.body = body.clone();
    if let Ok(mut text) = text_q.single_mut() {
        *text = Text::new(body);
    }
}
