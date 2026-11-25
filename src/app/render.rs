use sdl2::render::Texture;

use super::Plotter;

/// The night overlay's opacity, over the chart and the photo button alike.
/// Night mode is not built yet: nothing is dimmed.
pub(super) const NIGHT_OPACITY: u8 = 0;
use crate::screen::Screen;
use crate::Mode;

impl Plotter<'_> {
    /// Mid-drag: the chart texture redrawn at the view from tiles already
    /// loaded, nothing else. The full frame follows the release.
    pub(super) fn redraw_chart_texture(&mut self, texture: &mut Texture) -> Result<(), String> {
        let (pixels, texture_rect) = self.session.renderer.render_draft();
        texture
            .update(
                Some(texture_rect),
                &pixels,
                (texture_rect.width() * 4) as usize,
            )
            .map_err(|error| error.to_string())
    }

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
            let collision_positions: Vec<_> = self
                .session
                .collision_risks
                .iter()
                .map(|risk| risk.position)
                .collect();
            shapes.extend(crate::ais::shapes_without(
                &self.session.ais_targets,
                &collision_positions,
            ));
        }
        if self.alarm_blink_on {
            shapes.extend(self.session.collision_risks.iter().map(|risk| {
                let target = self
                    .session
                    .ais_targets
                    .iter()
                    .find(|target| target.position == Some(risk.position));
                crate::overlay::Shape::Collision {
                    at: risk.position,
                    course: target.and_then(|target| target.course_over_ground),
                    label: format!(
                        "{} {}",
                        if risk.kind == crate::collision::RiskKind::ProximityUncertain {
                            "PROXIMITÉ AIS"
                        } else {
                            "COLLISION"
                        },
                        risk.target
                    ),
                    uncertain: risk.kind == crate::collision::RiskKind::ProximityUncertain,
                }
            }));
        }
        if self.session.state.display_track {
            shapes.extend(self.session.track.shape(Some(self.session.boat.position())));
        }
        if self.session.state.display_waypoint {
            shapes.extend(self.session.waypoints.shapes());
        }
        if let Some(b) = self.session.highlight {
            shapes.push(crate::overlay::Shape::Outline(vec![
                (b.west, b.north),
                (b.east, b.north),
                (b.east, b.south),
                (b.west, b.south),
            ]));
        }
        let draws = crate::overlay::plan(&self.session.renderer.view(), &shapes);
        screen.paint(&draws).map_err(anyhow::Error::msg)?;

        screen
            .night_layer(NIGHT_OPACITY)
            .map_err(anyhow::Error::msg)?;
        let chart = self.session.chart_status();
        self.display_buttombar(screen, &chart);
        if self.session.collision_alarm_active() {
            let risk = self
                .session
                .collision_risks
                .iter()
                .find(|risk| risk.kind == crate::collision::RiskKind::ProximityUncertain)
                .unwrap_or(&self.session.collision_risks[0]);
            let uncertain = risk.kind == crate::collision::RiskKind::ProximityUncertain;
            let message = if uncertain {
                format!(
                    "PROXIMITE AIS INCERTAINE : {} — distance {:.2} NM",
                    risk.target, risk.cpa_nm
                )
            } else {
                format!(
                    "ALERTE COLLISION : {} — CPA {:.2} NM / TCPA {:.0} min",
                    risk.target, risk.cpa_nm, risk.tcpa_minutes
                )
            };
            screen
                .show_collision_alarm(&message, uncertain)
                .map_err(anyhow::Error::msg)?;
        }
        if let Some(streamer) = &self.streamer {
            streamer
                .capture(&screen.canvas)
                .map_err(anyhow::Error::msg)?;
        }
        if let Some(button) = self.session.photo_button() {
            let (centre, zoom) = self.session.view_centre_and_zoom();
            let thumbnail = button.preview.as_ref().and_then(|preview| {
                self.thumbnails.get(
                    &preview.path,
                    &preview.levels,
                    centre,
                    zoom,
                    super::photo_button::SIZE,
                )
            });
            self.photo_button
                .show(&mut screen.canvas, &button, thumbnail.as_ref());
            screen
                .night_rect(self.photo_button.area(), NIGHT_OPACITY)
                .map_err(anyhow::Error::msg)?;
        }
        self.topbar.show(&mut screen.canvas);
        if let Some(action) = self.session.confirmation.pending() {
            self.confirm_dialog
                .show(&mut screen.canvas, action)
                .map_err(anyhow::Error::msg)?;
        }
        if self.chart_manager.is_open() {
            self.chart_manager
                .show(&mut screen.canvas, &self.session.catalogue);
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
