//! Candidate site scoring for new demand anchors.

use crate::ids::TileCoord;
use crate::stations::{IndustryRegistry, StationRegistry, StationService};
use crate::town::GROWTH_RADIUS;
use crate::track::TrackTerrain;

/// Soft influence of station service at a tile (0..=1), matching the Service overlay.
pub fn service_influence_at(
    tile: TileCoord,
    stations: &StationRegistry,
    service: &StationService,
) -> f32 {
    let mut best = 0.0_f32;
    for station in stations.iter() {
        let dx = (station.tile.x - tile.x).abs();
        let dy = (station.tile.y - tile.y).abs();
        let dist = dx.max(dy);
        if dist > GROWTH_RADIUS {
            continue;
        }
        let score = service.score(station.id).score as f32 / 100.0;
        let falloff = 1.0 - (dist as f32) / ((GROWTH_RADIUS + 1) as f32);
        let v = (score * falloff).clamp(0.0, 1.0);
        if v > best {
            best = v;
        }
    }
    best
}

/// Manhattan distance to nearest existing station or industry tile.
pub fn min_anchor_distance(
    tile: TileCoord,
    stations: &StationRegistry,
    industries: &IndustryRegistry,
) -> i32 {
    let mut best = i32::MAX;
    for s in stations.iter() {
        let d = (s.tile.x - tile.x).abs() + (s.tile.y - tile.y).abs();
        if d < best {
            best = d;
        }
    }
    for i in industries.iter() {
        let d = (i.tile.x - tile.x).abs() + (i.tile.y - tile.y).abs();
        if d < best {
            best = d;
        }
    }
    if best == i32::MAX {
        0
    } else {
        best
    }
}

/// Cheap routing-question heuristic: sample the line toward the nearest station
/// and count water crossings + steep height steps. Higher = more interesting.
pub fn routing_interest(
    tile: TileCoord,
    stations: &StationRegistry,
    terrain: &TrackTerrain,
) -> i32 {
    let Some(nearest) = stations
        .iter()
        .min_by_key(|s| (s.tile.x - tile.x).abs() + (s.tile.y - tile.y).abs())
    else {
        return 0;
    };
    let mut interest = 0i32;
    let mut water_hits = 0i32;
    let mut height_steps = 0i32;
    let mut prev_h = terrain.height_at(tile).unwrap_or(0);
    for sample in sample_line(tile, nearest.tile) {
        if terrain.is_water(sample) {
            water_hits += 1;
        } else if let Some(h) = terrain.height_at(sample) {
            let dh = (h as i32 - prev_h as i32).abs();
            if dh >= 2 {
                height_steps += 1;
            }
            prev_h = h;
        }
    }
    // A bit of water or a ridge is interesting; pure open plains less so.
    interest += water_hits.min(6) * 3;
    interest += height_steps.min(8) * 2;
    interest
}

/// Bresenham-ish sample of tiles from `a` to `b` (exclusive of endpoints).
fn sample_line(a: TileCoord, b: TileCoord) -> Vec<TileCoord> {
    let mut out = Vec::new();
    let mut x0 = a.x;
    let mut y0 = a.y;
    let x1 = b.x;
    let y1 = b.y;
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        if !(x0 == a.x && y0 == a.y) && !(x0 == x1 && y0 == y1) {
            out.push(TileCoord { x: x0, y: y0 });
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
    out
}

/// Pick the best land tile for a new demand anchor.
///
/// Prefers low service influence, then routing interest, then distance from
/// existing anchors (at least `min_spacing`, preferring farther as `prefer_far`).
pub fn pick_demand_site(
    terrain: &TrackTerrain,
    stations: &StationRegistry,
    industries: &IndustryRegistry,
    service: &StationService,
    min_spacing: i32,
    prefer_far: i32,
    max_influence: f32,
) -> Option<TileCoord> {
    let w = terrain.width();
    let h = terrain.height();
    if w < 8 || h < 8 {
        return None;
    }

    let mut best: Option<(TileCoord, i64)> = None;
    // Stride sampling keeps this cheap on 64²+ maps.
    let stride = ((w.max(h) / 24).max(1)) as i32;

    for y in (3..(h as i32 - 3)).step_by(stride as usize) {
        for x in (3..(w as i32 - 3)).step_by(stride as usize) {
            let tile = TileCoord { x, y };
            if !terrain.contains(tile) || terrain.is_water(tile) {
                continue;
            }
            if terrain.height_at(tile).unwrap_or(0) >= crate::track::MOUNTAIN_HEIGHT_MIN {
                continue;
            }
            if crate::track::local_slope(terrain, tile) > crate::track::MAX_GRADE + 1 {
                continue;
            }
            if stations.at(tile, crate::track::GROUND_LAYER).is_some()
                || industries.at(tile).is_some()
            {
                continue;
            }
            let influence = service_influence_at(tile, stations, service);
            if influence > max_influence {
                continue;
            }
            let dist = min_anchor_distance(tile, stations, industries);
            if dist < min_spacing {
                continue;
            }
            let interest = routing_interest(tile, stations, terrain);
            // Score: outside coverage first, then routing question, then reach.
            let score = ((1.0 - influence) * 10_000.0) as i64
                + (interest as i64) * 80
                + (dist.min(prefer_far + 20) as i64) * 3;
            match best {
                Some((_, s)) if s >= score => {}
                _ => best = Some((tile, score)),
            }
        }
    }

    // If stride missed everything, fall back to denser scan near preferred distance.
    if best.is_none() {
        for y in 3..(h as i32 - 3) {
            for x in 3..(w as i32 - 3) {
                let tile = TileCoord { x, y };
                if terrain.is_water(tile) {
                    continue;
                }
                if terrain.height_at(tile).unwrap_or(0) >= crate::track::MOUNTAIN_HEIGHT_MIN {
                    continue;
                }
                if crate::track::local_slope(terrain, tile) > crate::track::MAX_GRADE + 1 {
                    continue;
                }
                if stations.at(tile, crate::track::GROUND_LAYER).is_some()
                    || industries.at(tile).is_some()
                {
                    continue;
                }
                let influence = service_influence_at(tile, stations, service);
                if influence > max_influence {
                    continue;
                }
                let dist = min_anchor_distance(tile, stations, industries);
                if dist < min_spacing {
                    continue;
                }
                let interest = routing_interest(tile, stations, terrain);
                let score = ((1.0 - influence) * 10_000.0) as i64
                    + (interest as i64) * 80
                    + (dist as i64);
                match best {
                    Some((_, s)) if s >= score => {}
                    _ => best = Some((tile, score)),
                }
            }
        }
    }

    best.map(|(t, _)| t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stations::StationService;
    use crate::track::GROUND_LAYER;

    fn flat_land(w: u32, h: u32) -> TrackTerrain {
        TrackTerrain::new(w, h, (0..w * h).map(|_| (false, 2i8)))
    }

    #[test]
    fn influence_zero_when_scores_zero() {
        let mut stations = StationRegistry::new();
        let id = stations.insert("A", TileCoord { x: 10, y: 10 }, GROUND_LAYER);
        let mut service = StationService::default();
        service.ensure(id);
        let v = service_influence_at(TileCoord { x: 10, y: 10 }, &stations, &service);
        assert_eq!(v, 0.0);
    }

    #[test]
    fn pick_avoids_occupied_and_near_anchors() {
        let terrain = flat_land(32, 32);
        let mut stations = StationRegistry::new();
        let industries = IndustryRegistry::new();
        let mut service = StationService::default();
        let id = stations.insert("Hub", TileCoord { x: 16, y: 16 }, GROUND_LAYER);
        service.ensure(id);
        let site = pick_demand_site(
            &terrain,
            &stations,
            &industries,
            &service,
            8,
            16,
            0.05,
        );
        let site = site.expect("should find a site");
        assert!((site.x - 16).abs() + (site.y - 16).abs() >= 8);
    }
}
