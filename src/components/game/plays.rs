use mlbt_api::plays::{Count, Play};

#[derive(Default)]
pub struct PlayEvent {
    pub code: Option<String>,
    // TODO
}

#[derive(Default)]
pub struct PlayResult {
    pub at_bat_index: u8,
    pub description: String,
    pub rbi: u8,
    pub away_score: u8,
    pub home_score: u8,
    pub count: Count,
    pub is_out: bool,
    pub is_scoring_play: bool,
    /// The API's `eventType` slug, e.g. `single`, `home_run`, `field_error`. Carried so a
    /// linescore can be rebuilt from the plays the viewer has actually reached, rather
    /// than read from the API's whole-game one.
    pub event_type: Option<String>,
    pub events: Vec<PlayEvent>,
}

impl PlayResult {
    /// Whether the play put a hit on the board, for a reconstructed linescore.
    pub fn is_hit(&self) -> bool {
        matches!(
            self.event_type.as_deref(),
            Some("single" | "double" | "triple" | "home_run")
        )
    }

    /// Whether the play put an error on the board, charged to the fielding team.
    pub fn is_error(&self) -> bool {
        matches!(self.event_type.as_deref(), Some("field_error"))
    }
}

impl From<&Play> for PlayResult {
    fn from(play: &Play) -> Self {
        Self {
            at_bat_index: play.about.at_bat_index,
            description: play.result.description.clone().unwrap_or_default(),
            rbi: play.result.rbi.unwrap_or(0),
            away_score: play.result.away_score.unwrap_or(0),
            home_score: play.result.home_score.unwrap_or(0),
            count: play.count.clone(),
            is_out: play.result.is_out.unwrap_or(false),
            is_scoring_play: play.about.is_scoring_play.unwrap_or(false),
            event_type: play.result.event_type.clone(),
            events: play
                .play_events
                .iter()
                .map(|e| PlayEvent {
                    code: e.details.call.as_ref().map(|d| d.code.clone()),
                })
                .collect(),
        }
    }
}
