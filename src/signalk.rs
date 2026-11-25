use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio_tungstenite::{
    connect_async, tungstenite::Error as WsError, tungstenite::Message, MaybeTlsStream,
    WebSocketStream,
};

use crate::ais::AisTarget;
use crate::boat::BoatState;
use crate::config::{SignalKConfig, SignalKPaths};
pub type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Les donnees SignalK collectees au fil de l'eau
#[derive(Debug, Default, Clone)]
pub struct SignalKData {
    pub latitude: f64,
    pub longitude: f64,
    pub position_valid: bool,
    pub cog: f64,
    pub sog: f64,
}

/// What one Signal K delta message changed, so the task only forwards the
/// data that actually moved.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DeltaSummary {
    /// Own vessel position, course or speed changed.
    pub self_changed: bool,
    /// Own vessel position was updated (a new track point).
    pub self_position: bool,
    /// At least one AIS target was created or updated.
    pub ais_changed: bool,
}

/// Le client complet : connexion WebSocket + dernieres donnees connues
pub struct SignalKClient {
    write: SplitSink<WsStream, Message>,
    read: SplitStream<WsStream>,
    pub data: SignalKData,
    pub targets: HashMap<String, AisTarget>,
    self_context: Option<String>,
    url: String,
    paths: SignalKPaths,
}

impl SignalKClient {
    /// Se connecte (avec retry automatique) et cree le client
    pub async fn connect(config: SignalKConfig) -> Self {
        let ws_stream = wait_for_signalk_connection(&config.url).await;
        let (write, read) = ws_stream.split();

        SignalKClient {
            write,
            read,
            data: SignalKData::default(),
            targets: HashMap::new(),
            self_context: None,
            url: config.url,
            paths: config.paths,
        }
    }

    /// Reconnecte (par ex. apres une coupure), en gardant les dernieres donnees
    pub async fn reconnect(&mut self) {
        let ws_stream = wait_for_signalk_connection(&self.url).await;
        let (write, read) = ws_stream.split();
        self.write = write;
        self.read = read;
    }

    /// Lit le prochain message et met a jour `self.data` si c'est du SignalK delta
    pub async fn next_update(&mut self) -> Option<DeltaSummary> {
        while let Some(msg) = self.read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(text.as_str()) {
                        return Some(self.decode_updates(&json));
                    }
                }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => continue,
                Ok(Message::Close(_)) => return None,
                Err(e) => {
                    crate::diagnostics::record("WARN", format!("Erreur de lecture WebSocket: {e}"));
                    return None;
                }
                _ => continue,
            }
        }
        None
    }

    /// verifie que le premier message du sereveur est bien reçu
    pub async fn check_connect(&mut self) -> bool {
        let msg = self.read.next().await;
        match msg {
            Some(Ok(Message::Text(text))) => {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(text.as_str()) {
                    let status = &json["name"];
                    {
                        if status == "signalk-server" {
                            self.self_context = json["self"].as_str().map(str::to_string);
                            if self.self_context.is_none() {
                                crate::diagnostics::record("WARN", "Le serveur Signal K n'a pas fourni le contexte du bateau (self)");
                            }
                            crate::diagnostics::record("INFO", "Identification SignalK réussie");
                            return true;
                        } else {
                            crate::diagnostics::record(
                                "WARN",
                                "Réponse initiale SignalK inattendue",
                            );
                            return false;
                        }
                    }
                }
            }
            Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => return false,
            Some(Ok(Message::Close(_))) => return false,
            Some(Err(e)) => {
                crate::diagnostics::record("WARN", format!("Erreur de lecture WebSocket: {e}"));
                return false;
            }
            _ => return false,
        }
        false
    }

    /// suscribe les paths du tableau SUBSRBES_PATH au sever signalk
    pub async fn subscribe(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        #[derive(Serialize, Deserialize)]
        struct Subscribe {
            context: String,
            subscribe: Vec<Delta>,
        }

        #[derive(Serialize, Deserialize)]
        struct Delta {
            path: String,
            format: String,
            //period : u32,
            //policy: String,
            //#[serde(rename = "minPeriod")] // Assure la compatibilité avec le JSON de Signal K
            //min_period: u32,
        }

        crate::diagnostics::record("INFO", "Souscription SignalK aux données bateau et AIS");
        let self_paths = [
            self.paths.position.as_str(),
            self.paths.course_over_ground.as_str(),
            self.paths.speed_over_ground.as_str(),
        ];
        let deltas: Vec<Delta> = self_paths
            .iter()
            .map(|path| Delta {
                path: path.to_string(),
                format: "delta".to_string(),
                //period: PERIOD_MS,
                //policy: "instant".to_string(), // "instant" pousse les données dès qu'elles changent
                //min_period: PERIOD_MS,
            })
            .collect();

        let payload = Subscribe {
            context: "vessels.self".to_string(),
            subscribe: deltas,
        };
        let json_string = serde_json::to_string(&payload)?;
        self.write.send(Message::Text(json_string.into())).await?;

        let ais_paths = [
            self.paths.ais_position.as_str(),
            self.paths.ais_name.as_str(),
            self.paths.ais_course_over_ground.as_str(),
            self.paths.ais_speed_over_ground.as_str(),
        ];
        let ais_payload = Subscribe {
            context: "vessels.*".to_string(),
            subscribe: ais_paths
                .iter()
                .map(|path| Delta {
                    path: path.to_string(),
                    format: "delta".to_string(),
                })
                .collect(),
        };
        let json_string = serde_json::to_string(&ais_payload)?;
        self.write.send(Message::Text(json_string.into())).await?;
        Ok(())
    }

    /// interpretes les messages updates
    fn decode_updates(&mut self, v: &Value) -> DeltaSummary {
        apply_delta(
            &mut self.data,
            &mut self.targets,
            &self.paths,
            self.self_context.as_deref(),
            v,
        )
    }
}

/// Applies one Signal K delta message to the own-vessel data and the AIS
/// target map, and reports what changed.
fn apply_delta(
    data: &mut SignalKData,
    targets: &mut HashMap<String, AisTarget>,
    paths: &SignalKPaths,
    self_context: Option<&str>,
    v: &Value,
) -> DeltaSummary {
    let mut summary = DeltaSummary::default();
    let Some(updates) = v["updates"].as_array() else {
        return summary;
    };
    for update in updates {
        let Some(values) = update["values"].as_array() else {
            continue;
        };
        let context = update["context"]
            .as_str()
            .or_else(|| v["context"].as_str())
            .unwrap_or("vessels.self");
        for value in values {
            let val = &value["value"];
            let Some(path) = value["path"].as_str() else {
                continue;
            };
            if is_self_context(context, self_context) {
                if path == paths.position {
                    if let (Some(latitude), Some(longitude)) =
                        (val["latitude"].as_f64(), val["longitude"].as_f64())
                    {
                        if latitude.is_finite()
                            && longitude.is_finite()
                            && (-90.0..=90.0).contains(&latitude)
                            && (-180.0..=180.0).contains(&longitude)
                        {
                            data.latitude = latitude;
                            data.longitude = longitude;
                            data.position_valid = true;
                            summary.self_position = true;
                            summary.self_changed = true;
                        }
                    }
                } else if path == paths.course_over_ground {
                    if let Some(cog) = val.as_f64() {
                        data.cog = cog;
                        summary.self_changed = true;
                    }
                } else if path == paths.speed_over_ground {
                    if let Some(sog) = val.as_f64() {
                        data.sog = sog;
                        summary.self_changed = true;
                    }
                }
            } else if update_ais_target(targets, paths, context, path, val) {
                summary.ais_changed = true;
            }
        }
    }
    summary
}

fn is_self_context(context: &str, self_context: Option<&str>) -> bool {
    context == "vessels.self" || self_context == Some(context)
}

/// Updates the AIS target for `context` from one path/value pair.
/// Returns true when the target was created or changed.
fn update_ais_target(
    targets: &mut HashMap<String, AisTarget>,
    paths: &SignalKPaths,
    context: &str,
    path: &str,
    value: &Value,
) -> bool {
    if !context.starts_with("vessels.") || context == "vessels.self" {
        return false;
    }

    let target = targets
        .entry(context.to_string())
        .or_insert_with(|| AisTarget::new(context.to_string()));

    if path == paths.ais_position {
        if let (Some(latitude), Some(longitude)) =
            (value["latitude"].as_f64(), value["longitude"].as_f64())
        {
            if latitude.is_finite()
                && longitude.is_finite()
                && (-90.0..=90.0).contains(&latitude)
                && (-180.0..=180.0).contains(&longitude)
            {
                target.position = Some((longitude, latitude));
            }
        }
    } else if path == paths.ais_name {
        if let Some(name) = value.as_str().filter(|name| !name.trim().is_empty()) {
            target.name = Some(name.to_string());
        }
    } else if path == paths.ais_course_over_ground {
        if let Some(course) = value.as_f64().filter(|course| course.is_finite()) {
            target.course_over_ground = Some(course);
        }
    } else if path == paths.ais_speed_over_ground {
        if let Some(speed) = value
            .as_f64()
            .filter(|speed| speed.is_finite() && *speed >= 0.0)
        {
            target.speed_over_ground = Some(speed);
        }
    } else {
        return false;
    }
    target.update_received();
    true
}

async fn wait_for_signalk_connection(url: &str) -> WsStream {
    let mut attempt: u32 = 0;

    loop {
        attempt += 1;
        match connect_async(url).await {
            Ok((ws_stream, response)) => {
                crate::diagnostics::record(
                    "INFO",
                    format!(
                        "Connexion SignalK établie après {attempt} tentative(s), status {}",
                        response.status()
                    ),
                );
                return ws_stream;
            }
            Err(WsError::Http(response)) if response.status().as_u16() == 429 => {
                let wait = response
                    .headers()
                    .get("Retry-After")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or_else(|| (2u64.pow(attempt.min(5))).min(60));

                crate::diagnostics::record(
                    "WARN",
                    format!("SignalK HTTP 429: attente {wait}s (tentative {attempt})"),
                );
                tokio::time::sleep(Duration::from_secs(wait)).await;
            }
            Err(error) => {
                crate::diagnostics::record(
                    "WARN",
                    format!("Connexion SignalK échouée (tentative {attempt}): {error}"),
                );
                let delay = Duration::from_secs((attempt.min(10)) as u64);
                tokio::time::sleep(delay).await;
            }
        }
    }
}

// Tache dediee a signalk : connexion, souscription, reception des deltas,
// et reconnexion automatique en cas de coupure. Tourne independamment de
// la boucle SDL, ne bloque donc jamais le rendu / les events clavier-souris.
pub async fn signalk_task(
    state_tx: watch::Sender<BoatState>,
    track_tx: tokio::sync::mpsc::UnboundedSender<(f64, f64)>,
    ais_tx: watch::Sender<Vec<AisTarget>>,
    config: SignalKConfig,
) {
    crate::diagnostics::record(
        "INFO",
        format!("Tâche SignalK démarrée, serveur={}", config.url),
    );
    let mut signalk = SignalKClient::connect(config).await;
    if !signalk.check_connect().await {
        crate::diagnostics::record("WARN", "La vérification initiale SignalK n'a pas abouti");
    }

    loop {
        if let Err(error) = signalk.subscribe().await {
            crate::diagnostics::record("ERROR", format!("Échec de souscription SignalK: {error}"));
        }

        // boucle normale de reception des deltas signalk
        let mut last_eviction = Instant::now();
        while let Some(summary) = signalk.next_update().await {
            if summary.self_changed {
                let _ = state_tx.send(BoatState {
                    longitude: signalk.data.longitude,
                    latitude: signalk.data.latitude,
                    position_valid: signalk.data.position_valid,
                    course_over_ground: signalk.data.cog,
                    speed_over_ground: signalk.data.sog,
                    heading: None,
                    fixed_at: signalk.data.position_valid.then(Instant::now),
                    heading_at: None,
                });
            }
            if summary.self_position {
                let _ = track_tx.send((signalk.data.longitude, signalk.data.latitude));
            }

            // Targets that have gone quiet are dropped here, so the map does
            // not grow for the whole passage. Checked at most once a second.
            let mut ais_changed = summary.ais_changed;
            let now = Instant::now();
            if now.duration_since(last_eviction) >= Duration::from_secs(1) {
                ais_changed |= crate::ais::retain_fresh(&mut signalk.targets, now);
                last_eviction = now;
            }
            if ais_changed {
                let _ = ais_tx.send(signalk.targets.values().cloned().collect());
            }
        }

        crate::diagnostics::record("WARN", "Connexion SignalK interrompue, reconnexion");
        signalk.reconnect().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decodes_ais_target_and_ignores_own_vessel() {
        let context = "vessels.urn:mrn:imo:mmsi:123456789";
        let mut targets = HashMap::new();
        let paths = SignalKPaths {
            ais_position: "custom.position".into(),
            ais_name: "custom.name".into(),
            ais_course_over_ground: "custom.course".into(),
            ais_speed_over_ground: "custom.speed".into(),
            ..SignalKPaths::default()
        };

        update_ais_target(
            &mut targets,
            &paths,
            context,
            &paths.ais_position,
            &json!({"latitude": -17.7, "longitude": 178.1}),
        );
        update_ais_target(
            &mut targets,
            &paths,
            context,
            &paths.ais_name,
            &json!("Example"),
        );
        update_ais_target(
            &mut targets,
            &paths,
            context,
            &paths.ais_course_over_ground,
            &json!(1.25),
        );
        update_ais_target(
            &mut targets,
            &paths,
            context,
            &paths.ais_speed_over_ground,
            &json!(4.5),
        );

        let target = targets.get(context).unwrap();
        assert_eq!(target.position, Some((178.1, -17.7)));
        assert_eq!(target.name.as_deref(), Some("Example"));
        assert_eq!(target.course_over_ground, Some(1.25));
        assert_eq!(target.speed_over_ground, Some(4.5));

        update_ais_target(
            &mut targets,
            &paths,
            context,
            &paths.ais_position,
            &json!({"latitude": 91.0, "longitude": 181.0}),
        );
        update_ais_target(
            &mut targets,
            &paths,
            "vessels.self",
            &paths.ais_position,
            &json!({"latitude": 0.0, "longitude": 0.0}),
        );

        assert_eq!(targets.len(), 1);
        assert_eq!(targets.get(context).unwrap().position, Some((178.1, -17.7)));
    }

    #[test]
    fn delta_summary_tells_own_position_apart_from_ais_updates() {
        let paths = SignalKPaths::default();
        let mut data = SignalKData::default();
        let mut targets = HashMap::new();

        let own = json!({"context": "vessels.self", "updates": [{"values": [
            {"path": "navigation.position", "value": {"latitude": 36.95, "longitude": 26.98}}
        ]}]});
        let summary = apply_delta(&mut data, &mut targets, &paths, None, &own);
        assert!(summary.self_position);
        assert!(summary.self_changed);
        assert!(!summary.ais_changed);

        let course = json!({"context": "vessels.self", "updates": [{"values": [
            {"path": "navigation.courseOverGroundLastMinute", "value": 1.5}
        ]}]});
        let summary = apply_delta(&mut data, &mut targets, &paths, None, &course);
        assert_eq!(
            summary,
            DeltaSummary {
                self_changed: true,
                self_position: false,
                ais_changed: false
            }
        );
        assert_eq!(data.cog, 1.5);
        assert_eq!((data.longitude, data.latitude), (26.98, 36.95));
        assert!(targets.is_empty());

        let other = json!({"context": "vessels.urn:mrn:imo:mmsi:123456789", "updates": [{"values": [
            {"path": "navigation.position", "value": {"latitude": 37.0, "longitude": 27.0}}
        ]}]});
        let summary = apply_delta(&mut data, &mut targets, &paths, None, &other);
        assert!(!summary.self_position);
        assert!(summary.ais_changed);
        assert_eq!(targets.len(), 1);

        let unrelated = json!({"context": "vessels.self", "updates": [{"values": [
            {"path": "environment.depth.belowKeel", "value": 12.5}
        ]}]});
        let summary = apply_delta(&mut data, &mut targets, &paths, None, &unrelated);
        assert!(!summary.self_position);
        assert!(!summary.ais_changed);
    }

    #[test]
    fn recognizes_server_reported_self_context() {
        let self_context = Some("vessels.urn:mrn:imo:mmsi:123456789");
        assert!(is_self_context(
            "vessels.urn:mrn:imo:mmsi:123456789",
            self_context
        ));
        assert!(is_self_context("vessels.self", self_context));
        assert!(!is_self_context(
            "vessels.urn:mrn:imo:mmsi:987654321",
            self_context
        ));
    }
}
