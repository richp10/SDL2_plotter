use sdl2::event::Event;
use sdl2::keyboard::Mod;
use std::time::Instant;

use super::{AppAction, Mouse, PendingWaypoint, Plotter, Press, LONG_CLICK, MOVE_TOLERANCE};
use crate::{screen, topbar, Mode};

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
                        self.waypoints
                            .add(pending.position, pending.name.trim().to_string());
                        if let Err(error) = self.waypoints.save() {
                            eprintln!("Erreur sauvegarde des waypoints : {error}");
                        }
                        println!("Waypoint ajouté : {}", pending.name.trim());
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

    pub(super) fn handle_keyboard_event(&mut self, event: &Event, _map: &mut screen::Map<'_>) {
        if self.pending_waypoint.is_some() {
            self.handle_pending_waypoint_event(event);
            return;
        }

        match event {
            Event::KeyDown {
                keycode: Some(keycode),
                keymod,
                repeat: false,
                ..
            } => {
                let fast = keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD);
                let distance = if fast { 1000 } else { 100 };
                let action = match *keycode {
                    sdl2::keyboard::Keycode::Escape => match self.mode {
                        Mode::Measurement => {
                            self.mesure.clear();
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
                    sdl2::keyboard::Keycode::Tab => Some(AppAction::CycleMode),
                    sdl2::keyboard::Keycode::P => Some(AppAction::PreviousWaypoint),
                    sdl2::keyboard::Keycode::N => Some(AppAction::NextWaypoint),
                    _ => None,
                };
                if let Some(action) = action {
                    self.execute_action(action);
                }
            }
            _ => {}
        }
    }

    pub(super) fn handle_topbar_event(&mut self, x: u32, y: u32) {
        if let Some(name) = self.topbar.event(x, y) {
            let action = match name.as_str() {
                topbar::EDIT_ROUTE => Some(AppAction::SetMode(Mode::EditRoute)),
                topbar::EDIT_WP => Some(AppAction::SetMode(Mode::EditWaypoint)),
                topbar::EDIT_MESURE => Some(AppAction::SetMode(Mode::Measurement)),
                topbar::CLEAR_ROUTE => Some(AppAction::ClearRoute),
                topbar::FOLLOW => Some(AppAction::ToggleFollow),
                topbar::PREVIOUS => Some(AppAction::PreviousWaypoint),
                topbar::GOTO => Some(AppAction::ToggleGoto),
                topbar::NEXT => Some(AppAction::NextWaypoint),
                topbar::QUITTE => Some(AppAction::Quit),
                topbar::CLEAR_TRACK => Some(AppAction::ClearTrack),
                topbar::SAVE_TRACK => Some(AppAction::SaveTrack),
                topbar::CLEAR_WP => Some(AppAction::ClearWaypoints),
                topbar::SAVE_WP => Some(AppAction::SaveWaypoints),
                topbar::SHOW_WP => Some(AppAction::ShowWaypoints),
                topbar::HIDE_WP => Some(AppAction::HideWaypoints),
                topbar::SHOW_TRACK => Some(AppAction::ShowTrack),
                topbar::HIDE_TRACK => Some(AppAction::HideTrack),
                topbar::SHOW_ROUTE => Some(AppAction::ShowRoute),
                topbar::HIDE_ROUTE => Some(AppAction::HideRoute),
                topbar::SHOW_AIS => Some(AppAction::ShowAis),
                topbar::HIDE_AIS => Some(AppAction::HideAis),
                _ => self
                    .charts_name
                    .iter()
                    .position(|chart_name| chart_name == &name)
                    .map(AppAction::ChangeChart),
            };
            if let Some(action) = action {
                self.execute_action(action);
            }
        }
    }

    pub(super) fn match_event(
        &mut self,
        event: Event,
        map: &mut screen::Map<'_>,
        texture: &sdl2::render::Texture,
    ) -> Result<(), String> {
        match &event {
            Event::KeyDown {
                keycode, repeat, ..
            } => {
                crate::diagnostics::record(
                    "DEBUG",
                    format!("Événement clavier: touche={keycode:?}, répétition={repeat}"),
                );
                self.handle_keyboard_event(&event, map);
            }
            Event::TextInput { .. } => {
                crate::diagnostics::record("DEBUG", "Événement de saisie texte");
                self.handle_keyboard_event(&event, map);
            }
            Event::MouseButtonDown {
                mouse_btn, x, y, ..
            } => {
                crate::diagnostics::record(
                    "DEBUG",
                    format!("Souris bouton enfoncé: {mouse_btn:?} ({x}, {y})"),
                );
                if self.pending_waypoint.is_none() {
                    self.handle_mouse_event(&event, map, texture)?;
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
                    self.handle_mouse_event(&event, map, texture)?;
                }
            }
            Event::MouseWheel { y, .. } => {
                crate::diagnostics::record("DEBUG", format!("Molette souris: {y}"));
                if self.pending_waypoint.is_none() {
                    self.handle_mouse_event(&event, map, texture)?;
                }
            }
            Event::MouseMotion { .. } => {
                if self.pending_waypoint.is_none() {
                    self.handle_mouse_event(&event, map, texture)?;
                }
            }
            Event::Quit { .. } => {
                crate::diagnostics::record("INFO", "Événement SDL Quit reçu");
                self.running = false;
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

    fn handle_mouse_down(&mut self, x: u32, y: u32) {
        if self.topbar.mouse(x, y) {
            self.handle_topbar_event(x, y);
        } else if self.topbar.is_open() {
            self.topbar.close();
            self.press = None;
        } else {
            self.drag_start = (x as i32, y as i32);
            self.dragging_measure_start = self.mode == Mode::Measurement
                && self
                    .mesure
                    .find_mark1_at(&self.renderer, x as i32, y as i32, 12);
            self.press = Some(Press {
                start: Instant::now(),
                x: x as i32,
                y: y as i32,
                moved: false,
                long_fired: false,
            });
        }
    }

    fn handle_mouse_up(&mut self, x: i32, y: i32, map: &mut screen::Map<'_>) {
        if let Some(p) = self.press.take() {
            if p.moved {
                if self.dragging_measure_start {
                    self.dragging_measure_start = false;
                    self.dirty = true;
                    self.mouse = Mouse::Move;
                } else if self.is_draging {
                    let dx = x - self.drag_start.0;
                    let dy = y - self.drag_start.1;
                    self.dirty = true;
                    self.is_draging = false;
                    self.execute_action(AppAction::Pan(dx, dy));
                    self.mouse = Mouse::Cross;
                } else {
                    match self.mode {
                        Mode::EditRoute => self.route.save(),
                        Mode::EditWaypoint => {
                            if let Err(error) = self.waypoints.save() {
                                eprintln!("Erreur sauvegarde des waypoints : {error}");
                            }
                        }
                        Mode::Measurement => {}
                    }
                }
            } else if !p.long_fired {
                if self.dragging_measure_start {
                    self.dragging_measure_start = false;
                    self.mouse = Mouse::Move;
                } else {
                    self.handle_short_click(map, x, y);
                }
            }
        }
    }

    fn handle_mouse_motion(
        &mut self,
        x: u32,
        y: u32,
        xrel: i32,
        yrel: i32,
        map: &mut screen::Map<'_>,
        texture: &sdl2::render::Texture,
    ) -> Result<(), String> {
        self.topbar.motion(x, y);
        if self.topbar.mouse(x, y) {
            self.mouse = Mouse::Arrow;
        } else {
            let (x, y) = (x as i32, y as i32);
            self.mouse = Mouse::Cross;
            match self.mode {
                Mode::EditRoute => {
                    self.handle_mouse_motion_edit_route(x, y, xrel, yrel, map, texture)?
                }
                Mode::EditWaypoint => {
                    self.handle_mouse_motion_edit_waypoint(x, y, xrel, yrel, map, texture)?
                }
                Mode::Measurement => {
                    self.handle_mouse_motion_measurement(x, y, xrel, yrel, map, texture)?
                }
            };
        }
        Ok(())
    }

    fn handle_mouse_wheel(&mut self, delta: i32) {
        if delta > 0 {
            self.state.zoom = self.renderer.zoom_increase(&self.state.map);
        } else if delta < 0 {
            self.state.zoom = self.renderer.zoom_decrease();
        }
        self.state.map_offset = (0, 0);
        self.dirty = true;
    }

    pub(super) fn handle_mouse_event(
        &mut self,
        event: &Event,
        map: &mut screen::Map<'_>,
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
                    self.handle_mouse_up(*x, *y, map);
                }
            }
            Event::MouseMotion {
                x, y, xrel, yrel, ..
            } => {
                if let (Ok(x), Ok(y), Ok(xrel), Ok(yrel)) = (
                    u32::try_from(*x),
                    u32::try_from(*y),
                    i32::try_from(*xrel),
                    i32::try_from(*yrel),
                ) {
                    self.handle_mouse_motion(x, y, xrel, yrel, map, texture)?;
                }
            }
            Event::MouseWheel { y, direction, .. } => {
                let delta = match direction {
                    sdl2::mouse::MouseWheelDirection::Flipped => -y.signum(),
                    _ => y.signum(),
                };
                self.handle_mouse_wheel(delta);
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_move_map(
        &mut self,
        x: i32,
        y: i32,
        xrel: i32,
        yrel: i32,
        map: &mut screen::Map<'_>,
        texture: &sdl2::render::Texture,
    ) -> Result<(), String> {
        if let Some(p) = self.press.as_mut() {
            if !p.moved {
                let (dx, dy) = (x - p.x, y - p.y);
                if dx * dx + dy * dy > MOVE_TOLERANCE * MOVE_TOLERANCE {
                    p.moved = true;
                    self.is_draging = true;
                    self.drag_start = (x, y);
                    self.xdrag = 0;
                    self.ydrag = 0;
                }
            }
            if p.moved && self.is_draging {
                self.xdrag += xrel;
                self.ydrag += yrel;
                map.drag_map(texture, self.xdrag, self.ydrag)?;
                self.mouse = Mouse::Hand;
            }
        }
        Ok(())
    }

    fn handle_mouse_motion_measurement(
        &mut self,
        x: i32,
        y: i32,
        xrel: i32,
        yrel: i32,
        map: &mut screen::Map<'_>,
        texture: &sdl2::render::Texture,
    ) -> Result<(), String> {
        let over_mark1 = self.mesure.find_mark1_at(&self.renderer, x, y, 12);
        if let Some(p) = self.press.as_mut() {
            if self.dragging_measure_start {
                let (dx, dy) = (x - p.x, y - p.y);
                if dx * dx + dy * dy > MOVE_TOLERANCE * MOVE_TOLERANCE {
                    p.moved = true;
                }
                if p.moved {
                    let dx = map.screen_width as i32 / 2 - x;
                    let dy = map.screen_height as i32 / 2 - y;
                    self.mesure.modify_mark1(&self.renderer, dx, dy);
                    self.mouse = Mouse::Move;
                    self.dirty = true;
                }
            } else {
                self.handle_move_map(x, y, xrel, yrel, map, texture)?;
            }
        } else if over_mark1 {
            self.mouse = Mouse::Move;
        } else {
            self.mouse = Mouse::Cross;
        }
        Ok(())
    }

    fn handle_click_mesure(&mut self, map: &mut screen::Map<'_>, x: i32, y: i32) {
        let dx = map.screen_width as i32 / 2 - x;
        let dy = map.screen_height as i32 / 2 - y;
        self.mesure.add_mark(&self.renderer, dx, dy);
        self.dirty = true;
    }

    fn handle_click_edit_route(&mut self, map: &mut screen::Map<'_>, x: i32, y: i32) {
        self.selected = self.route.find_point_at(&self.renderer, (x, y), 8);
        if self.selected == None {
            if let Some(k) = self.route.find_midpoint_at(&self.renderer, (x, y), 8) {
                let (longitude, latitude) = self.route.midpoints()[k];
                self.route.insert(k, longitude, latitude);
            } else {
                let dx = map.screen_width as i32 / 2 - x;
                let dy = map.screen_height as i32 / 2 - y;
                let (longitude, latitude) = self.renderer.screen_to_gps(dx, dy);
                self.route.add(longitude, latitude);
            }
            self.mouse = Mouse::Cross;
        }
        self.route.save();
        self.dirty = true;
    }

    fn handle_click_edit_wp(&mut self, map: &mut screen::Map<'_>, x: i32, y: i32) {
        let dx = map.screen_width as i32 / 2 - x;
        let dy = map.screen_height as i32 / 2 - y;
        self.pending_waypoint = Some(PendingWaypoint {
            position: self.renderer.screen_to_gps(dx, dy),
            name: String::new(),
        });
        self.text_input.start();
        self.dirty = true;
    }

    fn handle_short_click(&mut self, map: &mut screen::Map<'_>, x: i32, y: i32) {
        match self.mode {
            Mode::Measurement => self.handle_click_mesure(map, x, y),
            Mode::EditRoute => self.handle_click_edit_route(map, x, y),
            Mode::EditWaypoint => self.handle_click_edit_wp(map, x, y),
        };
    }

    fn handle_long_click_edit_route(&mut self, x: i32, y: i32) {
        if let Some(i) = self.route.find_point_at(&self.renderer, (x, y), 8) {
            self.route.remove(i);
            self.route.save();
            self.dirty = true;
            self.mouse = Mouse::Cross;
        }
    }

    fn handle_long_click_edit_wp(&mut self, x: i32, y: i32) {
        if let Some(i) = self.waypoints.find_point_at(&self.renderer, (x, y), 8) {
            self.waypoints.remove(i);
            if let Err(error) = self.waypoints.save() {
                eprintln!("Erreur sauvegarde des waypoints : {error}");
            }
            self.dirty = true;
            self.mouse = Mouse::Cross;
        }
    }

    fn handle_long_click(&mut self, x: i32, y: i32) {
        match self.mode {
            Mode::EditRoute => self.handle_long_click_edit_route(x, y),
            Mode::EditWaypoint => self.handle_long_click_edit_wp(x, y),
            Mode::Measurement => {}
        };
    }

    pub(super) fn long_click(&mut self) {
        let click_position = if let Some(p) = self.press.as_mut() {
            if !p.moved && !p.long_fired && p.start.elapsed() >= LONG_CLICK {
                p.long_fired = true;
                Some((p.x, p.y))
            } else {
                None
            }
        } else {
            None
        };

        if let Some((x, y)) = click_position {
            self.handle_long_click(x, y);
        }
    }

    fn handle_mouse_motion_edit_route(
        &mut self,
        x: i32,
        y: i32,
        xrel: i32,
        yrel: i32,
        map: &mut screen::Map<'_>,
        texture: &sdl2::render::Texture,
    ) -> Result<(), String> {
        let previous_selection = self.selected;
        self.selected = self.route.find_point_at(&self.renderer, (x, y), 8);

        if self.selected.is_some() {
            if self.selected != previous_selection {
                crate::diagnostics::record(
                    "DEBUG",
                    format!("Sélection route modifiée: {:?}", self.selected),
                );
            }
            self.mouse = Mouse::Move;
        }
        if self
            .route
            .find_midpoint_at(&self.renderer, (x, y), 8)
            .is_some()
        {
            self.mouse = Mouse::Arrow;
        }
        if let Some(p) = self.press.as_mut() {
            if let Some(i) = self.selected {
                if !p.moved {
                    p.moved = true;
                }
                if p.moved {
                    let dx = map.screen_width as i32 / 2 - x;
                    let dy = map.screen_height as i32 / 2 - y;
                    self.mouse = Mouse::Move;
                    let (lon, lat) = self.renderer.screen_to_gps(dx, dy);
                    self.route.modify(i, lon, lat);
                    self.dirty = true;
                }
            } else {
                self.handle_move_map(x, y, xrel, yrel, map, texture)?;
            }
        }
        Ok(())
    }

    fn handle_mouse_motion_edit_waypoint(
        &mut self,
        x: i32,
        y: i32,
        xrel: i32,
        yrel: i32,
        map: &mut screen::Map<'_>,
        texture: &sdl2::render::Texture,
    ) -> Result<(), String> {
        let previous_selection = self.selected;
        self.selected = self.waypoints.find_point_at(&self.renderer, (x, y), 8);

        if self.selected.is_some() {
            if self.selected != previous_selection {
                crate::diagnostics::record(
                    "DEBUG",
                    format!("Sélection waypoint modifiée: {:?}", self.selected),
                );
            }
            self.mouse = Mouse::Move;
        }

        if let Some(p) = self.press.as_mut() {
            if let Some(i) = self.selected {
                if !p.moved {
                    p.moved = true;
                }
                if p.moved {
                    let dx = map.screen_width as i32 / 2 - x;
                    let dy = map.screen_height as i32 / 2 - y;
                    self.mouse = Mouse::Move;
                    let (lon, lat) = self.renderer.screen_to_gps(dx, dy);
                    self.waypoints.modify(i, (lon, lat));
                    self.dirty = true;
                }
            } else {
                self.handle_move_map(x, y, xrel, yrel, map, texture)?;
            }
        }
        Ok(())
    }
}
