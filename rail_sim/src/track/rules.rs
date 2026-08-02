//! Placement validation: bounds, ground layer, water / bridge span, grade, terrain.

use crate::ids::TileCoord;

use super::cost::{
    local_slope, tile_build_cost, GROUND_LAYER, MAX_BRIDGE_SPAN, MAX_GRADE, MOUNTAIN_HEIGHT_MIN,
};
use super::dir::{bearing_separation_deg, step, TrackLinks, DIR_COUNT};
use super::network::TrackNetwork;
use super::terrain::TrackTerrain;

/// Minimum divergence between two legs of a turnout, in tenths of a degree.
///
/// Brief 01 §5.2: *"Minimum turnout divergence is one direction step; anything
/// shallower is refused at placement time, because it cannot be drawn."* One
/// step of the sixteen-point rose is 22.5°, and the `junction` plate puts the
/// hard floor near 10° — below that the flangeway that defines a frog is half a
/// pixel and the turnout dies permanently.
///
/// The threshold is in degrees rather than rose steps on purpose. The realised
/// rose is knight's moves on a square lattice, so its steps are 26.57° and
/// 18.43° alternately, not an even 22.5°; a step count would call both of those
/// "one step" and let the undrawable one through.
pub const MIN_TURNOUT_DIVERGENCE_TENTHS: u16 = 225;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementError {
    OutOfBounds,
    /// Autofill anchors do not lie along one of the sixteen directions.
    NotStraight,
    /// Only ground layer (`0`) is editable in MVP.
    InvalidLayer,
    AlreadyOccupied,
    /// Water crossing wider than [`MAX_BRIDGE_SPAN`] on both axes / path run.
    BridgeTooLong { span: u32 },
    /// Absolute height delta to a neighbour exceeds [`MAX_GRADE`].
    GradeTooSteep { grade: u8 },
    /// Cliff / mountain band — not buildable.
    TerrainForbidden,
    /// Two legs of a junction diverge by less than one direction step, so no
    /// turnout can be drawn between them (brief 01 §5.2).
    TurnoutTooShallow { divergence_tenths: u16 },
    /// A half-step (knight's-move) run passes over a tile that already carries
    /// track, so the shallow link cannot form. Naming the tile lets the ghost
    /// point at what is in the way.
    HalfStepBlocked { tile: TileCoord },
    InsufficientFunds,
    /// Demolish target missing.
    UnknownTrack,
}

/// Refuse a junction whose legs are closer together than one direction step.
///
/// A node with fewer than three legs is plain track — a straight or a curve —
/// and has no turnout to draw, so it is always fine. Three or more legs is a
/// turnout, and every pair of them has to be far enough apart that a frog can
/// exist between them.
///
/// # What this actually catches on a square grid
///
/// Nothing, today, and that is a result rather than an oversight. A half-step
/// link only forms while both tiles it crosses are free of track, and those two
/// tiles are exactly the tiles of the compass steps beside it
/// ([`intermediate_tiles`](super::dir::intermediate_tiles)). So a node can never
/// hold a half-step *and* an adjacent compass step: the tightest junction the
/// grid can produce is two rose steps, 36.87°, comfortably drawable.
///
/// The check is kept because that guarantee is a property of the linking rule,
/// not of the type system. Relax the linking rule to "at least one tile free"
/// and the 18.43° `NNE`/`NE` pair becomes reachable immediately — this is what
/// refuses it.
pub fn turnout_divergence_ok(links: TrackLinks) -> Result<(), PlacementError> {
    let dirs: Vec<usize> = links.dirs().collect();
    if dirs.len() < 3 {
        return Ok(());
    }
    let mut tightest = f32::MAX;
    for i in 0..dirs.len() {
        for j in (i + 1)..dirs.len() {
            tightest = tightest.min(bearing_separation_deg(dirs[i], dirs[j]));
        }
    }
    let tenths = (tightest * 10.0).round().max(0.0) as u16;
    if tenths < MIN_TURNOUT_DIVERGENCE_TENTHS {
        return Err(PlacementError::TurnoutTooShallow {
            divergence_tenths: tenths,
        });
    }
    Ok(())
}

/// Every half-step a piece at `tile` would link along must have its two crossed
/// tiles clear — otherwise the run is paid for and does not connect.
///
/// Only meaningful for autofill along a half-step direction; single-tile placement
/// simply loses the link, which is not an error.
pub fn half_step_run_clear(
    network: &TrackNetwork,
    tile: TileCoord,
    layer: u8,
    dir: usize,
) -> Result<(), PlacementError> {
    match network.blocked_intermediates(tile, layer, dir).first() {
        Some(&blocked) => Err(PlacementError::HalfStepBlocked { tile: blocked }),
        None => Ok(()),
    }
}

/// Whether a water tile may accept a bridge (narrow enough on at least one axis).
pub fn water_bridge_allowed(terrain: &TrackTerrain, tile: TileCoord) -> Result<(), PlacementError> {
    if !terrain.is_water(tile) {
        return Ok(());
    }
    let h = terrain.water_span_horizontal(tile);
    let v = terrain.water_span_vertical(tile);
    let span = h.min(v);
    if span > MAX_BRIDGE_SPAN {
        Err(PlacementError::BridgeTooLong { span })
    } else {
        Ok(())
    }
}

/// Contiguous water tiles along a polyline (for autofill bridge checks).
///
/// Rejects if any maximal run of consecutive water tiles on the path exceeds
/// [`MAX_BRIDGE_SPAN`].
///
/// A half-step leg is two tiles along one axis and one along the other, so the
/// polyline is *sparse* there: the tiles it crosses carry no track but the deck
/// still spans them. They are folded in before the run is measured, otherwise a
/// shallow run would undercount its own water crossing by two thirds.
pub fn path_bridge_spans_ok(
    terrain: &TrackTerrain,
    path: &[TileCoord],
) -> Result<(), PlacementError> {
    let mut run = 0u32;
    let mut worst = 0u32;
    for tile in walk_path(path) {
        if !terrain.contains(tile) {
            return Err(PlacementError::OutOfBounds);
        }
        if terrain.is_water(tile) {
            run += 1;
            worst = worst.max(run);
        } else {
            run = 0;
        }
    }
    if worst > MAX_BRIDGE_SPAN {
        Err(PlacementError::BridgeTooLong { span: worst })
    } else {
        Ok(())
    }
}

/// Every tile a path physically crosses, in order, including the tiles a
/// half-step leg passes over.
///
/// Identical to `path` for orthogonal and diagonal runs, so nothing about the
/// eight-direction behaviour moves.
pub fn walk_path(path: &[TileCoord]) -> Vec<TileCoord> {
    let mut out: Vec<TileCoord> = Vec::with_capacity(path.len());
    for (i, &tile) in path.iter().enumerate() {
        out.push(tile);
        let Some(&next) = path.get(i + 1) else {
            continue;
        };
        if let Some(dir) = super::dir::dir_index(tile, next) {
            if let Some(mids) = super::dir::intermediate_tiles(tile, dir) {
                out.extend_from_slice(&mids);
            }
        }
    }
    out
}

/// Refuse a path when any consecutive pair exceeds [`MAX_GRADE`].
pub fn path_grades_ok(terrain: &TrackTerrain, path: &[TileCoord]) -> Result<(), PlacementError> {
    for w in path.windows(2) {
        let a = w[0];
        let b = w[1];
        if !terrain.contains(a) || !terrain.contains(b) {
            return Err(PlacementError::OutOfBounds);
        }
        // Water height is a flood tag, not a climb — skip grade on any water leg.
        if terrain.is_water(a) || terrain.is_water(b) {
            continue;
        }
        let ha = terrain.height_at(a).unwrap_or(0);
        let hb = terrain.height_at(b).unwrap_or(0);
        let grade = (ha as i16 - hb as i16).unsigned_abs() as u8;
        if grade > MAX_GRADE {
            return Err(PlacementError::GradeTooSteep { grade });
        }
    }
    Ok(())
}

/// Grade from `tile` to any track it would actually link to on the same layer.
///
/// Reads the link set the new piece would acquire rather than every neighbouring
/// tile, so a half-step whose crossed tiles are occupied — and which therefore
/// will not link — does not impose a grade limit the player cannot see.
pub fn grade_to_neighbors_ok(
    network: &TrackNetwork,
    terrain: &TrackTerrain,
    tile: TileCoord,
    layer: u8,
) -> Result<(), PlacementError> {
    if terrain.is_water(tile) {
        return Ok(());
    }
    let our_h = terrain.height_at(tile).unwrap_or(0);
    let links = network.links_for(tile, layer);
    for i in 0..DIR_COUNT {
        if !links.has(i) {
            continue;
        }
        let n = step(tile, i);
        if terrain.is_water(n) {
            continue;
        }
        let nh = terrain.height_at(n).unwrap_or(0);
        let grade = (our_h as i16 - nh as i16).unsigned_abs() as u8;
        if grade > MAX_GRADE {
            return Err(PlacementError::GradeTooSteep { grade });
        }
    }
    Ok(())
}

fn land_buildable(terrain: &TrackTerrain, tile: TileCoord) -> Result<(), PlacementError> {
    let height = terrain.height_at(tile).unwrap_or(0);
    if height >= MOUNTAIN_HEIGHT_MIN {
        return Err(PlacementError::TerrainForbidden);
    }
    // Extremely steep local relief (cliff face) even below mountain band.
    let slope = local_slope(terrain, tile);
    if slope > MAX_GRADE + 1 {
        return Err(PlacementError::GradeTooSteep { grade: slope });
    }
    Ok(())
}

pub fn validate_tile_empty(
    network: &TrackNetwork,
    terrain: &TrackTerrain,
    tile: TileCoord,
    layer: u8,
) -> Result<bool, PlacementError> {
    if layer != GROUND_LAYER {
        return Err(PlacementError::InvalidLayer);
    }
    if !terrain.contains(tile) {
        return Err(PlacementError::OutOfBounds);
    }
    if network.id_at(tile, layer).is_some() {
        return Err(PlacementError::AlreadyOccupied);
    }
    let is_bridge = terrain.is_water(tile);
    if is_bridge {
        water_bridge_allowed(terrain, tile)?;
    } else {
        land_buildable(terrain, tile)?;
    }
    grade_to_neighbors_ok(network, terrain, tile, layer)?;
    // The junction this build would create has to be drawable (brief 01 §5.2).
    turnout_divergence_ok(network.links_for(tile, layer))?;
    // Ensure cost table accepts the tile (mountain / etc.).
    let _ = tile_build_cost(terrain, tile)?;
    Ok(is_bridge)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::dir::{dir_from_clock, DIR16};

    fn links_of(dirs: &[usize]) -> TrackLinks {
        let mut l = TrackLinks::empty();
        for &d in dirs {
            l.set(d);
        }
        l
    }

    #[test]
    fn plain_track_is_never_a_turnout() {
        assert!(turnout_divergence_ok(TrackLinks::empty()).is_ok());
        assert!(turnout_divergence_ok(links_of(&[2])).is_ok());
        // Even the sharpest two-leg pair is a curve, not a junction.
        assert!(turnout_divergence_ok(links_of(&[0, 8])).is_ok());
    }

    /// The rule the brief states: a leg one nominal step off the through route
    /// is the shallowest turnout allowed.
    #[test]
    fn a_one_step_turnout_is_allowed() {
        // N + S through route, NNE diverging at 26.57°.
        assert!(turnout_divergence_ok(links_of(&[0, 4, 8])).is_ok());
        // N + S with NE diverging at 45°.
        assert!(turnout_divergence_ok(links_of(&[0, 4, 1])).is_ok());
    }

    /// 18.43° is below one nominal step and cannot be drawn. The linking rule
    /// makes this unreachable through placement; this is the guard that keeps it
    /// unreachable if the linking rule is ever relaxed.
    #[test]
    fn a_sub_step_turnout_is_refused() {
        // NNE (26.57°) beside NE (45°) — 18.43° apart.
        let err = turnout_divergence_ok(links_of(&[8, 1, 4])).unwrap_err();
        assert_eq!(
            err,
            PlacementError::TurnoutTooShallow {
                divergence_tenths: 184
            }
        );
        // N beside NNE is 26.57° and survives, so the threshold is not just
        // "any adjacent rose pair".
        assert!(turnout_divergence_ok(links_of(&[0, 8, 4])).is_ok());
    }

    /// Every pair the grid can actually realise at one node clears the bar.
    #[test]
    fn the_realised_rose_never_needs_the_refusal() {
        for c in 0..16usize {
            let a = dir_from_clock(c);
            let b = dir_from_clock(c + 2);
            let straight = dir_from_clock(c + 8);
            assert!(
                turnout_divergence_ok(links_of(&[a, b, straight])).is_ok(),
                "two-step junction at clock {c} should be drawable"
            );
        }
    }

    #[test]
    fn walk_path_expands_only_half_step_legs() {
        let ortho = vec![TileCoord { x: 0, y: 0 }, TileCoord { x: 1, y: 0 }];
        assert_eq!(walk_path(&ortho), ortho);

        let diag = vec![TileCoord { x: 0, y: 0 }, TileCoord { x: 1, y: 1 }];
        assert_eq!(walk_path(&diag), diag);

        // ENE = (2, 1): crosses (1,0) and (1,1).
        assert_eq!(DIR16[9], (2, 1));
        let half = vec![TileCoord { x: 0, y: 0 }, TileCoord { x: 2, y: 1 }];
        let walked = walk_path(&half);
        assert_eq!(walked.len(), 4);
        assert!(walked.contains(&TileCoord { x: 1, y: 0 }));
        assert!(walked.contains(&TileCoord { x: 1, y: 1 }));
    }
}
