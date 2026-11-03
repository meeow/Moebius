//! Starting a run, the same for every harness: the run, its worktree and its token are made here, and the harness is
//! asked only to start its process in them (the `launch` it passes in).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use super::lifecycle::{RunningRun, card_run, detached_run, launch_or_undo};
use super::prompts::{prompt_for, run_prompt};
use crate::app::{App, ChatImage};
use crate::board::{StartError, Variant};
use crate::config::AgentConfig;
use crate::harness::remote::Runners;
use crate::repos::Worktree;
use crate::router::Router;
use crate::store::{Card, Repository, Run};

/// What a harness is asked to start the process of: the run, with the agent as the run has it, its card if it has one,
/// the prompt it is given, and the images attached to a chat's first message.
pub(in crate::harness) struct Launching<'a> {
    pub(in crate::harness) config: &'a AgentConfig,
    pub(in crate::harness) run: RunningRun,
    pub(in crate::harness) card: Option<&'a Card>,
    pub(in crate::harness) prompt: Option<&'a str>,
    pub(in crate::harness) images: &'a [ChatImage],
}

/// How a harness starts the process of a run.
pub(in crate::harness) type Launch<'a> = &'a dyn Fn(Launching) -> Result<(), String>;

/// Starts a run of `card` by `agent`, as `config` has it with the model and runner `variant` names in place of its own,
/// for `purpose`, on a worktree from the card's latest code branch (or the repository's base) and the prompt of the card.
pub(in crate::harness) fn start_card(
    app: &Arc<App>,
    (router, runners): (&Router, &Arc<Runners>),
    card: &Card,
    (agent, mut config): (&str, AgentConfig),
    (purpose, note): (&str, Option<&str>),
    variant: Variant,
    launch: Launch,
) -> Result<Run, StartError> {
    if variant.runner.is_some() {
        config.runner = variant.runner.map(String::from);
    }
    if let Some(model) = variant.model {
        config.model = model.into();
    }
    let prompt = run_prompt(app, card, purpose, note);
    let code_branch = app
        .store
        .code_branch(card.id)
        .map_err(|e| StartError::Store(e.to_string()))?;
    let worktree = |repository: &Repository, clone: &Path, run_id: i64| {
        let base = code_branch.unwrap_or_else(|| repository.base_ref());
        app.repos
            .add_worktree(&repository.name, clone, run_id, &base)
    };
    run_card(
        app,
        (router, runners),
        card,
        (agent, &config),
        (purpose, &prompt),
        worktree,
        launch,
    )
}

/// Starts a trial run of `card` by `agent`, as `definition` has it, on a branch of its own named after `proposal` and
/// the card (see `Runs::start_trial`).
pub(in crate::harness) fn start_trial(
    app: &Arc<App>,
    (router, runners): (&Router, &Arc<Runners>),
    card: &Card,
    (agent, definition): (&str, &AgentConfig),
    (proposal, base): (i64, &str),
    launch: Launch,
) -> Result<Run, StartError> {
    let prompt = prompt_for(app, card, "code", None);
    let worktree = |repository: &Repository, clone: &Path, _run_id: i64| {
        let name = &repository.name;
        app.repos
            .add_trial_worktree(name, clone, proposal, card.id, base)
    };
    run_card(
        app,
        (router, runners),
        card,
        (agent, definition),
        ("trial", &prompt),
        worktree,
        launch,
    )
}

fn run_card(
    app: &Arc<App>,
    routing: (&Router, &Arc<Runners>),
    card: &Card,
    (agent, config): (&str, &AgentConfig),
    (purpose, prompt): (&str, &str),
    worktree: impl FnOnce(&Repository, &Path, i64) -> crate::repos::Result<Worktree>,
    launch: Launch,
) -> Result<Run, StartError> {
    let running = card_run(app, routing, card, (agent, config), purpose, worktree)?;
    launch_or_undo(app, routing.0, running, |run| {
        launch(Launching {
            config,
            run,
            card: Some(card),
            prompt: Some(prompt),
            images: &[],
        })
    })
}

/// Starts `agent` for `purpose`, with no card, on `prompt` in a fresh detached worktree of the clone `place` names (see
/// [`detached_run`]), checked out at `base`, with `files` written into it.
pub(in crate::harness) fn start_detached(
    app: &Arc<App>,
    router: &Router,
    (agent, purpose): (&str, &str),
    place: (&str, &Path, Option<i64>),
    (base, prompt, files): (&str, &str, &BTreeMap<String, String>),
    config: &AgentConfig,
    launch: Launch,
) -> Result<Run, StartError> {
    let running = detached_run(app, router, (agent, purpose), place, (base, files), config)?;
    launch_or_undo(app, router, running, |run| {
        launch(Launching {
            config,
            run,
            card: None,
            prompt: Some(prompt),
            images: &[],
        })
    })
}
