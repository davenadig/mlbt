use crate::components::standings::Team;

/// Selection belongs to the viewed game; status describes the last audio command.
#[derive(Default)]
pub struct ListenState {
    pub open: bool,
    pub home: bool,
    pub busy: bool,
    pub message: String,
    game_id: u64,
}

impl ListenState {
    pub fn open(&mut self, game_id: u64, away: Team, home: Team, favorite: Option<Team>) {
        if self.game_id != game_id {
            self.home = favorite.is_none_or(|team| team.id != away.id || team.id == home.id);
            self.game_id = game_id;
        }
        self.open = true;
    }

    pub fn selected(&self, away: Team, home: Team) -> Team {
        if self.home { home } else { away }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn team(id: u16) -> Team {
        Team {
            id,
            ..Team::default()
        }
    }

    #[test]
    fn defaults_to_favorite_then_home_and_preserves_selection_per_game() {
        let mut state = ListenState::default();
        state.open(1, team(111), team(147), Some(team(111)));
        assert_eq!(state.selected(team(111), team(147)).id, 111);
        state.home = true;
        state.open(1, team(111), team(147), Some(team(111)));
        assert!(state.home);
        state.open(2, team(121), team(147), Some(team(111)));
        assert!(state.home);
    }
}
