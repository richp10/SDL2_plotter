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
        // State may have moved since the loop synced (boat and track updates).
        self.session.sync_view();
        let (pixels, texture_rect) = self.session.renderer.render_frame();
        texture
            .update(
                Some(texture_rect),
                &pixels,
                (texture_rect.width() * 4) as usize,
            )
            .map_err(|error| anyhow::anyhow!(error))?;
        screen
            .show_map(texture, self.session.renderer.placement())
            .map_err(anyhow::Error::msg)?;

        if self.session.mode == Mode::Measurement {
            self.session.mesure.show(&self.session.renderer, screen);
        }

        if self.session.state.display_route {
            self.display_route(screen);
        }

        self.session
            .boat
            .show(&self.session.renderer, screen)
            .map_err(anyhow::Error::msg)?;

        if self.session.state.display_ais {
            crate::ais::show_targets(&self.ais_targets, &self.session.renderer, screen)
                .map_err(anyhow::Error::msg)?;
        }

        if self.session.state.display_track {
            self.session.track.show(
                screen,
                &self.session.renderer,
                Some(self.session.boat.position()),
            );
        }

        if self.session.state.display_waypoint {
            self.session
                .waypoints
                .show(screen, &self.session.renderer)
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
        if let Some(action) = self.session.confirmation.pending() {
            self.confirm_dialog
                .show(&mut screen.canvas, action)
                .map_err(anyhow::Error::msg)?;
        }
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

        let navigation = if self.session.state.goto {
            self.session
                .route
                .calc_route_mn_from_boat(self.session.state.nav_id, self.session.boat.position())
        } else {
            None
        };
        let message_left = match navigation {
            Some((dist_wp, bearing_wp, dist_total)) => format!(
                "From Boat to Next Waypoint :  {:.1} Mn   {:.0}° / DTD : {:.1} Mn",
                dist_wp, bearing_wp, dist_total
            ),
            None => self.session.route.show_route_len(),
        };

        let mut message_right = match self.session.mode {
            Mode::EditRoute => "Mode Edit Route".to_string(),
            Mode::EditWaypoint => "Mode Edit Waypoint".to_string(),
            Mode::Measurement => "Mode Measurement".to_string(),
        };

        message_right += if self.session.state.follow {
            " / Follow"
        } else {
            ""
        };
        message_right += if self.session.state.course_up {
            " / Course Up"
        } else {
            " / North Up"
        };
        self.buttombar
            .show(&mut screen.canvas, &message_left, &message_right);
    }

    pub(super) fn display_route(&mut self, screen: &mut Screen<'_>) {
        if self.session.state.goto {
            self.session.route.show_route_from_boat(
                &self.session.renderer,
                screen,
                self.session.mode,
                self.session.state.nav_id,
                self.session.boat.position(),
            );
        } else {
            self.session.route.show_route_since_id(
                &self.session.renderer,
                screen,
                self.session.mode,
                0,
            );
        }
    }
}
