use sdl2::render::Texture;

use super::Plotter;
use crate::screen::Screen;
use crate::Mode;

impl Plotter<'_> {
    pub(super) fn display(
        &mut self,
        screen: &mut Screen<'_>,
        texture: &mut Texture,
    ) -> anyhow::Result<()> {
        let rotation = if self.state.course_up
            && self.boat.position_valid
            && self.boat.course_over_ground.is_finite()
        {
            self.boat.course_over_ground
        } else {
            0.0
        };
        self.renderer.set_rotation(rotation);

        if self.state.position.is_none() {
            self.state.position = Some(self.renderer.initial_center());
        }

        let (lon, lat) = match self.state.follow {
            true if super::valid_boat_position(&self.boat) => {
                (self.boat.longitude, self.boat.latitude)
            }
            _ => self
                .state
                .position
                .ok_or(anyhow::anyhow!("position de la carte inconnue"))?,
        };

        let (pixels, texture_rect) = self.renderer.get_raw_rgba(lon, lat, self.state.map_offset);
        texture
            .update(
                Some(texture_rect),
                &pixels,
                (texture_rect.width() * 4) as usize,
            )
            .map_err(|error| anyhow::anyhow!(error))?;
        screen
            .show_map(texture, self.renderer.placement())
            .map_err(anyhow::Error::msg)?;

        if self.mode == Mode::Measurement {
            self.mesure.show(&self.renderer, screen);
        }

        if self.state.display_route {
            self.display_route(screen);
        }

        self.boat
            .show(&self.renderer, screen)
            .map_err(anyhow::Error::msg)?;

        if self.state.display_ais {
            crate::ais::show_targets(&self.ais_targets, &self.renderer, screen)
                .map_err(anyhow::Error::msg)?;
        }

        if self.state.display_track {
            self.track
                .show(screen, &self.renderer, Some(self.boat.position()));
        }

        if self.state.display_waypoint {
            self.waypoints
                .show(screen, &self.renderer)
                .map_err(anyhow::Error::msg)?;
        }

        screen.night_layer(0).map_err(anyhow::Error::msg)?;
        self.display_buttombar(screen);
        if let Some(streamer) = &self.streamer {
            streamer
                .capture(&screen.canvas)
                .map_err(anyhow::Error::msg)?;
        }
        self.topbar.show(&mut screen.canvas);
        self.confirm_dialog
            .show(&mut screen.canvas)
            .map_err(anyhow::Error::msg)?;
        screen.canvas.present();

        Ok(())
    }

    pub(super) fn mouse(&mut self, screen: &mut Screen<'_>) {
        match self.mouse {
            super::Mouse::Arrow => screen.cursor_arrow.set(),
            super::Mouse::Cross => screen.cursor_cross.set(),
            super::Mouse::Hand => screen.cursor_hand.set(),
            super::Mouse::Move => screen.cursor_size.set(),
        }
    }

    pub(super) fn display_buttombar(&mut self, screen: &mut Screen<'_>) {
        if let Some(pending) = &self.pending_waypoint {
            let message_left = format!("Nom du waypoint : {}", pending.name);
            let message_right = "Entrée : valider | Échap : annuler";
            self.buttombar
                .show(&mut screen.canvas, &message_left, message_right);
            return;
        }

        let navigation = if self.state.goto {
            self.route
                .calc_route_mn_from_boat(self.state.nav_id, self.boat.position())
        } else {
            None
        };
        let message_left = match navigation {
            Some((dist_wp, bearing_wp, dist_total)) => format!(
                "From Boat to Next Waypoint :  {:.1} Mn   {:.0}° / DTD : {:.1} Mn",
                dist_wp, bearing_wp, dist_total
            ),
            None => self.route.show_route_len(),
        };

        let mut message_right = match self.mode {
            Mode::EditRoute => "Mode Edit Route".to_string(),
            Mode::EditWaypoint => "Mode Edit Waypoint".to_string(),
            Mode::Measurement => "Mode Measurement".to_string(),
        };

        message_right += if self.state.follow { " / Follow" } else { "" };
        message_right += if self.state.course_up {
            " / Course Up"
        } else {
            " / North Up"
        };
        self.buttombar
            .show(&mut screen.canvas, &message_left, &message_right);
    }

    pub(super) fn display_route(&mut self, screen: &mut Screen<'_>) {
        if self.state.goto {
            self.route.show_route_from_boat(
                &self.renderer,
                screen,
                self.mode,
                self.state.nav_id,
                self.boat.position(),
            );
        } else {
            self.route
                .show_route_since_id(&self.renderer, screen, self.mode, 0);
        }
    }
}
