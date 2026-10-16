//! Layout maths. Everything here is a pure function of (index, focus, parameters),
//! which keeps the display modes easy to tweak and add. Units are centimetres.

use bevy::prelude::*;

use crate::config::{LayoutKind, LayoutParams};

/// Height of the shelf board's top, in cm before `item_scale`.
pub const SHELF_Y: f32 = -9.5;

/// The angled shelf: a line along the floor of the scene, running off into the distance at
/// `p.angle` degrees: to the left for a positive angle (the case fronts then turn towards the
/// viewer), to the right for a negative one. The selected game stands `p.depth` cm in front.
pub struct ShelfFrame {
    /// Point on the shelf's front edge, on the board, where the selection's slot is.
    pub origin: Vec3,
    /// Along the shelf (towards the far end).
    pub axis: Vec3,
    /// Out of the front of the shelf, towards the viewer.
    pub normal: Vec3,
    /// Turns a game so its spine (local -X) faces out of the shelf.
    pub spine_out: Quat,
}

pub fn shelf_frame(p: &LayoutParams, s: f32) -> ShelfFrame {
    let a = p.angle.abs().to_radians();
    let side = if p.angle >= 0.0 { -1.0 } else { 1.0 };
    let (sin, cos) = a.sin_cos();
    let axis = Vec3::new(side * cos, 0.0, -sin);
    let normal = Vec3::new(side * sin, 0.0, cos);
    ShelfFrame {
        origin: Vec3::new(0.0, SHELF_Y * s, 0.0) - normal * p.depth * s,
        axis,
        normal,
        spine_out: Quat::from_rotation_y(cos.atan2(-side * sin)),
    }
}

pub struct Pose {
    pub pos: Vec3,
    pub rot: Quat,
    pub scale: f32,
    pub visible: bool,
    /// 1 = fully drawn, falling to 0 towards the edge of what the layout shows.
    pub fade: f32,
}

/// How many items (or shelves / grid cells) the fade-out at the edges takes.
pub const FADE_SPAN: f32 = 2.5;

/// 1 inside `limit`, fading to 0 over `span` beyond it.
fn edge_fade(dist: f32, limit: f32, span: f32) -> f32 {
    ease((limit + span - dist) / span)
}

pub struct LayoutCtx<'a> {
    pub kind: LayoutKind,
    pub p: &'a LayoutParams,
    /// width, height, depth of the item in cm
    pub dims: Vec3,
    pub index: usize,
    /// Smoothly animated (fractional) focus position.
    pub focus: f32,
    pub spacing_scale: f32,
    pub item_scale: f32,
    /// Shelf, stack, pile: centre of every item when packed by thickness.
    pub prefix: &'a [f32],
    /// Grid: point of the wall at the centre of the screen, in grid units (see `grid_pos`).
    pub grid_center: Vec2,
    /// Camera zoom factor (1 = default); more items are visible when > 1.
    pub zoom: f32,
    /// Packed start of every item, and the gap between packed items (see `prefix`).
    pub starts: &'a [f32],
    pub gap: f32,
    /// Looping list: packed length of one lap (the list repeats every `prefix.len()` items
    /// and this far along); 0 = the list has ends.
    pub period: f32,
}

fn jitter(i: usize) -> f32 {
    let mut x = (i as u32).wrapping_mul(2654435761);
    x ^= x >> 15;
    x = x.wrapping_mul(2246822519);
    x ^= x >> 13;
    (x & 0xffff) as f32 / 32767.5 - 1.0
}

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Packed value of item `i` in `v`. Looping (`period > 0`), indices past either end carry on
/// into the next / previous lap; otherwise they stop at the ends.
fn packed_at(v: &[f32], i: i64, period: f32) -> f32 {
    let n = v.len() as i64;
    if n == 0 {
        return 0.0;
    }
    if period > 0.0 {
        v[i.rem_euclid(n) as usize] + i.div_euclid(n) as f32 * period
    } else {
        v[i.clamp(0, n - 1) as usize]
    }
}

/// Packed position of the (fractional) focus along `prefix`.
fn packed_focus(prefix: &[f32], focus: f32, period: f32) -> f32 {
    let f0 = focus.floor() as i64;
    lerp(packed_at(prefix, f0, period), packed_at(prefix, f0 + 1, period), focus - focus.floor())
}

/// Row spacing of the wall relative to column spacing.
pub const GRID_ROW: f32 = 1.25;

/// Wall: the closest zoom at which a whole row still fits across the screen.
pub fn wall_min_zoom(p: &LayoutParams, spacing_scale: f32, fov: f32, aspect: f32) -> f32 {
    let row = p.columns.max(1) as f32 * p.spacing * spacing_scale * 1.02;
    row / (2.0 * p.cam_distance.max(1.0) * (fov * 0.5).tan() * aspect.max(0.1))
}

pub fn grid_pos(i: usize, cols: usize) -> Vec2 {
    Vec2::new((i % cols) as f32, -((i / cols) as f32) * GRID_ROW)
}

/// `grid_pos` for any index, carrying on above row 0 when negative (a looping wall).
pub fn grid_pos_i(i: i64, cols: usize) -> Vec2 {
    let c = cols.max(1) as i64;
    Vec2::new(i.rem_euclid(c) as f32, -(i.div_euclid(c) as f32) * GRID_ROW)
}

/// Grid position of a fractional index, between its two neighbours.
fn grid_pos_f(f: f32, cols: usize) -> Vec2 {
    let i = f.floor() as i64;
    grid_pos_i(i, cols).lerp(grid_pos_i(i + 1, cols), f - f.floor())
}

pub fn pose(c: &LayoutCtx) -> Pose {
    let p = c.p;
    let rel = c.index as f32 - c.focus;
    let ar = rel.abs();
    // Zoomed out, the camera sees further, so more items stay alive.
    let half = p.visible * 0.5 * c.zoom.max(1.0);
    let sp = c.spacing_scale;
    let s = c.item_scale;
    let sel = ease(1.0 - ar);
    // Items reach a little past `visible` and fade out there instead of popping.
    // The carousel is a closed ring: never show more than half of it either side, or items
    // further round would wrap onto the ones in front when zoomed out.
    let limit = if c.kind == LayoutKind::Carousel {
        half.min(std::f32::consts::PI / p.angle.to_radians().max(0.01) - FADE_SPAN - 0.3).max(0.5)
    } else {
        half
    };
    let fade = edge_fade(ar, limit, FADE_SPAN);
    let mut out = Pose { pos: Vec3::ZERO, rot: Quat::IDENTITY, scale: s, visible: fade > 0.0, fade };

    match c.kind {
        LayoutKind::CoverFlow => {
            let a = rel.clamp(-1.0, 1.0);
            let x = if ar < 1.0 {
                rel * p.center_gap * sp
            } else {
                rel.signum() * (p.center_gap + (ar - 1.0) * p.spacing) * sp
            };
            out.pos = Vec3::new(x, 0.0, -ease(a.abs()) * p.depth - (ar - 1.0).max(0.0) * 0.5);
            out.rot = Quat::from_rotation_y(-a * p.angle.to_radians());
        }
        LayoutKind::Carousel | LayoutKind::Helix => {
            let r = p.radius * sp;
            let th = rel * p.angle.to_radians();
            let y = if c.kind == LayoutKind::Helix { -rel * p.spacing * sp } else { 0.0 };
            out.pos = Vec3::new(r * th.sin(), y, r * th.cos() - r + sel * 2.5);
            out.rot = Quat::from_rotation_y(th);
        }
        LayoutKind::Stack => {
            // A neat tower of cases lying flat with their spines towards you. The selected
            // case slides out of the tower and stands up facing you.
            let at = c.prefix.get(c.index).copied().unwrap_or(0.0);
            let y = -(at - packed_focus(c.prefix, c.focus, c.period));
            // Lying flat, spine (local -X) facing the camera, top of the case to the left.
            let flat = Quat::from_rotation_y(90f32.to_radians()) * Quat::from_rotation_x(-90f32.to_radians());
            let in_tower = Vec3::new(p.center_gap * 0.5 * sp, y, -c.dims.x * 0.5 * s);
            let out_pos = Vec3::new(-p.center_gap * 0.5 * sp, 0.0, p.depth);
            let upright = Quat::from_rotation_y(p.angle.to_radians());
            out.pos = in_tower.lerp(out_pos, sel);
            out.rot = flat.slerp(upright, sel);
        }
        LayoutKind::Pile => {
            // A loose pile of cases lying face up, seen from above. The selected case is
            // lifted off the top and leans towards you; cases you have passed slide off to
            // the left. depth scales how far the selection lifts.
            let at = c.prefix.get(c.index).copied().unwrap_or(0.0);
            let y = -(at - packed_focus(c.prefix, c.focus, c.period));
            let j = jitter(c.index);
            let lie = Quat::from_rotation_y(j * 0.18) * Quat::from_rotation_x(-90f32.to_radians());
            let in_pile = Vec3::new(j * 1.4, y - 9.0, j * 0.8);
            let lifted = Vec3::new(0.0, 3.0 * p.depth, 6.0 * p.depth);
            let lean = Quat::from_rotation_x(-28f32.to_radians());
            if rel >= 0.0 {
                out.pos = in_pile.lerp(lifted, sel);
                out.rot = lie.slerp(lean, sel);
            } else {
                out.pos = lifted + Vec3::new(rel * 26.0 * sp, rel * 2.0, rel * 3.0);
                out.rot = lean * Quat::from_rotation_y(rel * 0.5);
                out.fade = edge_fade(-rel, 1.0, 1.0);
                out.visible = out.fade > 0.0;
            }
        }
        LayoutKind::Shelf => {
            // Aurora-style shelf at an angle to the viewer: spines out along the front edge,
            // running off into the distance. The selection slides out of its slot towards you
            // and turns to face you; its neighbours part a little to leave its gap.
            let f = shelf_frame(p, s);
            let along = c.prefix.get(c.index).copied().unwrap_or(0.0) - packed_focus(c.prefix, c.focus, c.period);
            let part = rel.clamp(-1.0, 1.0) * p.center_gap * 0.25 * sp;
            let on_shelf = f.origin + f.axis * (along + part) - f.normal * (c.dims.x * 0.5 * s) + Vec3::Y * (c.dims.y * 0.5 * s);
            let pulled = Vec3::new(0.0, (SHELF_Y + c.dims.y * 0.5 + 1.0) * s, 0.0);
            out.pos = on_shelf.lerp(pulled, sel);
            out.rot = f.spine_out.slerp(Quat::IDENTITY, sel);
            // The shelf ahead fades into the distance; the games already passed (coming
            // towards the camera) stay a while longer, then fade.
            out.fade = if rel >= 0.0 { edge_fade(rel, half, FADE_SPAN * 2.0) } else { edge_fade(-rel, 7.0 * c.zoom.max(1.0), 4.0) };
            out.visible = out.fade > 0.0;
        }
        LayoutKind::Towers | LayoutKind::MessyTowers => {
            // Two piles of cases lying flat, spines towards you, turned a little towards the
            // middle: games already passed on the left, the ones to come on the right, nearest
            // on top. The selection stands up in the middle; scrolling takes the top case off
            // one pile and drops the previous selection onto the other.
            let messy = c.kind == LayoutKind::MessyTowers;
            let side = if rel >= 0.0 { 1.0 } else { -1.0 };
            // How far down its pile the case's centre is: the thickness of every case above it
            // plus half its own (from the packing, so mixed case sizes stack without gaps).
            let at = |v: &[f32], i: i64| packed_at(v, i, c.period);
            let end = |i: i64| 2.0 * at(c.prefix, i) - at(c.starts, i);
            let f0 = c.focus.floor() as i64;
            let fr = c.focus - c.focus.floor();
            let sel_start = lerp(at(c.starts, f0), at(c.starts, f0 + 1), fr);
            let sel_end = lerp(end(f0), end(f0 + 1), fr);
            let centre_i = at(c.prefix, c.index as i64);
            let down = if rel >= 0.0 { centre_i - (sel_end + c.gap) } else { (sel_start - c.gap) - centre_i }.max(0.0);
            // Lying flat with the spine (local -X) towards the camera, then turned inwards.
            let flat = Quat::from_rotation_y(90f32.to_radians()) * Quat::from_rotation_x(-90f32.to_radians());
            let mut rot = Quat::from_rotation_y(-side * p.angle.to_radians()) * flat;
            let mut pos = Vec3::new(side * p.center_gap * sp, p.radius * s - down, 0.0);
            if messy {
                // Each case knocked a little askew, the pile twisting slowly as it goes down.
                let j = jitter(c.index);
                let k = jitter(c.index + 7919);
                rot = Quat::from_rotation_y(j * 0.32 + side * down * 0.05) * rot;
                pos += Vec3::new(j * 1.8 * s, 0.0, k * 1.6 * s);
            }
            if ar < 1.0 {
                // Between the middle and the top of its pile.
                let e = ease(ar);
                let centre = Vec3::new(0.0, 0.0, p.depth * s);
                pos = centre.lerp(pos, e);
                rot = Quat::IDENTITY.slerp(rot, e);
            }
            out.pos = pos;
            out.rot = rot;
        }
        LayoutKind::Spiral | LayoutKind::Stacks => {
            // Selection in front; everything else on the side it belongs to, starting at
            // `center_gap` and receding. Between 0 and 1 the item blends from the front
            // position to the head of its side.
            let side = |sign: f32, r: f32| -> (Vec3, Quat) {
                let t = (r - 1.0).max(0.0);
                if c.kind == LayoutKind::Spiral {
                    let th = t * p.angle.to_radians();
                    let pos = Vec3::new(
                        sign * (p.center_gap + p.radius * (1.0 - th.cos())) * sp,
                        p.radius * th.sin() * sp,
                        -2.0 - t * p.spacing * sp,
                    );
                    (pos, Quat::from_rotation_y(-sign * 30f32.to_radians()) * Quat::from_rotation_z(sign * th * 0.5))
                } else {
                    let pos = Vec3::new(sign * (p.center_gap + t * p.radius) * sp, 0.0, -2.0 - t * p.spacing * sp);
                    (pos, Quat::from_rotation_y(-sign * p.angle.to_radians()))
                }
            };
            let sign = if rel >= 0.0 { 1.0 } else { -1.0 };
            let front = (Vec3::new(0.0, 0.0, p.depth), Quat::IDENTITY);
            let (pos, rot) = if ar < 1.0 {
                let (sp_, sr) = side(sign, 1.0);
                let e = ease(ar);
                (front.0.lerp(sp_, e), front.1.slerp(sr, e))
            } else {
                side(sign, ar)
            };
            out.pos = pos;
            out.rot = rot;
        }
        LayoutKind::Table => {
            // Face up on a table, turned a little from square on; rows run away from you.
            // The selection lifts off and comes towards you, tipped to face the camera above.
            let cols = p.columns.max(1);
            let cell = p.spacing * sp;
            let gp = grid_pos(c.index, cols);
            let fp = grid_pos_f(c.focus, cols);
            let a = ease(1.0 - (gp - fp).length());
            let d = (gp - c.grid_center) * cell;
            let yaw = Quat::from_rotation_y(p.angle.to_radians());
            // Rows are deeper than columns are wide (cases are taller than wide).
            let on_table = yaw * Vec3::new(d.x, 0.0, d.y * 1.1) + Vec3::new(0.0, p.look_height * s + c.dims.z * 0.5 * s, 0.0);
            let lying = yaw * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
            // Picked up into the middle of the screen, a good way towards the camera, square on.
            let (cam, look) = camera_for(p);
            let lifted = look.lerp(cam, 0.5);
            let facing = Transform::IDENTITY.looking_to(lifted - cam, Vec3::Y).rotation;
            out.pos = on_table.lerp(lifted, a);
            out.rot = lying.slerp(facing, a);
            // Big boxes shrink to fit their spot on the table (full size once picked up).
            let fit = (cell * 0.99 / c.dims.x).min(cell * 1.1 * GRID_ROW * 0.99 / c.dims.y).min(1.0);
            out.scale = s * lerp(fit, 1.0, a);
            let z = c.zoom.max(0.2);
            let fx = edge_fade(d.x.abs() / cell, (cols as f32 * 0.5 + 1.2) * z, 1.5);
            let fy = edge_fade(d.y.abs() / (cell * GRID_ROW), (p.rows as f32 * 0.5 + 1.0) * z, 1.0);
            out.fade = fx.min(fy);
            out.visible = out.fade > 0.0;
        }
        LayoutKind::StandUp => {
            // A row leaning back, bottoms on the floor; the selection stands up and steps
            // forward, pivoting on its bottom edge.
            let lean = p.angle.to_radians() * (1.0 - sel);
            let floor = -9.0 * s;
            // Wide boxes shrink to their spot in the row so they do not overlap.
            let fit = (p.spacing * sp * 0.97 / c.dims.x).min(1.0);
            let h = c.dims.y * 0.5 * s * fit;
            // Centre of a case leaning back by `lean` about its bottom edge.
            let centre = Vec3::new(0.0, h * lean.cos(), -h * lean.sin());
            out.pos = Vec3::new(rel * p.spacing * sp, floor, sel * p.depth * s) + centre;
            out.rot = Quat::from_rotation_x(-lean);
            out.scale = s * fit;
        }
        LayoutKind::Spines => {
            // Upright side by side, spines towards you; the selection slides out and turns
            // to show its cover, its neighbours making room.
            let along = c.prefix.get(c.index).copied().unwrap_or(0.0) - packed_focus(c.prefix, c.focus, c.period);
            let part = rel.clamp(-1.0, 1.0) * p.center_gap * sp;
            let in_row = Vec3::new(along + part, 0.0, 0.0);
            let out_pos = Vec3::new(0.0, 0.0, p.depth * s);
            out.pos = in_row.lerp(out_pos, sel);
            out.rot = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2).slerp(Quat::IDENTITY, sel);
        }
        LayoutKind::Grid => {
            let cols = p.columns.max(1);
            let cell = p.spacing * sp;
            let gp = grid_pos(c.index, cols);
            // Pop-out follows the (smoothed) selection, the wall itself follows the pan centre.
            let fp = grid_pos_f(c.focus, cols);
            let a = ease(1.0 - (gp - fp).length());
            let d = (gp - c.grid_center) * cell;
            // The selection comes out to the middle of the screen, a little bigger.
            let on_wall = Vec3::new(d.x, d.y, 0.0);
            out.pos = on_wall.lerp(Vec3::new(0.0, p.look_height * s, 12.0 * s), a);
            out.rot = (Quat::from_rotation_y(-d.x / cell * 0.05) * Quat::from_rotation_x(d.y / cell * 0.04)).slerp(Quat::IDENTITY, a);
            let fit = (cell * 0.92 / c.dims.x.max(c.dims.y)).min(1.0);
            out.scale = s * fit * (1.0 + 0.3 * a);
            let z = c.zoom.max(0.2);
            let fx = edge_fade(d.x.abs() / cell, (cols as f32 * 0.5 + 1.2) * z, 1.5);
            let fy = edge_fade(d.y.abs() / (cell * GRID_ROW), (p.rows as f32 * 0.5 + 1.0) * z, 1.0);
            out.fade = fx.min(fy);
            out.visible = out.fade > 0.0;
        }
    }
    out
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Camera position and look-at target for a layout.
pub fn camera_for(p: &LayoutParams) -> (Vec3, Vec3) {
    (Vec3::new(0.0, p.cam_height, p.cam_distance), Vec3::new(0.0, p.look_height, 0.0))
}

/// How many items on each side of the selection must exist as entities. Grows when the
/// camera is zoomed out, so the wider view is filled.
pub fn window_radius(kind: LayoutKind, p: &LayoutParams, base: usize, zoom: f32) -> usize {
    let z = zoom.max(1.0);
    let rows = (p.rows as f32 * z).ceil() as usize;
    let need = match kind {
        LayoutKind::Grid | LayoutKind::Table => p.columns.max(1) * (rows / 2 + 4),
        // Everything up to the far end of the shelf (the radius is symmetric; the passed side
        // is faded out).
        LayoutKind::Shelf => (p.visible * 0.5 * z + FADE_SPAN * 2.0).ceil() as usize + 3,
        _ => (p.visible * 0.5 * z + FADE_SPAN).ceil() as usize + 3,
    };
    base.max(need)
}
