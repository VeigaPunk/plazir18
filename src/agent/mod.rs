//! Agent session + tools (feature = "agent").

pub mod session;
pub mod tool_loop;
pub mod tools;

pub use session::{Session, SessionStore};
pub use tool_loop::{
    ToolLoopOutcome, run_tool_loop, run_tool_loop_final, run_tool_rounds,
    stream_after_tools_enabled, tool_loop_enabled_for,
};
pub use tools::{Tool, expand_at_files, run_tool, write_agents_md};
