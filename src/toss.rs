//! Picking a game up: Ctrl or Shift + left drag on a game takes it out of the display as a
//! real object. It hangs from where it was grabbed and swings as it is pulled about, and
//! knocks the games around it, which wobble and settle back into place (the display keeps
//! working meanwhile). Let go of the key and it drops into the void; once it is off the screen
//! it is gone, and slides back into its place later ("Dragged games come back after").

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rapier3d::prelude::*;

use crate::app::{MainCamera, ViewState};
use crate::egg::{to_bevy, to_iso, Sim};
use crate::render::Item;

/// Gravity in cm/s², as in the easter egg.
const GRAVITY: f32 = -650.0;
/// Seconds a dropped game keeps knocking into others before it falls through them (so one
/// resting on a stack still leaves), and the most it may fall before it is taken away.
const SOLID_TIME: f32 = 4.0;
const MAX_FALL: f32 = 30.0;
/// Seconds the slide back into place takes.
pub const RETURN_TIME: f32 = 0.8;
/// Neighbours: spring back to their place (1/s²) and damping (1/s).
const SPRING: f32 = 260.0;
const DAMP: f32 = 22.0;

/// Collision groups: picked-up games hit everything, the games around them only those.
const LOOSE: Group = Group::GROUP_1;
const HELD_IN_PLACE: Group = Group::GROUP_2;

#[derive(Clone, Copy, PartialEq)]
enum State {
    /// On the end of the cursor.
    Held,
    /// Let go of with the key still down: falls, but stays until the key is let go.
    Floating,
    /// Key let go: dropping out of sight (since when).
    Falling(f32),
}

struct Loose {
    body: RigidBodyHandle,
    state: State,
    /// Where it is held, in its own frame (scaled world units).
    anchor: Point<f32>,
    /// Distance from the camera of the plane it is dragged across.
    depth: f32,
}

struct Neighbour {
    body: RigidBodyHandle,
    /// Where the display has it (set by `animate_items` each frame).
    place: (Vec3, Quat),
    /// Seconds it has been settled and clear of everything loose.
    calm: f32,
}

#[derive(Resource, Default)]
pub struct Toss {
    sim: Option<Sim>,
    acc: f32,
    loose: HashMap<String, Loose>,
    near: HashMap<String, Neighbour>,
    /// Games dropped into the void, and when they come back.
    away: HashMap<String, f32>,
    /// Games sliding back into place: when they started, and whether they were put at the
    /// start of the slide yet.
    returning: HashMap<String, (f32, bool)>,
    /// Where each physical game is now (loose ones and their neighbours).
    poses: HashMap<String, (Vec3, Quat)>,
    /// The left press this frame picked a game up (the display ignores it).
    pub took_press: bool,
    rescan: f32,
    /// The selection last frame (a loose game that becomes the selection goes back to the display).
    last_selected: Option<String>,
}

/// What the display should do with a game this frame.
pub enum Part {
    /// Not involved: lay it out as usual.
    Normal,
    /// Physical: put it here.
    Placed(Vec3, Quat),
    /// Shoved about: its place in the display is reported with `set_place`, and it is here.
    Shoved(Vec3, Quat),
    /// Gone into the void.
    Away,
    /// Sliding back in: 0..1 through the slide, and true on its first frame.
    Returning(f32, bool),
}

impl Toss {
    /// Picked up, thrown, shoved by one, or sliding back: physics is moving it, not the display.
    pub fn physical(&self, id: &str) -> bool {
        self.loose.contains_key(id) || self.near.contains_key(id) || self.returning.contains_key(id)
    }

    pub fn part(&mut self, id: &str, now: f32) -> Part {
        if self.loose.contains_key(id) {
            if let Some(&(p, r)) = self.poses.get(id) {
                return Part::Placed(p, r);
            }
        }
        if self.away.contains_key(id) {
            return Part::Away;
        }
        if let Some((start, placed)) = self.returning.get_mut(id) {
            let first = !*placed;
            *placed = true;
            return Part::Returning(((now - *start) / RETURN_TIME).clamp(0.0, 1.0), first);
        }
        if self.near.contains_key(id) {
            if let Some(&(p, r)) = self.poses.get(id) {
                return Part::Shoved(p, r);
            }
        }
        Part::Normal
    }

    pub fn set_place(&mut self, id: &str, pos: Vec3, rot: Quat) {
        if let Some(n) = self.near.get_mut(id) {
            n.place = (pos, rot);
        }
    }

    /// Anything going on (otherwise the systems do nothing).
    fn busy(&self) -> bool {
        !self.loose.is_empty() || !self.near.is_empty() || !self.away.is_empty() || !self.returning.is_empty()
    }

    fn remove(&mut self, h: RigidBodyHandle) {
        if let Some(sim) = self.sim.as_mut() {
            sim.bodies.remove(h, &mut sim.islands, &mut sim.colliders, &mut sim.impulse, &mut sim.multibody, true);
        }
    }
}

pub struct TossPlugin;

impl Plugin for TossPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Toss>();
    }
}

fn new_sim() -> Sim {
    let mut sim = Sim::new();
    sim.params.dt = 1.0 / 120.0;
    sim
}

/// A physical copy of an item where it is now.
fn add_body(sim: &mut Sim, item: &Item, tf: &Transform, loose: bool) -> RigidBodyHandle {
    let b = RigidBodyBuilder::dynamic()
        .position(to_iso(tf.translation, tf.rotation))
        .linear_damping(if loose { 0.4 } else { 0.0 })
        .angular_damping(if loose { 1.6 } else { 0.0 })
        .gravity_scale(if loose { 1.0 } else { 0.0 })
        .ccd_enabled(loose)
        .build();
    let h = sim.bodies.insert(b);
    // A shade smaller than the item, so games packed against each other do not start out
    // overlapping (which would throw them apart).
    let half = item.dims * tf.scale * 0.5 * 0.96;
    let groups = if loose { InteractionGroups::new(LOOSE, Group::ALL) } else { InteractionGroups::new(HELD_IN_PLACE, LOOSE) };
    // The games held in place are much heavier than a loose one, so one resting on a stack
    // barely presses it down, while a shove still knocks them about.
    let density = if loose { 0.02 } else { 0.12 };
    let c = ColliderBuilder::cuboid(half.x.max(0.05), half.y.max(0.05), half.z.max(0.05)).density(density).friction(0.6).restitution(0.15).collision_groups(groups).build();
    sim.colliders.insert_with_parent(c, h, &mut sim.bodies);
    h
}

/// Ctrl / Shift + left press on a game picks it up (or takes hold of one already loose).
#[allow(clippy::too_many_arguments)]
pub fn toss_input(
    mut toss: ResMut<Toss>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    view: Res<ViewState>,
    menu: Res<crate::settings::Menu>,
    egg: Res<crate::egg::Egg>,
    ui: Query<&Interaction>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    items: Query<(&Item, &GlobalTransform, &InheritedVisibility)>,
    tfs: Query<(&Item, &Transform)>,
) {
    toss.took_press = false;
    let modifier = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if !buttons.just_pressed(MouseButton::Left) || !modifier {
        return;
    }
    if view.inspecting() || view.egg || egg.active() || menu.open || menu.just_closed || ui.iter().any(|i| *i != Interaction::None) {
        return;
    }
    let Ok(window) = windows.get_single() else { return };
    let Some((index, r, n)) = crate::app::pick_at(window, &camera, &items, None) else { return };
    let Some((item, gtf)) = items.iter().find(|(i, ..)| i.view_index == index).map(|(i, g, _)| (i, g)) else { return };
    let Some((_, tf)) = tfs.iter().find(|(i, _)| i.view_index == index) else { return };
    let Some(cam_tf) = camera.get_single().ok().map(|(_, t)| *t) else { return };
    // Where on it the cursor is: the face it hit, at the spot across it.
    let local = (r + n) * item.dims * 0.5;
    let hit = gtf.transform_point(local);
    let depth = (hit - cam_tf.translation()).dot(*cam_tf.forward());
    let id = item.game_id.clone();
    let toss = &mut *toss;
    toss.took_press = true;
    let sim = toss.sim.get_or_insert_with(new_sim);
    // A neighbour being shoved about becomes the one picked up.
    if let Some(nb) = toss.near.remove(&id) {
        sim.bodies.remove(nb.body, &mut sim.islands, &mut sim.colliders, &mut sim.impulse, &mut sim.multibody, true);
    }
    let body = match toss.loose.get(&id) {
        Some(l) => l.body,
        None => add_body(sim, item, tf, true),
    };
    let Some(b) = sim.bodies.get_mut(body) else { return };
    b.set_gravity_scale(1.0, true);
    let anchor = b.position().inverse_transform_point(&point![hit.x, hit.y, hit.z]);
    toss.loose.insert(id.clone(), Loose { body, state: State::Held, anchor, depth });
    toss.away.remove(&id);
    toss.returning.remove(&id);
    toss.rescan = 0.0;
}

/// Runs the picked-up games and the games they knock about; drops them when the key is let
/// go and brings them back later.
#[allow(clippy::too_many_arguments)]
pub fn toss_step(
    mut toss: ResMut<Toss>,
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    items: Query<(&Item, &Transform)>,
    cfg: Res<crate::config::Config>,
    view: Res<ViewState>,
) {
    let picked = (toss.last_selected != view.selected_id).then(|| view.selected_id.clone()).flatten();
    toss.last_selected = view.selected_id.clone();
    if !toss.busy() {
        toss.sim = None;
        return;
    }
    let now = time.elapsed_seconds();
    let dt = time.delta_seconds().min(0.1);
    let toss = &mut *toss;
    // A loose game (not in hand) that is scrolled to leaves the physics and is shown like any
    // other selection.
    if let Some(id) = picked.filter(|id| toss.loose.get(id).is_some_and(|l| l.state != State::Held)) {
        if let Some(l) = toss.loose.remove(&id) {
            toss.remove(l.body);
        }
    }

    // Back from the void.
    let back: Vec<String> = toss.away.iter().filter(|(_, &t)| now >= t).map(|(id, _)| id.clone()).collect();
    for id in back {
        toss.away.remove(&id);
        toss.returning.insert(id, (now, false));
    }
    toss.returning.retain(|_, (start, _)| now - *start < RETURN_TIME);

    // Let go of the mouse: it floats; let go of the key: it drops.
    let modifier = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let Some(sim) = toss.sim.as_mut() else { return };
    for l in toss.loose.values_mut() {
        if l.state == State::Held && !buttons.pressed(MouseButton::Left) {
            l.state = State::Floating;
        }
        if !modifier && matches!(l.state, State::Held | State::Floating) {
            l.state = State::Falling(now);
            if let Some(b) = sim.bodies.get_mut(l.body) {
                b.set_gravity_scale(1.0, true);
                b.set_linear_damping(0.0);
                b.set_angular_damping(0.3);
            }
        }
    }

    // The held game follows the cursor across a plane facing the camera, pulled by the spot
    // it was grabbed at (so it hangs and swings from there).
    let ray = windows.get_single().ok().and_then(|w| w.cursor_position()).and_then(|c| {
        let (cam, tf) = camera.get_single().ok()?;
        Some((cam.viewport_to_world(tf, c)?, *tf))
    });
    for l in toss.loose.values() {
        if l.state != State::Held {
            continue;
        }
        let (Some((ray, cam_tf)), Some(b)) = (ray, sim.bodies.get_mut(l.body)) else { continue };
        let f = *cam_tf.forward();
        let along = ray.direction.dot(f);
        if along < 1e-3 {
            continue;
        }
        let target = ray.origin + *ray.direction * (l.depth / along);
        let held = b.position() * l.anchor;
        let pull = Vec3::new(target.x - held.x, target.y - held.y, target.z - held.z);
        let want = (pull * 16.0).clamp_length_max(900.0);
        let v = b.velocity_at_point(&held);
        let dv = vector![want.x, want.y, want.z] - v;
        // A share of the difference each frame, through the grabbed spot (which turns it).
        let m = b.mass();
        b.apply_impulse_at_point(dv * (m * 0.35), held, true);
    }

    // The games near anything loose take part, held to their places by springs.
    toss.rescan -= dt;
    if toss.rescan <= 0.0 {
        toss.rescan = 0.15;
        let loose: Vec<(Vec3, f32)> = toss
            .loose
            .iter()
            .filter_map(|(id, l)| {
                let (p, _) = to_bevy(sim.bodies.get(l.body)?.position());
                let size = items.iter().find(|(i, _)| i.game_id == *id).map_or(20.0, |(i, t)| (i.dims * t.scale).length() * 0.5);
                Some((p, size))
            })
            .collect();
        for (item, tf) in &items {
            let id = &item.game_id;
            if toss.loose.contains_key(id) || toss.away.contains_key(id) || toss.returning.contains_key(id) || toss.near.contains_key(id) {
                continue;
            }
            let size = (item.dims * tf.scale).length() * 0.5;
            if loose.iter().any(|(p, s)| p.distance(tf.translation) < (s + size) * 1.1) && toss.near.len() < 40 {
                let body = add_body(sim, item, tf, false);
                toss.near.insert(id.clone(), Neighbour { body, place: (tf.translation, tf.rotation), calm: 0.0 });
            }
        }
        // Gone from the display (filtered out, list rebuilt): drop it.
        let present: std::collections::HashSet<&str> = items.iter().map(|(i, _)| i.game_id.as_str()).collect();
        let gone: Vec<String> = toss.loose.keys().chain(toss.near.keys()).filter(|id| !present.contains(id.as_str())).cloned().collect();
        for id in gone {
            if let Some(h) = toss.loose.remove(&id).map(|l| l.body).or_else(|| toss.near.remove(&id).map(|n| n.body)) {
                sim.bodies.remove(h, &mut sim.islands, &mut sim.colliders, &mut sim.impulse, &mut sim.multibody, true);
            }
        }
    }

    // Step in fixed slices.
    toss.acc = (toss.acc + dt).min(0.1);
    let step = sim.params.dt;
    while toss.acc >= step {
        toss.acc -= step;
        // Springs pulling the neighbours back to their places.
        for n in toss.near.values() {
            let Some(b) = sim.bodies.get_mut(n.body) else { continue };
            let (p, r) = to_bevy(b.position());
            let (tp, tr) = n.place;
            // Far off (the display scrolled on): go straight there.
            if p.distance(tp) > 40.0 {
                b.set_position(to_iso(tp, tr), true);
                b.set_linvel(vector![0.0, 0.0, 0.0], true);
                b.set_angvel(vector![0.0, 0.0, 0.0], true);
                continue;
            }
            let (lv, av) = (*b.linvel(), *b.angvel());
            let (v, w) = (Vec3::new(lv.x, lv.y, lv.z), Vec3::new(av.x, av.y, av.z));
            let a = (tp - p) * SPRING - v * DAMP;
            // q and -q are the same turn; take the short way round (else it spins a full flip).
            let q = tr * r.inverse();
            let err = if q.w < 0.0 { -q } else { q }.to_scaled_axis();
            let aw = err * SPRING - w * DAMP;
            let (v, w) = (v + a * step, w + aw * step);
            b.set_linvel(vector![v.x, v.y, v.z], true);
            b.set_angvel(vector![w.x, w.y, w.z], true);
        }
        sim.step_with(GRAVITY);
    }

    // Read back where everything is.
    toss.poses.clear();
    for (id, h) in toss.loose.iter().map(|(id, l)| (id, l.body)).chain(toss.near.iter().map(|(id, n)| (id, n.body))) {
        if let Some(b) = sim.bodies.get(h) {
            toss.poses.insert(id.clone(), to_bevy(b.position()));
        }
    }

    // Dropped and off the screen (a good way past its edge): gone, back later. A while after it
    // was dropped it stops hitting the others, so it cannot stay resting on them.
    let view = camera.get_single().ok();
    let size = windows.get_single().map(|w| Vec2::new(w.width(), w.height())).unwrap_or(Vec2::new(1920.0, 1080.0));
    let mut fallen = Vec::new();
    for (id, l) in &toss.loose {
        let State::Falling(t) = l.state else { continue };
        let Some(b) = sim.bodies.get(l.body) else { continue };
        let p = to_bevy(b.position()).0;
        let on_screen = view.and_then(|(cam, tf)| cam.world_to_viewport(tf, p)).is_some_and(|v| v.cmpge(-size * 0.25).all() && v.cmple(size * 1.25).all());
        if !on_screen || now - t > MAX_FALL {
            fallen.push(id.clone());
        } else if now - t > SOLID_TIME {
            for &c in b.colliders() {
                if let Some(c) = sim.colliders.get_mut(c) {
                    c.set_collision_groups(InteractionGroups::none());
                }
            }
        }
    }
    let away = cfg.animation.toss_respawn.clamp(0.0, 600.0);
    for id in fallen {
        if let Some(l) = toss.loose.remove(&id) {
            sim.bodies.remove(l.body, &mut sim.islands, &mut sim.colliders, &mut sim.impulse, &mut sim.multibody, true);
        }
        toss.away.insert(id, now + away);
    }

    // Neighbours back in place and clear of everything loose are handed back to the display.
    let loose: Vec<Vec3> = toss.loose.values().filter_map(|l| sim.bodies.get(l.body)).map(|b| to_bevy(b.position()).0).collect();
    let mut done = Vec::new();
    for (id, n) in toss.near.iter_mut() {
        let Some(b) = sim.bodies.get(n.body) else {
            done.push(id.clone());
            continue;
        };
        let (p, r) = to_bevy(b.position());
        let settled = p.distance(n.place.0) < 0.05 && r.angle_between(n.place.1) < 0.01 && b.linvel().norm() < 0.5 && b.angvel().norm() < 0.05;
        let clear = loose.iter().all(|q| q.distance(p) > 40.0);
        n.calm = if settled && (clear || loose.is_empty()) { n.calm + dt } else { 0.0 };
        if n.calm > 0.3 {
            done.push(id.clone());
        }
    }
    for id in done {
        if let Some(n) = toss.near.remove(&id) {
            toss.remove(n.body);
        }
    }
}
