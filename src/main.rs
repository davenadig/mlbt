mod app;
mod components;
mod config;
mod draw;
mod keys;
mod state;
mod ui;

use crate::app::App;
use crate::state::messages::{NetworkRequest, NetworkResponse, RefreshableRequest, UiEvent};
use crate::state::network::{LoadingState, NetworkWorker};
use crate::state::refresher::PeriodicRefresher;
use crossterm::event::{self as crossterm_event, Event, KeyEventKind};
use crossterm::{cursor, execute, terminal};
use log::error;
use std::io::Stdout;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::{io, panic};
use tokio::sync::{Mutex, mpsc};
use tui::{Terminal, backend::CrosstermBackend};

const PREFERRED_TERMINAL_WIDTH: u16 = 120;

/// How often held live snapshots are checked for release. Bounds how far past its
/// configured delay a snapshot can sit, so the shortest offered delay (5s) is still
/// accurate to within about 10%.
const DELAY_RELEASE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().any(|a| matches!(a.as_str(), "--version" | "-V")) {
        println!("mlbt {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    better_panic::install();
    ensure_preferred_terminal_width();

    let backend = CrosstermBackend::new(io::stdout());
    let terminal = Terminal::new(backend)?;

    setup_panic_hook();
    setup_terminal();

    // initialize logging
    tui_logger::init_logger(log::LevelFilter::Error)?;
    tui_logger::set_default_level(log::LevelFilter::Error);

    let app = Arc::new(Mutex::new(App::new()));

    let (ui_event_tx, ui_event_rx) = mpsc::channel::<UiEvent>(100);
    let (network_req_tx, network_req_rx) = mpsc::channel::<RefreshableRequest>(100);
    let (network_resp_tx, network_resp_rx) = mpsc::channel::<NetworkResponse>(100);

    // input handler thread
    let input_handler = tokio::spawn(input_handler_task(ui_event_tx.clone()));

    // network thread
    let network_worker = NetworkWorker::new(network_req_rx, network_resp_tx);
    let network_task = tokio::spawn(network_worker.run());

    // periodic update thread
    let periodic_updater = PeriodicRefresher::new(network_req_tx.clone());
    let periodic_task = tokio::spawn(periodic_updater.run(app.clone()));

    // send initial app started event
    let _ = ui_event_tx.send(UiEvent::AppStarted).await;

    // run the main UI loop
    main_ui_loop(terminal, app, ui_event_rx, network_req_tx, network_resp_rx).await;

    input_handler.abort();
    network_task.abort();
    periodic_task.abort();

    Ok(())
}

fn preferred_terminal_size(width: u16, height: u16) -> Option<(u16, u16)> {
    (width < PREFERRED_TERMINAL_WIDTH).then_some((PREFERRED_TERMINAL_WIDTH, height))
}

fn iterm_session_unique_id(session_id: &str) -> Option<&str> {
    let (_, unique_id) = session_id.rsplit_once(':')?;
    (!unique_id.is_empty()
        && unique_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-'))
    .then_some(unique_id)
}

fn resize_iterm_session(width: u16) -> bool {
    let Ok(session_id) = std::env::var("ITERM_SESSION_ID") else {
        return false;
    };
    let Some(unique_id) = iterm_session_unique_id(&session_id) else {
        return false;
    };
    let script = format!(
        "tell application \"iTerm2\" to repeat with w in windows\nrepeat with t in tabs of w\nrepeat with s in sessions of t\nif unique ID of s is \"{unique_id}\" then\nset columns of s to {width}\nreturn\nend if\nend repeat\nend repeat\nend repeat"
    );

    Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn ensure_preferred_terminal_width() {
    let Ok((width, height)) = terminal::size() else {
        return;
    };
    let Some((width, height)) = preferred_terminal_size(width, height) else {
        return;
    };

    if resize_iterm_session(width) {
        return;
    }
    let _ = execute!(io::stdout(), terminal::SetSize(width, height));
}

async fn main_ui_loop(
    mut terminal: Terminal<CrosstermBackend<Stdout>>,
    app: Arc<Mutex<App>>,
    mut ui_events: mpsc::Receiver<UiEvent>,
    network_requests: mpsc::Sender<RefreshableRequest>,
    mut network_responses: mpsc::Receiver<NetworkResponse>,
) {
    let mut loading = LoadingState::default();
    let mut delay_release = tokio::time::interval(DELAY_RELEASE_INTERVAL);
    // Last badge text drawn, so the release timer only repaints when the badge would
    // actually change. The badge ticks every second in both phases; the timer fires
    // twice as often, and a full Gameday repaint twice a second for an unchanged
    // screen is not worth it.
    let mut last_delay_status = None;

    loop {
        tokio::select! {
            Some(ui_event) = ui_events.recv() => {
                let should_redraw = handle_ui_event(ui_event, &app, &network_requests).await;
                if should_redraw && !loading.is_loading {
                    let mut app_guard = app.lock().await;
                    draw::draw(&mut terminal, &mut app_guard, loading);
                }
            }

            Some(response) = network_responses.recv() => {
                let should_redraw = handle_network_response(response, &app, &network_requests, &mut loading).await;
                if should_redraw {
                    let mut app_guard = app.lock().await;
                    draw::draw(&mut terminal, &mut app_guard, loading);
                }
            }

            // A snapshot held by the broadcast-delay buffer comes due on the clock,
            // not on an event, so nothing else would wake the loop to display it.
            _ = delay_release.tick() => {
                let mut app_guard = app.lock().await;
                let released = app_guard.release_due_live_data();
                // The badge counts down while the buffer is cold and reports the
                // measured lag once it is warm, so it needs a repaint whenever its
                // text moves — nothing else would wake the loop for it.
                let status = app_guard.tv_delay_status();
                let badge_changed = status != last_delay_status;
                last_delay_status = status;
                if (released || badge_changed) && !loading.is_loading {
                    draw::draw(&mut terminal, &mut app_guard, loading);
                }
            }
        }
    }
}

async fn handle_ui_event(
    ui_event: UiEvent,
    app: &Arc<Mutex<App>>,
    network_requests: &mpsc::Sender<RefreshableRequest>,
) -> bool {
    match ui_event {
        UiEvent::AppStarted => {
            let _ = network_requests
                .send(NetworkRequest::Initialize.into())
                .await;
            true // Redraw immediately to show loading state
        }
        UiEvent::KeyPressed(key_event) => {
            keys::handle_key_bindings(key_event, app, network_requests).await;
            true // Redraw after key handling
        }
        UiEvent::Resize => true, // Redraw on resize
    }
}

async fn handle_network_response(
    response: NetworkResponse,
    app: &Arc<Mutex<App>>,
    network_requests: &mpsc::Sender<RefreshableRequest>,
    loading: &mut LoadingState,
) -> bool {
    match response {
        NetworkResponse::LoadingStateChanged { loading_state } => {
            *loading = loading_state;
            // Always redraw for loading changes
            return true;
        }
        NetworkResponse::ScheduleLoaded { schedule } => {
            let game_id_to_load = {
                let mut guard = app.lock().await;
                guard.update_schedule(&schedule)
            };

            if let Some(game_id) = game_id_to_load {
                let _ = network_requests
                    .send(NetworkRequest::GameData { game_id }.into())
                    .await;
            }
        }
        NetworkResponse::GameDataLoaded {
            game,
            win_probability,
            fetched_at,
        } => {
            let mut guard = app.lock().await;
            // With a broadcast delay set this holds the snapshot instead of applying
            // it; `DELAY_RELEASE_INTERVAL` below is what puts it on screen later.
            let applied = guard.queue_live_data(game, win_probability, fetched_at);
            if !applied {
                return false;
            }
        }
        NetworkResponse::StandingsLoaded { standings } => {
            let mut guard = app.lock().await;
            guard.state.standings.update(&standings);
        }
        NetworkResponse::StatsLoaded { stats } => {
            let mut guard = app.lock().await;
            guard.state.stats.update(&stats);
        }
        NetworkResponse::PlayerProfileLoaded { data, game_type } => {
            let mut guard = app.lock().await;
            guard.update_player_profile(data, game_type);
        }
        NetworkResponse::TeamPageLoaded {
            team_id,
            date,
            schedule,
            roster,
            transactions,
        } => {
            let mut guard = app.lock().await;
            guard.update_team_page(team_id, date, &schedule, &roster, &transactions);
        }
        NetworkResponse::TeamRosterLoaded {
            team_id,
            roster,
            roster_type,
        } => {
            let mut guard = app.lock().await;
            guard.update_team_roster(team_id, &roster, roster_type);
        }
        NetworkResponse::Initialized => {
            // Teams must be loaded before the schedule so international team names resolve.
            let date = {
                let guard = app.lock().await;
                guard.state.schedule.date_selector.date
            };
            let _ = network_requests
                .send(NetworkRequest::Schedule { date }.into())
                .await;
        }
        NetworkResponse::Error { message } => {
            error!("Network error: {message}");
        }
    }
    // Only redraw if not loading
    !loading.is_loading
}

async fn input_handler_task(ui_events: mpsc::Sender<UiEvent>) {
    loop {
        if let Ok(event) = crossterm_event::read() {
            let ui_event = match event {
                Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                    Some(UiEvent::KeyPressed(key_event))
                }
                Event::Resize(_, _) => Some(UiEvent::Resize),
                Event::Mouse(_) => None, // Ignore mouse events for now
                _ => None,
            };

            if let Some(ui_event) = ui_event
                && ui_events.send(ui_event).await.is_err()
            {
                break; // Channel closed, exit task
            }
        }
    }
}

fn setup_terminal() {
    let mut stdout = io::stdout();

    execute!(stdout, cursor::Hide).unwrap();
    execute!(stdout, terminal::EnterAlternateScreen).unwrap();
    execute!(stdout, terminal::Clear(terminal::ClearType::All)).unwrap();

    terminal::enable_raw_mode().unwrap();
}

fn cleanup_terminal() {
    let mut stdout = io::stdout();

    execute!(stdout, cursor::MoveTo(0, 0)).unwrap();
    execute!(stdout, terminal::Clear(terminal::ClearType::All)).unwrap();
    execute!(stdout, terminal::LeaveAlternateScreen).unwrap();
    execute!(stdout, cursor::Show).unwrap();

    terminal::disable_raw_mode().unwrap();
}

fn setup_panic_hook() {
    panic::set_hook(Box::new(|panic_info| {
        cleanup_terminal();
        better_panic::Settings::auto().create_panic_handler()(panic_info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_narrow_terminals_to_preferred_width() {
        assert_eq!(preferred_terminal_size(99, 30), Some((120, 30)));
    }

    #[test]
    fn leaves_preferred_and_wider_terminals_alone() {
        assert_eq!(preferred_terminal_size(120, 30), None);
        assert_eq!(preferred_terminal_size(160, 40), None);
    }

    #[test]
    fn extracts_and_validates_iterm_session_id() {
        assert_eq!(
            iterm_session_unique_id("w0t0p0:75824422-569B-4BEF-B656-08141AD401DF"),
            Some("75824422-569B-4BEF-B656-08141AD401DF")
        );
        assert_eq!(iterm_session_unique_id("not-an-iterm-session"), None);
        assert_eq!(iterm_session_unique_id("w0:bad-id!'"), None);
    }
}
