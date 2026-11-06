pub mod input;
pub mod palette;
pub mod terminal;
pub mod view;

pub use terminal::{SpawnOptions, Terminal};
pub use view::{TerminalEvent, TerminalStyle, TerminalView};
