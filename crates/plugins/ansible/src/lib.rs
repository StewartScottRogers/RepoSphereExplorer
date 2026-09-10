//! Ansible playbook file type plugin: core and presentation halves.
//!
//! A playbook is a sequence of plays, each naming what it runs against
//! and what it does there. This reads the plays and their hosts, every
//! task with the module it calls, the roles, the handlers, the variable
//! names, and whether privilege escalation is asked for.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &[];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One task in a play.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// Its `name`, when it has one.
    pub name: Option<String>,
    /// The module it calls.
    pub module: String,
    /// Whether a `when` decides if it runs.
    pub conditional: bool,
    /// Whether it runs once per item of a loop.
    pub looped: bool,
    /// The handler it notifies, when it notifies one.
    pub notifies: Option<String>,
}

/// One play in the playbook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Play {
    /// Its `name`, when it has one.
    pub name: Option<String>,
    /// The host pattern it runs against.
    pub hosts: String,
    /// Whether it asks for privilege escalation.
    pub escalates: bool,
    /// Its tasks, `pre_tasks` and `post_tasks` included.
    pub tasks: Vec<Task>,
    /// The roles it includes.
    pub roles: Vec<String>,
    /// The handlers it declares, by name.
    pub handlers: Vec<String>,
    /// The variable names it sets. Names only: a value can be a secret.
    pub variables: Vec<String>,
}

/// View data produced by [`AnsibleCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnsibleView {
    /// Every play, in order.
    pub plays: Vec<Play>,
    /// Every distinct module the playbook calls.
    pub modules: Vec<String>,
    /// Variables whose value reaches into the vault.
    pub vault_references: Vec<String>,
    /// Tasks with no name, which report as their module and so are hard
    /// to find in a failing run.
    pub tasks_without_a_name: Vec<String>,
    /// Handlers no task notifies, which therefore never fire.
    pub handlers_never_notified: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The keys that belong to a task itself rather than naming its module.
const TASK_KEYWORDS: &[&str] = &[
    "name",
    "when",
    "loop",
    "with_items",
    "with_dict",
    "with_fileglob",
    "notify",
    "register",
    "tags",
    "become",
    "become_user",
    "become_method",
    "vars",
    "args",
    "ignore_errors",
    "changed_when",
    "failed_when",
    "delegate_to",
    "until",
    "retries",
    "delay",
    "no_log",
    "environment",
    "run_once",
    "check_mode",
    "any_errors_fatal",
    "throttle",
    "listen",
    "block",
    "rescue",
    "always",
];

/// How deep `line` is indented, in spaces.
fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The key of a `key:` or `key: value` line.
fn key_of(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('-') {
        return None;
    }
    let (key, _) = trimmed.split_once(':')?;
    Some(key.trim().trim_matches('"').trim_matches('\''))
}

/// The value of a `key: value` line, or `None` when it only opens a block.
fn value_of(line: &str) -> Option<String> {
    let (_, value) = line.trim().split_once(':')?;
    let value = value.trim().trim_matches('"').trim_matches('\'');
    (!value.is_empty()).then(|| value.to_owned())
}

/// Splits `block` into its sequence items.
///
/// Each item's leading `- ` becomes two spaces, so the key it carries on
/// the dash line lines up with the keys on the lines below it.
fn items(block: &[&str]) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut dash: Option<usize> = None;
    for line in block {
        if line.trim().is_empty() || line.trim().starts_with('#') {
            continue;
        }
        let depth = indent(line);
        let opens = line.trim_start().starts_with("- ") || line.trim() == "-";
        if opens && dash.is_none_or(|at| depth <= at) {
            dash = Some(depth);
            out.push(vec![line.replacen('-', " ", 1)]);
        } else if let Some(current) = out.last_mut() {
            current.push((*line).to_owned());
        }
    }
    out
}

/// The lines of `block` that sit under the key `wanted`.
fn section<'a>(block: &'a [String], wanted: &str) -> Vec<&'a str> {
    let Some(at) = block.iter().position(|line| key_of(line) == Some(wanted)) else {
        return Vec::new();
    };
    let depth = indent(&block[at]);
    block[at + 1..]
        .iter()
        .take_while(|line| line.trim().is_empty() || indent(line) > depth)
        .map(String::as_str)
        .collect()
}

/// The value of the key `wanted` at the top level of `block`.
fn field(block: &[String], wanted: &str) -> Option<String> {
    let depth = block.first().map_or(0, |line| indent(line));
    block
        .iter()
        .filter(|line| indent(line) == depth)
        .find(|line| key_of(line) == Some(wanted))
        .and_then(|line| value_of(line))
}

/// The task `block` declares.
fn read_task(block: &[String]) -> Task {
    let depth = block.first().map_or(0, |line| indent(line));
    let keys: Vec<&str> = block
        .iter()
        .filter(|line| indent(line) == depth)
        .filter_map(|line| key_of(line))
        .collect();
    Task {
        name: field(block, "name"),
        module: keys
            .iter()
            .find(|key| !TASK_KEYWORDS.contains(key))
            .map_or_else(|| "unrecognised".to_owned(), |key| (*key).to_owned()),
        conditional: keys.contains(&"when"),
        looped: keys
            .iter()
            .any(|key| *key == "loop" || key.starts_with("with_")),
        notifies: field(block, "notify"),
    }
}

/// The roles `block` includes, whether written plainly or as a mapping.
fn read_roles(block: &[String]) -> Vec<String> {
    items(&section(block, "roles"))
        .iter()
        .filter_map(|role| {
            // `- common` names one; `- role: nginx` names the same thing
            // the long way.
            field(role, "role").or_else(|| {
                let first = role.first()?.trim().to_owned();
                (!first.is_empty() && !first.contains(':')).then_some(first)
            })
        })
        .collect()
}

/// The play `block` declares.
fn read_play(block: &[String]) -> Play {
    let mut tasks: Vec<Task> = Vec::new();
    for heading in ["pre_tasks", "tasks", "post_tasks"] {
        tasks.extend(
            items(&section(block, heading))
                .iter()
                .map(|task| read_task(task)),
        );
    }
    let handlers = items(&section(block, "handlers"));
    let variables = section(block, "vars");
    Play {
        name: field(block, "name"),
        hosts: field(block, "hosts").unwrap_or_else(|| "unstated".to_owned()),
        escalates: field(block, "become").is_some_and(|said| said == "true" || said == "yes"),
        tasks,
        roles: read_roles(block),
        handlers: handlers
            .iter()
            .filter_map(|handler| field(handler, "name"))
            .collect(),
        variables: variables
            .iter()
            .filter(|line| indent(line) == variables.first().map_or(0, |first| indent(first)))
            .filter_map(|line| key_of(line))
            .map(str::to_owned)
            .collect(),
    }
}

/// Everything [`AnsibleView`] holds, read from `text`.
fn parse(text: &str) -> AnsibleView {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| line.trim() != "---" && line.trim() != "...")
        .collect();
    let plays: Vec<Play> = items(&lines).iter().map(|play| read_play(play)).collect();

    let mut view = AnsibleView {
        plays,
        modules: Vec::new(),
        vault_references: Vec::new(),
        tasks_without_a_name: Vec::new(),
        handlers_never_notified: Vec::new(),
        truncated: false,
    };
    for play in &view.plays {
        let where_it_is = play.name.clone().unwrap_or_else(|| play.hosts.clone());
        for task in &play.tasks {
            if !view.modules.contains(&task.module) {
                view.modules.push(task.module.clone());
            }
            if task.name.is_none() {
                view.tasks_without_a_name
                    .push(format!("{} in {where_it_is}", task.module));
            }
        }
        for handler in &play.handlers {
            if !play
                .tasks
                .iter()
                .any(|task| task.notifies.as_ref() == Some(handler))
            {
                view.handlers_never_notified
                    .push(format!("{handler} in {where_it_is}"));
            }
        }
    }
    for line in &lines {
        if (line.contains("vault_") || line.contains("!vault"))
            && let Some(key) = key_of(line)
        {
            view.vault_references.push(key.to_owned());
        }
    }
    view.vault_references.sort_unstable();
    view.vault_references.dedup();
    view
}

/// Whether `text` is an Ansible playbook.
fn looks_like_it(text: &str) -> bool {
    let view = parse(text);
    // A play is a mapping with `hosts` and something to do. Without both,
    // this is some other sequence of mappings.
    !view.plays.is_empty()
        && view.plays.iter().all(|play| play.hosts != "unstated")
        && view
            .plays
            .iter()
            .any(|play| !play.tasks.is_empty() || !play.roles.is_empty())
}

/// The Ansible playbook plugin's core half.
#[derive(Debug, Default)]
pub struct AnsibleCore;

impl PluginCore for AnsibleCore {
    fn name(&self) -> &'static str {
        "ansible"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A playbook is YAML, and `yaml` owns the extension. Saying so is
        // what settles the file on the narrower reading (D13).
        &["yaml"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        // The plays and their tasks are the whole of what a playbook
        // says, and they are on the view already.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Ansible playbook plugin's presentation half.
#[derive(Debug, Default)]
pub struct AnsiblePresentation;

impl PluginPresentation for AnsiblePresentation {
    fn name(&self) -> &'static str {
        "ansible"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "ANS",
            tint: 0x00ee_0000,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: AnsibleView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!("{} play(s):", view.plays.len()));
        for play in &view.plays {
            lines.push(format!(
                "  {} -> {}{}",
                play.name.as_deref().unwrap_or("(unnamed play)"),
                play.hosts,
                if play.escalates {
                    " (escalates privilege)"
                } else {
                    ""
                }
            ));
            if !play.roles.is_empty() {
                lines.push(format!("      roles: {}", play.roles.join(", ")));
            }
            if !play.variables.is_empty() {
                lines.push(format!(
                    "      variables (names only): {}",
                    play.variables.join(", ")
                ));
            }
            for task in &play.tasks {
                let mut notes = Vec::new();
                if task.conditional {
                    notes.push("conditional".to_owned());
                }
                if task.looped {
                    notes.push("looped".to_owned());
                }
                if let Some(handler) = &task.notifies {
                    notes.push(format!("notifies {handler}"));
                }
                let notes = if notes.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", notes.join(", "))
                };
                lines.push(format!(
                    "      {} - {}{notes}",
                    task.name.as_deref().unwrap_or("(unnamed task)"),
                    task.module
                ));
            }
            if !play.handlers.is_empty() {
                lines.push(format!("      handlers: {}", play.handlers.join(", ")));
            }
        }
        if !view.modules.is_empty() {
            lines.push(format!("Modules used: {}", view.modules.join(", ")));
        }
        if !view.vault_references.is_empty() {
            lines.push(format!(
                "Reaches into the vault: {}",
                view.vault_references.join(", ")
            ));
        }
        if !view.tasks_without_a_name.is_empty() {
            lines.push("No name, so a failing run reports these by module and".to_owned());
            lines.push("gives no hint which one it was:".to_owned());
            for entry in &view.tasks_without_a_name {
                lines.push(format!("  {entry}"));
            }
        }
        if !view.handlers_never_notified.is_empty() {
            lines.push("Declared but never notified, so these handlers never".to_owned());
            lines.push("fire:".to_owned());
            for entry in &view.handlers_never_notified {
                lines.push(format!("  {entry}"));
            }
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{AnsibleCore, AnsiblePresentation, AnsibleView, items, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const PLAYBOOK: &str = concat!(
        "---\n",
        "- name: Configure web servers\n",
        "  hosts: webservers\n",
        "  become: true\n",
        "  vars:\n",
        "    http_port: 80\n",
        "    api_token: \"{{ vault_api_token }}\"\n",
        "  roles:\n",
        "    - common\n",
        "    - role: nginx\n",
        "  tasks:\n",
        "    - name: Install packages\n",
        "      ansible.builtin.package:\n",
        "        name: \"{{ item }}\"\n",
        "        state: present\n",
        "      loop:\n",
        "        - nginx\n",
        "        - curl\n",
        "      when: ansible_os_family == 'Debian'\n",
        "      notify: restart nginx\n",
        "    - ansible.builtin.command: /usr/bin/true\n",
        "  handlers:\n",
        "    - name: restart nginx\n",
        "      ansible.builtin.service:\n",
        "        name: nginx\n",
        "        state: restarted\n",
        "    - name: reload firewall\n",
        "      ansible.builtin.service:\n",
        "        name: firewalld\n",
        "        state: reloaded\n",
        "\n",
        "- name: Configure databases\n",
        "  hosts: dbservers\n",
        "  tasks:\n",
        "    - name: Start the server\n",
        "      ansible.builtin.service:\n",
        "        name: postgresql\n",
        "        state: started\n",
    );

    #[test]
    fn sniffs_a_playbook() {
        assert!(AnsibleCore.sniff(PLAYBOOK.as_bytes()));
    }

    #[test]
    fn does_not_claim_any_sequence_of_mappings() {
        assert!(!AnsibleCore.sniff(b"- name: alpha\n  size: 1\n- name: beta\n  size: 2\n"));
        assert!(!AnsibleCore.sniff(b"- hosts: all\n"));
        assert!(!AnsibleCore.sniff(b""));
    }

    #[test]
    fn it_says_it_specialises_yaml() {
        assert_eq!(AnsibleCore.specialises(), &["yaml"]);
    }

    #[test]
    fn a_dash_line_keeps_its_key_in_line_with_the_rest() {
        let split = items(&["- name: one", "  size: 1", "- name: two"]);

        assert_eq!(split.len(), 2);
        assert_eq!(
            split[0],
            vec!["  name: one".to_owned(), "  size: 1".to_owned()]
        );
    }

    #[test]
    fn reads_each_play_and_its_target() {
        let view = parse(PLAYBOOK);

        assert_eq!(view.plays.len(), 2);
        assert_eq!(view.plays[0].hosts, "webservers");
        assert!(view.plays[0].escalates);
        assert!(!view.plays[1].escalates);
        assert_eq!(
            view.plays[0].variables,
            vec!["http_port".to_owned(), "api_token".to_owned()]
        );
    }

    #[test]
    fn a_task_keyword_is_not_the_module() {
        let view = parse(PLAYBOOK);

        let install = &view.plays[0].tasks[0];
        assert_eq!(
            install.module, "ansible.builtin.package",
            "`name`, `loop`, `when` and `notify` belong to the task, not to a module"
        );
        assert!(install.conditional);
        assert!(install.looped);
        assert_eq!(install.notifies.as_deref(), Some("restart nginx"));
    }

    #[test]
    fn reads_a_role_written_either_way() {
        let view = parse(PLAYBOOK);

        assert_eq!(
            view.plays[0].roles,
            vec!["common".to_owned(), "nginx".to_owned()]
        );
    }

    #[test]
    fn names_the_task_with_no_name() {
        let view = parse(PLAYBOOK);

        assert_eq!(
            view.tasks_without_a_name,
            vec!["ansible.builtin.command in Configure web servers".to_owned()]
        );
    }

    #[test]
    fn names_the_handler_nothing_notifies() {
        let view = parse(PLAYBOOK);

        assert_eq!(
            view.handlers_never_notified,
            vec!["reload firewall in Configure web servers".to_owned()],
            "`restart nginx` is notified; this one is not"
        );
    }

    #[test]
    fn finds_the_variable_that_reaches_into_the_vault() {
        let view = parse(PLAYBOOK);

        assert_eq!(view.vault_references, vec!["api_token".to_owned()]);
    }

    #[test]
    fn presents_both_warnings_with_their_reasons() {
        let data = serde_json::to_value(parse(PLAYBOOK)).unwrap();

        let lines = AnsiblePresentation.present(&data);

        assert_eq!(lines[0], "2 play(s):");
        assert!(lines.iter().any(|line| line.contains("which one it was")));
        assert!(lines.iter().any(|line| line.contains("never")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/ansible/site.yml");

        let data = AnsibleCore.view(&path).unwrap();
        let view: AnsibleView = serde_json::from_value(data).unwrap();

        assert!(view.plays.len() >= 2);
        assert!(view.plays.iter().any(|play| play.escalates));
        assert!(view.plays.iter().any(|play| !play.roles.is_empty()));
        assert!(view.plays.iter().any(|play| !play.handlers.is_empty()));
        assert!(view.plays.iter().any(|play| !play.variables.is_empty()));
        assert!(
            view.plays
                .iter()
                .flat_map(|play| &play.tasks)
                .any(|task| task.looped)
        );
        assert!(
            view.plays
                .iter()
                .flat_map(|play| &play.tasks)
                .any(|task| task.conditional)
        );
        assert!(view.modules.len() >= 3);
        assert!(!view.vault_references.is_empty());
        assert!(!view.tasks_without_a_name.is_empty());
        assert!(!view.handlers_never_notified.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::AnsibleCore),
            plugin_api::PluginPresentation::extensions(&crate::AnsiblePresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
