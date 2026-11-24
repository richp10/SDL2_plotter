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

        // Overlays: each module says what it wants drawn, in chart
        // coordinates; the plan projects, culls and styles it; the screen paints.
        let mut shapes = Vec::new();
        if self.session.mode == Mode::Measurement {
            shapes.extend(self.session.mesure.shapes());
        }
        if self.session.state.display_route {
            let navigation = self
                .session
                .state
                .goto
                .then(|| (self.session.state.nav_id, self.session.boat.position()));
            shapes.extend(self.session.route.shapes(self.session.mode, navigation));
        }
        shapes.extend(self.session.boat.shape());
        if self.session.state.display_ais {
            shapes.extend(crate::ais::shapes(&self.session.ais_targets));
        }
        if self.session.state.display_track {
            shapes.extend(self.session.track.shape(Some(self.session.boat.position())));
        }
        if self.session.state.display_waypoint {
            shapes.extend(self.session.waypoints.shapes());
        }
        let draws = crate::overlay::plan(&self.session.renderer.view(), &shapes);
        screen.paint(&draws).map_err(anyhow::Error::msg)?;

        screen.night_layer(0).map_err(anyhow::Error::msg)?;
        let chart = self.session.chart_status();
        self.display_buttombar(screen, &chart.text);
        if chart.imagery {
            self.buttombar.show_photo_badge(&mut screen.canvas);
        }
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

    pub(super) fn display_buttombar(&mut self, screen: &mut Screen<'_>, chart: &str) {
        if let Some(pending) = &self.pending_waypoint {
            let message_left = format!("Nom du waypoint : {}", pending.name);
            let message_right = "Entrée : valider | Échap : annuler";
            self.buttombar
                .show(&mut screen.canvas, &message_left, "", message_right);
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
            .show(&mut screen.canvas, &message_left, chart, &message_right);
    }
}
