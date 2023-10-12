use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use gtk::{
    gio::{AppInfo, Icon, ThemedIcon},
    prelude::AppInfoExt,
};
use hashbrown::{HashMap, HashSet};
use once_cell::sync::Lazy;

use crate::i18n::i18n;

use super::processes::{Containerization, Process, ProcessAction, ProcessItem};

// Adapted from Mission Center: https://gitlab.com/mission-center-devs/mission-center/
static DATA_DIRS: Lazy<Vec<PathBuf>> = Lazy::new(|| {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
    let mut data_dirs: Vec<PathBuf> = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| format!("/usr/share:{}/.local/share", home))
        .split(':')
        .map(PathBuf::from)
        .collect();
    data_dirs.push(PathBuf::from(format!("{}/.local/share", home)));
    data_dirs
});

#[derive(Debug, Clone, Default)]
pub struct AppsContext {
    apps: HashMap<String, App>,
    processes: HashMap<i32, Process>,
    processes_assigned_to_apps: HashSet<i32>,
}

/// Convenience struct for displaying running applications and
/// displaying a "System Processes" item.
#[derive(Debug, Clone)]
pub struct AppItem {
    pub id: Option<String>,
    pub display_name: String,
    pub icon: Icon,
    pub description: Option<String>,
    pub memory_usage: usize,
    pub cpu_time_ratio: f32,
    pub processes_amount: usize,
    pub containerization: Containerization,
}

/// Represents an application installed on the system. It doesn't
/// have to be running (i.e. have alive processes).
#[derive(Debug, Clone)]
pub struct App {
    processes: Vec<i32>,
    pub display_name: String,
    pub description: Option<String>,
    pub icon: Icon,
    pub id: String,
}

impl App {
    fn sanitize_appid<S: Into<String>>(a: S) -> String {
        let mut appid: String = a.into();
        if appid.ends_with(".desktop") {
            appid = appid[0..appid.len() - 8].to_string();
        }
        appid
    }

    pub fn all() -> Vec<App> {
        DATA_DIRS
            .iter()
            .flat_map(|path| {
                let applications_path = path.join("applications");
                let expanded_path = expanduser::expanduser(applications_path.to_string_lossy())
                    .unwrap_or(applications_path);
                expanded_path.read_dir().ok().map(|read| {
                    read.filter_map(|file_res| {
                        file_res
                            .ok()
                            .and_then(|file| Self::from_desktop_file(file.path()).ok())
                    })
                })
            })
            .flatten()
            .collect()
    }

    pub fn from_desktop_file<P: AsRef<Path>>(file_path: P) -> Result<App> {
        let ini = ini::Ini::load_from_file(file_path.as_ref())?;
        let desktop_entry = ini
            .section(Some("Desktop Entry"))
            .context("no desktop entry section")?;
        let id = file_path
            .as_ref()
            .file_stem()
            .context("desktop file has no file stem")?
            .to_string_lossy();
        Ok(App {
            processes: Vec::new(),
            display_name: desktop_entry.get("Name").unwrap_or(&id).to_string(),
            description: desktop_entry.get("Comment").map(|s| s.to_string()),
            icon: ThemedIcon::new(desktop_entry.get("Icon").unwrap_or("generic-process")).into(),
            id: id.to_string(),
        })
    }

    pub fn from_app_info(app_info: &AppInfo) -> Result<App> {
        if let Some(id) = app_info
            .id()
            .map(|gstring| Self::sanitize_appid(gstring.to_string()))
        {
            Ok(App {
                processes: Vec::new(),
                display_name: app_info.display_name().to_string(),
                description: app_info.description().map(|gs| gs.to_string()),
                id,
                icon: app_info
                    .icon()
                    .unwrap_or_else(|| ThemedIcon::new("generic-process").into()),
            })
        } else {
            bail!("AppInfo has no id")
        }
    }

    pub fn refresh(&mut self, apps: &mut AppsContext) {
        self.processes = self
            .processes_iter_mut(apps)
            .filter_map(|p| if p.alive { Some(p.data.pid) } else { None })
            .collect();
    }

    /// Adds a process to the processes `HashMap` and also
    /// updates the `Process`' icon to the one of this
    /// `App`
    pub fn add_process(&mut self, process: &mut Process) {
        process.icon = self.icon.clone();
        self.processes.push(process.data.pid);
    }

    pub fn remove_process(&mut self, process: &Process) {
        self.processes.retain(|p| *p != process.data.pid);
    }

    #[must_use]
    pub fn is_running(&self, apps: &AppsContext) -> bool {
        self.processes_iter(apps).count() > 0
    }

    pub fn processes_iter<'a>(&'a self, apps: &'a AppsContext) -> impl Iterator<Item = &Process> {
        apps.all_processes()
            .filter(move |process| self.processes.contains(&process.data.pid) && process.alive)
    }

    pub fn processes_iter_mut<'a>(
        &'a mut self,
        apps: &'a mut AppsContext,
    ) -> impl Iterator<Item = &mut Process> {
        apps.all_processes_mut()
            .filter(move |process| self.processes.contains(&process.data.pid) && process.alive)
    }

    #[must_use]
    pub fn memory_usage(&self, apps: &AppsContext) -> usize {
        self.processes_iter(apps)
            .map(|process| process.data.memory_usage)
            .sum()
    }

    #[must_use]
    pub fn cpu_time(&self, apps: &AppsContext) -> u64 {
        self.processes_iter(apps)
            .map(|process| process.data.cpu_time)
            .sum()
    }

    #[must_use]
    pub fn cpu_time_timestamp(&self, apps: &AppsContext) -> u64 {
        self.processes_iter(apps)
            .map(|process| process.data.cpu_time_timestamp)
            .sum::<u64>()
            .checked_div(self.processes.len() as u64) // the timestamps of the last cpu time check should be pretty much equal but to be sure, take the average of all of them
            .unwrap_or(0)
    }

    #[must_use]
    pub fn cpu_time_before(&self, apps: &AppsContext) -> u64 {
        self.processes_iter(apps)
            .map(|process| process.cpu_time_before)
            .sum()
    }

    #[must_use]
    pub fn cpu_time_before_timestamp(&self, apps: &AppsContext) -> u64 {
        self.processes_iter(apps)
            .map(|process| process.cpu_time_before_timestamp)
            .sum::<u64>()
            .checked_div(self.processes.len() as u64)
            .unwrap_or(0)
    }

    #[must_use]
    pub fn cpu_time_ratio(&self, apps: &AppsContext) -> f32 {
        self.processes_iter(apps)
            .map(Process::cpu_time_ratio)
            .sum::<f32>()
            .clamp(0.0, 1.0)
    }

    pub fn execute_process_action(
        &self,
        apps: &AppsContext,
        action: ProcessAction,
    ) -> Vec<Result<()>> {
        self.processes_iter(apps)
            .map(|process| process.execute_process_action(action))
            .collect()
    }
}

impl AppsContext {
    /// Creates a new `Apps` object, this operation is quite expensive
    /// so try to do it only one time during the lifetime of the program.
    ///
    /// # Errors
    ///
    /// Will return `Err` if there are problems getting the list of
    /// running processes.
    pub async fn new() -> Result<AppsContext> {
        let mut apps: HashMap<String, App> = App::all()
            .into_iter()
            .map(|app| (app.id.clone(), app))
            .collect();

        let mut processes = HashMap::new();
        let processes_list = Process::all().await?;
        let mut processes_assigned_to_apps = HashSet::new();

        for mut process in processes_list {
            if let Some(app) = apps.get_mut(process.data.cgroup.as_deref().unwrap_or_default()) {
                processes_assigned_to_apps.insert(process.data.pid);
                app.add_process(&mut process);
            }
            processes.insert(process.data.pid, process);
        }

        Ok(AppsContext {
            apps,
            processes,
            processes_assigned_to_apps,
        })
    }

    pub fn get_process(&self, pid: i32) -> Option<&Process> {
        self.processes.get(&pid)
    }

    pub fn get_app(&self, id: &str) -> Option<&App> {
        self.apps.get(id)
    }

    #[must_use]
    pub fn all_processes(&self) -> impl Iterator<Item = &Process> {
        self.processes.values().filter(|p| p.alive)
    }

    #[must_use]
    pub fn all_processes_mut(&mut self) -> impl Iterator<Item = &mut Process> {
        self.processes.values_mut().filter(|p| p.alive)
    }

    /// Returns a `HashMap` of running processes. For more info, refer to
    /// `ProcessItem`.
    pub fn process_items(&self) -> HashMap<i32, ProcessItem> {
        self.all_processes()
            .filter(|process| !process.data.commandline.is_empty()) // find a way to display procs without commandlines
            .map(|process| {
                (
                    process.data.pid,
                    self.process_item(process.data.pid).unwrap(),
                )
            })
            .collect()
    }

    pub fn process_item(&self, pid: i32) -> Option<ProcessItem> {
        self.get_process(pid).map(|process| ProcessItem {
            pid: process.data.pid,
            display_name: process.data.comm.clone(),
            icon: process.icon.clone(),
            memory_usage: process.data.memory_usage,
            cpu_time_ratio: process.cpu_time_ratio(),
            commandline: Process::sanitize_cmdline(process.data.commandline.clone()),
            containerization: process.data.containerization.clone(),
            cgroup: process.data.cgroup.clone(),
            uid: process.data.uid,
        })
    }

    /// Returns a `HashMap` of running graphical applications. For more info,
    /// refer to `AppItem`.
    #[must_use]
    pub fn app_items(&self) -> HashMap<Option<String>, AppItem> {
        let mut app_pids = HashSet::new();

        let mut return_map = self
            .apps
            .iter()
            .filter(|(_, app)| app.is_running(self) && !app.id.starts_with("xdg-desktop-portal"))
            .map(|(_, app)| {
                app.processes_iter(self).for_each(|process| {
                    app_pids.insert(process.data.pid);
                });

                let containerization = if app
                    .processes_iter(self)
                    .filter(|process| {
                        !process.data.commandline.starts_with("bwrap")
                            && !process.data.commandline.is_empty()
                    })
                    .any(|process| process.data.containerization == Containerization::Flatpak)
                {
                    Containerization::Flatpak
                } else {
                    Containerization::None
                };

                (
                    Some(app.id.clone()),
                    AppItem {
                        id: Some(app.id.clone()),
                        display_name: app.display_name.clone(),
                        icon: app.icon.clone(),
                        description: app.description.clone(),
                        memory_usage: app.memory_usage(self),
                        cpu_time_ratio: app.cpu_time_ratio(self),
                        processes_amount: app.processes_iter(self).count(),
                        containerization,
                    },
                )
            })
            .collect::<HashMap<Option<String>, AppItem>>();

        let system_cpu_ratio = self
            .all_processes()
            .filter(|process| !app_pids.contains(&process.data.pid) && process.alive)
            .map(Process::cpu_time_ratio)
            .sum();

        let system_memory_usage: usize = self
            .all_processes()
            .filter(|process| !app_pids.contains(&process.data.pid) && process.alive)
            .map(|process| process.data.memory_usage)
            .sum();

        return_map.insert(
            None,
            AppItem {
                id: None,
                display_name: i18n("System Processes"),
                icon: ThemedIcon::new("system-processes").into(),
                description: None,
                memory_usage: system_memory_usage,
                cpu_time_ratio: system_cpu_ratio,
                processes_amount: self.processes.len(),
                containerization: Containerization::None,
            },
        );
        return_map
    }

    /// Refreshes the statistics about the running applications and processes.
    ///
    /// # Errors
    ///
    /// Will return `Err` if there are problems getting the new list of
    /// running processes or if there are anomalies in a process procfs
    /// directory.
    pub async fn refresh(&mut self) -> Result<()> {
        let newly_gathered_processes = Process::all().await?;
        let mut updated_processes = HashSet::new();

        for mut refreshed_process in newly_gathered_processes {
            updated_processes.insert(refreshed_process.data.pid);
            // refresh our old processes
            if let Some(old_process) = self.processes.get_mut(&refreshed_process.data.pid) {
                old_process.cpu_time_before = old_process.data.cpu_time;
                old_process.cpu_time_before_timestamp = old_process.data.cpu_time_timestamp;
                old_process.data = refreshed_process.data.clone();
            } else {
                // this is a new process, see if it belongs to a graphical app

                if self
                    .processes_assigned_to_apps
                    .contains(&refreshed_process.data.pid)
                {
                    continue;
                }

                if let Some(app) = self
                    .apps
                    .get_mut(refreshed_process.data.cgroup.as_deref().unwrap_or_default())
                {
                    self.processes_assigned_to_apps
                        .insert(refreshed_process.data.pid);
                    app.add_process(&mut refreshed_process);
                }

                self.processes
                    .insert(refreshed_process.data.pid, refreshed_process);
            }
        }

        // all the not-updated processes have unfortunately died, probably
        for process in self.processes.values_mut() {
            if !updated_processes.contains(&process.data.pid) {
                process.alive = false;
            }
        }

        Ok(())
    }
}
