//! Curve construction and sampling utilities for Pump.

/// Number of normalized samples stored in one pump cycle table.
pub const CURVE_TABLE_LEN: usize = 1024;

/// One freehand point inside the curve editor's normalized coordinate system.
///
/// `x` is horizontal position in `[0, 1]` across one cycle.
/// `y` is gain-shape value in `[0, 1]` where `0` is max duck and `1` is unity.
#[derive(Debug, Copy, Clone)]
pub struct FreehandPoint {
    /// Horizontal cycle position.
    pub x: f32,
    /// Vertical gain-shape value.
    pub y: f32,
}

/// Build the default sidechain-style curve used on plugin initialization.
///
/// The shape performs a fast initial dip followed by a smooth recovery.
pub fn default_sidechain_curve() -> [f32; CURVE_TABLE_LEN] {
    let mut curve = [1.0_f32; CURVE_TABLE_LEN];
    let attack_end = 0.06_f32;
    let floor = 0.05_f32;

    for (index, sample) in curve.iter_mut().enumerate() {
        let phase = index as f32 / (CURVE_TABLE_LEN - 1) as f32;
        *sample = if phase < attack_end {
            let t = phase / attack_end;
            1.0 + (floor - 1.0) * t
        } else {
            let t = ((phase - attack_end) / (1.0 - attack_end)).clamp(0.0, 1.0);
            let eased = t.powf(1.8);
            floor + (1.0 - floor) * eased
        }
        .clamp(0.0, 1.0);
    }

    curve
}

/// Sample the curve table with linear interpolation at a normalized phase.
pub fn sample_curve(curve: &[f32; CURVE_TABLE_LEN], phase: f32) -> f32 {
    let wrapped = phase.rem_euclid(1.0);
    let scaled = wrapped * (CURVE_TABLE_LEN as f32 - 1.0);
    let left_index = scaled.floor() as usize;
    let right_index = (left_index + 1).min(CURVE_TABLE_LEN - 1);
    let frac = scaled - left_index as f32;
    lerp(curve[left_index], curve[right_index], frac).clamp(0.0, 1.0)
}

/// Convert freehand points into a full fixed-size curve table.
///
/// Points are clamped, sorted by `x`, then resampled using Catmull-Rom interpolation.
/// If there are not enough valid points, the fallback curve is returned unchanged.
pub fn points_to_curve(
    points: &[FreehandPoint],
    fallback: &[f32; CURVE_TABLE_LEN],
) -> [f32; CURVE_TABLE_LEN] {
    if points.len() < 2 {
        return *fallback;
    }

    let mut normalized: Vec<FreehandPoint> = points
        .iter()
        .copied()
        .filter(|point| point.x.is_finite() && point.y.is_finite())
        .map(|point| FreehandPoint {
            x: point.x.clamp(0.0, 1.0),
            y: point.y.clamp(0.0, 1.0),
        })
        .collect();

    if normalized.len() < 2 {
        return *fallback;
    }

    normalized.sort_by(|a, b| a.x.total_cmp(&b.x));

    let mut deduped: Vec<FreehandPoint> = Vec::with_capacity(normalized.len());
    for point in normalized {
        if let Some(last) = deduped.last_mut() {
            if (point.x - last.x).abs() < 1.0e-4 {
                last.y = point.y;
                continue;
            }
        }
        deduped.push(point);
    }

    if deduped.len() < 2 {
        return *fallback;
    }

    if deduped[0].x > 0.0 {
        deduped.insert(
            0,
            FreehandPoint {
                x: 0.0,
                y: deduped[0].y,
            },
        );
    } else {
        deduped[0].x = 0.0;
    }

    let last_index = deduped.len() - 1;
    if deduped[last_index].x < 1.0 {
        deduped.push(FreehandPoint {
            x: 1.0,
            y: deduped[last_index].y,
        });
    } else {
        deduped[last_index].x = 1.0;
    }

    let mut curve = [1.0_f32; CURVE_TABLE_LEN];
    let mut seg = 0usize;
    for (index, sample) in curve.iter_mut().enumerate() {
        let t = index as f32 / (CURVE_TABLE_LEN - 1) as f32;
        while seg + 1 < deduped.len() && t > deduped[seg + 1].x {
            seg += 1;
        }

        let seg_start = seg.min(deduped.len().saturating_sub(2));
        let p0 = deduped[seg_start.saturating_sub(1)];
        let p1 = deduped[seg_start];
        let p2 = deduped[(seg_start + 1).min(deduped.len() - 1)];
        let p3 = deduped[(seg_start + 2).min(deduped.len() - 1)];

        let span = (p2.x - p1.x).max(1.0e-6);
        let local = ((t - p1.x) / span).clamp(0.0, 1.0);
        *sample = catmull_rom(p0.y, p1.y, p2.y, p3.y, local).clamp(0.0, 1.0);
    }

    curve
}

fn catmull_rom(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * ((2.0 * p1)
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::{
        default_sidechain_curve, points_to_curve, sample_curve, FreehandPoint, CURVE_TABLE_LEN,
    };

    #[test]
    fn default_curve_stays_bounded() {
        let curve = default_sidechain_curve();
        assert!(curve.iter().all(|value| (0.0..=1.0).contains(value)));
    }

    #[test]
    fn sampled_curve_interpolates_between_neighbors() {
        let mut curve = [0.0_f32; CURVE_TABLE_LEN];
        curve[0] = 0.0;
        curve[1] = 1.0;
        let sample = sample_curve(&curve, 0.0006);
        assert!(sample > 0.0);
        assert!(sample < 1.0);
    }

    #[test]
    fn points_to_curve_handles_reversed_input_order() {
        let fallback = default_sidechain_curve();
        let points = [
            FreehandPoint { x: 1.0, y: 1.0 },
            FreehandPoint { x: 0.0, y: 0.0 },
            FreehandPoint { x: 0.5, y: 0.2 },
        ];
        let curve = points_to_curve(&points, &fallback);
        assert!(curve.iter().all(|value| (0.0..=1.0).contains(value)));
    }

    #[test]
    fn points_to_curve_uses_fallback_for_too_few_points() {
        let fallback = default_sidechain_curve();
        let points = [FreehandPoint { x: 0.2, y: 0.3 }];
        let curve = points_to_curve(&points, &fallback);
        assert_eq!(curve, fallback);
    }
}
