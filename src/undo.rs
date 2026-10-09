//! Undo and redo for file operations.
//!
//! The history records what a completed [`Operation`] changed on disk, as a [`Change`]. Undoing a
//! change plans the operations that put things back, after checking that the disk still looks the
//! way the change left it. A plan never overwrites: if anything is missing or in the way, the step
//! is refused with a [`Refusal`] instead.
//!
//! Nothing here touches the disk except through a [`Probe`], so the logic is tested with a fake.
//! The app runs the planned operations through its normal operation queue.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::fl;
use crate::operation::{Operation, OperationSelection};

/// How many steps can be undone.
pub const HISTORY_LIMIT: usize = 20;

/// Messages for the undo history, nested in the app's message.
#[derive(Clone, Debug)]
pub enum Message {
    Undo,
    Redo,
    /// Trash items found for a restore plan; empty if none were found.
    Restore(Vec<trash::TrashItem>),
}

/// The user-facing kind of a step, used for the "Undo <kind>" label.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Copy,
    Move,
    NewFile,
    NewFolder,
    Rename,
    Trash,
}

impl Kind {
    pub fn name(self) -> String {
        match self {
            Self::Copy => fl!("undo-kind-copy"),
            Self::Move => fl!("undo-kind-move"),
            Self::NewFile => fl!("undo-kind-new-file"),
            Self::NewFolder => fl!("undo-kind-new-folder"),
            Self::Rename => fl!("undo-kind-rename"),
            Self::Trash => fl!("undo-kind-trash"),
        }
    }
}

/// What an operation changed on disk.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Change {
    /// Items moved or renamed, as `(before, after)` pairs.
    Moved(Vec<(PathBuf, PathBuf)>),
    /// Items that did not exist before.
    Created(Vec<PathBuf>),
    /// Items moved to the trash, by their original paths.
    Trashed(Vec<PathBuf>),
}

impl Change {
    /// The change that undoing this one makes.
    pub fn reversed(&self) -> Self {
        match self {
            Self::Moved(pairs) => Self::Moved(
                pairs
                    .iter()
                    .map(|(before, after)| (after.clone(), before.clone()))
                    .collect(),
            ),
            Self::Created(paths) => Self::Trashed(paths.clone()),
            Self::Trashed(paths) => Self::Created(paths.clone()),
        }
    }

    /// Plan the work that undoes this change, or refuse if the disk no longer matches it.
    pub fn undo_plan(&self, probe: &impl Probe) -> Result<Plan, Refusal> {
        match self {
            Self::Moved(pairs) => {
                let mut renames = Vec::new();
                // Items that keep their name go back with one move per original folder.
                let mut moves: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
                for (before, after) in pairs {
                    if !probe.exists(after) {
                        return Err(Refusal::Missing(after.clone()));
                    }
                    // A case-only rename leaves `before` resolving to the same entry as `after`.
                    if probe.exists(before) && !probe.same_entry(before, after) {
                        return Err(Refusal::Occupied(before.clone()));
                    }
                    let Some(parent) = before.parent() else {
                        return Err(Refusal::NoFolder(before.clone()));
                    };
                    if !probe.exists(parent) {
                        return Err(Refusal::NoFolder(parent.to_path_buf()));
                    }
                    if before.file_name() == after.file_name() {
                        moves
                            .entry(parent.to_path_buf())
                            .or_default()
                            .push(after.clone());
                    } else {
                        renames.push(Operation::Rename {
                            from: after.clone(),
                            to: before.clone(),
                        });
                    }
                }
                renames.extend(moves.into_iter().map(|(to, paths)| Operation::Move {
                    paths,
                    to,
                    cross_device_copy: false,
                }));
                Ok(Plan::Operations(renames))
            }
            Self::Created(paths) => {
                if let Some(missing) = paths.iter().find(|path| !probe.exists(path)) {
                    return Err(Refusal::Missing(missing.clone()));
                }
                Ok(Plan::Operations(vec![Operation::Delete {
                    paths: paths.clone(),
                }]))
            }
            Self::Trashed(paths) => {
                if let Some(occupied) = paths.iter().find(|path| probe.exists(path)) {
                    return Err(Refusal::Occupied(occupied.clone()));
                }
                Ok(Plan::Restore(paths.clone()))
            }
        }
    }
}

/// The work that undoes a change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Plan {
    /// Operations to run through the operation queue.
    Operations(Vec<Operation>),
    /// Restore these original paths from the trash.
    Restore(Vec<PathBuf>),
}

/// Why a step cannot be undone or redone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// The item is no longer where the operation left it.
    Missing(PathBuf),
    /// Something else now exists where the item would go back to.
    Occupied(PathBuf),
    /// The folder the item would go back to no longer exists.
    NoFolder(PathBuf),
}

impl Refusal {
    pub fn message(&self, kind: Kind, redo: bool) -> String {
        let action = kind.name();
        let reason = match self {
            Self::Missing(path) => fl!("undo-refused-missing", path = path.display().to_string()),
            Self::Occupied(path) => {
                fl!("undo-refused-occupied", path = path.display().to_string())
            }
            Self::NoFolder(path) => {
                fl!("undo-refused-no-folder", path = path.display().to_string())
            }
        };
        if redo {
            fl!("redo-refused", action = action, reason = reason)
        } else {
            fl!("undo-refused", action = action, reason = reason)
        }
    }
}

/// Read access to the disk, for checking preconditions.
pub trait Probe {
    /// Whether anything, including a dangling symlink, exists at `path`.
    fn exists(&self, path: &Path) -> bool;
    /// Whether `a` and `b` name the same directory entry.
    fn same_entry(&self, a: &Path, b: &Path) -> bool;
}

/// The real file system.
pub struct Disk;

impl Probe for Disk {
    fn exists(&self, path: &Path) -> bool {
        path.symlink_metadata().is_ok()
    }

    fn same_entry(&self, a: &Path, b: &Path) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            match (a.symlink_metadata(), b.symlink_metadata()) {
                (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
                _ => false,
            }
        }
        #[cfg(not(unix))]
        {
            a == b
        }
    }
}

/// One undoable step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Step {
    pub kind: Kind,
    pub change: Change,
}

impl Step {
    /// The step a completed operation adds to the history, if it can be undone.
    ///
    /// `conflicted` says that a destination already existed when the operation started. The
    /// replace dialog may then have overwritten, skipped or renamed items, so the result is not
    /// known well enough to undo.
    pub fn from_operation(
        op: &Operation,
        selection: &OperationSelection,
        conflicted: bool,
    ) -> Option<Self> {
        let (kind, change) = match op {
            Operation::Rename { from, to } => (
                Kind::Rename,
                Change::Moved(vec![(from.clone(), to.clone())]),
            ),
            Operation::Move {
                paths,
                to,
                cross_device_copy: false,
            } if !conflicted => {
                let pairs: Vec<_> = paths
                    .iter()
                    .filter_map(|from| Some((from.clone(), to.join(from.file_name()?))))
                    .filter(|(from, to)| from != to)
                    .collect();
                if pairs.is_empty() {
                    return None;
                }
                (Kind::Move, Change::Moved(pairs))
            }
            Operation::Copy { .. } if !conflicted && !selection.selected.is_empty() => {
                (Kind::Copy, Change::Created(selection.selected.clone()))
            }
            Operation::NewFolder { path } => (Kind::NewFolder, Change::Created(vec![path.clone()])),
            Operation::NewFile { path } => (Kind::NewFile, Change::Created(vec![path.clone()])),
            Operation::Delete { paths } if !paths.is_empty() => {
                (Kind::Trash, Change::Trashed(paths.clone()))
            }
            _ => return None,
        };
        Some(Self { kind, change })
    }
}

/// Whether any destination of a copy or move already exists, before it starts.
pub fn destinations_occupied(op: &Operation, probe: &impl Probe) -> bool {
    let (paths, to, copy) = match op {
        Operation::Copy { paths, to } => (paths, to, true),
        Operation::Move { paths, to, .. } => (paths, to, false),
        _ => return false,
    };
    paths.iter().any(|from| {
        // Copying into the item's own folder makes a uniquely named duplicate.
        if copy && from.parent() == Some(to.as_path()) {
            return false;
        }
        from.file_name()
            .is_some_and(|name| probe.exists(&to.join(name)))
    })
}

/// The bounded undo and redo history.
#[derive(Debug, Default)]
pub struct History {
    undo: VecDeque<Step>,
    redo: Vec<Step>,
    /// Operations started by undo or redo, which must not be recorded as new steps.
    replaying: HashSet<u64>,
    /// Operations whose destinations were occupied when they started.
    conflicted: HashSet<u64>,
}

impl History {
    /// Note an operation as it starts.
    pub fn started(&mut self, id: u64, op: &Operation, probe: &impl Probe) {
        if destinations_occupied(op, probe) {
            self.conflicted.insert(id);
        }
    }

    /// Note that an operation was started by undo or redo.
    pub fn replaying(&mut self, id: u64) {
        self.replaying.insert(id);
    }

    /// Record a completed operation.
    pub fn completed(&mut self, id: u64, op: &Operation, selection: &OperationSelection) {
        let conflicted = self.conflicted.remove(&id);
        if self.replaying.remove(&id) {
            return;
        }
        if let Some(step) = Step::from_operation(op, selection, conflicted) {
            self.push(step);
        }
    }

    /// Forget a failed or cancelled operation.
    pub fn failed(&mut self, id: u64) {
        self.conflicted.remove(&id);
        self.replaying.remove(&id);
    }

    /// Add a new step. This clears the redo history.
    pub fn push(&mut self, step: Step) {
        self.redo.clear();
        self.undo.push_back(step);
        while self.undo.len() > HISTORY_LIMIT {
            self.undo.pop_front();
        }
    }

    /// Drop the trash step for `paths`, after the toast's own Undo restored them.
    pub fn forget_trashed(&mut self, paths: &[PathBuf]) {
        self.undo
            .retain(|step| !matches!(&step.change, Change::Trashed(trashed) if trashed == paths));
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.undo.len()
    }

    pub fn next_undo(&self) -> Option<Kind> {
        self.undo.back().map(|step| step.kind)
    }

    pub fn next_redo(&self) -> Option<Kind> {
        self.redo.last().map(|step| step.kind)
    }

    /// Take the most recent step and plan its undo.
    ///
    /// On success the step moves to the redo history. A refused step is dropped, so the next undo
    /// reaches the step before it.
    pub fn undo(&mut self, probe: &impl Probe) -> Option<Result<(Kind, Plan), (Kind, Refusal)>> {
        let step = self.undo.pop_back()?;
        Some(match step.change.undo_plan(probe) {
            Ok(plan) => {
                self.redo.push(Step {
                    kind: step.kind,
                    change: step.change.reversed(),
                });
                Ok((step.kind, plan))
            }
            Err(refusal) => Err((step.kind, refusal)),
        })
    }

    /// Take the most recently undone step and plan running it again.
    pub fn redo(&mut self, probe: &impl Probe) -> Option<Result<(Kind, Plan), (Kind, Refusal)>> {
        let step = self.redo.pop()?;
        Some(match step.change.undo_plan(probe) {
            Ok(plan) => {
                self.undo.push_back(Step {
                    kind: step.kind,
                    change: step.change.reversed(),
                });
                Ok((step.kind, plan))
            }
            Err(refusal) => Err((step.kind, refusal)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A fake disk: a set of paths, with optional aliases for case-insensitive names.
    #[derive(Default)]
    struct Fake {
        paths: HashSet<PathBuf>,
        aliases: HashMap<PathBuf, PathBuf>,
    }

    impl Fake {
        fn with(paths: &[&str]) -> Self {
            Self {
                paths: paths.iter().map(PathBuf::from).collect(),
                aliases: HashMap::new(),
            }
        }

        fn resolve(&self, path: &Path) -> PathBuf {
            self.aliases
                .get(path)
                .cloned()
                .unwrap_or_else(|| path.to_path_buf())
        }
    }

    impl Probe for Fake {
        fn exists(&self, path: &Path) -> bool {
            self.paths.contains(&self.resolve(path))
        }

        fn same_entry(&self, a: &Path, b: &Path) -> bool {
            self.exists(a) && self.resolve(a) == self.resolve(b)
        }
    }

    fn p(path: &str) -> PathBuf {
        PathBuf::from(path)
    }

    fn sel(selected: &[&str]) -> OperationSelection {
        OperationSelection {
            ignored: Vec::new(),
            selected: selected.iter().map(PathBuf::from).collect(),
        }
    }

    #[test]
    fn rename_inverse_renames_back() {
        let op = Operation::Rename {
            from: p("/d/a"),
            to: p("/d/b"),
        };
        let step = Step::from_operation(&op, &sel(&[]), false).unwrap();
        assert_eq!(step.kind, Kind::Rename);
        let plan = step.change.undo_plan(&Fake::with(&["/d", "/d/b"])).unwrap();
        assert_eq!(
            plan,
            Plan::Operations(vec![Operation::Rename {
                from: p("/d/b"),
                to: p("/d/a"),
            }])
        );
    }

    #[test]
    fn move_inverse_moves_back_grouped_by_folder() {
        let op = Operation::Move {
            paths: vec![p("/x/a"), p("/y/b"), p("/x/c")],
            to: p("/t"),
            cross_device_copy: false,
        };
        let step = Step::from_operation(&op, &sel(&[]), false).unwrap();
        assert_eq!(step.kind, Kind::Move);
        let probe = Fake::with(&["/x", "/y", "/t", "/t/a", "/t/b", "/t/c"]);
        assert_eq!(
            step.change.undo_plan(&probe).unwrap(),
            Plan::Operations(vec![
                Operation::Move {
                    paths: vec![p("/t/a"), p("/t/c")],
                    to: p("/x"),
                    cross_device_copy: false,
                },
                Operation::Move {
                    paths: vec![p("/t/b")],
                    to: p("/y"),
                    cross_device_copy: false,
                },
            ])
        );
    }

    #[test]
    fn move_is_not_recorded_after_a_conflict_or_as_a_cross_device_copy() {
        let op = Operation::Move {
            paths: vec![p("/x/a")],
            to: p("/t"),
            cross_device_copy: false,
        };
        assert_eq!(Step::from_operation(&op, &sel(&[]), true), None);
        let op = Operation::Move {
            paths: vec![p("/x/a")],
            to: p("/t"),
            cross_device_copy: true,
        };
        assert_eq!(Step::from_operation(&op, &sel(&[]), false), None);
    }

    #[test]
    fn move_into_its_own_folder_is_not_recorded() {
        let op = Operation::Move {
            paths: vec![p("/t/a")],
            to: p("/t"),
            cross_device_copy: false,
        };
        assert_eq!(Step::from_operation(&op, &sel(&[]), false), None);
    }

    #[test]
    fn new_items_and_copies_are_undone_by_trashing() {
        for (op, kind) in [
            (Operation::NewFolder { path: p("/d/n") }, Kind::NewFolder),
            (Operation::NewFile { path: p("/d/n") }, Kind::NewFile),
        ] {
            let step = Step::from_operation(&op, &sel(&["/d/n"]), false).unwrap();
            assert_eq!(step.kind, kind);
            assert_eq!(
                step.change.undo_plan(&Fake::with(&["/d/n"])).unwrap(),
                Plan::Operations(vec![Operation::Delete {
                    paths: vec![p("/d/n")]
                }])
            );
        }

        let op = Operation::Copy {
            paths: vec![p("/d/a")],
            to: p("/d"),
        };
        let step = Step::from_operation(&op, &sel(&["/d/a copy 1"]), false).unwrap();
        assert_eq!(step.change, Change::Created(vec![p("/d/a copy 1")]));
        // A copy that met an existing destination, or produced nothing, is not undoable.
        assert_eq!(Step::from_operation(&op, &sel(&["/d/a"]), true), None);
        assert_eq!(Step::from_operation(&op, &sel(&[]), false), None);
    }

    #[test]
    fn trash_is_undone_by_restoring() {
        let op = Operation::Delete {
            paths: vec![p("/d/a")],
        };
        let step = Step::from_operation(&op, &sel(&[]), false).unwrap();
        assert_eq!(step.kind, Kind::Trash);
        assert_eq!(
            step.change.undo_plan(&Fake::with(&["/d"])).unwrap(),
            Plan::Restore(vec![p("/d/a")])
        );
        assert_eq!(
            step.change.undo_plan(&Fake::with(&["/d", "/d/a"])),
            Err(Refusal::Occupied(p("/d/a")))
        );
    }

    #[test]
    fn other_operations_are_not_recorded() {
        for op in [
            Operation::EmptyTrash,
            Operation::PermanentlyDelete {
                paths: vec![p("/a")].into(),
            },
            Operation::Restore { items: Vec::new() },
            Operation::SetPermissions {
                path: p("/a"),
                mode: 0o644,
            },
        ] {
            assert_eq!(Step::from_operation(&op, &sel(&["/a"]), false), None);
        }
    }

    #[test]
    fn refuses_when_the_item_is_gone() {
        let change = Change::Moved(vec![(p("/d/a"), p("/d/b"))]);
        assert_eq!(
            change.undo_plan(&Fake::with(&["/d"])),
            Err(Refusal::Missing(p("/d/b")))
        );
        let change = Change::Created(vec![p("/d/n")]);
        assert_eq!(
            change.undo_plan(&Fake::with(&["/d"])),
            Err(Refusal::Missing(p("/d/n")))
        );
    }

    #[test]
    fn refuses_to_overwrite() {
        let change = Change::Moved(vec![(p("/d/a"), p("/d/b"))]);
        assert_eq!(
            change.undo_plan(&Fake::with(&["/d", "/d/a", "/d/b"])),
            Err(Refusal::Occupied(p("/d/a")))
        );
    }

    #[test]
    fn refuses_when_the_folder_is_gone() {
        let change = Change::Moved(vec![(p("/x/a"), p("/t/a"))]);
        assert_eq!(
            change.undo_plan(&Fake::with(&["/t", "/t/a"])),
            Err(Refusal::NoFolder(p("/x")))
        );
    }

    #[test]
    fn case_only_rename_is_undoable() {
        let mut probe = Fake::with(&["/d", "/d/A"]);
        probe.aliases.insert(p("/d/a"), p("/d/A"));
        let change = Change::Moved(vec![(p("/d/a"), p("/d/A"))]);
        assert!(change.undo_plan(&probe).is_ok());
    }

    #[test]
    fn history_is_bounded() {
        let mut history = History::default();
        for i in 0..HISTORY_LIMIT + 5 {
            history.push(Step {
                kind: Kind::NewFile,
                change: Change::Created(vec![PathBuf::from(format!("/f{i}"))]),
            });
        }
        assert_eq!(history.len(), HISTORY_LIMIT);
        // The oldest steps were dropped.
        let mut probe = Fake::default();
        for i in 0..HISTORY_LIMIT + 5 {
            probe.paths.insert(PathBuf::from(format!("/f{i}")));
        }
        let mut last = None;
        while let Some(result) = history.undo(&probe) {
            last = Some(result.unwrap().1);
        }
        assert_eq!(
            last,
            Some(Plan::Operations(vec![Operation::Delete {
                paths: vec![p("/f5")]
            }]))
        );
    }

    #[test]
    fn undo_then_redo_round_trips_and_new_steps_clear_redo() {
        let mut history = History::default();
        history.completed(
            1,
            &Operation::Rename {
                from: p("/d/a"),
                to: p("/d/b"),
            },
            &sel(&["/d/b"]),
        );
        assert_eq!(history.next_undo(), Some(Kind::Rename));

        let (_, plan) = history.undo(&Fake::with(&["/d", "/d/b"])).unwrap().unwrap();
        assert_eq!(
            plan,
            Plan::Operations(vec![Operation::Rename {
                from: p("/d/b"),
                to: p("/d/a"),
            }])
        );
        assert_eq!(history.next_undo(), None);
        assert_eq!(history.next_redo(), Some(Kind::Rename));

        let (_, plan) = history.redo(&Fake::with(&["/d", "/d/a"])).unwrap().unwrap();
        assert_eq!(
            plan,
            Plan::Operations(vec![Operation::Rename {
                from: p("/d/a"),
                to: p("/d/b"),
            }])
        );
        assert_eq!(history.next_undo(), Some(Kind::Rename));

        history.undo(&Fake::with(&["/d", "/d/b"]));
        history.push(Step {
            kind: Kind::NewFile,
            change: Change::Created(vec![p("/d/n")]),
        });
        assert_eq!(history.next_redo(), None);
    }

    #[test]
    fn replayed_and_failed_operations_are_not_recorded() {
        let mut history = History::default();
        let op = Operation::NewFolder { path: p("/d/n") };
        history.replaying(7);
        history.completed(7, &op, &sel(&["/d/n"]));
        assert_eq!(history.len(), 0);
        history.started(8, &op, &Fake::default());
        history.failed(8);
        assert_eq!(history.len(), 0);
        assert!(history.replaying.is_empty() && history.conflicted.is_empty());
    }

    #[test]
    fn occupied_destinations_are_detected_before_start() {
        let probe = Fake::with(&["/t/a"]);
        let mut history = History::default();
        let op = Operation::Move {
            paths: vec![p("/x/a")],
            to: p("/t"),
            cross_device_copy: false,
        };
        history.started(1, &op, &probe);
        history.completed(1, &op, &sel(&["/t/a"]));
        assert_eq!(history.len(), 0);
        // Duplicating into the same folder never conflicts.
        let dup = Operation::Copy {
            paths: vec![p("/t/a")],
            to: p("/t"),
        };
        assert!(!destinations_occupied(&dup, &probe));
    }

    #[test]
    fn toast_undo_forgets_the_trash_step() {
        let mut history = History::default();
        let op = Operation::Delete {
            paths: vec![p("/d/a")],
        };
        history.completed(1, &op, &sel(&[]));
        history.forget_trashed(&[p("/d/b")]);
        assert_eq!(history.len(), 1);
        history.forget_trashed(&[p("/d/a")]);
        assert_eq!(history.len(), 0);
    }

    #[test]
    fn refused_steps_are_dropped() {
        let mut history = History::default();
        history.push(Step {
            kind: Kind::NewFile,
            change: Change::Created(vec![p("/old")]),
        });
        history.push(Step {
            kind: Kind::NewFile,
            change: Change::Created(vec![p("/gone")]),
        });
        let probe = Fake::with(&["/old"]);
        assert!(matches!(history.undo(&probe), Some(Err(_))));
        assert!(matches!(history.undo(&probe), Some(Ok(_))));
        assert_eq!(history.next_redo(), Some(Kind::NewFile));
    }

    /// Rename a real file, then run the planned inverse through the operation code.
    #[compio::test]
    async fn rename_then_undo_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("before.txt");
        let to = dir.path().join("after.txt");
        std::fs::write(&from, b"hello").unwrap();

        let (tx, _rx) = cosmic::iced::futures::channel::mpsc::channel(1);
        let tx = std::sync::Arc::new(tokio::sync::Mutex::new(tx));
        let op = Operation::Rename {
            from: from.clone(),
            to: to.clone(),
        };
        let mut history = History::default();
        history.started(1, &op, &Disk);
        let selection = op
            .clone()
            .perform(&tx, crate::operation::Controller::default())
            .await
            .unwrap();
        history.completed(1, &op, &selection);
        assert!(to.exists() && !from.exists());

        let Some(Ok((Kind::Rename, Plan::Operations(ops)))) = history.undo(&Disk) else {
            panic!("rename should be undoable");
        };
        for op in ops {
            op.perform(&tx, crate::operation::Controller::default())
                .await
                .unwrap();
        }
        assert!(from.exists() && !to.exists());
        assert_eq!(std::fs::read(&from).unwrap(), b"hello");

        // With something new in the way, redo refuses instead of overwriting it.
        std::fs::write(&to, b"newer").unwrap();
        assert_eq!(
            history.redo(&Disk),
            Some(Err((Kind::Rename, Refusal::Occupied(to.clone()))))
        );
        assert_eq!(std::fs::read(&to).unwrap(), b"newer");
    }
}
