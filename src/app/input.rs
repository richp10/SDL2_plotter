//! SDL events in, gestures and actions out. Nothing in here decides what a
//! click means: the recogniser turns events into gestures and the mode's
//! tool on `Session` acts on them. What stays here is what needs SDL: the
//! text input for a waypoint name, the cursor, and dragging the chart
//! texture while the user pans.

use sdl2::event::Event;
use sdl2::keyboard::Mod;
use sdl2::rect::Point;
use std::time::Instant;

use super::chart_manager::{Cell, ChartEdit, Hit};
use super::gesture::Gesture;
use super::tools::{ClickRequest, Target};
use super::{AppAction, Mouse, PendingWaypoint, Plotter};
use crate::screen::Screen;
use crate::Mode;

impl Plotter<'_> {
    pub(super) fn handle_pending_waypoint_event(&mut self, event: &Event) {
        match event {
            Event::TextInput { text, .. } => {
                if let Some(pending) = self.pending_waypoint.as_mut() {
                    for character in text.chars().filter(|character| !character.is_control()) {
                        if pending.name.chars().count() >= 64 {
                            break;
                        }
                        pending.name.push(character);
                    }
                }
                self.dirty = true;
            }
            Event::KeyDown {
                keycode: Some(sdl2::keyboard::Keycode::Return | sdl2::keyboard::Keycode::KpEnter),
                repeat: false,
                ..
            } => {
                if self
                    .pending_waypoint
                    .as_ref()
                    .is_some_and(|pending| !pending.name.trim().is_empty())
                {
                    if let Some(pending) = self.pending_waypoint.take() {
                        self.text_input.stop();
                        self.session.name_waypoint(pending.position, &pending.name);
                    }
                }
                self.dirty = true;
            }
            Event::KeyDown {
                keycode: Some(sdl2::keyboard::Keycode::Escape),
                repeat: false,
                ..
            } => {
                self.pending_waypoint = None;
                self.text_input.stop();
                self.dirty = true;
            }
            Event::KeyDown {
                keycode: Some(sdl2::keyboard::Keycode::Backspace),
                repeat: false,
                ..
            } => {
                if let Some(pending) = self.pending_waypoint.as_mut() {
                    pending.name.pop();
                }
                self.dirty = true;
            }
            _ => {}
        }
    }

    /// Every event while the chart manager is open: taps on its cells,
    /// the wheel for pages, typing a name, Escape to close.
    pub(super) fn handle_chart_manager_event(&mut self, event: &Event) {
        let rows = self.session.catalogue.files().len();
        match event {
            Event::MouseButtonDown {
                mouse_btn: sdl2::mouse::MouseButton::Left,
                x,
                y,
                ..
            } => {
                if let Some(hit) = self.chart_manager.hit(*x, *y, rows) {
                    self.chart_manager_hit(hit);
                }
            }
            Event::MouseWheel { y, .. } => self.chart_manager.page(-y.signum(), rows),
            Event::TextInput { text, .. } => {
                if let Some((_, buffer)) = self.chart_manager.typing.as_mut() {
                    for c in text.chars().filter(|c| !c.is_control()) {
                        if buffer.chars().count() < 64 {
                            buffer.push(c);
                        }
                    }
                }
            }
            Event::KeyDown {
                keycode: Some(keycode),
                repeat: false,
                ..
            } => match (*keycode, self.chart_manager.typing.is_some()) {
                (sdl2::keyboard::Keycode::Return | sdl2::keyboard::Keycode::KpEnter, true) => {
                    if let Some((file, name)) = self.chart_manager.typing.take() {
                        self.text_input.stop();
                        self.session.edit_chart(&file, ChartEdit::Rename(name));
                    }
                }
                (sdl2::keyboard::Keycode::Backspace, true) => {
                    if let Some((_, buffer)) = self.chart_manager.typing.as_mut() {
                        buffer.pop();
                    }
                }
                (sdl2::keyboard::Keycode::Escape, true) => {
                    self.chart_manager.typing = None;
                    self.text_input.stop();
                }
                (sdl2::keyboard::Keycode::Escape, false) => self.close_chart_manager(),
                _ => {}
            },
            _ => return,
        }
        self.dirty = true;
    }

    fn chart_manager_hit(&mut self, hit: Hit) {
        let rows = self.session.catalogue.files().len();
        let (index, cell) = match hit {
            Hit::Close => return self.close_chart_manager(),
            Hit::NextPage => return self.chart_manager.page(1, rows),
            Hit::PreviousPage => return self.chart_manager.page(-1, rows),
            Hit::Row(index, cell) => (index, cell),
        };
        let Some(file) = self.session.catalogue.files().get(index) else {
            return;
        };
        let (name, display, reviewed) = (file.file_name.clone(), file.name.clone(), file.reviewed);
        let edit = match cell {
            Cell::Up => ChartEdit::Up,
            Cell::Down => ChartEdit::Down,
            Cell::Layer => ChartEdit::NextLayer,
            Cell::Role => ChartEdit::ToggleRole,
            Cell::Shown => ChartEdit::ToggleHidden,
            Cell::Zooms => return,
            Cell::Name if !reviewed => ChartEdit::Review,
            Cell::Name => {
                self.chart_manager.typing = Some((name, display));
                self.text_input.start();
                return;
            }
            Cell::Where => {
                self.close_chart_manager();
                self.session.locate_chart(&name);
                return;
            }
        };
        self.session.edit_chart(&name, edit);
    }

    pub(super) fn handle_keyboard_event(&mut self, event: &Event) {
        if self.pending_waypoint.is_some() {
            self.handle_pending_waypoint_event(event);
            return;
        }

        if let Event::KeyDown {
            keycode: Some(keycode),
            keymod,
            repeat: false,
            ..
        } = event
        {
            let fast = keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD);
            let distance = if fast { 1000 } else { 100 };
            let action = match *keycode {
                sdl2::keyboard::Keycode::Escape => match self.session.mode {
                    Mode::Measurement => {
                        self.session.mesure.clear();
                        self.dirty = true;
                        None
                    }
                    Mode::EditRoute => None,
                    Mode::EditWaypoint => None,
                },
                sdl2::keyboard::Keycode::Up => Some(AppAction::Pan(0, distance)),
                sdl2::keyboard::Keycode::Down => Some(AppAction::Pan(0, -distance)),
                sdl2::keyboard::Keycode::Left => Some(AppAction::Pan(distance, 0)),
                sdl2::keyboard::Keycode::Right => Some(AppAction::Pan(-distance, 0)),
                sdl2::keyboard::Keycode::KpPlus => Some(AppAction::ZoomIn),
                sdl2::keyboard::Keycode::KpMinus => Some(AppAction::ZoomOut),
                sdl2::keyboard::Keycode::F => Some(AppAction::ToggleFollow),
                sdl2::keyboard::Keycode::L => Some(AppAction::ToggleLock),
                sdl2::keyboard::Keycode::I => Some(AppAction::NextPhoto),
                sdl2::keyboard::Keycode::C => Some(AppAction::OpenChartManager),
                sdl2::keyboard::Keycode::Tab => Some(AppAction::CycleMode),
                sdl2::keyboard::Keycode::P => Some(AppAction::PreviousWaypoint),
                sdl2::keyboard::Keycode::N => Some(AppAction::NextWaypoint),
                _ => None,
            };
            if let Some(action) = action {
                self.execute_action(action);
            }
        }
    }

    pub(super) fn handle_topbar_event(&mut self, x: u32, y: u32) {
        if let Some(action) = self.topbar.event(x, y) {
            self.execute_action(action);
        }
    }

    pub(super) fn match_event(
        &mut self,
        event: Event,
        screen: &mut Screen<'_>,
        texture: &sdl2::render::Texture,
    ) -> Result<(), String> {
        if let Event::Quit { .. } = event {
            // Falls through to the normal handling below.
        } else if self.chart_manager.is_open() {
            self.handle_chart_manager_event(&event);
            return Ok(());
        }
        if self.session.confirmation.is_open() {
            match event {
                Event::KeyDown {
                    keycode: Some(sdl2::keyboard::Keycode::Escape),
                    repeat: false,
                    ..
                } => {
                    self.session.confirmation.resolve(false);
                    self.dirty = true;
                }
                Event::KeyDown {
                    keycode:
                        Some(sdl2::keyboard::Keycode::Return | sdl2::keyboard::Keycode::KpEnter),
                    repeat: false,
                    ..
                } => {
                    if let Some(action) = self.session.confirmation.resolve(true) {
                        self.apply_confirmation(action);
                    }
                }
                Event::MouseButtonUp {
                    mouse_btn: sdl2::mouse::MouseButton::Left,
                    x,
                    y,
                    ..
                } => {
                    if let Some(confirm) = self.confirm_dialog.handle_click(x, y) {
                        if let Some(action) = self.session.confirmation.resolve(confirm) {
                            self.apply_confirmation(action);
                        } else {
                            self.dirty = true;
                        }
                    }
                }
                _ => {}
            }
            return Ok(());
        }

        match &event {
            Event::KeyDown {
                keycode, repeat, ..
            } => {
                crate::diagnostics::record(
                    "DEBUG",
                    format!("Événement clavier: touche={keycode:?}, répétition={repeat}"),
                );
                self.handle_keyboard_event(&event);
            }
            Event::TextInput { .. } => {
                crate::diagnostics::record("DEBUG", "Événement de saisie texte");
                self.handle_keyboard_event(&event);
            }
            Event::MouseButtonDown {
                mouse_btn, x, y, ..
            } => {
                crate::diagnostics::record(
                    "DEBUG",
                    format!("Souris bouton enfoncé: {mouse_btn:?} ({x}, {y})"),
                );
                if self.pending_waypoint.is_none() {
                    self.handle_mouse_event(&event, screen, texture)?;
                }
            }
            Event::MouseButtonUp {
                mouse_btn, x, y, ..
            } => {
                crate::diagnostics::record(
                    "DEBUG",
                    format!("Souris bouton relâché: {mouse_btn:?} ({x}, {y})"),
                );
                if self.pending_waypoint.is_none() {
                    self.handle_mouse_event(&event, screen, texture)?;
                }
            }
            Event::MouseWheel { y, .. } => {
                crate::diagnostics::record("DEBUG", format!("Molette souris: {y}"));
                if self.pending_waypoint.is_none() {
                    self.handle_mouse_event(&event, screen, texture)?;
                }
            }
            Event::MouseMotion { .. } => {
                if self.pending_waypoint.is_none() {
                    self.handle_mouse_event(&event, screen, texture)?;
                }
            }
            Event::Quit { .. } => {
                crate::diagnostics::record("INFO", "Événement SDL Quit reçu");
                self.session.running = false;
            }
            Event::Window { win_event, .. } => {
                crate::diagnostics::record(
                    "DEBUG",
                    format!("Événement fenêtre SDL: {win_event:?}"),
                );
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn handle_mouse_event(
        &mut self,
        event: &Event,
        screen: &mut Screen<'_>,
        texture: &sdl2::render::Texture,
    ) -> Result<(), String> {
        match event {
            Event::MouseButtonDown {
                mouse_btn, x, y, ..
            } => {
                if *mouse_btn == sdl2::mouse::MouseButton::Left {
                    if let (Ok(x), Ok(y)) = (u32::try_from(*x), u32::try_from(*y)) {
                        self.handle_mouse_down(x, y);
                    }
                }
            }
            Event::MouseButtonUp {
                mouse_btn, x, y, ..
            } => {
                if *mouse_btn == sdl2::mouse::MouseButton::Left {
                    self.handle_mouse_up(Point::new(*x, *y));
                }
            }
            Event::MouseMotion { x, y, .. } => {
                if let (Ok(ux), Ok(uy)) = (u32::try_from(*x), u32::try_from(*y)) {
                    self.handle_mouse_motion(ux, uy, screen, texture)?;
                }
            }
            Event::MouseWheel { y, direction, .. } => {
                let delta = match direction {
                    sdl2::mouse::MouseWheelDirection::Flipped => -y.signum(),
                    _ => y.signum(),
                };
                if delta > 0 {
                    self.execute_action(AppAction::ZoomIn);
                } else if delta < 0 {
                    self.execute_action(AppAction::ZoomOut);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_mouse_down(&mut self, x: u32, y: u32) {
        if !self.topbar.is_open() && y < crate::topbar::HEIGHT {
            // A menu is about to open: say which photos exist here now.
            self.topbar.set_layers(self.session.layer_labels());
        }
        if self.topbar.mouse(x, y) {
            self.handle_topbar_event(x, y);
            // Opening, switching or closing a menu changes the frame even
            // when no entry was chosen; without this the menu is only drawn
            // when something else (a Signal K update) redraws.
            self.dirty = true;
        } else if self.topbar.is_open() {
            self.topbar.close();
            self.dirty = true;
        } else if self
            .photo_button
            .area()
            .contains_point((x as i32, y as i32))
            && self.session.photo_button().is_some()
        {
            self.execute_action(AppAction::NextPhoto);
        } else {
            self.gestures
                .press(Point::new(x as i32, y as i32), Instant::now());
        }
    }

    fn handle_mouse_up(&mut self, at: Point) {
        match self.gestures.release(at) {
            Some(Gesture::Click(at)) => {
                if let Some(ClickRequest::NameWaypoint(position)) = self.session.click(at) {
                    self.pending_waypoint = Some(PendingWaypoint {
                        position,
                        name: String::new(),
                    });
                    self.text_input.start();
                }
                self.dirty = true;
            }
            Some(Gesture::DragEnd { origin, at }) => {
                match self.grab.take() {
                    Some(Target::Chart) => {
                        self.execute_action(AppAction::Pan(at.x - origin.x, at.y - origin.y));
                    }
                    Some(target) => self.session.drag_end(target),
                    None => {}
                }
                self.mouse = Mouse::Cross;
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn handle_mouse_motion(
        &mut self,
        x: u32,
        y: u32,
        screen: &mut Screen<'_>,
        texture: &sdl2::render::Texture,
    ) -> Result<(), String> {
        if self.topbar.motion(x, y) {
            self.dirty = true;
        }
        if self.topbar.mouse(x, y) {
            self.mouse = Mouse::Arrow;
            return Ok(());
        }
        let at = Point::new(x as i32, y as i32);
        match self.gestures.motion(at) {
            Some(Gesture::DragStart { origin, at }) => {
                // The grab is decided once, at the press, and then locked.
                self.grab = Some(self.session.target_at(origin));
                self.drag(origin, at, screen, texture)
            }
            Some(Gesture::DragMove { origin, at }) => self.drag(origin, at, screen, texture),
            _ => {
                self.mouse = match self.session.target_at(at) {
                    Target::RoutePoint(_) | Target::Waypoint(_) | Target::MeasureMark => {
                        Mouse::Move
                    }
                    Target::RouteHandle(_) => Mouse::Arrow,
                    Target::Chart => Mouse::Cross,
                };
                Ok(())
            }
        }
    }

    fn drag(
        &mut self,
        origin: Point,
        at: Point,
        screen: &mut Screen<'_>,
        texture: &sdl2::render::Texture,
    ) -> Result<(), String> {
        match self.grab {
            Some(Target::Chart) => {
                // The chart is dragged as a texture for immediacy; the pan
                // becomes an action when the button is released.
                screen.drag_map(
                    texture,
                    (at.x - origin.x, at.y - origin.y),
                    self.session.renderer.placement(),
                )?;
                self.mouse = Mouse::Hand;
            }
            Some(target) => {
                let moved = self.session.drag_to(target, at);
                if moved {
                    self.mouse = Mouse::Move;
                    self.dirty = true;
                }
            }
            None => {}
        }
        Ok(())
    }

    /// Called once per loop iteration: fires the long press when due.
    pub(super) fn fire_long_press(&mut self) {
        if let Some(Gesture::LongPress(at)) = self.gestures.tick(Instant::now()) {
            if self.session.long_press(at) {
                self.mouse = Mouse::Cross;
                self.dirty = true;
            }
        }
    }

    /// True while the chart texture is being dragged, when the frame is
    /// presented by `drag` rather than by `display`.
    pub(super) fn dragging_chart(&self) -> bool {
        self.gestures.is_dragging() && self.grab == Some(Target::Chart)
    }
}
