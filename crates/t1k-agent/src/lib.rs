//! Claude Code integration: hook protocol, socket server, transcript tailing, per-tab agent state.
//!
//! Hooks are the source of truth for status. The transcript adds what hooks don't carry:
//! Claude's own title (`ai-title`), recaps (`away_summary`), PR links and API errors.

pub mod agents;
pub mod helper;
pub mod hooks;
pub mod outputs;
pub mod server;
pub mod state;
pub mod statusline;
pub mod transcript;

pub use state::{AgentState, Status};
