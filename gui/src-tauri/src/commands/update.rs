//! Updating an app in three steps the user can follow: plan (what will be
//! installed, shown for confirmation), download (cancellable, verified), and
//! install (may ask for a password through polkit).

use std::collections::HashMap;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use forager_core::history::{self, Event, Kind, Via};
use forager_core::install;
use forager_core::model::Format;
use forager_core::platform::Platform;
use forager_core::update::{self, Download, Plan, Progress, SummaryLine};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use super::apps::AppView;
use crate::state::{CmdResult, blocking, load_store};

/// One update in progress per install: app id and format, since an app
/// installed as both an rpm and a Flatpak updates each in its own format.
#[derive(Default)]
pub struct Updates(Mutex<HashMap<String, Session>>);

fn key(id: &str, format: Format) -> String {
    format!("{id}/{format}")
}

struct Session {
    plan: Plan,
    download: Option<Download>,
    cancel: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanView {
    pub from: Option<String>,
    pub to: String,
    pub missing: bool,
    pub format: Format,
    pub lines: Vec<SummaryLine>,
    pub unverified: bool,
    pub needs_root: bool,
    /// The release is older than the install it replaces.
    pub downgrade: bool,
    /// Release skipped once this is installed.
    pub skip_after: Option<String>,
    /// Release notes of the version to install.
    pub notes: Option<String>,
}

/// One planned update of an app, or why it cannot be made.
#[derive(Debug, Clone, Serialize)]
pub struct PlanResult {
    pub plan: Option<PlanView>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadedView {
    pub package: Option<String>,
    /// The distro's repositories carry the same package at this version.
    pub repo_version: Option<String>,
    /// Not installed before, or installed in another format: the distro
    /// overlap deserves a question. Later updates only mention it.
    pub first_install: bool,
}

/// Payload of `update-progress`.
#[derive(Debug, Clone, Serialize)]
pub struct ProgressEvent {
    pub id: String,
    pub phase: &'static str,
    pub done: u64,
    pub total: Option<u64>,
    pub percent: Option<u32>,
}

pub const PROGRESS_EVENT: &str = "update-progress";

/// What updating `id` takes: one plan per install that needs it, none when
/// it is up to date; or, with `tag`, what moving every install to that
/// release takes. Each plan waits for `start_update` with its format.
#[tauri::command]
pub async fn plan_update(updates: State<'_, Updates>, id: String, tag: Option<String>) -> CmdResult<Vec<PlanResult>> {
    let key_id = id.clone();
    let planned = blocking(move || {
        let mut store = load_store()?;
        let app = store.get_mut(&id).map_err(|e| e.to_string())?;
        let recorded = app.install_list();
        let platform = Platform::detect();
        let plans = match &tag {
            Some(tag) => update::plans_for(app, &platform, tag),
            None => update::plans(app, &platform),
        }
        .map_err(|e| e.to_string())?;
        // Installed or changed outside Forager: keep what the system reports.
        if app.install_list() != recorded {
            store.save().map_err(|e| e.to_string())?;
        }
        let app = store.get(&id).map_err(|e| e.to_string())?;
        Ok(plans
            .into_iter()
            .map(|plan| {
                plan.map(|plan| {
                    let view = PlanView {
                        from: plan.from.clone(),
                        to: plan.release.tag.clone(),
                        missing: plan.missing,
                        format: plan.format,
                        lines: update::summary(app, &plan),
                        unverified: plan.unverified(),
                        needs_root: plan.format.needs_root(),
                        downgrade: plan.downgrade,
                        skip_after: plan.skip_after.clone(),
                        notes: plan.release.notes.clone(),
                    };
                    (plan, view)
                })
                .map_err(|e| e.to_string())
            })
            .collect::<Vec<_>>())
    })
    .await?;
    let mut sessions = updates.0.lock().unwrap();
    Ok(planned
        .into_iter()
        .map(|planned| match planned {
            Ok((plan, view)) => {
                sessions.insert(
                    key(&key_id, plan.format),
                    Session {
                        plan,
                        download: None,
                        cancel: Arc::new(AtomicBool::new(false)),
                    },
                );
                PlanResult {
                    plan: Some(view),
                    error: None,
                }
            }
            Err(error) => PlanResult {
                plan: None,
                error: Some(error),
            },
        })
        .collect())
}

/// Downloads and verifies the planned asset, reporting progress, and keeps
/// it for `finish_update`.
#[tauri::command]
pub async fn start_update(
    app: AppHandle,
    updates: State<'_, Updates>,
    id: String,
    format: Format,
) -> CmdResult<DownloadedView> {
    let session_key = key(&id, format);
    let (plan, cancel) = {
        let sessions = updates.0.lock().unwrap();
        let session = sessions.get(&session_key).ok_or("no update planned for this app")?;
        (session.plan.clone(), session.cancel.clone())
    };
    let (handle, key) = (app.clone(), id.clone());
    let result = blocking(move || {
        let first_install = load_store()?
            .get(&key)
            .map(|a| a.install_of(plan.format).is_none())
            .unwrap_or(true);
        let download = update::download(&plan, &Platform::detect(), &mut |p| {
            emit_progress(&handle, &key, p);
            if cancel.load(Ordering::Relaxed) {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
        .map_err(|e| e.to_string())?;
        Ok((download, first_install))
    })
    .await;

    let mut sessions = updates.0.lock().unwrap();
    match result {
        Ok((download, first_install)) => {
            let view = DownloadedView {
                package: download.package.clone(),
                repo_version: download.repo_version.clone(),
                first_install,
            };
            match sessions.get_mut(&session_key) {
                Some(session) => session.download = Some(download),
                // Cancelled while finishing: dropping the download deletes it.
                None => return Err("cancelled".into()),
            }
            Ok(view)
        }
        Err(e) => {
            sessions.remove(&session_key);
            if e != "cancelled" {
                record_failure(&id, format, &e);
            }
            Err(e)
        }
    }
}

/// What `finish_update` did.
#[derive(Debug, Clone, Serialize)]
pub struct FinishedView {
    pub app: AppView,
    /// This was Forager updating itself: the window runs the old version
    /// until restarted.
    pub restart_needed: bool,
}

/// Installs the downloaded asset and records it.
#[tauri::command]
pub async fn finish_update(
    app: AppHandle,
    updates: State<'_, Updates>,
    id: String,
    format: Format,
) -> CmdResult<FinishedView> {
    let session = updates
        .0
        .lock()
        .unwrap()
        .remove(&key(&id, format))
        .ok_or("no update downloaded for this app")?;
    let download = session.download.ok_or("the download has not finished")?;
    let plan = session.plan;
    let (handle, key) = (app.clone(), id.clone());
    let result = blocking(move || {
        let mut store = load_store()?;
        let installed = update::install(&mut store, &key, &plan, &download, &Platform::detect(), &mut |p| {
            emit_progress(&handle, &key, p);
            ControlFlow::Continue(())
        })
        .map_err(|e| e.to_string())?;
        let mut event = Event::new(
            &key,
            if plan.from.is_some() {
                Kind::Updated
            } else {
                Kind::Installed
            },
            Via::Gui,
        );
        event.from = plan.from.clone();
        event.to = Some(plan.release.tag.clone());
        event.format = Some(plan.format);
        history::record(&event).ok();
        let app = store.get(&key).map_err(|e| e.to_string())?;
        Ok(FinishedView {
            app: AppView::from(app),
            restart_needed: install::is_running_self(&installed),
        })
    })
    .await;
    if let Err(e) = &result {
        record_failure(&id, format, e);
    }
    result
}

/// Stops a download in progress, or drops one waiting to be installed.
#[tauri::command]
pub fn cancel_update(updates: State<'_, Updates>, id: String, format: Format) {
    if let Some(session) = updates.0.lock().unwrap().remove(&key(&id, format)) {
        session.cancel.store(true, Ordering::Relaxed);
    }
}

fn emit_progress(app: &AppHandle, id: &str, progress: Progress) {
    let event = match progress {
        Progress::Download { done, total } => ProgressEvent {
            id: id.to_string(),
            phase: "download",
            done,
            total,
            percent: total.filter(|t| *t > 0).map(|t| (done * 100 / t) as u32),
        },
        Progress::Install { percent } => ProgressEvent {
            id: id.to_string(),
            phase: "install",
            done: 0,
            total: None,
            percent: Some(percent),
        },
    };
    app.emit(PROGRESS_EVENT, event).ok();
}

fn record_failure(id: &str, format: Format, message: &str) {
    let mut event = Event::new(id, Kind::Failed, Via::Gui);
    event.message = Some(message.to_string());
    event.format = Some(format);
    history::record(&event).ok();
}
