//! Easter egg (press Ctrl five times): every game in the list rains down onto a table and
//! piles up with real physics, then one of them rises out of the pile and becomes the selection.

use std::f32::consts::{FRAC_PI_2, TAU};

use bevy::prelude::*;
use rapier3d::na::{Quaternion, UnitQuaternion};
use rapier3d::prelude::*;

use crate::app::{Action, Actions, Toast, ViewState};
use crate::config::Config;
use crate::covers::{CoverRequest, CoverService};
use crate::library::Library;
use crate::render::{self, DiscMaterial, Gfx, SpawnCtx, SpineFont};

/// Height of the table top.
const TABLE_Y: f32 = -14.0;
/// Seconds over which the games are dropped: a steady stream of 20 to 40 games a second, so
/// big libraries take longer instead of all landing at once (which was slow to simulate).
const DROP_SPAN: f32 = 8.0;
/// Gravity in cm/s² (a little under the real 981 so the fall reads well).
const GRAVITY: f32 = -650.0;

/// A falling game (also marks it so the normal layout leaves it alone).
#[derive(Component)]
pub struct EggPiece;

#[derive(Component)]
pub struct EggDecor;

struct Piece {
    entity: Entity,
    view_index: usize,
    /// When it is let go, and where from.
    release: f32,
    start: Isometry<f32>,
    collider: SharedShape,
    body: Option<RigidBodyHandle>,
    /// Slid off the table and fell out of sight (taken out of the simulation).
    gone: bool,
}

struct Sim {
    bodies: RigidBodySet,
    colliders: ColliderSet,
    pipeline: PhysicsPipeline,
    islands: IslandManager,
    broad: DefaultBroadPhase,
    narrow: NarrowPhase,
    impulse: ImpulseJointSet,
    multibody: MultibodyJointSet,
    ccd: CCDSolver,
    query: QueryPipeline,
    params: IntegrationParameters,
}

impl Sim {
    fn new() -> Self {
        let mut params = IntegrationParameters::default();
        // The world is in centimetres.
        params.length_unit = 100.0;
        params.dt = 1.0 / 60.0;
        // Overlapping cases in a crowded pile are pushed apart gently: the default allows
        // 10 m/s, which made the pile burst outwards as it built up.
        params.normalized_max_corrective_velocity = 0.6;
        params.num_solver_iterations = std::num::NonZeroUsize::new(8).unwrap();
        Sim {
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad: DefaultBroadPhase::new(),
            narrow: NarrowPhase::new(),
            impulse: ImpulseJointSet::new(),
            multibody: MultibodyJointSet::new(),
            ccd: CCDSolver::new(),
            query: QueryPipeline::new(),
            params,
        }
    }

    fn step(&mut self) {
        self.pipeline.step(
            &vector![0.0, GRAVITY, 0.0],
            &self.params,
            &mut self.islands,
            &mut self.broad,
            &mut self.narrow,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse,
            &mut self.multibody,
            &mut self.ccd,
            Some(&mut self.query),
            &(),
            &(),
        );
    }
}

#[derive(Default, PartialEq, Clone, Copy)]
enum Phase {
    #[default]
    Off,
    /// Games raining down and settling.
    Falling,
    /// The chosen game rising out of the pile towards the camera.
    Rising,
    /// Held in front of the camera for a moment before the launcher comes back.
    Showing,
}

#[derive(Resource, Default)]
pub struct Egg {
    phase: Phase,
    t: f32,
    phase_t: f32,
    acc: f32,
    sim: Option<Sim>,
    pieces: Vec<Piece>,
    winner: Option<usize>,
    rise_from: Option<Isometry<f32>>,
    rise_to: Option<Isometry<f32>>,
    cancel: bool,
    rng: u64,
    /// When Ctrl was pressed (the last few presses with nothing else in between).
    shifts: Vec<f32>,
    /// The case being dragged with the mouse, if any.
    grab: Option<Grab>,
    /// Last time the pile was touched (the pick waits until it has been left alone).
    last_touch: f32,
    /// Enter / Space: pick a game now.
    pick_now: bool,
    /// Camera: where it starts (position, target) and the zoom applied to that (wheel).
    cam: (Vec3, Vec3),
    zoom: f32,
}

struct Grab {
    body: RigidBodyHandle,
    /// Where it was grabbed, in the body's own frame.
    anchor: Point<f32>,
    /// Height it is dragged at (just above where it was picked up).
    height: f32,
}

impl Egg {
    pub fn active(&self) -> bool {
        self.phase != Phase::Off
    }
    fn rand(&mut self) -> f32 {
        // xorshift64*
        let mut x = self.rng.max(1);
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        ((x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32) / (1u64 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.rand()
    }
}

pub struct EggPlugin;

impl Plugin for EggPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Egg>();
    }
}

fn to_bevy(iso: &Isometry<f32>) -> (Vec3, Quat) {
    let t = iso.translation.vector;
    let q = iso.rotation.quaternion();
    (Vec3::new(t.x, t.y, t.z), Quat::from_xyzw(q.i, q.j, q.k, q.w))
}

fn to_iso(pos: Vec3, rot: Quat) -> Isometry<f32> {
    let q = UnitQuaternion::from_quaternion(Quaternion::new(rot.w, rot.x, rot.y, rot.z));
    Isometry::from_parts(vector![pos.x, pos.y, pos.z].into(), q)
}

/// Counts Ctrl presses (five in a row, a second or so apart at most, starts it) and, while it
/// runs, keeps every other action away from the launcher; Esc / Backspace / Q stop it.
pub fn egg_input(keys: Res<ButtonInput<KeyCode>>, time: Res<Time>, mut egg: ResMut<Egg>, mut actions: ResMut<Actions>) {
    let now = time.elapsed_seconds();
    if egg.active() {
        if actions.0.iter().any(|a| matches!(a, Action::Quit | Action::CloseCase)) {
            egg.cancel = true;
        }
        if actions.0.iter().any(|a| matches!(a, Action::Launch | Action::Open)) {
            egg.pick_now = true;
        }
        // The zoom keys move the camera in and out (see `egg_drag` for the wheel).
        for a in &actions.0 {
            match a {
                Action::ZoomIn => egg.zoom = (egg.zoom / 1.12).clamp(0.35, 2.5),
                Action::ZoomOut => egg.zoom = (egg.zoom * 1.12).clamp(0.35, 2.5),
                _ => {}
            }
        }
        actions.0.clear();
        return;
    }
    let ctrl = keys.any_just_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let other = keys.get_just_pressed().any(|k| !matches!(k, KeyCode::ControlLeft | KeyCode::ControlRight));
    if other {
        egg.shifts.clear();
    }
    if ctrl {
        if egg.shifts.last().is_some_and(|&t| now - t > 1.2) {
            egg.shifts.clear();
        }
        egg.shifts.push(now);
        if egg.shifts.len() >= 5 {
            egg.shifts.clear();
            egg.phase = Phase::Falling;
            egg.t = -1.0; // set up on the next run
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn egg_run(
    mut commands: Commands,
    time: Res<Time>,
    mut egg: ResMut<Egg>,
    mut view: ResMut<ViewState>,
    lib: Res<Library>,
    cfg: Res<Config>,
    mut gfx: ResMut<Gfx>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut disc_materials: ResMut<Assets<DiscMaterial>>,
    mut images: ResMut<Assets<Image>>,
    font: Res<SpineFont>,
    mut svc: ResMut<CoverService>,
    mut toast: ResMut<Toast>,
    mut pieces_q: Query<(&mut Transform, &mut Visibility, &mut render::Fade), With<EggPiece>>,
    decor: Query<Entity, With<EggDecor>>,
) {
    if !egg.active() {
        return;
    }
    let egg = &mut *egg;
    let dt = time.delta_seconds().min(0.1);

    // Stop (Esc): clear everything away and go back to the list as it was.
    let finish = |commands: &mut Commands, egg: &mut Egg, view: &mut ViewState| {
        for p in egg.pieces.drain(..) {
            commands.entity(p.entity).despawn_recursive();
        }
        for e in &decor {
            commands.entity(e).despawn_recursive();
        }
        egg.sim = None;
        egg.phase = Phase::Off;
        egg.cancel = false;
        // The normal items were put away when it started; they come back with the covers
        // loaded in the meantime (no respawn, which would forget them).
        view.egg = false;
        view.egg_cam = None;
    };
    if egg.cancel {
        finish(&mut commands, egg, &mut view);
        return;
    }

    // ---- set up -------------------------------------------------------------------------------
    if egg.t < 0.0 {
        if view.view.is_empty() {
            egg.phase = Phase::Off;
            return;
        }
        egg.rng = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7) | 1;
        egg.t = 0.0;
        egg.phase_t = 0.0;
        egg.acc = 0.0;
        egg.winner = None;
        egg.grab = None;
        egg.pick_now = false;
        egg.last_touch = 0.0;
        // The normal items go away while this runs (they come back afterwards), and an open
        // case is closed.
        view.inspect = None;
        view.inspect_open = false;
        view.inspect_t = 0.0;
        view.egg = true;
        view.respawn = true;

        // Every game in the list, in a random order.
        let mut order: Vec<usize> = (0..view.view.len()).collect();
        for i in (1..order.len()).rev() {
            let j = (egg.rand() * (i + 1) as f32) as usize % (i + 1);
            order.swap(i, j);
        }
        let n = order.len();

        // A table big enough for the pile, and a camera looking down at it.
        let s = cfg.display.item_scale;
        // On the small side, so a big pile spills over the edges.
        let width = (((n as f32) * 14.0 * 19.0 * s * s / 2.2).sqrt() * 0.6).clamp(45.0, 130.0);
        let depth = width * 0.68;
        // Framed wider than the table, so the games sliding off its edges are seen falling.
        let frame = (((n as f32) * 14.0 * 19.0 * s * s / 2.2).sqrt() * 1.1).clamp(80.0, 220.0);
        let cam = Vec3::new(0.0, TABLE_Y + frame * 0.62 + 18.0, frame * 0.72 + 22.0);
        let look = Vec3::new(0.0, TABLE_Y + 4.0, 0.0);
        view.egg_cam = Some((cam, look));
        egg.cam = (cam, look);
        egg.zoom = 1.0;

        let mut sim = Sim::new();
        // Grey, like the bookshelf.
        let wood = materials.add(StandardMaterial { base_color: Color::srgb(0.46, 0.47, 0.5), perceptual_roughness: 0.45, metallic: 0.15, ..default() });
        let thick = 4.0;
        commands.spawn((
            PbrBundle {
                mesh: meshes.add(bevy::math::primitives::Cuboid::new(width, thick, depth)),
                material: wood,
                transform: Transform::from_xyz(0.0, TABLE_Y - thick * 0.5, 0.0),
                ..default()
            },
            EggDecor,
        ));
        // The table top, slippery, with nothing round it: games that slide off fall away.
        sim.colliders.insert(ColliderBuilder::cuboid(width * 0.5, thick * 0.5, depth * 0.5).translation(vector![0.0, TABLE_Y - thick * 0.5, 0.0]).friction(0.4).build());

        let style = cfg.display.item_style;
        let mut cx = SpawnCtx {
            gfx: &mut gfx,
            meshes: &mut meshes,
            materials: &mut materials,
            disc_materials: &mut disc_materials,
            images: &mut images,
            font: font.0.as_ref(),
            cfg: &cfg,
        };
        let step = (DROP_SPAN / n.max(1) as f32).clamp(0.025, 0.05);
        let mut pieces = Vec::with_capacity(n);
        for (k, &vi) in order.iter().enumerate() {
            let Some(game) = view.view.get(vi).and_then(|&li| lib.games.get(li)) else { continue };
            let e = render::spawn_item(&mut commands, &mut cx, game, vi, style);
            let dims = render::item_dims(&cfg, game.platform, style) * s;
            commands.entity(e).insert((EggPiece, Visibility::Hidden));
            svc.request(CoverRequest::from_game(game, &cfg));
            // Discs are cylinders (axis along the item's local Z), everything else a box.
            let round = style == crate::config::ItemStyle::Disc;
            let collider = if round {
                SharedShape::compound(vec![(Isometry::rotation(vector![FRAC_PI_2, 0.0, 0.0]), SharedShape::cylinder(dims.z.max(0.3) * 0.5, dims.x * 0.5))])
            } else {
                SharedShape::cuboid(dims.x * 0.5, dims.y * 0.5, dims.z.max(0.3) * 0.5)
            };
            let pos = Vec3::new(egg.range(-0.38, 0.38) * width, TABLE_Y + 70.0 + egg.range(0.0, 50.0), egg.range(-0.36, 0.36) * depth);
            // Mostly cover up (as cases tossed on a table tend to be), some face down, all at a tilt.
            let flip = if egg.rand() < 0.25 { std::f32::consts::PI } else { 0.0 };
            let rot = Quat::from_rotation_y(egg.range(0.0, TAU)) * Quat::from_rotation_x(-FRAC_PI_2 + flip + egg.range(-0.7, 0.7)) * Quat::from_rotation_y(egg.range(-0.5, 0.5));
            pieces.push(Piece { entity: e, view_index: vi, release: k as f32 * step, start: to_iso(pos, rot), collider, body: None, gone: false });
        }
        egg.pieces = pieces;
        egg.sim = Some(sim);
        toast.show("Shuffling your games...");
        return;
    }

    egg.t += dt;
    egg.phase_t += dt;
    let t = egg.t;

    // ---- let games go -------------------------------------------------------------------------
    let mut released = Vec::new();
    for (i, p) in egg.pieces.iter().enumerate() {
        if p.body.is_none() && t >= p.release && egg.phase == Phase::Falling {
            released.push(i);
        }
    }
    for i in released {
        let (spin, push) = (vector![egg.range(-4.0, 4.0), egg.range(-4.0, 4.0), egg.range(-4.0, 4.0)], vector![egg.range(-20.0, 20.0), -60.0, egg.range(-20.0, 20.0)]);
        let sim = egg.sim.as_mut().unwrap();
        let p = &mut egg.pieces[i];
        let body = RigidBodyBuilder::dynamic().position(p.start).angvel(spin).linvel(push).linear_damping(0.15).angular_damping(0.6).ccd_enabled(true).build();
        let h = sim.bodies.insert(body);
        // Plastic on plastic: they still slide over each other, but settle instead of skating.
        let col = ColliderBuilder::new(p.collider.clone()).density(0.6).friction(0.35).restitution(0.05).build();
        sim.colliders.insert_with_parent(col, h, &mut sim.bodies);
        p.body = Some(h);
        if let Ok((mut tf, mut vis, mut fade)) = pieces_q.get_mut(p.entity) {
            let (pos, rot) = to_bevy(&p.start);
            *tf = Transform::from_translation(pos).with_rotation(rot);
            *vis = Visibility::Inherited;
            fade.value = 1.0;
        }
    }

    // ---- the chosen game rises out of the pile --------------------------------------------------
    let all_out = egg.pieces.iter().all(|p| p.body.is_some());
    if egg.phase == Phase::Falling && (all_out || egg.pick_now) {
        let sim = egg.sim.as_ref().unwrap();
        // Games still falling off the edge do not count.
        let resting = egg
            .pieces
            .iter()
            .filter(|p| !p.gone)
            .filter_map(|p| p.body)
            .filter_map(|h| sim.bodies.get(h))
            .filter(|b| b.translation().y > TABLE_Y - 3.0)
            .all(|b| b.is_sleeping() || b.linvel().norm() < 4.0);
        let last = egg.pieces.iter().map(|p| p.release).fold(0.0, f32::max);
        // Not while the pile is being played with: wait until it has been left alone a moment.
        let quiet = if egg.grab.is_some() { 0.0 } else { t - egg.last_touch };
        if egg.pick_now || (resting && t > last + 1.6 && quiet > 1.5) || (t > last + 5.0 && quiet > 4.0) {
            egg.pick_now = false;
            egg.grab = None;
            // One of the games still on the table.
            let on_table: Vec<usize> = (0..egg.pieces.len())
                .filter(|&i| !egg.pieces[i].gone && egg.pieces[i].body.and_then(|h| sim.bodies.get(h)).is_some_and(|b| b.translation().y > TABLE_Y - 2.0))
                .collect();
            if on_table.is_empty() {
                egg.cancel = true;
                return;
            }
            let w = on_table[(egg.rand() * on_table.len() as f32) as usize % on_table.len()];
            egg.winner = Some(w);
            let sim = egg.sim.as_mut().unwrap();
            if let Some(b) = egg.pieces[w].body.and_then(|h| sim.bodies.get_mut(h)) {
                egg.rise_from = Some(*b.position());
                b.set_body_type(RigidBodyType::KinematicPositionBased, true);
            }
            // In front of the camera, square on to it.
            let (cam, look) = view.egg_cam.unwrap_or((Vec3::new(0.0, 60.0, 90.0), Vec3::ZERO));
            let at = cam.lerp(look, 0.38);
            let face = Transform::IDENTITY.looking_to(at - cam, Vec3::Y).rotation;
            egg.rise_to = Some(to_iso(at, face));
            // Wake the pile so it shifts as the game pushes its way out.
            for p in &egg.pieces {
                if let Some(b) = p.body.and_then(|h| sim.bodies.get_mut(h)) {
                    b.wake_up(true);
                }
            }
            egg.phase = Phase::Rising;
            egg.phase_t = 0.0;
        }
    }
    if egg.phase == Phase::Rising {
        let (Some(w), Some(from), Some(to)) = (egg.winner, egg.rise_from, egg.rise_to) else { return };
        let dur = 2.0;
        let u = (egg.phase_t / dur).clamp(0.0, 1.0);
        let (p0, r0) = to_bevy(&from);
        let (p1, r1) = to_bevy(&to);
        // Up out of the pile first (slowly, shoving the others), then over to the camera.
        let up = p0 + Vec3::Y * 28.0;
        let e = |x: f32| x * x * (3.0 - 2.0 * x);
        let (pos, rot) = if u < 0.45 {
            let a = e(u / 0.45);
            (p0.lerp(up, a), r0.slerp(Quat::from_rotation_y(r0.to_euler(EulerRot::YXZ).0) * Quat::from_rotation_x(-0.4), a * 0.5))
        } else {
            let a = e((u - 0.45) / 0.55);
            let mid = r0.slerp(Quat::from_rotation_y(r0.to_euler(EulerRot::YXZ).0) * Quat::from_rotation_x(-0.4), 0.5);
            (up.lerp(p1, a), mid.slerp(r1, a))
        };
        // Spinning as it goes: two whole turns, slowing down to finish facing the camera.
        let rot = Quat::from_rotation_y(2.0 * TAU * (1.0 - e(u))) * rot;
        let sim = egg.sim.as_mut().unwrap();
        if let Some(b) = egg.pieces[w].body.and_then(|h| sim.bodies.get_mut(h)) {
            b.set_next_kinematic_position(to_iso(pos, rot));
        }
        if u >= 1.0 {
            egg.phase = Phase::Showing;
            egg.phase_t = 0.0;
            if let Some(g) = view.view.get(egg.pieces[w].view_index).and_then(|&li| lib.games.get(li)) {
                toast.show(format!("How about... {}?", g.title));
            }
        }
    }
    if egg.phase == Phase::Showing && egg.phase_t > 1.6 {
        // Back to the launcher with the chosen game selected (and picked up on the wall / table).
        let vi = egg.winner.map(|w| egg.pieces[w].view_index);
        finish(&mut commands, egg, &mut view);
        if let Some(vi) = vi {
            view.select(vi, &lib);
            view.focus = vi as f32;
            view.grid_free = false;
        }
        return;
    }

    // ---- physics ------------------------------------------------------------------------------
    let Some(sim) = egg.sim.as_mut() else { return };
    egg.acc = (egg.acc + dt).min(4.0 / 60.0);
    while egg.acc >= sim.params.dt {
        sim.step();
        egg.acc -= sim.params.dt;
    }
    for p in &mut egg.pieces {
        if p.gone {
            continue;
        }
        let Some(h) = p.body else { continue };
        let Some(b) = sim.bodies.get(h) else { continue };
        let (pos, rot) = to_bevy(b.position());
        if let Ok((mut tf, mut vis, _)) = pieces_q.get_mut(p.entity) {
            tf.translation = pos;
            tf.rotation = rot;
            // Fallen well below the table and off the screen: stop simulating it.
            if pos.y < TABLE_Y - 250.0 {
                *vis = Visibility::Hidden;
            }
        }
        if pos.y < TABLE_Y - 250.0 {
            sim.bodies.remove(h, &mut sim.islands, &mut sim.colliders, &mut sim.impulse, &mut sim.multibody, true);
            p.gone = true;
        }
    }
}






/// Before the pick: drag cases round the table with the left mouse button (let go while moving
/// to throw one), and turn the one held by moving the mouse with the right button down too. The
/// Ctrl + wheel zooms (all the way through).
#[allow(clippy::too_many_arguments)]
pub fn egg_drag(
    time: Res<Time>,
    mut egg: ResMut<Egg>,
    mut view: ResMut<ViewState>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut motion: EventReader<bevy::input::mouse::MouseMotion>,
    mut wheel: EventReader<bevy::input::mouse::MouseWheel>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<crate::app::MainCamera>>,
) {
    let turn: Vec2 = motion.read().map(|m| m.delta).sum();
    if !egg.active() || egg.t < 0.0 {
        wheel.clear();
        egg.grab = None;
        return;
    }
    // Ctrl + wheel zooms (the wheel on its own does nothing here).
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    for w in wheel.read() {
        let y = if w.unit == bevy::input::mouse::MouseScrollUnit::Pixel { w.y / 40.0 } else { w.y };
        if y != 0.0 && ctrl {
            egg.zoom = (egg.zoom * if y > 0.0 { 1.0 / 1.1 } else { 1.1 }).clamp(0.35, 2.5);
        }
    }
    let (cam0, look) = egg.cam;
    view.egg_cam = Some((look + (cam0 - look) * egg.zoom, look));
    if egg.phase != Phase::Falling {
        egg.grab = None;
        return;
    }
    let t = egg.t;
    let ray = windows.get_single().ok().and_then(|w| w.cursor_position()).and_then(|c| {
        let (cam, tf) = camera.get_single().ok()?;
        cam.viewport_to_world(tf, c)
    });
    let egg = &mut *egg;
    let Some(sim) = egg.sim.as_mut() else { return };
    if buttons.just_released(MouseButton::Left) {
        egg.grab = None;
    }
    if buttons.just_pressed(MouseButton::Left) {
        if let Some(r) = ray {
            let o = r.origin;
            let d = *r.direction;
            let ray = Ray::new(point![o.x, o.y, o.z], vector![d.x, d.y, d.z]);
            if let Some((col, toi)) = sim.query.cast_ray(&sim.bodies, &sim.colliders, &ray, 2000.0, true, QueryFilter::only_dynamic()) {
                if let Some(h) = sim.colliders.get(col).and_then(|c| c.parent()) {
                    let hit = ray.point_at(toi);
                    if let Some(b) = sim.bodies.get(h) {
                        let anchor = b.position().inverse_transform_point(&hit);
                        egg.grab = Some(Grab { body: h, anchor, height: hit.y.max(TABLE_Y) + 5.0 });
                        egg.last_touch = t;
                    }
                }
            }
        }
    }
    let Some(g) = egg.grab.as_ref() else { return };
    let Some(r) = ray else { return };
    egg.last_touch = t;
    // Follow the cursor across a level just above the pile.
    let (o, d) = (r.origin, *r.direction);
    if d.y.abs() < 1e-3 {
        return;
    }
    let k = (g.height - o.y) / d.y;
    if k <= 0.0 {
        return;
    }
    let target = o + d * k;
    let Some(b) = sim.bodies.get_mut(g.body) else {
        egg.grab = None;
        return;
    };
    let held = b.position() * g.anchor;
    let pull = Vec3::new(target.x - held.x, target.y - held.y, target.z - held.z);
    // A stiff spring towards the cursor, with a speed limit; the case keeps this speed when let
    // go, which throws it.
    let v = (pull * 14.0).clamp_length_max(700.0);
    b.set_linvel(vector![v.x, v.y, v.z], true);
    if buttons.pressed(MouseButton::Right) {
        // Turn it with the mouse: sideways spins it round, up and down tips it towards you.
        let (cam, _) = view.egg_cam.unwrap_or(egg.cam);
        let to_cam = (cam - Vec3::new(held.x, held.y, held.z)).normalize_or(Vec3::Z);
        let right = Vec3::Y.cross(to_cam).normalize_or(Vec3::X);
        let dt = time.delta_seconds().max(1e-3);
        let w = (Vec3::Y * turn.x + right * turn.y) * (0.012 / dt);
        let w = w.clamp_length_max(30.0);
        b.set_angvel(vector![w.x, w.y, w.z], true);
    } else {
        let w = *b.angvel() * (1.0 - (time.delta_seconds() * 6.0).min(0.9));
        b.set_angvel(w, true);
    }
}

