//! Sonos ad muting belongs to one Gameday view, never a detached service.
//! Keep the target explicit so future radio controls cannot silently mute Sonos.
use serde::Deserialize;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Default, Deserialize)]
pub struct AdStatus {
    #[serde(default)]
    pub state: String,
    pub game_pk: u64,
    pub lag: Option<f64>,
    pub muted: bool,
    #[serde(default)]
    pub mute_margin: Option<f64>,
    #[serde(default)]
    pub unmute_margin: Option<f64>,
    #[serde(default)]
    pub next_action: Option<String>,
    #[serde(default)]
    pub next_action_at: Option<f64>,
    #[serde(default)]
    pub message: String,
}

struct Session {
    child: Child,
    input: Option<ChildStdin>,
    updates: Receiver<AdStatus>,
    game_id: u64,
    last_update: Instant,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl Session {
    fn send(&mut self, command: &str) -> std::io::Result<()> {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| std::io::Error::other("helper disconnected"))?;
        writeln!(
            input,
            "{}",
            serde_json::json!({"command": command, "ts": ts})
        )?;
        input.flush()
    }

    fn stop(&mut self) -> bool {
        let _ = self.send("quit");
        self.input.take(); // EOF is a second, independent shutdown signal.
        let deadline = Instant::now() + Duration::from_secs(12);
        let clean = loop {
            match self.child.try_wait() {
                Ok(Some(status)) => break status.success(),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                Ok(None) | Err(_) => {
                    let _ = self.child.kill();
                    let _ = self.child.wait(); // reap; no stale helper survives this view
                    break false;
                }
            }
        };
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        clean
    }
}

#[derive(Default)]
pub struct AdMuteState {
    session: Option<Session>,
    pub status: AdStatus,
}

pub fn helper_command(home: &Path, game_id: u64) -> Command {
    let mut command = Command::new(home.join("GitHub/cogspace/tools/bin/cog-audiomute"));
    command.args(["tv", "--managed", "--game-pk", &game_id.to_string()]);
    command.stdin(Stdio::piped()).stdout(Stdio::piped());
    command
}

impl AdMuteState {
    pub fn active(&self) -> bool {
        self.session.is_some()
    }

    pub fn start(&mut self, game_id: u64) {
        if self.active() {
            self.stop();
            return;
        }
        let Some(home) = std::env::var_os("HOME") else {
            self.status.message = "Cannot find Cogspace: HOME is unset".into();
            return;
        };
        let home = Path::new(&home);
        let log_dir = home.join(".cache/cogspace/baseball/logs");
        let result = std::fs::create_dir_all(&log_dir).and_then(|()| {
            let log = std::fs::File::create(log_dir.join("mlbt-audiomute.log"))?;
            let mut command = helper_command(home, game_id);
            command.stderr(Stdio::from(log));
            self.launch(command, game_id)
        });
        if let Err(err) = result {
            self.status.message = format!("Cannot start Sonos ads: {err}");
        }
    }

    fn launch(&mut self, mut command: Command, game_id: u64) -> std::io::Result<()> {
        let mut child = command.spawn()?;
        let input = child.stdin.take();
        let output = child.stdout.take().expect("helper stdout is piped");
        let (tx, rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(output).lines().map_while(Result::ok) {
                if let Ok(status) = serde_json::from_str::<AdStatus>(&line)
                    && tx.send(status).is_err()
                {
                    break;
                }
            }
        });
        self.status = AdStatus {
            state: "starting".into(),
            game_pk: game_id,
            ..AdStatus::default()
        };
        self.session = Some(Session {
            child,
            input,
            updates: rx,
            game_id,
            last_update: Instant::now(),
            reader: Some(reader),
        });
        Ok(())
    }

    pub fn command(&mut self, command: &str) {
        if let Some(session) = &mut self.session {
            if session.send(command).is_err() {
                self.stop();
                self.status.message = "Sonos helper disconnected; stopped".into();
            }
        } else {
            self.status.message = "Press A to enable Sonos ads first".into();
        }
    }

    /// Called from the UI timer, independent of MLB network responses.
    pub fn tick(&mut self, visible_game: Option<u64>) {
        let Some(session) = &mut self.session else {
            return;
        };
        if visible_game != Some(session.game_id) {
            self.stop();
            return;
        }
        while let Ok(status) = session.updates.try_recv() {
            if status.game_pk == session.game_id {
                session.last_update = Instant::now();
                self.status = status;
            }
        }
        if let Ok(Some(exit)) = session.child.try_wait() {
            let mut ended = self.session.take().unwrap();
            if let Some(reader) = ended.reader.take() {
                let _ = reader.join();
            }
            while let Ok(status) = ended.updates.try_recv() {
                self.status = status;
            }
            if !exit.success() {
                self.status.message =
                    "Sonos helper failed; check mlbt-audiomute.log (quit any standalone muter)"
                        .into();
            }
            self.status.state = "stopped".into();
            return;
        }
        if session.last_update.elapsed() > Duration::from_secs(10) {
            self.stop();
            self.status.message = "Sonos helper stopped responding; stopped".into();
            return;
        }
        if session.send("heartbeat").is_err() {
            self.stop();
            self.status.message = "Sonos helper disconnected; stopped".into();
        }
    }

    pub fn stop(&mut self) {
        if let Some(mut session) = self.session.take() {
            let clean = session.stop();
            while let Ok(status) = session.updates.try_recv() {
                self.status = status;
            }
            if !clean || self.status.muted {
                self.status.message =
                    "Helper stopped; Sonos unmute unconfirmed—check soundbar".into();
            } else {
                self.status.message.clear();
            }
            self.status.state = "stopped".into();
        }
    }

    pub fn label(&self) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        self.label_at(now)
    }

    fn label_at(&self, now: f64) -> String {
        let state = if !self.active() {
            "OFF"
        } else if self.status.muted {
            "MUTED"
        } else if self.status.state == "starting" {
            "STARTING"
        } else {
            "ON"
        };
        let lag = self
            .status
            .lag
            .map(|s| format!("{s:.1}s"))
            .unwrap_or_else(|| "unset".into());
        let margin =
            |value: Option<f64>| value.map(|s| format!("{s}s")).unwrap_or_else(|| "?".into());
        let countdown = if self.active() && self.status.state == "running" {
            match (
                self.status.next_action.as_deref(),
                self.status.next_action_at,
            ) {
                (Some(action @ ("mute" | "unmute")), Some(at)) if at.is_finite() => {
                    let verb = if action == "mute" {
                        "Muting"
                    } else {
                        "Unmuting"
                    };
                    if at <= now {
                        format!(" | {verb} now")
                    } else {
                        format!(" | {verb} in {:.0}s", (at - now).ceil())
                    }
                }
                _ => String::new(),
            }
        } else {
            String::new()
        };
        format!(
            "Sonos ads {state}{countdown} | Delay {lag} | Mute +{} / Unmute -{}",
            margin(self.status.mute_margin),
            margin(self.status.unmute_margin)
        )
    }
}

impl Drop for AdMuteState {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_targets_selected_game_and_reuses_saved_sync() {
        let cmd = helper_command(Path::new("/Users/test"), 824787);
        assert_eq!(
            cmd.get_args().collect::<Vec<_>>(),
            ["tv", "--managed", "--game-pk", "824787"]
        );
    }

    #[test]
    fn status_makes_target_delay_and_padding_explicit() {
        let mut state = AdMuteState::default();
        state.status.lag = Some(11.2);
        state.status.mute_margin = Some(10.0);
        state.status.unmute_margin = Some(10.0);
        assert_eq!(
            state.label(),
            "Sonos ads OFF | Delay 11.2s | Mute +10s / Unmute -10s"
        );
    }

    #[test]
    fn padding_label_uses_helper_settings() {
        let mut state = AdMuteState::default();
        state.status = serde_json::from_str(
            r#"{"game_pk":824787,"lag":6,"muted":false,"mute_margin":12,"unmute_margin":8}"#,
        )
        .unwrap();
        assert!(state.label().ends_with("Mute +12s / Unmute -8s"));
    }

    fn fake_helper() -> Command {
        let mut cmd = Command::new("python3");
        cmd.args(["-u", "-c", "import sys,json\nfor line in sys.stdin:\n if json.loads(line)['command']=='quit': break"]);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        cmd
    }

    #[test]
    fn countdown_ticks_and_clears_with_schedule_and_owner() {
        let mut state = AdMuteState::default();
        state.launch(fake_helper(), 111).unwrap();
        state.status = serde_json::from_str(
            r#"{"state":"running","game_pk":111,"lag":6,"muted":false,"mute_margin":10,"unmute_margin":10,"next_action":"mute","next_action_at":1010}"#,
        ).unwrap();
        assert!(state.label_at(1000.0).contains("Muting in 10s"));
        assert!(state.label_at(1001.2).contains("Muting in 9s"));
        assert!(state.label_at(1011.0).contains("Muting now"));
        state.status.muted = true;
        state.status.next_action = Some("unmute".into());
        state.status.next_action_at = Some(1100.0);
        assert!(state.label_at(1089.0).contains("Unmuting in 11s"));
        state.status.next_action = None;
        state.status.next_action_at = None;
        assert!(!state.label_at(1090.0).contains("Unmuting"));
        state.status.next_action = Some("unmute".into());
        state.status.next_action_at = Some(1100.0);
        state.stop();
        assert!(!state.label_at(1090.0).contains("Unmuting"));
    }

    #[test]
    fn countdown_is_visible_in_top_row_on_narrow_terminals() {
        use tui::{Terminal, backend::TestBackend, widgets::Paragraph};
        let mut state = AdMuteState::default();
        state.launch(fake_helper(), 111).unwrap();
        state.status.state = "running".into();
        state.status.lag = Some(6.4);
        state.status.mute_margin = Some(10.0);
        state.status.unmute_margin = Some(10.0);
        state.status.next_action_at = Some(1010.0);
        for (action, expected) in [("mute", "Muting in 10s"), ("unmute", "Unmuting in 10s")] {
            state.status.next_action = Some(action.into());
            state.status.muted = action == "unmute";
            for width in [80, 120] {
                let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
                terminal
                    .draw(|f| f.render_widget(Paragraph::new(state.label_at(1000.0)), f.area()))
                    .unwrap();
                let buffer = terminal.backend().buffer();
                let row: String = (0..width).map(|x| buffer[(x, 0)].symbol()).collect();
                assert!(row.contains(expected), "{row}");
                assert!(row.contains("Delay 6.4s"), "{row}");
            }
        }
    }

    #[test]
    fn leaving_view_or_switching_game_reaps_helper() {
        for next in [None, Some(222)] {
            let mut state = AdMuteState::default();
            state.launch(fake_helper(), 111).unwrap();
            state.tick(Some(111));
            assert!(state.active());
            state.tick(next);
            assert!(!state.active());
            assert_eq!(state.status.state, "stopped");
            assert!(state.status.message.is_empty());
        }
    }

    #[test]
    fn every_tab_exit_and_game_reset_stop_the_owned_helper() {
        use crate::app::{App, MenuItem};
        for tab in [
            MenuItem::Scoreboard,
            MenuItem::Stats,
            MenuItem::Standings,
            MenuItem::Help,
            MenuItem::DatePicker,
        ] {
            let mut app = App::new();
            app.state.active_tab = MenuItem::Gameday;
            app.state.gameday.ads.launch(fake_helper(), 111).unwrap();
            app.update_tab(tab);
            assert!(!app.state.gameday.ads.active());
            assert!(app.state.gameday.ads.status.message.is_empty());
        }
        let mut app = App::new();
        app.state.gameday.game.game_id = 111;
        app.state.gameday.ads.launch(fake_helper(), 111).unwrap();
        app.state.gameday.reset(Some(222));
        assert!(!app.state.gameday.ads.active());
    }

    #[test]
    fn dropping_owner_closes_and_reaps_child() {
        let marker = std::env::temp_dir().join(format!("mlbt-ads-drop-{}", std::process::id()));
        // Record that the child actually reached cleanup.
        let mut cmd = Command::new("python3");
        cmd.args(["-u", "-c", "import sys,json,pathlib\nfor line in sys.stdin:\n if json.loads(line)['command']=='quit': break\npathlib.Path(sys.argv[1]).write_text('stopped')", marker.to_str().unwrap()]);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        {
            let mut state = AdMuteState::default();
            state.launch(cmd, 111).unwrap();
        }
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "stopped");
        let _ = std::fs::remove_file(marker);
    }
}
