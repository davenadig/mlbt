//! Broadcast-delay buffer for live game data.
//!
//! The Stats API is a few seconds behind the field; a television broadcast is
//! considerably further behind than that. Watching a game on TV with mlbt open
//! therefore spoils every pitch — the terminal shows the result while the pitch
//! is still in the air on screen.
//!
//! This holds each live snapshot for a configured number of seconds before the
//! app applies it, so the display sits *behind* the broadcast rather than ahead
//! of it. Delay is measured from the moment the snapshot was fetched from the
//! API (`fetched_at` on the response), not from when the UI happened to receive
//! it, so a snapshot served out of the network cache still ages correctly.
//!
//! Only `Live` games are held. A `Preview` or `Final` game has nothing left to
//! spoil, and delaying it would just stall the display for no reason.

use mlbt_api::live::LiveResponse;
use mlbt_api::schedule::AbstractGameState;
use mlbt_api::win_probability::WinProbabilityResponse;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Upper bound on held snapshots. The refresher polls live data every 3s and the
/// longest offered delay is 120s, so a healthy buffer holds about forty. The cap is
/// only a guard against a pathological burst; overflow drops the oldest, which is
/// the entry that was about to be discarded anyway (see `take_due`).
///
/// Each entry is two `Arc`s, so a deep queue costs pointers, not payloads.
const MAX_BUFFERED: usize = 128;

/// A fetched-but-not-yet-displayed live snapshot.
struct Buffered {
    fetched_at: Instant,
    game: Arc<LiveResponse>,
    win_probability: Arc<WinProbabilityResponse>,
}

/// Delay settings + the snapshots currently being held.
#[derive(Default)]
pub struct DelayBuffer {
    queue: VecDeque<Buffered>,
    /// game_pk of the snapshots in the queue, so a game switch can flush them.
    game_pk: Option<u64>,
    /// When the snapshot currently on screen was fetched. This is what makes the
    /// delay checkable rather than a matter of faith: the badge reports the real age
    /// of what is displayed, so a delay that is not deep enough for the broadcast
    /// shows up as a number instead of as a spoiled pitch.
    displayed_at: Option<Instant>,
}

/// What the delay is doing right now, for the ui badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelayPhase {
    /// Nothing shown for this game yet; seconds until the first snapshot is released.
    Warming(u64),
    /// Measured seconds between what is on screen and what the API last handed over.
    Behind(u64),
}

/// What the caller should do with a snapshot handed to [`DelayBuffer::accept`].
pub enum Accepted {
    /// Apply it now — the delay is off, or the game is not live.
    Now(Arc<LiveResponse>, Arc<WinProbabilityResponse>),
    /// It was queued. Nothing to display yet.
    Held,
}

impl DelayBuffer {
    /// Take a freshly received snapshot. Returns whether it should be applied
    /// immediately or was queued for later release by [`take_due`].
    ///
    /// [`take_due`]: DelayBuffer::take_due
    pub fn accept(
        &mut self,
        game: Arc<LiveResponse>,
        win_probability: Arc<WinProbabilityResponse>,
        fetched_at: Instant,
        delay: Duration,
    ) -> Accepted {
        // A game switch invalidates everything held for the previous game. Without
        // this the countdown would describe a game that is no longer on screen.
        if self.game_pk != Some(game.game_pk) {
            self.queue.clear();
            self.displayed_at = None;
            self.game_pk = Some(game.game_pk);
        }

        if !should_delay(&game, delay) {
            self.queue.clear();
            self.displayed_at = Some(fetched_at);
            return Accepted::Now(game, win_probability);
        }

        self.queue.push_back(Buffered {
            fetched_at,
            game,
            win_probability,
        });
        while self.queue.len() > MAX_BUFFERED {
            self.queue.pop_front();
        }
        Accepted::Held
    }

    /// Release the newest snapshot that has aged past `delay`, discarding any
    /// older ones that also came due.
    ///
    /// Discarding is correct, not a shortcut: every `LiveResponse` is a complete
    /// snapshot of the game, so applying the intermediate ones would only render
    /// states that are already superseded. It also makes shortening the delay
    /// (45s → 5s) settle in one step instead of replaying the backlog.
    pub fn take_due(
        &mut self,
        delay: Duration,
        now: Instant,
    ) -> Option<(Arc<LiveResponse>, Arc<WinProbabilityResponse>)> {
        let due = self
            .queue
            .iter()
            .rposition(|b| now.duration_since(b.fetched_at) >= delay)?;
        // everything at or before `due` is due; keep only the newest of them
        let mut newest = None;
        for _ in 0..=due {
            newest = self.queue.pop_front();
        }
        newest.map(|b| {
            self.displayed_at = Some(b.fetched_at);
            (b.game, b.win_probability)
        })
    }

    /// How far behind the API the display actually is, or `None` before the first
    /// snapshot for this game has been shown.
    ///
    /// Measured, not assumed. The configured delay is a floor: the display is also
    /// behind by however long ago the released snapshot was fetched, which is why the
    /// poll interval wants to be short. Reporting the real figure is the only way to
    /// tell "the delay is too shallow for this broadcast" from "the delay is broken".
    pub fn displayed_age(&self, now: Instant) -> Option<Duration> {
        Some(now.duration_since(self.displayed_at?))
    }

    /// Whole seconds until the next held snapshot is released, or `None` when
    /// nothing is held. Rounded up so a countdown reaches 0 only on release.
    pub fn seconds_until_release(&self, delay: Duration, now: Instant) -> Option<u64> {
        let next = self.queue.front()?;
        let waited = now.duration_since(next.fetched_at);
        let remaining = delay.saturating_sub(waited);
        Some(remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0))
    }

    /// Pretend a snapshot fetched at `at` is on screen. Test-only seam so ui tests
    /// can exercise the warm badge without standing up the whole network path.
    #[cfg(test)]
    pub fn set_displayed_at(&mut self, at: Instant) {
        self.displayed_at = Some(at);
    }

    #[cfg(test)]
    fn is_holding(&self) -> bool {
        !self.queue.is_empty()
    }

    #[cfg(test)]
    fn depth(&self) -> usize {
        self.queue.len()
    }
}

// Turning the delay off deliberately does not flush the queue: `take_due` with a
// zero delay finds every held snapshot due and releases the newest, so the display
// jumps straight to the most recent data instead of waiting out the old delay.

/// Whether a snapshot should be held at all. A `Preview` game has not started and
/// a `Final` one is over; delaying either only stalls the display.
fn should_delay(game: &LiveResponse, delay: Duration) -> bool {
    !delay.is_zero()
        && matches!(
            game.game_data.status.abstract_game_state,
            Some(AbstractGameState::Live)
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlbt_api::schedule::Status;

    fn response(game_pk: u64, state: AbstractGameState) -> Arc<LiveResponse> {
        Arc::new(LiveResponse {
            game_pk,
            game_data: mlbt_api::live::GameData {
                status: Status {
                    abstract_game_state: Some(state),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        })
    }

    fn wp() -> Arc<WinProbabilityResponse> {
        Arc::new(WinProbabilityResponse::default())
    }

    fn is_held(accepted: &Accepted) -> bool {
        matches!(accepted, Accepted::Held)
    }

    const DELAY: Duration = Duration::from_secs(30);

    #[test]
    fn zero_delay_applies_immediately() {
        let mut buffer = DelayBuffer::default();
        let accepted = buffer.accept(
            response(1, AbstractGameState::Live),
            wp(),
            Instant::now(),
            Duration::ZERO,
        );
        assert!(!is_held(&accepted));
        assert!(!buffer.is_holding());
    }

    #[test]
    fn non_live_games_are_never_held() {
        // a final game has nothing left to spoil, and holding a preview would stall
        // the display before first pitch for no reason
        for state in [AbstractGameState::Final, AbstractGameState::Preview] {
            let mut buffer = DelayBuffer::default();
            let accepted = buffer.accept(response(1, state), wp(), Instant::now(), DELAY);
            assert!(!is_held(&accepted), "{state:?} should not be held");
            assert!(!buffer.is_holding());
        }
    }

    #[test]
    fn live_snapshot_is_held_until_the_delay_elapses() {
        let mut buffer = DelayBuffer::default();
        let fetched = Instant::now();
        assert!(is_held(&buffer.accept(
            response(1, AbstractGameState::Live),
            wp(),
            fetched,
            DELAY
        )));

        assert!(
            buffer
                .take_due(DELAY, fetched + Duration::from_secs(29))
                .is_none()
        );
        assert!(buffer.take_due(DELAY, fetched + DELAY).is_some());
        assert!(!buffer.is_holding());
    }

    #[test]
    fn only_the_newest_due_snapshot_is_released() {
        // every response is a full snapshot, so replaying the backlog would render
        // states that are already superseded
        let mut buffer = DelayBuffer::default();
        let start = Instant::now();
        for (offset, pk_marker) in [(0, 10u64), (10, 11), (20, 12)] {
            let mut game = LiveResponse {
                game_pk: 1,
                ..Default::default()
            };
            game.game_data.status.abstract_game_state = Some(AbstractGameState::Live);
            game.meta_data.wait = pk_marker as i64; // stand-in for "which snapshot"
            buffer.accept(
                Arc::new(game),
                wp(),
                start + Duration::from_secs(offset),
                DELAY,
            );
        }

        // at +45s the first two (fetched at 0 and 10, so aged 45s and 35s) are due;
        // the third has only aged 25s
        let (released, _) = buffer
            .take_due(DELAY, start + Duration::from_secs(45))
            .expect("two snapshots are due");
        assert_eq!(released.meta_data.wait, 11, "released the newest due one");
        assert!(buffer.is_holding(), "the not-yet-due snapshot stays");
    }

    #[test]
    fn switching_games_flushes_the_previous_game() {
        let mut buffer = DelayBuffer::default();
        let fetched = Instant::now();
        buffer.accept(response(1, AbstractGameState::Live), wp(), fetched, DELAY);
        buffer.accept(response(2, AbstractGameState::Live), wp(), fetched, DELAY);

        let (released, _) = buffer
            .take_due(DELAY, fetched + DELAY)
            .expect("the new game's snapshot is due");
        assert_eq!(released.game_pk, 2);
        assert!(
            !buffer.is_holding(),
            "game 1's snapshot was dropped, not queued behind"
        );
    }

    #[test]
    fn shortening_the_delay_makes_held_snapshots_due() {
        let mut buffer = DelayBuffer::default();
        let fetched = Instant::now();
        buffer.accept(response(1, AbstractGameState::Live), wp(), fetched, DELAY);

        // still held at the original delay
        assert!(
            buffer
                .take_due(DELAY, fetched + Duration::from_secs(6))
                .is_none()
        );
        // immediately due once the delay drops to 5s
        assert!(
            buffer
                .take_due(Duration::from_secs(5), fetched + Duration::from_secs(6))
                .is_some()
        );
    }

    #[test]
    fn countdown_rounds_up_and_ends_at_release() {
        let mut buffer = DelayBuffer::default();
        let fetched = Instant::now();
        buffer.accept(response(1, AbstractGameState::Live), wp(), fetched, DELAY);

        assert_eq!(buffer.seconds_until_release(DELAY, fetched), Some(30));
        assert_eq!(
            buffer.seconds_until_release(DELAY, fetched + Duration::from_millis(500)),
            Some(30),
            "a partial second still reads as a full second remaining"
        );
        assert_eq!(
            buffer.seconds_until_release(DELAY, fetched + Duration::from_secs(30)),
            Some(0)
        );
    }

    #[test]
    fn nothing_is_displayed_until_the_first_release() {
        let mut buffer = DelayBuffer::default();
        let fetched = Instant::now();
        buffer.accept(response(1, AbstractGameState::Live), wp(), fetched, DELAY);
        assert_eq!(
            buffer.displayed_age(fetched + Duration::from_secs(10)),
            None
        );

        buffer.take_due(DELAY, fetched + DELAY);
        assert_eq!(
            buffer.displayed_age(fetched + DELAY),
            Some(DELAY),
            "the released snapshot is exactly as old as the delay"
        );
    }

    #[test]
    fn displayed_age_is_the_delay_plus_however_stale_the_release_was() {
        // this is the number the badge reports, and the reason it is worth reporting:
        // the poll interval sits on top of the configured delay, so what is on screen
        // is always a little further behind than the setting claims
        let mut buffer = DelayBuffer::default();
        let start = Instant::now();
        for offset in [0, 3, 6] {
            buffer.accept(
                response(1, AbstractGameState::Live),
                wp(),
                start + Duration::from_secs(offset),
                DELAY,
            );
        }

        // at +32s only the snapshot fetched at 0 is due, and it is 32s old
        buffer.take_due(DELAY, start + Duration::from_secs(32));
        assert_eq!(
            buffer.displayed_age(start + Duration::from_secs(32)),
            Some(Duration::from_secs(32))
        );
    }

    #[test]
    fn a_game_switch_forgets_what_was_displayed() {
        // the age belongs to a game, not to the screen; carrying it across a switch
        // would report the previous game's lag for the new one
        let mut buffer = DelayBuffer::default();
        let fetched = Instant::now();
        buffer.accept(response(1, AbstractGameState::Live), wp(), fetched, DELAY);
        buffer.take_due(DELAY, fetched + DELAY);
        assert!(buffer.displayed_age(fetched + DELAY).is_some());

        buffer.accept(
            response(2, AbstractGameState::Live),
            wp(),
            fetched + DELAY,
            DELAY,
        );
        assert_eq!(buffer.displayed_age(fetched + DELAY), None);
    }

    #[test]
    fn an_undelayed_game_reports_its_own_freshness() {
        // a final game is applied straight through, so the badge should say ~0s rather
        // than claim the configured delay it is not actually applying
        let mut buffer = DelayBuffer::default();
        let fetched = Instant::now();
        buffer.accept(response(1, AbstractGameState::Final), wp(), fetched, DELAY);
        assert_eq!(
            buffer.displayed_age(fetched + Duration::from_secs(2)),
            Some(Duration::from_secs(2))
        );
    }

    #[test]
    fn a_two_minute_delay_at_a_three_second_poll_fits_in_the_buffer() {
        // 120s / 3s is forty snapshots; the cap has to clear that or the deepest delay
        // would drop the very snapshots it is waiting on
        let mut buffer = DelayBuffer::default();
        let start = Instant::now();
        for offset in (0..120).step_by(3) {
            buffer.accept(
                response(1, AbstractGameState::Live),
                wp(),
                start + Duration::from_secs(offset),
                Duration::from_secs(120),
            );
        }
        assert_eq!(buffer.depth(), 40, "nothing was dropped");
    }

    #[test]
    fn buffer_is_bounded() {
        let mut buffer = DelayBuffer::default();
        let start = Instant::now();
        for i in 0..(MAX_BUFFERED * 2) {
            buffer.accept(
                response(1, AbstractGameState::Live),
                wp(),
                start + Duration::from_secs(i as u64),
                DELAY,
            );
        }
        assert_eq!(buffer.queue.len(), MAX_BUFFERED);
    }
}
