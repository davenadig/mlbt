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
        format!("Sonos ads {state} | Delay {lag} | Mute +5s / Unmute -5s")
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
        assert_eq!(
            state.label(),
            "Sonos ads OFF | Delay 11.2s | Mute +5s / Unmute -5s"
        );
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
