//! UI components, following ratatui's component architecture:
//! <https://ratatui.rs/concepts/application-patterns/component-architecture/>

mod browser;
mod confirm;
mod filter;
mod hand_in;
mod help;
mod report;
mod settings;
mod status_bar;

pub use browser::Browser;
pub use confirm::Confirm;
pub use filter::Filter;
pub use hand_in::HandIn;
pub use help::Help;
pub use report::Report;
pub use settings::Settings;
pub use status_bar::StatusBar;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyEvent, MouseEvent};
use ratatui::layout::Rect;

use crate::action::Action;

pub trait Component {
    /// Handles a key press. Returning `Some` marks the key as handled, so components
    /// further down in the z-order don't see it.
    fn handle_key_event(&mut self, _key: KeyEvent) -> Option<Action> {
        None
    }

    /// Handles a mouse event; same routing as keys (top-most component first).
    fn handle_mouse_event(&mut self, _mouse: MouseEvent) -> Option<Action> {
        None
    }

    /// Reacts to an action; may return a follow-up action.
    fn update(&mut self, _action: &Action) -> Option<Action> {
        None
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect);
}
