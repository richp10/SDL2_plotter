//! The channels every live-data source publishes on. A source is one task
//! that gets a `LiveFeed`; the main loop holds the `LiveReceivers` and
//! drains them once per iteration. A future source (NMEA 2000) plugs in
//! here, with no change to the loop.

use tokio::sync::{mpsc, watch};

use crate::ais::AisTarget;
use crate::boat::BoatState;

/// What a source sends: the boat state (latest value only), each new track
/// position, and the AIS list (latest value only).
pub struct LiveFeed {
    pub boat: watch::Sender<BoatState>,
    pub track: mpsc::UnboundedSender<(f64, f64)>,
    pub ais: watch::Sender<Vec<AisTarget>>,
}

/// The main loop's end of a `LiveFeed`.
pub struct LiveReceivers {
    pub boat: watch::Receiver<BoatState>,
    pub track: mpsc::UnboundedReceiver<(f64, f64)>,
    pub ais: watch::Receiver<Vec<AisTarget>>,
}

pub fn channels() -> (LiveFeed, LiveReceivers) {
    let (boat_tx, boat_rx) = watch::channel(BoatState::default());
    let (track_tx, track_rx) = mpsc::unbounded_channel();
    let (ais_tx, ais_rx) = watch::channel(Vec::new());
    (
        LiveFeed {
            boat: boat_tx,
            track: track_tx,
            ais: ais_tx,
        },
        LiveReceivers {
            boat: boat_rx,
            track: track_rx,
            ais: ais_rx,
        },
    )
}

/// The new value when the source changed it, else `None`. When the source
/// has gone (its sender dropped), logs one ERROR naming `what` and returns
/// `None` from then on: the app carries on with no live data.
pub fn take_change<T: Clone>(
    rx: &mut watch::Receiver<T>,
    closed_reported: &mut bool,
    what: &str,
) -> Option<T> {
    match rx.has_changed() {
        Ok(true) => Some(rx.borrow_and_update().clone()),
        Ok(false) => None,
        Err(_) => {
            if !*closed_reported {
                *closed_reported = true;
                crate::diagnostics::record(
                    "ERROR",
                    format!(
                        "La source de données ({what}) s'est arrêtée; plus de données en direct"
                    ),
                );
            }
            None
        }
    }
}

/// For a source that cannot run: logs `reason` as an ERROR, then keeps the
/// channels open with no data, so the app runs on without a boat.
pub async fn idle(feed: LiveFeed, reason: &str) {
    crate::diagnostics::record("ERROR", reason);
    std::future::pending::<()>().await;
    drop(feed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boat::BoatState;

    #[test]
    fn a_change_is_taken_once() {
        let (feed, mut rx) = channels();
        let mut reported = false;
        assert!(take_change(&mut rx.boat, &mut reported, "bateau").is_none());
        feed.boat.send(BoatState::default()).unwrap();
        assert!(take_change(&mut rx.boat, &mut reported, "bateau").is_some());
        assert!(take_change(&mut rx.boat, &mut reported, "bateau").is_none());
        assert!(!reported);
    }

    #[test]
    fn a_closed_channel_is_reported_once() {
        let (feed, mut rx) = channels();
        drop(feed);
        let mut reported = false;
        assert!(take_change(&mut rx.boat, &mut reported, "bateau").is_none());
        assert!(reported);
        assert!(take_change(&mut rx.boat, &mut reported, "bateau").is_none());
        assert!(reported);
    }
}
