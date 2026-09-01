use crate::components::game::live_game::GameState;
use crate::components::game::reveal::Cursor;
use mlbt_api::live::LiveResponse;
use mlbt_api::schedule::AbstractGameState;
use mlbt_api::win_probability::WinProbabilityResponse;
use tui::widgets::ScrollbarState;

#[derive(Default)]
pub struct GamedayState {
    pub panels: GamedayPanels,
    pub game: GameState,
    pub scoring_plays_only: bool,
    /// How far into the game the viewer has walked, or `None` when live.
    cursor: Option<Cursor>,
    /// Land on the first at bat when the next update arrives. Set when opening Gameday or
    /// switching games, since the new game's at bats load asynchronously.
    open_pending: bool,
    pub plays_scroll_offset: u16,
    pub plays_scroll_state: ScrollbarState,
}

impl GamedayState {
    pub fn selected_at_bat(&self) -> Option<u8> {
        self.cursor.map(|c| c.at_bat)
    }

    pub fn cursor(&self) -> Option<Cursor> {
        self.cursor
    }

    /// How many events the given at bat has to walk through. Events rather than pitches:
    /// pickoffs and substitutions share the vec and are worth stopping on.
    fn event_count(&self, at_bat: u8) -> usize {
        self.game
            .get_at_bat_by_index(at_bat)
            .map(|at_bat| at_bat.pitches.pitches.pitch_events.len())
            .unwrap_or(0)
    }

    /// The last event index of an at bat, or 0 when it has none yet.
    fn last_event(&self, at_bat: u8) -> usize {
        self.event_count(at_bat).saturating_sub(1)
    }

    fn set(&mut self, at_bat: u8, pitch: usize) {
        self.cursor = Some(Cursor { at_bat, pitch });
    }

    pub fn current_game_id(&self) -> u64 {
        self.game.game_id
    }

    pub fn is_final(&self) -> bool {
        matches!(
            self.game.abstract_game_state,
            Some(AbstractGameState::Final)
        )
    }

    pub fn reset(&mut self, game_id: Option<u64>) {
        let new_id = game_id.unwrap_or(0);

        if self.game.game_id != new_id {
            self.on_game_changing();
            self.game.reset();
            self.game.game_id = new_id;
        }
    }

    /// Update with latest data from the API, landing on the first at bat if opening the tab or a
    /// game switch left that pending. Detects a game switch here too since `GameData` responses
    /// from directly navigating games (rather than a schedule refresh) arrive without a prior call
    /// to `reset`.
    pub fn update(&mut self, live_data: &LiveResponse, win_probability: &WinProbabilityResponse) {
        if self.game.game_id != live_data.game_pk {
            self.on_game_changing();
        }
        self.game.update(live_data, win_probability);
        self.open_if_pending();
        self.clamp_cursor();
    }

    /// Keep the cursor inside the at bat it is on. A snapshot can be shorter than the one
    /// before it, and an at bat in progress grows pitch by pitch — neither should be able
    /// to leave the cursor pointing past the end of the event list.
    fn clamp_cursor(&mut self) {
        if let Some(cursor) = self.cursor {
            let last = self.last_event(cursor.at_bat);
            if cursor.pitch > last {
                self.set(cursor.at_bat, last);
            }
        }
    }

    /// Clear the selection and mark the jump to the top of the game as pending, to run once the
    /// new game's at bats load.
    fn on_game_changing(&mut self) {
        self.cursor = None;
        self.open_pending = true;
    }

    /// Open Gameday at the top of the game rather than at the live at bat, so walking forward
    /// through the arrow keys never runs ahead of a broadcast. Called every time the tab is
    /// entered; at bats load asynchronously, so this arms itself until there is one to land on.
    pub fn open_at_start(&mut self) {
        self.cursor = None;
        self.open_pending = true;
        self.open_if_pending();
    }

    fn open_if_pending(&mut self) {
        if !std::mem::take(&mut self.open_pending) {
            return;
        }
        self.start();
        // Nothing to select yet — the at bats arrive with a later response. Stay armed rather
        // than silently falling back to live, which is exactly the spoiler this avoids.
        self.open_pending = self.cursor.is_none();
    }

    /// The at bat indexes that can be navigated to, in game order. When `scoring_plays_only` is
    /// enabled this is restricted to scoring plays, so scrolling skips everything else.
    fn navigable_at_bats(&self) -> Vec<u8> {
        self.game
            .at_bats
            .values()
            .filter(|at_bat| !self.scoring_plays_only || at_bat.play_result.is_scoring_play)
            .map(|at_bat| at_bat.index)
            .collect()
    }

    /// The navigable at bat indexes and the position of the current selection within them. Returns
    /// `None` when there is nothing to navigate. The inner `None` means nothing is selected yet.
    fn navigable_selection(&self) -> Option<(Vec<u8>, Option<usize>)> {
        let indexes = self.navigable_at_bats();
        if indexes.is_empty() {
            return None;
        }
        let pos = self
            .cursor
            .and_then(|cursor| indexes.iter().position(|&i| i == cursor.at_bat));
        Some((indexes, pos))
    }

    /// Step one pitch forward, stopping on the newest pitch of the newest at bat.
    ///
    /// A pitch rather than a whole at bat because an at bat is itself a spoiler: landing on
    /// one shows where the runners ended up and what the count got to, which gives away the
    /// outcome before the broadcast has thrown the first pitch of it.
    ///
    /// Deliberately does not wrap. Walking forward to catch up with a broadcast should stall
    /// at the live edge, not loop back to the first inning and lose a place that took a
    /// hundred presses to reach. `None` is already the live edge, so it stays.
    pub fn next_pitch(&mut self) {
        let Some((indexes, pos)) = self.navigable_selection() else {
            return;
        };
        let (Some(p), Some(cursor)) = (pos, self.cursor) else {
            return;
        };
        if cursor.pitch + 1 < self.event_count(cursor.at_bat) {
            self.set(cursor.at_bat, cursor.pitch + 1);
        } else if p + 1 < indexes.len() {
            self.set(indexes[p + 1], 0);
        }
    }

    /// Step one pitch back, stopping on the first pitch of the game. From live this enters
    /// the game on its newest pitch.
    pub fn previous_pitch(&mut self) {
        let Some((indexes, pos)) = self.navigable_selection() else {
            return;
        };
        match (pos, self.cursor) {
            (Some(p), Some(cursor)) => {
                if cursor.pitch > 0 {
                    self.set(cursor.at_bat, cursor.pitch - 1);
                } else if p > 0 {
                    let previous = indexes[p - 1];
                    self.set(previous, self.last_event(previous));
                }
            }
            _ => {
                let last = indexes[indexes.len() - 1];
                self.set(last, self.last_event(last));
            }
        }
    }

    /// Jump to the end of the at bat, or to the end of the next one when already there.
    ///
    /// The coarse gear for catching up: an at bat is roughly four pitches, so this covers
    /// ground without ever landing somewhere that reveals more than the at bat you asked
    /// for. Like `next_pitch` it stops at the newest at bat rather than wrapping.
    pub fn next_at_bat(&mut self) {
        let Some((indexes, pos)) = self.navigable_selection() else {
            return;
        };
        let (Some(p), Some(cursor)) = (pos, self.cursor) else {
            return;
        };
        let last = self.last_event(cursor.at_bat);
        if cursor.pitch < last {
            self.set(cursor.at_bat, last);
        } else if p + 1 < indexes.len() {
            let next = indexes[p + 1];
            self.set(next, self.last_event(next));
        }
    }

    /// Jump to the first pitch of the at bat, or to the previous at bat's first pitch when
    /// already there. From live this enters on the newest at bat.
    pub fn previous_at_bat(&mut self) {
        let Some((indexes, pos)) = self.navigable_selection() else {
            return;
        };
        match (pos, self.cursor) {
            (Some(p), Some(cursor)) => {
                if cursor.pitch > 0 {
                    self.set(cursor.at_bat, 0);
                } else if p > 0 {
                    self.set(indexes[p - 1], 0);
                }
            }
            _ => self.set(indexes[indexes.len() - 1], 0),
        }
    }

    /// Go to "live" at bat by deselecting the current at bat. When the scoring play filter is
    /// enabled, "live" means the most recent scoring play instead, matching what enabling the
    /// filter while live already selects.
    pub fn live(&mut self) {
        self.cursor = None;
        if self.scoring_plays_only {
            self.snap_to_scoring_play();
        }
    }

    /// Go to the first pitch of the game.
    pub fn start(&mut self) {
        if let Some(first) = self.navigable_at_bats().first() {
            self.set(*first, 0);
        }
    }

    pub fn toggle_info(&mut self) {
        self.panels.info = !self.panels.info;
    }

    pub fn toggle_at_bat(&mut self) {
        self.panels.at_bat = !self.panels.at_bat;
    }

    pub fn toggle_boxscore(&mut self) {
        self.panels.boxscore = !self.panels.boxscore;
    }

    pub fn toggle_win_probability(&mut self) {
        self.panels.win_probability = !self.panels.win_probability;
    }

    pub fn toggle_pitch_tracker(&mut self) {
        self.panels.pitch_tracker = !self.panels.pitch_tracker;
    }

    pub fn toggle_movement(&mut self) {
        self.panels.movement = !self.panels.movement;
    }

    pub fn toggle_scoring_plays_only(&mut self) {
        self.scoring_plays_only = !self.scoring_plays_only;
        if self.scoring_plays_only {
            self.snap_to_scoring_play();
        }
    }

    /// Move the selection onto a scoring play so the cursor is visible while the filter is
    /// enabled. Snaps to the closest scoring play, which keeps a selection that already scores,
    /// and selects the most recent scoring play when live. A game with no scoring plays leaves the
    /// selection untouched.
    ///
    /// Lands on the play's *last* pitch. Filtering to scoring plays is a request to see the
    /// runs, so half-revealing one would hide the very thing being asked for.
    fn snap_to_scoring_play(&mut self) {
        let scoring = self.navigable_at_bats();
        if scoring.is_empty() {
            return;
        }
        let target = match self.cursor {
            Some(cursor) => scoring
                .iter()
                .min_by_key(|&&i| i.abs_diff(cursor.at_bat))
                .copied(),
            None => scoring.last().copied(),
        };
        if let Some(at_bat) = target {
            self.set(at_bat, self.last_event(at_bat));
        }
    }
}

/// Store which panels should be rendered in the Gameday tab.
#[derive(Debug, Copy, Clone)]
pub struct GamedayPanels {
    pub info: bool,
    pub at_bat: bool,
    pub boxscore: bool,
    pub win_probability: bool,
    /// The per-pitcher totals column beside the strike zone. Lives inside the at bat
    /// panel rather than beside it, so it is deliberately absent from `count`.
    pub pitch_tracker: bool,
    /// Spin and break appended to each pitch in the event list. Also inside the at
    /// bat panel, so also absent from `count`.
    pub movement: bool,
}

impl GamedayPanels {
    /// Return the number of panels that are active.
    pub fn count(&self) -> usize {
        self.info as usize + self.at_bat as usize + self.boxscore as usize
    }
}

impl Default for GamedayPanels {
    fn default() -> Self {
        GamedayPanels {
            info: true,
            at_bat: true,
            boxscore: false,
            win_probability: true,
            pitch_tracker: true,
            movement: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::game::at_bat::AtBat;
    use crate::components::game::pitch_event::PitchEvent;
    use mlbt_api::live::LiveData;
    use mlbt_api::plays::{About, Play, Plays};

    /// Build a state whose at bats have the given indexes, marking the listed indexes as scoring
    /// plays.
    fn state_with(indexes: &[u8], scoring: &[u8]) -> GamedayState {
        let mut state = GamedayState::default();
        insert_at_bats(&mut state, indexes, scoring);
        state
    }

    fn insert_at_bats(state: &mut GamedayState, indexes: &[u8], scoring: &[u8]) {
        for &index in indexes {
            let mut at_bat = AtBat {
                index,
                ..Default::default()
            };
            at_bat.play_result.is_scoring_play = scoring.contains(&index);
            state.game.at_bats.insert(index, at_bat);
        }
    }

    #[test]
    fn navigation_walks_every_play_normally() {
        let mut state = state_with(&[0, 1, 2], &[1]);
        state.start();

        assert_eq!(state.selected_at_bat(), Some(0));
        state.next_at_bat();
        assert_eq!(state.selected_at_bat(), Some(1));
        state.previous_at_bat();
        assert_eq!(state.selected_at_bat(), Some(0));
        // stops at the first at bat instead of wrapping to the end
        state.previous_at_bat();
        assert_eq!(state.selected_at_bat(), Some(0));
    }

    #[test]
    fn walking_forward_stops_at_the_newest_at_bat() {
        // the point of the arrow keys: right arrow catches up to the broadcast and then stalls,
        // rather than wrapping back to the first inning and losing the place
        let mut state = state_with(&[0, 1, 2], &[]);
        state.start();

        for _ in 0..5 {
            state.next_at_bat();
        }
        assert_eq!(state.selected_at_bat(), Some(2));
    }

    #[test]
    fn stepping_forward_from_live_stays_live() {
        // live is already the newest at bat, so there is nothing ahead of it to step to
        let mut state = state_with(&[0, 1, 2], &[]);
        state.live();

        state.next_at_bat();
        assert_eq!(state.selected_at_bat(), None);

        // stepping back from live enters the list at its end
        state.previous_at_bat();
        assert_eq!(state.selected_at_bat(), Some(2));
    }

    /// A state whose at bats each hold the given number of pitches, in order.
    fn state_with_pitches(pitch_counts: &[usize]) -> GamedayState {
        let mut state = GamedayState::default();
        for (index, &count) in pitch_counts.iter().enumerate() {
            let mut at_bat = AtBat {
                index: index as u8,
                ..Default::default()
            };
            at_bat.pitches.pitches.pitch_events =
                (0..count).map(|_| PitchEvent::default()).collect();
            state.game.at_bats.insert(index as u8, at_bat);
        }
        state
    }

    #[test]
    fn the_arrows_walk_pitches_not_at_bats() {
        // the whole point: three presses inside a four pitch at bat stay inside it
        let mut state = state_with_pitches(&[4, 3]);
        state.start();
        assert_eq!(
            state.cursor(),
            Some(Cursor {
                at_bat: 0,
                pitch: 0
            })
        );

        state.next_pitch();
        state.next_pitch();
        assert_eq!(
            state.cursor(),
            Some(Cursor {
                at_bat: 0,
                pitch: 2
            })
        );

        // the fourth press crosses into the next at bat, landing on its first pitch
        state.next_pitch();
        state.next_pitch();
        assert_eq!(
            state.cursor(),
            Some(Cursor {
                at_bat: 1,
                pitch: 0
            })
        );
    }

    #[test]
    fn stepping_back_lands_on_the_previous_at_bats_last_pitch() {
        let mut state = state_with_pitches(&[4, 3]);
        state.set(1, 0);

        state.previous_pitch();
        assert_eq!(
            state.cursor(),
            Some(Cursor {
                at_bat: 0,
                pitch: 3
            })
        );
    }

    #[test]
    fn walking_forward_stops_on_the_newest_pitch() {
        // catching up must stall at the live edge rather than wrap and lose the place
        let mut state = state_with_pitches(&[2, 2]);
        state.start();

        for _ in 0..10 {
            state.next_pitch();
        }
        assert_eq!(
            state.cursor(),
            Some(Cursor {
                at_bat: 1,
                pitch: 1
            })
        );
    }

    #[test]
    fn walking_back_stops_on_the_first_pitch() {
        let mut state = state_with_pitches(&[2, 2]);
        state.start();

        for _ in 0..10 {
            state.previous_pitch();
        }
        assert_eq!(
            state.cursor(),
            Some(Cursor {
                at_bat: 0,
                pitch: 0
            })
        );
    }

    #[test]
    fn the_bracket_keys_jump_a_whole_at_bat() {
        // `]` finishes the at bat you are in, then takes the next one whole
        let mut state = state_with_pitches(&[4, 3, 5]);
        state.start();

        state.next_at_bat();
        assert_eq!(
            state.cursor(),
            Some(Cursor {
                at_bat: 0,
                pitch: 3
            })
        );
        state.next_at_bat();
        assert_eq!(
            state.cursor(),
            Some(Cursor {
                at_bat: 1,
                pitch: 2
            })
        );

        // `[` rewinds to the top of the at bat, then to the top of the one before
        state.previous_at_bat();
        assert_eq!(
            state.cursor(),
            Some(Cursor {
                at_bat: 1,
                pitch: 0
            })
        );
        state.previous_at_bat();
        assert_eq!(
            state.cursor(),
            Some(Cursor {
                at_bat: 0,
                pitch: 0
            })
        );
    }

    #[test]
    fn a_growing_at_bat_does_not_drag_the_cursor_forward() {
        // the live at bat gains pitches between polls; the viewer's place must not move
        let mut state = state_with_pitches(&[4]);
        state.set(0, 1);

        state
            .game
            .at_bats
            .get_mut(&0)
            .unwrap()
            .pitches
            .pitches
            .pitch_events = (0..7).map(|_| PitchEvent::default()).collect();
        state.clamp_cursor();

        assert_eq!(
            state.cursor(),
            Some(Cursor {
                at_bat: 0,
                pitch: 1
            })
        );
    }

    #[test]
    fn a_shorter_snapshot_pulls_the_cursor_back_into_range() {
        // a snapshot can lose plays; the cursor must not point past the end of the vec
        let mut state = state_with_pitches(&[6]);
        state.set(0, 5);

        state
            .game
            .at_bats
            .get_mut(&0)
            .unwrap()
            .pitches
            .pitches
            .pitch_events = (0..2).map(|_| PitchEvent::default()).collect();
        state.clamp_cursor();

        assert_eq!(
            state.cursor(),
            Some(Cursor {
                at_bat: 0,
                pitch: 1
            })
        );
    }

    #[test]
    fn opening_gameday_lands_on_the_first_pitch_of_the_game() {
        let mut state = state_with_pitches(&[4, 3]);
        state.live();

        state.open_at_start();
        assert_eq!(
            state.cursor(),
            Some(Cursor {
                at_bat: 0,
                pitch: 0
            })
        );
    }

    #[test]
    fn opening_gameday_lands_on_the_first_at_bat() {
        let mut state = state_with(&[0, 1, 2], &[]);
        state.live();

        state.open_at_start();
        assert_eq!(state.selected_at_bat(), Some(0));
    }

    #[test]
    fn opening_gameday_before_at_bats_load_stays_armed() {
        // Gameday is entered before the GameData response arrives; falling back to live here is
        // exactly the spoiler this avoids, so the jump waits for the at bats
        let mut state = GamedayState::default();
        state.open_at_start();
        assert_eq!(state.selected_at_bat(), None);

        let win_probability = WinProbabilityResponse::default();
        state.update(&live_response(6, &[0, 1, 2], &[]), &win_probability);
        assert_eq!(state.selected_at_bat(), Some(0));

        // and a later update for the same game leaves the selection where the user put it
        state.next_at_bat();
        state.update(&live_response(6, &[0, 1, 2], &[]), &win_probability);
        assert_eq!(state.selected_at_bat(), Some(1));
    }

    #[test]
    fn navigation_skips_non_scoring_plays_when_filtered() {
        let mut state = state_with(&[0, 1, 2, 3, 4], &[1, 3]);
        state.scoring_plays_only = true;

        // only scoring plays are reachable, in order
        state.start();
        assert_eq!(state.selected_at_bat(), Some(1));
        state.next_at_bat();
        assert_eq!(state.selected_at_bat(), Some(3));
        // stops on the last scoring play instead of wrapping
        state.next_at_bat();
        assert_eq!(state.selected_at_bat(), Some(3));
        state.previous_at_bat();
        assert_eq!(state.selected_at_bat(), Some(1));
    }

    #[test]
    fn start_respects_scoring_filter() {
        let mut state = state_with(&[0, 1, 2, 3], &[2]);
        state.start();
        assert_eq!(state.selected_at_bat(), Some(0));

        state.scoring_plays_only = true;
        state.start();
        assert_eq!(state.selected_at_bat(), Some(2));
    }

    #[test]
    fn live_snaps_to_latest_scoring_play_when_filtered() {
        // this is what the `1` (Scoreboard) key calls on the way out of Gameday. Without the
        // filter check, repeatedly switching to Scoreboard and back to the same, already-loaded
        // game (no game id change, so nothing re-snaps on return) would strand the selection on
        // live instead of a scoring play.
        let mut state = state_with(&[0, 1, 2, 3, 4], &[1, 3]);
        state.scoring_plays_only = true;
        state.set(1, 0);

        state.live();
        assert_eq!(state.selected_at_bat(), Some(3));

        // idempotent across repeated round trips
        state.live();
        assert_eq!(state.selected_at_bat(), Some(3));

        // without the filter, live still means truly live
        state.scoring_plays_only = false;
        state.live();
        assert_eq!(state.selected_at_bat(), None);
    }

    #[test]
    fn toggle_snaps_selection_to_nearest_scoring_play() {
        let mut state = state_with(&[0, 1, 2, 3, 4], &[1, 4]);

        // selecting a non scoring play then filtering snaps to the closest scoring play
        state.set(3, 0);
        state.toggle_scoring_plays_only();
        assert_eq!(state.selected_at_bat(), Some(4));

        // an already scoring selection is left untouched
        state.toggle_scoring_plays_only(); // off
        state.set(1, 0);
        state.toggle_scoring_plays_only(); // on
        assert_eq!(state.selected_at_bat(), Some(1));

        // toggling on while live selects the most recent scoring play
        state.toggle_scoring_plays_only(); // off
        state.live();
        state.toggle_scoring_plays_only(); // on
        assert_eq!(state.selected_at_bat(), Some(4));
    }

    /// Build a `LiveResponse` for the given game with at bats at the given indexes, marking the
    /// listed indexes as scoring plays. Mirrors a real `GameDataLoaded` network response.
    fn live_response(game_pk: u64, indexes: &[u8], scoring: &[u8]) -> LiveResponse {
        let all_plays = indexes
            .iter()
            .map(|&index| Play {
                about: About {
                    at_bat_index: index,
                    is_scoring_play: Some(scoring.contains(&index)),
                    ..Default::default()
                },
                ..Default::default()
            })
            .collect();
        LiveResponse {
            game_pk,
            live_data: LiveData {
                plays: Plays {
                    all_plays: Some(all_plays),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn a_shorter_snapshot_takes_the_plays_feed_back_with_it() {
        // `all_plays` is the complete list on every snapshot, so the play map is a copy of it,
        // not a merge into the previous one. Merging let any play the feed had ever seen
        // survive a snapshot that did not contain it, and the feed was then the only part of
        // the screen that could not go backwards.
        let mut state = GamedayState::default();
        let win_probability = WinProbabilityResponse::default();

        state.update(&live_response(1, &[0, 1, 2, 3, 4], &[]), &win_probability);
        assert_eq!(state.game.at_bats.len(), 5);

        state.update(&live_response(1, &[0, 1, 2], &[]), &win_probability);
        assert_eq!(state.game.at_bats.len(), 3, "the later plays are gone");
        assert!(state.game.get_at_bat_by_index(4).is_none());
    }

    #[test]
    fn switching_games_via_direct_navigation_opens_at_the_start() {
        // this is the path taken when the user selects a different game with j/k on the
        // Scoreboard tab: `GameData` responses arrive via `update` directly, with no prior call to
        // `reset` (that only happens on a schedule refresh).
        let mut state = state_with(&[0, 1, 2], &[1]);
        state.set(1, 0);

        let win_probability = WinProbabilityResponse::default();
        let response = live_response(6, &[0, 1, 2, 3], &[1, 3]);
        state.update(&response, &win_probability);

        assert_eq!(state.game.game_id, 6);
        assert_eq!(state.selected_at_bat(), Some(0));

        // a later update for the same game doesn't drag the selection along
        state.next_at_bat();
        assert_eq!(state.selected_at_bat(), Some(1));
        state.update(&response, &win_probability);
        assert_eq!(state.selected_at_bat(), Some(1));
    }

    #[test]
    fn switching_games_with_the_scoring_filter_opens_at_the_first_scoring_play() {
        // the start of the game, not the latest scoring play: the latest one is the spoiler
        let mut state = state_with(&[0, 1, 2], &[1]);
        state.scoring_plays_only = true;
        state.set(1, 0);

        let win_probability = WinProbabilityResponse::default();
        state.update(&live_response(6, &[0, 1, 2, 3], &[1, 3]), &win_probability);

        assert_eq!(state.selected_at_bat(), Some(1));
    }

    #[test]
    fn switching_games_via_schedule_refresh_opens_at_the_start() {
        // this is the path taken on a schedule poll: `reset` runs first (before at bats have
        // loaded), then the `GameData` response arrives separately via `update`.
        let mut state = state_with(&[0, 1, 2], &[1]);
        state.set(1, 0);

        state.reset(Some(6));
        assert_eq!(state.selected_at_bat(), None);

        let win_probability = WinProbabilityResponse::default();
        state.update(&live_response(6, &[0, 1, 2, 3], &[1, 3]), &win_probability);
        assert_eq!(state.selected_at_bat(), Some(0));
    }

    #[test]
    fn filter_keeps_selection_without_scoring_plays() {
        let mut state = state_with(&[0, 1, 2], &[]);
        state.set(1, 0);

        // toggling the filter on has no scoring play to snap to
        state.toggle_scoring_plays_only();
        assert_eq!(state.selected_at_bat(), Some(1));

        // going to the start has nowhere to go either
        state.start();
        assert_eq!(state.selected_at_bat(), Some(1));
    }

    #[test]
    fn navigation_is_a_noop_without_navigable_plays() {
        // no at bats at all
        let mut empty = GamedayState::default();
        empty.next_at_bat();
        empty.previous_at_bat();
        assert_eq!(empty.selected_at_bat(), None);

        // at bats exist but none score, with the filter on
        let mut none_score = state_with(&[0, 1], &[]);
        none_score.scoring_plays_only = true;
        none_score.next_at_bat();
        none_score.previous_at_bat();
        assert_eq!(none_score.selected_at_bat(), None);
    }
}
