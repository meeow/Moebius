//! Automations: recurring work that runs on its own. An automation is a name, a schedule (a cron expression in the
//! server's local time, see `cron`), an agent, a prompt, where its chats work (a repository, the harness configuration
//! repository or an empty directory) and whether it is on.
//!
//! The server's schedule looks at the clock once a minute ([`on_schedule`]): each automation that is on and came due
//! since it last looked starts a chat with its agent and prompt, marked as started by the automation (`origin`
//! `automation`). A firing due while the chat of the one before still runs is recorded as skipped. A run never waits
//! for a person: once an automation's chat has answered and waits for a message, the schedule ends it, so that what it
//! changed is offered as a change and lands in the inbox, as when a person ends a chat. A firing that came due while
//! the server was not running is not made up. Each firing is recorded, with its chat, and published as
//! `automation_fired`.

pub mod cron;

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use time::{Duration, PrimitiveDateTime};

use super::{App, AppError, Result};
use crate::blobs::redact;
use crate::board::Runs;
use crate::bus::Event;
use crate::store::{Automation, Firing, NewAutomation, Run};

/// The origin of a chat an automation started (see `Run::origin`).
pub const AUTOMATION_ORIGIN: &str = "automation";

/// How many firings of an automation are listed.
const FIRINGS_LISTED: i64 = 50;

/// How far back the schedule looks for a firing it missed, as when a check came late; anything older is not made up.
const LOOK_BACK: Duration = Duration::minutes(5);

/// An automation as the API shows it: as stored, with the next time it fires and its latest firing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationView {
    #[serde(flatten)]
    pub automation: Automation,
    /// When it fires next, as Unix seconds; none while it is off or when its schedule never comes due.
    pub next_firing: Option<i64>,
    pub last_firing: Option<Firing>,
}

/// Whether the chat `run` of an automation still runs: it has not ended and has not answered and waited since.
fn still_runs(app: &App, run: &Run) -> Result<bool> {
    if run.ended_at.is_some() {
        return Ok(false);
    }
    Ok(!(run.status == "waiting" && app.store.answered_seq(run.id)? > 0))
}

impl App {
    /// Checks what an automation is made of: a name, a schedule that parses, an agent `runs` has, a prompt, and one
    /// place at most, which exists.
    fn check_automation(&self, runs: &dyn Runs, new: &NewAutomation) -> Result<NewAutomation> {
        let new = NewAutomation {
            name: new.name.trim().to_string(),
            schedule: new
                .schedule
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
            agent: new.agent.trim().to_string(),
            prompt: redact(new.prompt.trim()),
            ..new.clone()
        };
        if new.name.is_empty() {
            return Err(AppError::Invalid("an automation needs a name".into()));
        }
        cron::parse(&new.schedule).map_err(AppError::Invalid)?;
        if !runs.has_agent(&new.agent) {
            return Err(AppError::Invalid(format!("no agent named {}", new.agent)));
        }
        if new.prompt.is_empty() {
            return Err(AppError::Invalid("an automation needs a prompt".into()));
        }
        if new.harness && new.repository.is_some() {
            return Err(AppError::Invalid(
                "an automation works in a repository or in the harness configuration, not both"
                    .into(),
            ));
        }
        if let Some(id) = new.repository {
            self.repository(id)?;
        }
        if new.harness {
            self.harness_repo()?;
        }
        Ok(new)
    }

    /// A person adds an automation, or replaces the automation `id`.
    pub fn save_automation(
        &self,
        runs: &dyn Runs,
        id: Option<i64>,
        new: &NewAutomation,
    ) -> Result<AutomationView> {
        let new = self.check_automation(runs, new)?;
        let id = match id {
            Some(id) => {
                if !self.store.update_automation(id, &new)? {
                    return Err(AppError::NotFound(format!("automation {id}")));
                }
                id
            }
            None => {
                self.store
                    .add_automation(&new, &self.store.local_minute()?)?
                    .id
            }
        };
        self.automation(id)
    }

    /// Deletes the automation `id` and its firings; the chats it started stay.
    pub fn delete_automation(&self, id: i64) -> Result<()> {
        if !self.store.delete_automation(id)? {
            return Err(AppError::NotFound(format!("automation {id}")));
        }
        Ok(())
    }

    pub fn automation(&self, id: i64) -> Result<AutomationView> {
        let automation = self
            .store
            .automation(id)?
            .ok_or_else(|| AppError::NotFound(format!("automation {id}")))?;
        self.automation_view(automation)
    }

    /// The automations, by name.
    pub fn automations(&self) -> Result<Vec<AutomationView>> {
        self.store
            .automations()?
            .into_iter()
            .map(|automation| self.automation_view(automation))
            .collect()
    }

    fn automation_view(&self, automation: Automation) -> Result<AutomationView> {
        let now = cron::parse_minute(&self.store.local_minute()?);
        let next = match (automation.enabled, cron::parse(&automation.schedule), now) {
            (true, Ok(schedule), Some(now)) => schedule.next_after(now),
            _ => None,
        };
        let next_firing = match next {
            Some(next) => self
                .store
                .local_minute_to_unix(&cron::format_minute(next))?,
            None => None,
        };
        let last_firing = self.store.firings(automation.id, 1)?.pop();
        Ok(AutomationView {
            automation,
            next_firing,
            last_firing,
        })
    }

    /// The firings of the automation `id`, newest first, each with its chat.
    pub fn firings(&self, id: i64) -> Result<Vec<Firing>> {
        self.automation(id)?;
        Ok(self.store.firings(id, FIRINGS_LISTED)?)
    }

    /// A person fires the automation `id` now ("Run now"), whether it is on or not; skipped, as a firing of the
    /// schedule would be, while the chat of the one before still runs.
    pub fn run_automation(self: &Arc<Self>, runs: &dyn Runs, id: i64) -> Result<Firing> {
        let automation = self
            .store
            .automation(id)?
            .ok_or_else(|| AppError::NotFound(format!("automation {id}")))?;
        fire(self, runs, &automation, true)
    }
}

/// Fires `automation`: starts its chat, or records why not.
fn fire(app: &Arc<App>, runs: &dyn Runs, automation: &Automation, manual: bool) -> Result<Firing> {
    let before = app.store.last_started_firing(automation.id)?;
    let running = match before.as_ref().and_then(|f| f.run_id) {
        Some(run_id) => app
            .store
            .run(run_id)?
            .map(|run| still_runs(app, &run))
            .transpose()?
            .unwrap_or(false),
        None => false,
    };
    let firing = if running {
        let chat = before.and_then(|f| f.run_id).unwrap_or_default();
        let reason = format!("the chat of the firing before ({chat}) still runs");
        app.store
            .add_firing(automation.id, "skipped", None, manual, &reason)?
    } else {
        let started = app.start_chat(
            runs,
            Some(&automation.agent),
            automation.repository_id,
            automation.harness_repository,
            Some(&automation.prompt),
            &[],
            None,
            None,
            Some(AUTOMATION_ORIGIN),
            (None, None, &[]),
        );
        match started {
            Ok(run) => app
                .store
                .add_firing(automation.id, "started", Some(run.id), manual, "")?,
            Err(e) => {
                let reason = redact(&e.to_string());
                app.store
                    .add_firing(automation.id, "failed", None, manual, &reason)?
            }
        }
    };
    app.bus.publish(Event::AutomationFired {
        automation: automation.id,
        firing: firing.id,
        status: firing.status.clone(),
        run: firing.run_id,
    });
    Ok(firing)
}

/// The schedule's check, once a minute, at the local minute `now`: ends the chats of automations that have answered,
/// then fires each automation that is on and came due since the schedule last looked (at most [`LOOK_BACK`] ago).
pub fn on_schedule(app: &Arc<App>, runs: &dyn Runs, now: PrimitiveDateTime) -> Result<()> {
    for run_id in app.store.waiting_automation_chats()? {
        // A chat that ended meanwhile cannot be ended again; it is left as it is.
        if app.store.answered_seq(run_id)? > 0 {
            let _ = app.end_chat(runs, run_id);
        }
    }
    let earliest = now - LOOK_BACK;
    for automation in app.store.automations()? {
        let Ok(schedule) = cron::parse(&automation.schedule) else {
            continue;
        };
        if !automation.enabled {
            continue;
        }
        let checked = app
            .store
            .automation_checked(automation.id)?
            .as_deref()
            .and_then(cron::parse_minute)
            .map_or(now - Duration::minutes(1), |checked| checked.max(earliest));
        if schedule.next_after(checked).is_some_and(|next| next <= now) {
            fire(app, runs, &automation, false)?;
        }
    }
    app.store
        .set_automations_checked(&cron::format_minute(now))?;
    Ok(())
}
