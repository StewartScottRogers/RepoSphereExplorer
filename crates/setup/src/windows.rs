//! The parts of an install that only Windows has: the Start menu shortcut,
//! the Settings > Apps entry, and the processes an uninstall has to stop
//! before it can delete what they are running from.
//!
//! What goes in each of them is [`crate::layout`]'s to say; this module only
//! knows how to write it.

use crate::SetupError;
use crate::layout::{self, Plan};
use std::path::Path;

/// Writes the Start menu shortcut: the graphical application, in the install
/// folder, with the application's own icon and its description.
///
/// The icon is the executable's own first icon resource, so nothing has to
/// be placed beside the shortcut for it. Only the file is named here: the
/// index is a number of its own in the shortcut, and Windows reads the pair
/// back as the `file,0` that `install.ps1` writes.
pub fn write_shortcut(plan: &Plan) -> Result<(), SetupError> {
    let application = plan.application();
    let shortcut = plan.shortcut();
    std::fs::create_dir_all(&plan.start_menu_directory)?;
    let mut link = mslnk::ShellLink::new(&application).map_err(|err| {
        SetupError::Windows(format!(
            "could not describe a shortcut to {}: {err:?}",
            application.display()
        ))
    })?;
    link.set_name(Some(layout::SUMMARY.to_owned()));
    link.set_working_dir(Some(plan.prefix.display().to_string()));
    link.set_icon_location(Some(application.display().to_string()));
    link.create_lnk(&shortcut).map_err(|err| {
        SetupError::Windows(format!(
            "could not write the Start menu shortcut {}: {err:?}",
            shortcut.display()
        ))
    })
}

/// Writes the Settings > Apps entry, under this user's own hive: no
/// administrator rights, and no claim on anyone else's account.
pub fn register(plan: &Plan, kilobytes: u32) -> Result<(), SetupError> {
    let path = registry_path(&plan.registry_key)?;
    let key = windows_registry::CURRENT_USER.create(path).map_err(|err| {
        SetupError::Windows(format!("could not make {}: {err}", plan.registry_key))
    })?;
    // Taken by reference so the closure does not have to name the error type
    // `windows-registry` keeps to itself.
    let failed = |err: &dyn std::fmt::Display| {
        SetupError::Windows(format!("could not write to {}: {err}", plan.registry_key))
    };
    for (name, value) in plan.registry_strings() {
        key.set_string(name, &value).map_err(|err| failed(&err))?;
    }
    key.set_u32("EstimatedSize", kilobytes)
        .map_err(|err| failed(&err))?;
    // Settings offers Modify and Repair buttons unless it is told there are
    // none: this install has neither.
    key.set_u32("NoModify", 1).map_err(|err| failed(&err))?;
    key.set_u32("NoRepair", 1).map_err(|err| failed(&err))?;
    Ok(())
}

/// Removes a registry key named by a receipt, and says whether there was one
/// there to remove.
pub fn remove_registry_key(key: &str) -> Result<bool, SetupError> {
    let path = registry_path(key)?;
    if windows_registry::CURRENT_USER.open(path).is_err() {
        return Ok(false);
    }
    windows_registry::CURRENT_USER
        .remove_tree(path)
        .map_err(|err| SetupError::Windows(format!("could not remove {key}: {err}")))?;
    Ok(true)
}

/// Turns the `HKCU:\...` spelling a receipt and `install.ps1` use into the
/// path below `HKEY_CURRENT_USER` the registry itself wants.
fn registry_path(key: &str) -> Result<&str, SetupError> {
    key.strip_prefix(layout::REGISTRY_DRIVE).ok_or_else(|| {
        SetupError::Usage(format!(
            "a registry key must be under {}, this user's own hive: {key}",
            layout::REGISTRY_DRIVE
        ))
    })
}

/// Stops anything running from the install folder, so uninstall can delete
/// what it was running (#558): somebody removing the application from
/// Settings > Apps usually has it open.
pub fn stop_processes_in(prefix: &Path, report: &mut crate::Report<'_>) {
    let Ok(root) = std::path::absolute(prefix) else {
        return;
    };
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let ours = sysinfo::get_current_pid().ok();
    for (pid, process) in system.processes() {
        if ours == Some(*pid) {
            continue;
        }
        let Some(path) = process.exe() else { continue };
        if path.starts_with(&root) {
            report(format!("stopped {} (process {pid})", path.display()));
            process.kill();
            process.wait();
        }
    }
}
