pub mod glyphs;
pub mod input;
pub mod links;
pub mod palette;
pub mod process;
pub mod search;
pub mod terminal;
pub mod view;

pub use terminal::{SpawnOptions, Terminal};
pub use view::{TerminalEvent, TerminalStyle, TerminalView};
pub use alacritty_terminal::vte::ansi::Rgb;
