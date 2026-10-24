//! Updating an app in three steps the user can follow: plan (what will be
//! installed, shown for confirmation), download (cancellable, verified), and
//! install (may ask for a password through polkit).

use std::collections::HashMap;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use forager_core::history::{self, Event, Kind, Via};
use forager_core::model::Format;
use forager_core::platform::Platform;
use forager_core::update::{self, Download, Plan, Progress, SummaryLine};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use super::apps::AppView;
use crate::state::{CmdResult, blocking, load_store};

/// One update in progress per app id.
#[derive(Default)]
pub struct Updates(Mutex<HashMap<String, Session>>);

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

#[tauri::command]
pub async fn plan_update(updates: State<'_, Updates>, id: String) -> CmdResult<Option<PlanView>> {
    let key = id.clone();
    let planned = blocking(move || {
        let store = load_store()?;
        let app = store.get(&id).map_err(|e| e.to_string())?;
        let Some(plan) = update::plan(app, &Platform::detect()).map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        let view = PlanView {
            from: plan.from.clone(),
            to: plan.release.tag.clone(),
            missing: plan.missing,
            format: plan.format,
            lines: update::summary(app, &plan),
            unverified: plan.unverified(),
            needs_root: plan.format.needs_root(),
        };
        Ok(Some((plan, view)))
    })
    .await?;
    Ok(planned.map(|(plan, view)| {
        updates.0.lock().unwrap().insert(
            key,
            Session {
                plan,
                download: None,
                cancel: Arc::new(AtomicBool::new(false)),
            },
        );
        view
    }))
}

/// Downloads and verifies the planned asset, reporting progress, and keeps
/// it for `finish_update`.
#[tauri::command]
pub async fn start_update(app: AppHandle, updates: State<'_, Updates>, id: String) -> CmdResult<DownloadedView> {
    let (plan, cancel) = {
        let sessions = updates.0.lock().unwrap();
        let session = sessions.get(&id).ok_or("no update planned for this app")?;
        (session.plan.clone(), session.cancel.clone())
    };
    let (handle, key) = (app.clone(), id.clone());
    let result = blocking(move || {
        let first_install = load_store()?
            .get(&key)
            .map(|a| a.installed.as_ref().is_none_or(|i| i.format != plan.format))
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
            match sessions.get_mut(&id) {
                Some(session) => session.download = Some(download),
                // Cancelled while finishing: dropping the download deletes it.
                None => return Err("cancelled".into()),
            }
            Ok(view)
        }
        Err(e) => {
            sessions.remove(&id);
            if e != "cancelled" {
                record_failure(&id, &e);
            }
            Err(e)
        }
    }
}

/// Installs the downloaded asset and records it.
#[tauri::command]
pub async fn finish_update(app: AppHandle, updates: State<'_, Updates>, id: String) -> CmdResult<AppView> {
    let session = updates
        .0
        .lock()
        .unwrap()
        .remove(&id)
        .ok_or("no update downloaded for this app")?;
    let download = session.download.ok_or("the download has not finished")?;
    let plan = session.plan;
    let (handle, key) = (app.clone(), id.clone());
    let result = blocking(move || {
        let mut store = load_store()?;
        update::install(&mut store, &key, &plan, &download, &Platform::detect(), &mut |p| {
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
        Ok(AppView::from(app))
    })
    .await;
    if let Err(e) = &result {
        record_failure(&id, e);
    }
    result
}

/// Stops a download in progress, or drops one waiting to be installed.
#[tauri::command]
pub fn cancel_update(updates: State<'_, Updates>, id: String) {
    if let Some(session) = updates.0.lock().unwrap().remove(&id) {
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

fn record_failure(id: &str, message: &str) {
    let mut event = Event::new(id, Kind::Failed, Via::Gui);
    event.message = Some(message.to_string());
    history::record(&event).ok();
}
