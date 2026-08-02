//! Bottom-left complaint feed panel.

use bevy::prelude::*;
use rail_sim::ComplaintFeed;

#[derive(Component)]
pub struct ComplaintFeedRoot;

#[derive(Component)]
pub struct ComplaintFeedText;

pub fn setup_complaint_feed_ui(mut commands: Commands) {
    commands
        .spawn((
            ComplaintFeedRoot,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(12.0),
                left: Val::Px(12.0),
                width: Val::Px(360.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                padding: UiRect::all(Val::Px(8.0)),
                border_radius: BorderRadius::all(Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.08, 0.05, 0.05, 0.75)),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Complaints"),
                TextFont::from_font_size(14.0),
                TextColor(Color::srgb(0.95, 0.75, 0.7)),
            ));
            parent.spawn((
                ComplaintFeedText,
                Text::new("Town is quiet…"),
                TextFont::from_font_size(13.0),
                TextColor(Color::srgb(0.88, 0.82, 0.8)),
            ));
        });
}

pub fn update_complaint_feed_ui(
    feed: Res<ComplaintFeed>,
    mut text_q: Query<&mut Text, With<ComplaintFeedText>>,
) {
    let Ok(mut text) = text_q.single_mut() else {
        return;
    };
    if feed.is_empty() {
        *text = Text::new("Town is quiet…");
        return;
    }
    let body: String = feed
        .iter()
        .take(5)
        .map(|e| e.display_line())
        .collect::<Vec<_>>()
        .join("\n");
    *text = Text::new(body);
}
