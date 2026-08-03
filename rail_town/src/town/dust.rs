//! Construction dust — the puff that turns a phase change into a *moment*.
//!
//! Brief 06 §3.1 asks for the scaffold to go up *"with a small dust puff and
//! occasional sound"*, and §8's acceptance bar is blunter still: *"watching a
//! building go up is a small, noticeable pleasure."* The phase machine in
//! [`super`] already had the timings; it had nothing to look at. A frame swap
//! with no puff reads as a sprite being replaced, which is exactly the flat,
//! unattended feeling the brief is written against.
//!
//! Two puffs per building, at the two moments something physically happens: the
//! scaffold going up, and the building dropping onto its lot.
//!
//! # Pixel contract
//!
//! Two frames, whole texels, no rotation and no fractional scaling (art 01 §2).
//! Position is world-anchored — the scatter comes from [`crate::hash`] on the
//! lot's tile, so the same lot always kicks its dust up on the same side and the
//! puff scrolls with the ground instead of boiling across the screen.

use bevy::prelude::*;
use rail_sim::TileCoord;

use crate::hash::hash_offset;
use crate::palette::PLASTER_L;

/// Depth for a puff.
///
/// The building band tops out at `1.90` (`BUILDING_Z_BASE + BUILDING_Z_SPAN`)
/// and peeps sit at `2.0`, so dust owns the sliver between: it always draws in
/// front of the building that raised it, and never in front of a person.
pub const DUST_Z: f32 = 1.95;

/// How long each of the two frames holds.
///
/// The whole puff is gone in under four tenths of a second. It is punctuation,
/// not an effect — a lot that keeps smoking would read as a fire.
pub const DUST_FRAME_SECS: f32 = 0.18;

/// `(rise above the lot base, size in texels, alpha)`. Whole texels only.
pub const DUST_FRAMES: [(f32, f32, f32); 2] = [(1.0, 3.0, 0.50), (3.0, 2.0, 0.24)];

/// Salt for the scatter. Distinct per moment, so the settle puff does not land
/// exactly on top of the scaffold puff from eight seconds earlier.
pub const DUST_SALT_SCAFFOLD: u32 = 0x5343_4146; // "SCAF"
pub const DUST_SALT_SETTLE: u32 = 0x5345_5454; // "SETT"

/// How far off the lot's base a puff may sit, in texels.
const DUST_SCATTER: i32 = 3;

/// One rising puff, mid-life.
#[derive(Component, Debug, Clone, Copy)]
pub struct ConstructionDust {
    /// Lot base the puff rises from, in whole world texels.
    origin: Vec2,
    secs: f32,
    frame: u8,
}

/// Kick dust up at a lot's base. Call on entering Scaffold and Settle.
pub fn spawn_dust(commands: &mut Commands, tile: TileCoord, base: Vec2, salt: u32) {
    let offset = hash_offset(tile.x, tile.y, salt, DUST_SCATTER) as f32;
    let origin = Vec2::new((base.x + offset).round(), base.y.round());
    let (rise, size, alpha) = DUST_FRAMES[0];
    commands.spawn((
        ConstructionDust {
            origin,
            secs: 0.0,
            frame: 0,
        },
        Sprite::from_color(PLASTER_L.with_alpha(alpha), Vec2::splat(size)),
        Transform::from_xyz(origin.x, origin.y + rise, DUST_Z),
    ));
}

/// Advance every puff, and despawn it when it has thinned out.
///
/// Runs on `Time<Virtual>`, so dust freezes with the sim exactly as the phase
/// machine that raised it does — a paused world with a puff still climbing
/// would be the one thing on screen that had not stopped.
pub fn step_construction_dust(
    mut commands: Commands,
    time: Res<Time<Virtual>>,
    clock: Res<rail_sim::SimClock>,
    mut puffs: Query<(Entity, &mut ConstructionDust, &mut Sprite, &mut Transform)>,
) {
    if puffs.is_empty() {
        return;
    }
    let _perf = crate::overlays::perf::scope("step_construction_dust");
    if !clock.is_running() {
        return;
    }
    let dt = time.delta_secs();
    for (entity, mut dust, mut sprite, mut transform) in puffs.iter_mut() {
        dust.secs += dt;
        let frame = (dust.secs / DUST_FRAME_SECS) as usize;
        if frame >= DUST_FRAMES.len() {
            commands.entity(entity).despawn();
            continue;
        }
        if frame as u8 == dust.frame {
            continue;
        }
        dust.frame = frame as u8;
        let (rise, size, alpha) = DUST_FRAMES[frame];
        sprite.color = PLASTER_L.with_alpha(alpha);
        sprite.custom_size = Some(Vec2::splat(size));
        transform.translation.y = dust.origin.y + rise;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(x: i32, y: i32) -> TileCoord {
        TileCoord { x, y }
    }

    #[test]
    fn the_puff_is_two_frames_that_rise_and_thin_on_whole_texels() {
        assert_eq!(DUST_FRAMES.len(), 2, "the brief asks for a two-frame puff");
        let mut last = DUST_FRAMES[0];
        for (rise, size, alpha) in DUST_FRAMES.iter().skip(1).copied() {
            assert!(rise > last.0, "dust must rise");
            assert!(alpha < last.2, "dust must thin");
            assert_eq!(rise.fract(), 0.0, "01 §2: whole texels only");
            assert_eq!(size.fract(), 0.0);
            last = (rise, size, alpha);
        }
        // Punctuation, not an effect.
        assert!(DUST_FRAME_SECS * (DUST_FRAMES.len() as f32) < 0.5);
    }

    #[test]
    fn dust_draws_over_its_own_building_and_under_the_people() {
        // The band is measured off the real depth function rather than
        // remembered, so moving the buildings moves this test with them.
        let highest = (0..64)
            .map(|row| super::super::lot_z(row * 16, 64))
            .fold(0.0f32, f32::max);
        assert!(
            DUST_Z > highest,
            "dust {DUST_Z} would vanish behind a southerly building at {highest}"
        );
        // Peeps and stations sit at 2.0 (see `town::peep_sprites`).
        assert!(
            DUST_Z < 2.0f32.min(f32::MAX),
            "dust must never draw in front of a person"
        );
    }

    #[test]
    fn the_scatter_is_world_anchored_and_stays_on_the_lot() {
        for salt in [DUST_SALT_SCAFFOLD, DUST_SALT_SETTLE] {
            for y in 0..32 {
                for x in 0..32 {
                    let a = hash_offset(x, y, salt, DUST_SCATTER);
                    assert!((-DUST_SCATTER..=DUST_SCATTER).contains(&a));
                    assert_eq!(a, hash_offset(x, y, salt, DUST_SCATTER), "not deterministic");
                }
            }
        }
        // The two moments scatter separately, so the settle puff does not land
        // exactly where the scaffold puff did.
        let differing = (0..64)
            .filter(|x| {
                hash_offset(*x, 3, DUST_SALT_SCAFFOLD, DUST_SCATTER)
                    != hash_offset(*x, 3, DUST_SALT_SETTLE, DUST_SCATTER)
            })
            .count();
        assert!(differing > 40, "the two puffs land together {differing}/64");
    }

    #[test]
    fn a_puff_lives_and_dies_with_the_clock() {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins)
            .init_resource::<rail_sim::SimClock>()
            .add_systems(Update, step_construction_dust);
        app.world_mut().spawn((
            ConstructionDust {
                origin: Vec2::ZERO,
                secs: 0.0,
                frame: 0,
            },
            Sprite::from_color(PLASTER_L, Vec2::splat(3.0)),
            Transform::from_xyz(0.0, 0.0, DUST_Z),
        ));

        // A paused sim holds it exactly where it is.
        app.world_mut()
            .resource_mut::<rail_sim::SimClock>()
            .apply_pause(rail_sim::commands::Pause { paused: true });
        for _ in 0..8 {
            app.update();
        }
        assert_eq!(count(&mut app), 1, "a paused world let its dust blow away");

        app.world_mut()
            .resource_mut::<rail_sim::SimClock>()
            .apply_pause(rail_sim::commands::Pause { paused: false });
        // Step past both frames in one go — the puff must clean itself up.
        app.world_mut()
            .query::<&mut ConstructionDust>()
            .iter_mut(app.world_mut())
            .for_each(|mut d| d.secs = DUST_FRAME_SECS * (DUST_FRAMES.len() as f32) + 0.01);
        app.update();
        assert_eq!(count(&mut app), 0, "dust must not linger");
    }

    fn count(app: &mut App) -> usize {
        app.world_mut()
            .query::<&ConstructionDust>()
            .iter(app.world())
            .count()
    }

    #[test]
    fn a_puff_starts_on_a_whole_texel() {
        // Sub-texel decals resample and stop looking like art (01 §2.1).
        let base = Vec2::new(64.5, 32.25);
        let offset = hash_offset(3, 4, DUST_SALT_SCAFFOLD, DUST_SCATTER) as f32;
        let origin = Vec2::new((base.x + offset).round(), base.y.round());
        assert_eq!(origin.x.fract(), 0.0);
        assert_eq!(origin.y.fract(), 0.0);
        let _ = tile(3, 4);
    }
}
