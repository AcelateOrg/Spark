//! Discord Rich Presence.
//!
//! One worker thread per process owns the IPC connection, so presence survives script hot reload and a slow or
//! missing Discord never stalls a frame. Callers only post the *desired* state; the worker coalesces it,
//! respects Discord's rate limit (5 updates / 20 s) and reconnects on its own when Discord starts later or
//! restarts mid-game.
//!
//! ```no_run
//! spark_discord::connect("123456789012345678");
//! spark_discord::set(spark_discord::Presence { details: Some("In the menu".into()), ..Default::default() });
//! ```

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use discord_rich_presence::activity::{Activity, Assets, Button, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};
use serde_json::json;

/// Gap between two updates actually sent: Discord drops anything faster than 5 per 20 s.
const MIN_GAP: Duration = Duration::from_secs(4);
const RETRY: Duration = Duration::from_secs(15);
/// Discord gives no sign it went away until a write fails, so an idle connection re-sends what it shows now and
/// then: a Discord restart mid-game gets the presence back within this time instead of never.
const KEEPALIVE: Duration = Duration::from_secs(60);

/// Discord's own field limits; longer strings make it reject the whole activity.
const MAX_TEXT: usize = 128;
const MAX_BUTTONS: usize = 2;
const MAX_LABEL: usize = 32;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Presence {
    pub details: Option<String>,
    pub state: Option<String>,
    pub large_image: Option<String>,
    pub large_text: Option<String>,
    pub small_image: Option<String>,
    pub small_text: Option<String>,
    /// Unix seconds: Discord shows "elapsed" from `start` or a countdown to `end`.
    pub start: Option<i64>,
    pub end: Option<i64>,
    /// `(label, url)`, at most two; the url must be http(s).
    pub buttons: Vec<(String, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// `connect` was never called.
    Off,
    /// Waiting for Discord (not running, or the connection dropped).
    Connecting,
    Connected,
}

enum Cmd {
    Connect(String),
    Set(Option<Presence>),
    Shutdown,
}

static WORKER: OnceLock<Mutex<Option<Sender<Cmd>>>> = OnceLock::new();
static STATUS: Mutex<Status> = Mutex::new(Status::Off);

fn send(cmd: Cmd) {
    let cell = WORKER.get_or_init(|| Mutex::new(None));
    let Ok(mut slot) = cell.lock() else { return };
    if slot.is_none() {
        if matches!(cmd, Cmd::Shutdown) {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new().name("spark-discord".into()).spawn(move || Worker::new(rx).run());
        if let Err(e) = spawned {
            log::warn!("discord: cannot start the presence thread: {e}");
            return;
        }
        *slot = Some(tx);
    }
    let shutdown = matches!(cmd, Cmd::Shutdown);
    if let Some(tx) = slot.as_ref() {
        let _ = tx.send(cmd);
    }
    if shutdown {
        *slot = None;
    }
}

/// Start talking to Discord as application `app_id` (the Application ID from the Discord developer portal).
/// Calling it again with the same id does nothing; with another id it reconnects as that app.
pub fn connect(app_id: &str) {
    session_start();
    send(Cmd::Connect(app_id.trim().to_string()));
}

/// The presence to show. Repeated identical calls are free: only changes go out, at most one per 4 s.
pub fn set(p: Presence) {
    send(Cmd::Set(Some(p)));
}

/// Remove the presence (the game still shows as "Playing <app name>").
pub fn clear() {
    send(Cmd::Set(None));
}

/// Clear the presence and close the connection. `connect` starts it again.
pub fn shutdown() {
    send(Cmd::Shutdown);
}

pub fn status() -> Status {
    STATUS.lock().map(|s| *s).unwrap_or(Status::Off)
}

pub fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Unix seconds of the first call in this process: the "elapsed" clock that survives hot reload.
pub fn session_start() -> i64 {
    static START: OnceLock<i64> = OnceLock::new();
    *START.get_or_init(unix_now)
}

/// Clamp to `max` chars on a char boundary; Discord also rejects strings shorter than 2.
fn fit(s: &Option<String>, max: usize) -> Option<String> {
    let s = s.as_deref()?.trim();
    if s.chars().count() < 2 {
        return None;
    }
    Some(s.chars().take(max).collect())
}

fn activity(p: &Presence) -> Activity<'static> {
    let mut a = Activity::new();
    if let Some(s) = fit(&p.details, MAX_TEXT) {
        a = a.details(s);
    }
    if let Some(s) = fit(&p.state, MAX_TEXT) {
        a = a.state(s);
    }
    let mut assets = Assets::new();
    let mut any = false;
    for (field, max, set) in [
        (&p.large_image, 256, Assets::large_image::<String> as fn(Assets<'static>, String) -> Assets<'static>),
        (&p.large_text, MAX_TEXT, Assets::large_text::<String>),
        (&p.small_image, 256, Assets::small_image::<String>),
        (&p.small_text, MAX_TEXT, Assets::small_text::<String>),
    ] {
        if let Some(s) = fit(field, max) {
            assets = set(assets, s);
            any = true;
        }
    }
    if any {
        a = a.assets(assets);
    }
    if p.start.is_some() || p.end.is_some() {
        let mut t = Timestamps::new();
        if let Some(s) = p.start {
            t = t.start(s);
        }
        if let Some(e) = p.end {
            t = t.end(e);
        }
        a = a.timestamps(t);
    }
    let buttons = buttons(p);
    if !buttons.is_empty() {
        a = a.buttons(buttons.into_iter().map(|(l, u)| Button::new(l, u)).collect());
    }
    a
}

fn buttons(p: &Presence) -> Vec<(String, String)> {
    p.buttons
        .iter()
        .filter(|(_, url)| url.starts_with("https://") || url.starts_with("http://"))
        .filter_map(|(label, url)| Some((fit(&Some(label.clone()), MAX_LABEL)?, url.clone())))
        .take(MAX_BUTTONS)
        .collect()
}

struct Worker {
    rx: Receiver<Cmd>,
    app_id: Option<String>,
    client: Option<DiscordIpcClient>,
    /// What the game wants shown (`None` = nothing).
    want: Option<Presence>,
    /// What Discord has, as far as we know; `Some(None)` = cleared, `None` = unknown (fresh connection).
    shown: Option<Option<Presence>>,
    last_sent: Option<Instant>,
    retry_at: Instant,
}

impl Worker {
    fn new(rx: Receiver<Cmd>) -> Self {
        Self { rx, app_id: None, client: None, want: None, shown: None, last_sent: None, retry_at: Instant::now() }
    }

    fn run(mut self) {
        loop {
            let cmd = match self.rx.recv_timeout(self.wait()) {
                Ok(cmd) => Some(cmd),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => Some(Cmd::Shutdown),
            };
            if let Some(cmd) = cmd {
                if !self.apply(cmd) {
                    return;
                }
                // a burst of `set` calls (one per frame) collapses into the last one
                while let Ok(cmd) = self.rx.try_recv() {
                    if !self.apply(cmd) {
                        return;
                    }
                }
            }
            self.tick();
        }
    }

    /// `false` = the worker is done.
    fn apply(&mut self, cmd: Cmd) -> bool {
        match cmd {
            Cmd::Connect(id) => {
                if self.app_id.as_deref() != Some(id.as_str()) {
                    self.drop_client();
                    self.app_id = (!id.is_empty()).then_some(id);
                    self.retry_at = Instant::now();
                    set_status(if self.app_id.is_some() { Status::Connecting } else { Status::Off });
                }
            }
            Cmd::Set(p) => self.want = p,
            Cmd::Shutdown => {
                if let Some(c) = self.client.as_mut() {
                    let _ = c.clear_activity();
                }
                self.drop_client();
                set_status(Status::Off);
                return false;
            }
        }
        true
    }

    fn wait(&self) -> Duration {
        let now = Instant::now();
        if self.app_id.is_none() {
            return Duration::from_secs(3600);
        }
        if self.client.is_none() {
            return self.retry_at.saturating_duration_since(now).max(Duration::from_millis(50));
        }
        let ready = match self.last_sent {
            None => now,
            Some(t) if self.shown.as_ref() != Some(&self.want) => t + MIN_GAP,
            Some(t) => t + KEEPALIVE,
        };
        ready.saturating_duration_since(now).max(Duration::from_millis(50))
    }

    fn tick(&mut self) {
        let Some(id) = self.app_id.clone() else { return };
        let now = Instant::now();
        if self.client.is_none() {
            if now < self.retry_at {
                return;
            }
            match handshake(&id) {
                Ok(c) => {
                    log::info!("discord: connected");
                    self.client = Some(c);
                    // a fresh connection shows nothing yet: no point spending a rate-limit slot on a clear
                    self.shown = Some(None);
                    set_status(Status::Connected);
                }
                Err(e) => {
                    log::debug!("discord: not available ({e}), retrying in {}s", RETRY.as_secs());
                    self.retry_at = now + RETRY;
                    set_status(Status::Connecting);
                    return;
                }
            }
        }
        let gap = if self.shown.as_ref() == Some(&self.want) { KEEPALIVE } else { MIN_GAP };
        if self.last_sent.is_some_and(|t| now < t + gap) {
            return;
        }
        let Some(c) = self.client.as_mut() else { return };
        let result = match &self.want {
            Some(p) => c.set_activity(activity(p)),
            None => c.clear_activity(),
        }
        .and_then(|()| c.recv());
        self.last_sent = Some(now);
        match result {
            Ok((_, reply)) => {
                // a refused activity (bad image key, malformed button) is the game's bug: say it, don't resend
                if reply["evt"] == "ERROR" {
                    log::warn!("discord: presence rejected: {}", reply["data"]["message"].as_str().unwrap_or("?"));
                }
                self.shown = Some(self.want.clone());
            }
            Err(e) => {
                log::info!("discord: connection lost ({e})");
                self.drop_client();
                // Discord restarting has usually brought its pipe back already: try once right away
                self.retry_at = now;
                set_status(Status::Connecting);
            }
        }
    }

    fn drop_client(&mut self) {
        if let Some(mut c) = self.client.take() {
            let _ = c.close();
        }
        self.shown = None;
    }
}

/// `DiscordIpc::connect` reads the handshake reply but ignores it, so a wrong application id would look like a
/// working connection that silently shows nothing. Discord answers a good handshake with READY (op 1) and a bad one
/// with CLOSE (op 2) plus the reason.
fn handshake(app_id: &str) -> Result<DiscordIpcClient, String> {
    let mut c = DiscordIpcClient::new(app_id);
    c.connect_ipc().map_err(|e| e.to_string())?;
    let reply = c.send(json!({ "v": 1, "client_id": app_id }), 0).and_then(|()| c.recv());
    match reply {
        Ok((1, r)) if r["evt"] == "READY" => Ok(c),
        Ok((_, r)) => {
            let _ = c.close();
            let why = r["message"].as_str().unwrap_or("handshake refused").to_string();
            log::warn!("discord: application {app_id} refused: {why}");
            Err(why)
        }
        Err(e) => Err(e.to_string()),
    }
}

fn set_status(s: Status) {
    if let Ok(mut g) = STATUS.lock() {
        *g = s;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_trims_and_rejects_short() {
        assert_eq!(fit(&Some(" x ".into()), 10), None);
        assert_eq!(fit(&Some("привет мир".into()), 6).as_deref(), Some("привет"));
    }

    #[test]
    fn buttons_need_http_and_cap_at_two() {
        let p = Presence {
            buttons: vec![
                ("Steam".into(), "https://a".into()),
                ("bad".into(), "steam://x".into()),
                ("Site".into(), "https://b".into()),
                ("Third".into(), "https://c".into()),
            ],
            ..Default::default()
        };
        let b = buttons(&p);
        assert_eq!(b.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(), ["Steam", "Site"]);
        let _ = activity(&p);
    }
}
