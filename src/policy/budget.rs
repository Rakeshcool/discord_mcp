//! The rolling mutation budget.
//!
//! Every real mutation consumes one action from a 60-second window shared across
//! tools, and `discord_send_message` additionally consumes a per-channel send
//! budget. When a state file is configured the window is written there after
//! every consumed action, before the Discord request is made, so stopping and
//! starting the process cannot reset the limit. A write failure refuses the
//! action instead of sending it with a budget the next process cannot see.
//!
//! Exactly one running process owns a state file. Ownership is an exclusive lock
//! on a sidecar lock file, held for the life of the gate; a second process with
//! mutations enabled fails at startup instead of racing the first and
//! undercounting. The operating system releases the lock if the process exits or
//! crashes. A deployment that needs concurrent instances gives each one its own
//! `policy.autonomy.budget_state_file`; those windows are then separate limits,
//! not a shared one.

use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::discord::ids::ChannelId;
use crate::error::AppError;

/// The rolling window every budget is measured over, in milliseconds.
const BUDGET_WINDOW_MILLIS: u64 = 60_000;

/// The per-user default for the budget state file.
///
/// `%LOCALAPPDATA%` on Windows, the temporary directory elsewhere. It is a limit
/// ledger, not a secret: it holds timestamps and channel IDs only.
pub(crate) fn default_state_file() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("discord-mcp")
        .join("action-budget.json")
}

/// The serialized window. Timestamps are milliseconds since the Unix epoch.
#[derive(Debug, Serialize, Deserialize)]
struct BudgetState {
    actions: Vec<u64>,
    sends: HashMap<String, Vec<u64>>,
}

/// The open state file and its ownership lock, held together so neither can
/// exist without the other.
#[derive(Debug)]
struct Persistence {
    path: PathBuf,
    _lock: File,
}

/// Recent mutation timestamps, in memory only when no state file is set.
#[derive(Debug)]
pub(crate) struct ActionBudget {
    actions: VecDeque<u64>,
    sends: HashMap<ChannelId, VecDeque<u64>>,
    persistence: Option<Persistence>,
}

impl ActionBudget {
    /// A budget that lives for the process only. Tests use this; the shipping
    /// application uses [`ActionBudget::load`] with a state file.
    pub(crate) fn in_memory() -> Self {
        Self {
            actions: VecDeque::new(),
            sends: HashMap::new(),
            persistence: None,
        }
    }

    /// Load the window from `path`, creating an empty one when it does not exist.
    ///
    /// An unreadable, malformed or partially owned file is a startup error: if
    /// the window cannot be read or owned, starting anyway would silently reset
    /// it or share it with another process.
    pub(crate) fn load(path: PathBuf) -> Result<Self, AppError> {
        let lock = acquire_ownership(&path)?;
        let persistence = Persistence { path, _lock: lock };

        let bytes = match fs::read(&persistence.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    persistence: Some(persistence),
                    ..Self::in_memory()
                });
            }
            Err(error) => {
                return Err(AppError::configuration(format!(
                    "could not read the action budget at {}: {error}; fix the path or delete the file to reset it",
                    persistence.path.display()
                )));
            }
        };

        let state: BudgetState = serde_json::from_slice(&bytes).map_err(|error| {
            AppError::configuration(format!(
                "could not parse the action budget at {}: {error}; delete the file to reset it",
                persistence.path.display()
            ))
        })?;

        let mut budget = Self {
            persistence: Some(persistence),
            ..Self::in_memory()
        };
        budget.actions.extend(state.actions);
        for (raw, times) in state.sends {
            let channel = ChannelId::parse(&raw).map_err(|_| {
                AppError::configuration(format!(
                    "the action budget names `{raw}`, which is not a channel ID; delete the file to reset it"
                ))
            })?;
            budget.sends.entry(channel).or_default().extend(times);
        }
        budget.prune(now_millis());
        Ok(budget)
    }

    /// Consume one action from the shared window.
    pub(crate) fn consume_action(&mut self, budget: u16, action: &str) -> Result<(), AppError> {
        let now = now_millis();
        self.prune(now);
        let budget = usize::from(budget);
        if self.actions.len() >= budget {
            return Err(AppError::policy_blocked(format!(
                "policy.autonomy.max_actions_per_minute ({budget}) is exhausted; `{action}` was not sent"
            )));
        }
        self.actions.push_back(now);
        self.persist()
    }

    /// Consume one action from the shared window and one from `channel`'s send
    /// window. Both limits are checked before either counter moves.
    pub(crate) fn consume_send(
        &mut self,
        action_budget: u16,
        channel_budget: u16,
        channel: ChannelId,
        action: &str,
    ) -> Result<(), AppError> {
        let now = now_millis();
        self.prune(now);

        let action_budget = usize::from(action_budget);
        if self.actions.len() >= action_budget {
            return Err(AppError::policy_blocked(format!(
                "policy.autonomy.max_actions_per_minute ({action_budget}) is exhausted; `{action}` was not sent"
            )));
        }

        let channel_budget = usize::from(channel_budget);
        if self
            .sends
            .get(&channel)
            .is_some_and(|recent| recent.len() >= channel_budget)
        {
            return Err(AppError::policy_blocked(format!(
                "policy.autonomy.max_sends_per_channel_per_minute ({channel_budget}) is exhausted for channel {channel}; `{action}` was not sent"
            )));
        }

        self.actions.push_back(now);
        self.sends.entry(channel).or_default().push_back(now);
        self.persist()
    }

    fn prune(&mut self, now: u64) {
        prune_queue(&mut self.actions, now);
        for recent in self.sends.values_mut() {
            prune_queue(recent, now);
        }
        self.sends.retain(|_, recent| !recent.is_empty());
    }

    fn persist(&self) -> Result<(), AppError> {
        let Some(persistence) = &self.persistence else {
            return Ok(());
        };
        let state = BudgetState {
            actions: self.actions.iter().copied().collect(),
            sends: self
                .sends
                .iter()
                .map(|(channel, times)| (channel.as_string(), times.iter().copied().collect()))
                .collect(),
        };
        let bytes = serde_json::to_vec(&state).map_err(|error| {
            AppError::internal(format!("could not encode the action budget: {error}"))
        })?;

        if let Some(parent) = persistence.path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|error| {
                AppError::internal(format!(
                    "could not create the action budget directory {}: {error}; no action was sent",
                    parent.display()
                ))
            })?;
        }
        let temporary = sidecar(&persistence.path, "tmp");
        fs::write(&temporary, &bytes).map_err(|error| {
            AppError::internal(format!(
                "could not write the action budget at {}: {error}; no action was sent",
                temporary.display()
            ))
        })?;
        fs::rename(&temporary, &persistence.path).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            AppError::internal(format!(
                "could not replace the action budget at {}: {error}; no action was sent",
                persistence.path.display()
            ))
        })
    }
}

/// Take the exclusive ownership lock for `path` without waiting.
///
/// The lock lives in a sidecar file so the state file can still be replaced
/// atomically. It is advisory on Unix and mandatory on Windows; every path into
/// this budget goes through this function, so the distinction does not matter.
fn acquire_ownership(path: &Path) -> Result<File, AppError> {
    let lock_path = sidecar(path, "lock");
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| {
            AppError::configuration(format!(
                "could not open the action budget lock at {}: {error}",
                lock_path.display()
            ))
        })?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(AppError::configuration(format!(
            "the action budget at {} is already owned by another running instance; set policy.autonomy.budget_state_file to a distinct path for each concurrent instance",
            path.display()
        ))),
        Err(TryLockError::Error(error)) => Err(AppError::configuration(format!(
            "could not lock the action budget at {}: {error}",
            path.display()
        ))),
    }
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "action-budget.json".to_string());
    path.with_file_name(format!(".{file_name}.{suffix}"))
}

fn prune_queue(queue: &mut VecDeque<u64>, now: u64) {
    while queue
        .front()
        .is_some_and(|oldest| now.saturating_sub(*oldest) >= BUDGET_WINDOW_MILLIS)
    {
        queue.pop_front();
    }
}

fn now_millis() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}
