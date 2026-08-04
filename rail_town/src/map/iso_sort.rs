//! The one system that y-sorts the isometric world.
//!
//! Roughly twenty spawners across the presentation crate write a layer z and
//! then never think about depth again. Editing all of them is most of the diff
//! and none of the evaluation, so instead this runs once in `PostUpdate` — after
//! everything has moved, before transforms propagate — and rewrites z for every
//! root world sprite from the tile it is standing on.
//!
//! # Telling a layer from a sort
//!
//! The trick is that a fresh layer z and a sorted z live in different ranges
//! ([`BAND_FLOOR`]). A sprite whose z is *below* the floor was written this
//! frame by a gameplay system, so that value is its layer and gets remembered in
//! [`IsoLayer`]. A sprite whose z is above the floor and already carries an
//! [`IsoLayer`] is one of ours, holding still. A sprite above the floor with no
//! [`IsoLayer`] is an overlay that deliberately lives above the world — the
//! time-of-day tint, the Map View plate — and is left alone.
//!
//! Children keep their parent-relative z: a train's stop indicator and a track
//! piece's polish layer are offsets, not positions.

use bevy::prelude::*;

use super::iso_depth::{depth_z_at, BAND_FLOOR};
use super::terrain::iso::IsoTerrain;

/// Every root world sprite: it has art, it is not a child (children hold a
/// parent-relative offset), and it is not terrain (which sorts itself at spawn).
type WorldSprites<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static mut Transform,
        Option<&'static mut IsoLayer>,
    ),
    (With<Sprite>, Without<ChildOf>, Without<IsoTerrain>),
>;

/// The layer z a sprite was spawned with, kept so re-sorting is idempotent.
#[derive(Component, Debug, Clone, Copy)]
pub struct IsoLayer(pub f32);

/// Rewrite z for every world sprite from the tile under it.
pub fn iso_depth_sort(mut commands: Commands, mut sprites: WorldSprites) {
    let _perf = crate::overlays::perf::scope("iso_depth_sort");
    for (entity, mut transform, layer) in &mut sprites {
        let raw = transform.translation.z;
        let fresh = raw < BAND_FLOOR;
        let layer_z = match (&layer, fresh) {
            (_, true) => raw,
            (Some(known), false) => known.0,
            // Above the band and never adopted: an overlay, not the world.
            (None, false) => continue,
        };
        match layer {
            Some(mut known) => known.0 = layer_z,
            None => {
                commands.entity(entity).insert(IsoLayer(layer_z));
            }
        }
        transform.translation.z = depth_z_at(transform.translation.truncate(), layer_z);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::iso_depth::depth_z;
    use rail_map::tile_to_world;
    use rail_sim::ids::TileCoord;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_systems(Update, iso_depth_sort);
        app
    }

    fn z_of(app: &mut App, e: Entity) -> f32 {
        app.world()
            .entity(e)
            .get::<Transform>()
            .unwrap()
            .translation
            .z
    }

    #[test]
    fn a_world_sprite_is_sorted_by_the_tile_under_it() {
        let mut app = app();
        let near = TileCoord { x: 4, y: 4 };
        let far = TileCoord { x: 8, y: 8 };
        let mut spawn = |t: TileCoord, layer: f32| {
            let (x, y) = tile_to_world(t);
            app.world_mut()
                .spawn((Sprite::default(), Transform::from_xyz(x, y, layer)))
                .id()
        };
        let train_far = spawn(far, 3.0);
        let track_near = spawn(near, 1.0);
        app.update();

        assert_eq!(z_of(&mut app, track_near), depth_z(8, 1.0));
        assert_eq!(z_of(&mut app, train_far), depth_z(16, 3.0));
        assert!(
            z_of(&mut app, track_near) > z_of(&mut app, train_far),
            "the nearer row must draw over the further one, layer or not"
        );
    }

    /// Running twice must not walk z off into the distance.
    #[test]
    fn sorting_is_idempotent() {
        let mut app = app();
        let (x, y) = tile_to_world(TileCoord { x: 3, y: 7 });
        let e = app
            .world_mut()
            .spawn((Sprite::default(), Transform::from_xyz(x, y, 2.0)))
            .id();
        app.update();
        let once = z_of(&mut app, e);
        for _ in 0..5 {
            app.update();
        }
        assert_eq!(z_of(&mut app, e), once);
        assert_eq!(app.world().entity(e).get::<IsoLayer>().unwrap().0, 2.0);
    }

    /// A system that rewrites its own layer z every frame (trains do) must keep
    /// being honoured.
    #[test]
    fn a_freshly_written_layer_is_picked_up_again() {
        let mut app = app();
        let (x, y) = tile_to_world(TileCoord { x: 2, y: 2 });
        let e = app
            .world_mut()
            .spawn((Sprite::default(), Transform::from_xyz(x, y, 1.0)))
            .id();
        app.update();
        assert_eq!(z_of(&mut app, e), depth_z(4, 1.0));

        app.world_mut()
            .entity_mut(e)
            .get_mut::<Transform>()
            .unwrap()
            .translation
            .z = 4.5;
        app.update();
        assert_eq!(z_of(&mut app, e), depth_z(4, 4.5));
    }

    /// The day tint and the Map View plate sit above the world on purpose.
    #[test]
    fn an_overlay_above_the_band_is_left_alone() {
        let mut app = app();
        let tint = app
            .world_mut()
            .spawn((Sprite::default(), Transform::from_xyz(0.0, 0.0, 64.0)))
            .id();
        let plate = app
            .world_mut()
            .spawn((Sprite::default(), Transform::from_xyz(0.0, 0.0, 200.0)))
            .id();
        app.update();
        assert_eq!(z_of(&mut app, tint), 64.0);
        assert_eq!(z_of(&mut app, plate), 200.0);
        assert!(app.world().entity(tint).get::<IsoLayer>().is_none());
    }
}
