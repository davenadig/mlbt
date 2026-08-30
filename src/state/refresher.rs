use crate::app::{App, MenuItem};
use crate::state::messages::{NetworkRequest, RefreshableRequest};
use chrono::NaiveDate;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::interval;

pub struct PeriodicRefresher {
    network_requests: mpsc::Sender<RefreshableRequest>,
    /// Day seen on the last rollover check. `None` until the first check.
    prev_today: Option<NaiveDate>,
}

impl PeriodicRefresher {
    pub fn new(network_requests: mpsc::Sender<RefreshableRequest>) -> Self {
        Self {
            network_requests,
            prev_today: None,
        }
    }

    pub async fn run(mut self, app: std::sync::Arc<tokio::sync::Mutex<App>>) {
        // Live data drives the broadcast-delay buffer, so the poll interval is also the
        // delay's resolution: at 10s a "45s" delay really meant 45-55s and every release
        // jumped the display forward by up to ten seconds of game at once. Read once —
        // changing it takes a restart.
        let live_poll = {
            let app = app.lock().await;
            app.settings.live_poll_interval()
        };
        let mut live_interval = interval(live_poll);
        let mut schedule_interval = interval(Duration::from_secs(30)); // Schedule every 30s
        let mut standings_interval = interval(Duration::from_secs(1800)); // Standings every 30 minutes
        let mut stats_interval = interval(Duration::from_secs(1800)); // Stats every 30 minutes
        let mut rollover_interval = interval(Duration::from_secs(60)); // Check for a new day every minute

        loop {
            tokio::select! {
                // Live game data updates (frequent for active games)
                _ = live_interval.tick() => {
                    let (active_tab, target, delay_on) = {
                        let app = app.lock().await;
                        (app.state.active_tab, app.live_poll_target(), app.settings.tv_delay_on())
                    };

                    // With a delay set, poll from either live screen: the buffer holds
                    // per selected game, so keeping it warm on the Scoreboard is what
                    // stops a switch to Gameday from staring at a countdown.
                    let wants_live = active_tab == MenuItem::Gameday
                        || (delay_on && active_tab == MenuItem::Scoreboard);
                    if let Some(game_id) = target
                        && wants_live
                    {
                        let _ = self.network_requests.send(RefreshableRequest::force(NetworkRequest::GameData { game_id })).await;
                    }
                }

                // Schedule updates (moderate frequency)
                _ = schedule_interval.tick() => {
                    let (active_tab, date, game_id, delay_on) = {
                        let app = app.lock().await;
                        (app.state.active_tab, app.state.schedule.date_selector.date, app.state.schedule.get_selected_game_opt().unwrap_or(0), app.settings.tv_delay_on())
                    };

                    if active_tab == MenuItem::Scoreboard {
                        let _ = self.network_requests.send(RefreshableRequest::force(NetworkRequest::Schedule { date })).await;
                        // The live tick already covers game data when a delay is on;
                        // asking again here would just be a duplicate fetch.
                        if game_id > 0 && !delay_on {
                            let _ = self.network_requests.send(RefreshableRequest::force(NetworkRequest::GameData { game_id })).await;
                        }
                    }
                }

                // Standings updates (low frequency)
                _ = standings_interval.tick() => {
                    let (active_tab, date) = {
                        let app = app.lock().await;
                        (app.state.active_tab, app.state.standings.date_selector.date)
                    };

                    if active_tab == MenuItem::Standings {
                        let _ = self.network_requests.send(RefreshableRequest::force(NetworkRequest::Standings { date })).await;
                    }
                }

                // Stats updates (low frequency)
                _ = stats_interval.tick() => {
                    let (active_tab, date, stat_type) = {
                        let app = app.lock().await;
                        (app.state.active_tab, app.state.stats.date_selector.date, app.state.stats.stat_type)
                    };

                    if active_tab == MenuItem::Stats {
                        let _ = self.network_requests.send(RefreshableRequest::force(NetworkRequest::Stats { date, stat_type })).await;
                    }
                }

                // Roll date-driven tabs over to the new day when the system date changes
                _ = rollover_interval.tick() => {
                    let (today, requests) = {
                        let mut app = app.lock().await;
                        app.advance_dates_on_rollover(self.prev_today)
                    };
                    self.prev_today = Some(today);
                    for request in requests {
                        let _ = self.network_requests.send(RefreshableRequest::force(request)).await;
                    }
                }
            }
        }
    }
}
