//! Click picking and priority for world selection.

use rail_sim::{IndustryId, PeepId, StationId, TrackId, TrainId};

/// What a world click can select (Phase B slice).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Selectable {
    Peep(PeepId),
    Train(TrainId),
    Station(StationId),
    Industry(IndustryId),
    Track(TrackId),
}

/// Priority when several candidates overlap the same click.
/// Lower index wins: peep > train > station > industry > track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PickPriority {
    Peep = 0,
    Train = 1,
    Station = 2,
    Industry = 3,
    Track = 4,
}

impl Selectable {
    pub fn priority(self) -> PickPriority {
        match self {
            Self::Peep(_) => PickPriority::Peep,
            Self::Train(_) => PickPriority::Train,
            Self::Station(_) => PickPriority::Station,
            Self::Industry(_) => PickPriority::Industry,
            Self::Track(_) => PickPriority::Track,
        }
    }
}

/// Choose the highest-priority selectable from a candidate list.
pub fn resolve_pick(candidates: &[Selectable]) -> Option<Selectable> {
    candidates
        .iter()
        .copied()
        .min_by_key(|c| c.priority())
}

/// Axis-aligned hit test in world space (sprite-centered).
pub fn point_hits_sprite(point: bevy::math::Vec2, center: bevy::math::Vec2, size: bevy::math::Vec2) -> bool {
    let half = size * 0.5;
    point.x >= center.x - half.x
        && point.x <= center.x + half.x
        && point.y >= center.y - half.y
        && point.y <= center.y + half.y
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_priority_peep_over_train_over_station() {
        let peep = Selectable::Peep(PeepId(1));
        let train = Selectable::Train(TrainId(2));
        let station = Selectable::Station(StationId(3));
        let industry = Selectable::Industry(IndustryId(4));
        let track = Selectable::Track(TrackId(5));

        assert_eq!(
            resolve_pick(&[track, industry, station, train, peep]),
            Some(peep)
        );
        assert_eq!(resolve_pick(&[track, station, train]), Some(train));
        assert_eq!(resolve_pick(&[track, industry, station]), Some(station));
        assert_eq!(resolve_pick(&[track, industry]), Some(industry));
        assert_eq!(resolve_pick(&[track]), Some(track));
        assert_eq!(resolve_pick(&[]), None);
    }

    #[test]
    fn sprite_hit_is_inclusive_aabb() {
        use bevy::math::Vec2;
        let c = Vec2::new(16.0, 16.0);
        let size = Vec2::new(10.0, 10.0);
        assert!(point_hits_sprite(Vec2::new(16.0, 16.0), c, size));
        assert!(point_hits_sprite(Vec2::new(11.0, 11.0), c, size));
        assert!(!point_hits_sprite(Vec2::new(10.0, 16.0), c, size));
    }
}
