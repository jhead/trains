//! Screenshot harness for the isometric evaluation prototype. **Not shippable.**
//!
//! The whole point of this branch is that the owner looks at it, so it needs a
//! way to produce a picture without anyone sitting at the keyboard. Everything
//! here is off unless the environment asks for it:
//!
//! ```text
//! RAIL_TOWN_SHOT=/tmp/iso.png cargo run
//! RAIL_TOWN_SHOT=/tmp/iso.png RAIL_TOWN_SHOT_FRAME=400 RAIL_TOWN_SHOT_ZOOM=3 cargo run
//! ```
//!
//! It enters `Playing` (the same transition Begin makes, so the boot demo's
//! track and train come with it), waits `RAIL_TOWN_SHOT_FRAME` frames for the
//! world to settle, writes a PNG and quits.
//!
//! `RAIL_TOWN_SHOT_PICK=px,py` additionally runs one screen point through the
//! whole picking path the mouse uses — `viewport_to_world_2d`, then
//! [`rail_map::world_to_tile`] — and drops a marker on the tile that came back.
//! If the marker is under the crosshair in the shot, the cursor resolves to the
//! tile a player would say they were pointing at.

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};

use crate::shell::ShellState;

#[derive(Resource)]
struct ShotPlan {
    path: PathBuf,
    at_frame: u32,
    zoom: Option<u8>,
    /// A viewport point to resolve to a tile and mark, for picking checks.
    pick: Option<Vec2>,
    frame: u32,
    taken: bool,
}

fn env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

pub struct IsoShotPlugin;

impl Plugin for IsoShotPlugin {
    fn build(&self, app: &mut App) {
        let Ok(path) = std::env::var("RAIL_TOWN_SHOT") else {
            return;
        };
        app.insert_resource(ShotPlan {
            path: PathBuf::from(path),
            at_frame: env_u32("RAIL_TOWN_SHOT_FRAME", 240),
            zoom: std::env::var("RAIL_TOWN_SHOT_ZOOM")
                .ok()
                .and_then(|v| v.parse().ok()),
            pick: std::env::var("RAIL_TOWN_SHOT_PICK").ok().and_then(|v| {
                let (a, b) = v.split_once(',')?;
                Some(Vec2::new(a.trim().parse().ok()?, b.trim().parse().ok()?))
            }),
            frame: 0,
            taken: false,
        })
        .add_systems(Update, drive_shot);
    }
}

#[allow(clippy::too_many_arguments)] // A throwaway harness, not a system to keep.
fn drive_shot(
    mut commands: Commands,
    mut plan: ResMut<ShotPlan>,
    state: Res<State<ShellState>>,
    mut next: ResMut<NextState<ShellState>>,
    mut exit: MessageWriter<AppExit>,
    mut camera: Query<&mut Projection, With<crate::map::MapCamera>>,
    view: Query<(&Camera, &GlobalTransform), With<crate::map::MapCamera>>,
    diamond: Option<Res<crate::map::IsoDiamond>>,
) {
    plan.frame += 1;

    // Skip the title after a moment: the demo world needs a few frames to lay
    // its track and put a train on it.
    if plan.frame == 30 && *state.get() == ShellState::Title {
        next.set(ShellState::Playing);
    }
    if let (Some(zoom), true) = (plan.zoom, plan.frame == 60) {
        if let Ok(mut projection) = camera.single_mut() {
            if let Projection::Orthographic(ortho) = projection.as_mut() {
                ortho.scale = crate::map::ortho_scale_for_zoom(zoom);
            }
        }
    }
    // One frame before the shot, resolve the probe point and mark the tile.
    if let (Some(point), true) = (plan.pick, plan.frame + 1 == plan.at_frame) {
        if let (Ok((camera, cam_gt)), Some(diamond)) = (view.single(), diamond.as_ref()) {
            if let Ok(world) = camera.viewport_to_world_2d(cam_gt, point) {
                let tile = rail_map::world_to_tile(world.x, world.y);
                let (tx, ty) = rail_map::tile_to_world(tile);
                info!(
                    "iso pick: screen {point:?} -> world ({:.1}, {:.1}) -> tile {tile:?} \
                     -> back to ({tx:.1}, {ty:.1})",
                    world.x, world.y
                );
                // A magenta diamond on the answer, and a small dot on the exact
                // screen point asked about.
                commands.spawn((
                    diamond.sprite(Color::srgb(1.0, 0.0, 1.0).with_alpha(0.75), 1.0),
                    Transform::from_xyz(tx, ty, 8.0),
                ));
                commands.spawn((
                    Sprite::from_color(Color::WHITE, Vec2::splat(3.0)),
                    Transform::from_xyz(world.x, world.y, 300.0),
                ));
            }
        }
    }

    if plan.frame == plan.at_frame && !plan.taken {
        plan.taken = true;
        let path = plan.path.clone();
        info!("iso shot: writing {}", path.display());
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
    }
    // The observer writes on a later frame, so leave it room.
    if plan.taken && plan.frame > plan.at_frame + 30 {
        exit.write(AppExit::Success);
    }
}
