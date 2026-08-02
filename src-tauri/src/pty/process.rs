use std::collections::{HashMap, HashSet};

use sysinfo::{Pid, System};

use super::model::ChildProcess;

#[derive(Debug, Default)]
pub struct ProcessInspector {
    system: std::sync::Mutex<System>,
}

impl ProcessInspector {
    pub fn new() -> Self {
        Self {
            system: std::sync::Mutex::new(System::new()),
        }
    }

    pub fn children(&self, parent: u32) -> Vec<ChildProcess> {
        let mut system = self
            .system
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        system.refresh_processes();
        let mut children = children_from_system(&system, parent);
        children.sort_by_key(|process| process.pid);
        children
    }

    pub fn true_pid(&self, root: u32) -> u32 {
        let mut system = self
            .system
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        system.refresh_processes();

        let parent_by_child = system
            .processes()
            .iter()
            .filter_map(|(pid, process)| {
                process
                    .parent()
                    .map(|parent| (pid.as_u32(), parent.as_u32()))
            })
            .collect::<HashMap<_, _>>();
        follow_single_child_chain(root, &parent_by_child, 64)
    }

    pub fn cwd(&self, pid: u32) -> Option<String> {
        let mut system = self
            .system
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let pid = Pid::from_u32(pid);
        system.refresh_process(pid);
        system
            .process(pid)
            .and_then(|process| process.cwd())
            .and_then(|path| path.to_str())
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
    }
}

fn children_from_system(system: &System, parent: u32) -> Vec<ChildProcess> {
    system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let ppid = process.parent()?.as_u32();
            (ppid == parent).then(|| ChildProcess {
                pid: pid.as_u32(),
                ppid,
                command: process.name().to_owned(),
            })
        })
        .collect()
}

pub fn follow_single_child_chain(
    root: u32,
    parent_by_child: &HashMap<u32, u32>,
    max_depth: usize,
) -> u32 {
    let mut current = root;
    let mut visited = HashSet::from([root]);

    for _ in 0..max_depth {
        let mut children = parent_by_child
            .iter()
            .filter_map(|(child, parent)| (*parent == current).then_some(*child));
        let Some(child) = children.next() else {
            break;
        };
        if children.next().is_some() || !visited.insert(child) {
            break;
        }
        current = child;
    }

    current
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::follow_single_child_chain;

    #[test]
    fn follows_only_a_unique_acyclic_child_chain() {
        let tree = HashMap::from([(2, 1), (3, 2), (4, 3)]);
        assert_eq!(follow_single_child_chain(1, &tree, 64), 4);
    }

    #[test]
    fn stops_at_a_fork() {
        let tree = HashMap::from([(2, 1), (3, 1), (4, 2)]);
        assert_eq!(follow_single_child_chain(1, &tree, 64), 1);
    }

    #[test]
    fn stops_on_cycles_and_depth_limit() {
        let cycle = HashMap::from([(2, 1), (1, 2)]);
        assert_eq!(follow_single_child_chain(1, &cycle, 64), 2);

        let deep = HashMap::from([(2, 1), (3, 2), (4, 3)]);
        assert_eq!(follow_single_child_chain(1, &deep, 2), 3);
    }
}
