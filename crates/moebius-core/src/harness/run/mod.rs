//! The life of a run, the same for every harness: the prompts a run of a card is told (`prompts`), the agents Moebius
//! brings itself and the agent a run uses (`agents`), making the run, its worktree and its token and undoing them when
//! its process cannot be started (`lifecycle`), starting a run on a harness's `launch` (`start`), and ending it when
//! its process is gone (`end`). Pi-Bolt (`pibolt`) and Claude Code (`claude`) keep only how they spawn their process
//! (`process`) and how they read its output into run events (`reader`).

mod agents;
mod end;
mod lifecycle;
mod prompts;
mod start;
// Unix only: the fake `pi-bolt` and the fake `claude` are Python scripts started by their `#!` lines.
#[cfg(all(test, unix))]
pub(in crate::harness) mod testing;
#[cfg(all(test, unix))]
mod tests;

pub use agents::{
    AUTORESEARCHER_AGENT, BuiltInAgent, GRILLING_AGENT, MOEBIUS_AGENT, RETRO_AGENT, built_in_agent,
};
pub(crate) use agents::{
    agent_list, chat_agent, has_agent, resolve as resolve_agent, tell_where_moebius_runs,
};
pub(crate) use end::final_answer;
pub(in crate::harness) use end::{finish, keep};
pub(in crate::harness) use lifecycle::{RunningRun, chat_run, launch_or_undo, woken_chat};
pub(in crate::harness) use start::{Launching, start_card, start_detached, start_trial};
