use crate::state::gameday::GamedayPanels;
use crate::ui::input_popup;
use tui::layout::{Constraint, Layout, Rect, Size};

pub struct LayoutAreas {
    pub top_bar: Rect,
    pub main: Rect,
    pub footer: Rect,
}

const TOP_BAR_HEIGHT: u16 = 3; // length
const FOOTER_HEIGHT: u16 = 2;

impl LayoutAreas {
    pub fn new(size: Size) -> Self {
        let rect = Rect::new(0, 0, size.width, size.height);
        let [top, main, footer] = Layout::vertical([
            Constraint::Length(TOP_BAR_HEIGHT),
            Constraint::Fill(1),
            Constraint::Length(FOOTER_HEIGHT),
        ])
        .margin(1)
        .areas(rect);

        LayoutAreas {
            top_bar: top,
            main,
            footer,
        }
    }

    pub(crate) fn update(&mut self, size: Rect, full_screen: bool) {
        let constraints = match full_screen {
            true => vec![
                Constraint::Length(0),
                Constraint::Fill(1),
                Constraint::Length(FOOTER_HEIGHT),
            ],
            false => vec![
                Constraint::Length(TOP_BAR_HEIGHT),
                Constraint::Fill(1),
                Constraint::Length(FOOTER_HEIGHT),
            ],
        };
        let [top, main, footer] = Layout::vertical(constraints).areas(size);

        self.top_bar = top;
        self.main = main;
        self.footer = footer;
    }

    /// Create splits for displaying the line score on top and a box score below.
    /// If a game is Final, show the decision pitchers in between.
    pub fn for_boxscore(rect: Rect, decision_pitchers: u16) -> Vec<Rect> {
        let mut constraints = vec![
            Constraint::Length(4), // line score
            Constraint::Fill(1),   // box score
        ];
        if decision_pitchers > 0 {
            constraints.insert(1, Constraint::Length(decision_pitchers + 1)); // +1 for blank line
        }

        Layout::vertical(constraints)
            .horizontal_margin(2)
            .vertical_margin(1)
            .split(rect)
            .to_vec()
    }

    /// Create two splits for displaying the current matchup and the pitches for the at bat.
    pub fn for_at_bat(rect: Rect) -> [Rect; 2] {
        Layout::vertical([
            Constraint::Length(7), // matchup + on deck
            Constraint::Fill(1),   // pitches
        ])
        .areas(rect)
    }

    /// Create two splits for displaying game info for the current inning and the recent win
    /// probability. This is used in the `gameday` tab.
    pub fn for_info(rect: Rect, show_win_probability: bool) -> Vec<Rect> {
        let constraints = match show_win_probability {
            true => vec![Constraint::Fill(1), Constraint::Length(6)],
            false => vec![Constraint::Fill(1)],
        };
        Layout::vertical(constraints)
            .horizontal_margin(2)
            .vertical_margin(1)
            .spacing(1) // one row of padding between the plays and the win probability chart
            .split(rect)
            .to_vec()
    }

    /// Create the Gameday layouts based on how many of the panels are active.
    pub fn generate_gameday_panels(active: &GamedayPanels, area: Rect) -> Vec<Rect> {
        let constraints = match active.count() {
            0 | 1 => vec![Constraint::Percentage(100)],
            2 => Self::two_panel_constraints(active),
            3 => vec![
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
            ],
            _ => vec![],
        };
        Layout::horizontal(constraints.as_slice())
            .split(area)
            .to_vec()
    }

    /// With two panels open the at bat panel takes two thirds rather than half.
    ///
    /// It is the panel that has to fit something: a strike zone with a fixed aspect
    /// ratio, and beside it the pitch tracker column, which hides itself rather than
    /// squash the zone. An even split at 120 columns leaves 56 usable and the tracker
    /// does not fit; two thirds leaves 76 and it does.
    ///
    /// Panels render left to right as `[info, at_bat, boxscore]`, so which side gets
    /// the larger share depends on what else is open. Two panels that do not include
    /// the at bat panel keep the even split — neither of them needs the room.
    fn two_panel_constraints(active: &GamedayPanels) -> Vec<Constraint> {
        const LARGE: Constraint = Constraint::Ratio(2, 3);
        const SMALL: Constraint = Constraint::Ratio(1, 3);

        match (active.at_bat, active.info) {
            (false, _) => vec![Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)],
            (true, true) => vec![SMALL, LARGE],
            (true, false) => vec![LARGE, SMALL],
        }
    }

    /// Create a centered rectangle of 4 height and 42% width for the date picker.
    pub fn create_date_picker(area: Rect) -> Rect {
        input_popup::create_popup(area, 4, 42)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserves_the_last_row_for_key_hints() {
        let size = Size::new(100, 30);
        let mut areas = LayoutAreas::new(size);
        areas.update(Rect::new(0, 0, 100, 30), false);

        assert_eq!(areas.top_bar, Rect::new(0, 0, 100, 3));
        assert_eq!(areas.main, Rect::new(0, 3, 100, 25));
        assert_eq!(areas.footer, Rect::new(0, 28, 100, 2));
    }

    fn panels(info: bool, at_bat: bool, boxscore: bool) -> GamedayPanels {
        GamedayPanels {
            info,
            at_bat,
            boxscore,
            win_probability: false,
            pitch_tracker: true,
            movement: true,
        }
    }

    #[test]
    fn two_panels_give_the_at_bat_panel_two_thirds() {
        // the default pair: plays on the left, strike zone and tracker on the right
        let areas = LayoutAreas::generate_gameday_panels(
            &panels(true, true, false),
            Rect::new(0, 0, 120, 40),
        );
        assert_eq!(areas[0].width, 40, "info");
        assert_eq!(areas[1].width, 80, "at bat");
    }

    #[test]
    fn the_larger_share_follows_the_at_bat_panel_to_the_left() {
        // panels render [info, at_bat, boxscore]; with info off the at bat panel is
        // the left one, and the two thirds has to move with it
        let areas = LayoutAreas::generate_gameday_panels(
            &panels(false, true, true),
            Rect::new(0, 0, 120, 40),
        );
        assert_eq!(areas[0].width, 80, "at bat");
        assert_eq!(areas[1].width, 40, "boxscore");
    }

    #[test]
    fn two_panels_without_the_at_bat_panel_stay_even() {
        let areas = LayoutAreas::generate_gameday_panels(
            &panels(true, false, true),
            Rect::new(0, 0, 120, 40),
        );
        assert_eq!(areas[0].width, 60);
        assert_eq!(areas[1].width, 60);
    }

    #[test]
    fn two_thirds_is_enough_for_the_zone_and_the_tracker() {
        // the whole point of the change: 60 usable columns was not enough, 76 is
        let areas = LayoutAreas::generate_gameday_panels(
            &panels(true, true, false),
            Rect::new(0, 0, 120, 40),
        );
        let usable = areas[1].width - 4; // the at bat widget's horizontal margin
        assert!(crate::ui::gameday::pitch_tracker::fits_beside_zone(usable));
    }

    #[test]
    fn three_panels_stay_even() {
        let areas = LayoutAreas::generate_gameday_panels(
            &panels(true, true, true),
            Rect::new(0, 0, 120, 40),
        );
        assert_eq!(
            areas.iter().map(|a| a.width).collect::<Vec<_>>(),
            vec![40, 40, 40]
        );
    }

    #[test]
    fn full_screen_hides_tabs_but_keeps_key_hints() {
        let size = Size::new(80, 24);
        let mut areas = LayoutAreas::new(size);
        areas.update(Rect::new(0, 0, 80, 24), true);

        assert_eq!(areas.top_bar.height, 0);
        assert_eq!(areas.main, Rect::new(0, 0, 80, 22));
        assert_eq!(areas.footer, Rect::new(0, 22, 80, 2));
    }
}
