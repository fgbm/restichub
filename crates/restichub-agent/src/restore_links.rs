//! The paths of a restore that an existing entry of the target could carry out of the restore
//! roots (FR-RS-03, threat T31).
//!
//! Restic on Windows writes a file of the snapshot through an existing file link at its path. The
//! agent checks only the paths that the restore writes: `target/<node>` for each node of the
//! snapshot that the includes and excludes can select, and each ancestor of it inside the target.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use restichub_proto::restore_path::{PathFlavor, is_within};

/// The nodes of the snapshot that a restore with these includes and excludes can write.
///
/// The set is never smaller than the set of restic. An include narrows it by its literal leading
/// components only, and an exclude only when the whole pattern is a literal absolute path, which
/// restic excludes with its subtree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// `None` when an include can match below any directory.
    includes: Option<Vec<Vec<String>>>,
    excludes: Vec<Vec<String>>,
    flavor: PathFlavor,
}

impl Selection {
    #[must_use]
    pub fn new(includes: &[String], excludes: &[String], flavor: PathFlavor) -> Self {
        let mut prefixes = Vec::new();
        let mut everything = false;
        // Negated includes only take nodes away, so the wider set ignores them.
        for include in includes.iter().filter(|include| !include.starts_with('!')) {
            match literal_prefix(include, flavor) {
                Some((prefix, _)) if !prefix.is_empty() => prefixes.push(prefix),
                _ => everything = true,
            }
        }
        // A negated exclude selects a node again, so no exclude is certain then.
        let excludes = if excludes.iter().any(|exclude| exclude.starts_with('!')) {
            Vec::new()
        } else {
            excludes
                .iter()
                .filter_map(|exclude| match literal_prefix(exclude, flavor) {
                    Some((prefix, true)) if !prefix.is_empty() => Some(prefix),
                    _ => None,
                })
                .collect()
        };
        Self {
            includes: (!everything && !prefixes.is_empty()).then_some(prefixes),
            excludes,
            flavor,
        }
    }

    /// True when the restore can write the node or a node below it. `node` holds the components
    /// of the snapshot path.
    #[must_use]
    pub fn writes(&self, node: &[&str]) -> bool {
        if self
            .excludes
            .iter()
            .any(|exclude| starts_with(node, exclude, false))
        {
            return false;
        }
        // restic compares the case, a case-insensitive file system does not: the wider set wins.
        let fold = self.flavor == PathFlavor::Windows;
        self.includes.as_ref().is_none_or(|includes| {
            includes.iter().any(|include| {
                starts_with(node, include, fold) || starts_with_node(include, node, fold)
            })
        })
    }
}

/// The literal leading components of an absolute pattern, and whether they are the whole pattern.
/// `None` for a pattern that can match below any directory.
fn literal_prefix(pattern: &str, flavor: PathFlavor) -> Option<(Vec<String>, bool)> {
    let separators: &[char] = match flavor {
        PathFlavor::Windows => &['/', '\\'],
        PathFlavor::Posix => &['/'],
    };
    let rest = pattern.strip_prefix(separators)?;
    // restic cleans the pattern first: a `..` can move the path out of the leading components,
    // and a UNC volume turns into an empty part, which restic reads as `**`.
    if flavor == PathFlavor::Windows && rest.starts_with(separators) {
        return None;
    }
    let parts: Vec<&str> = rest
        .split(separators)
        .filter(|part| !part.is_empty())
        .collect();
    // A `..` after a wildcard part also cleans the wildcard away: `/C/*/../../D` is `/D`.
    if parts.contains(&"..") {
        return None;
    }
    let mut prefix = Vec::new();
    for part in parts {
        if part == "." {
            continue;
        }
        let wildcard = part.contains(['*', '?', '[', '$'])
            || (flavor == PathFlavor::Posix && part.contains('\\'));
        if wildcard {
            return Some((prefix, false));
        }
        prefix.push(part.to_owned());
    }
    Some((prefix, true))
}

fn same(left: &str, right: &str, fold: bool) -> bool {
    if fold {
        left.to_lowercase() == right.to_lowercase()
    } else {
        left == right
    }
}

fn starts_with(node: &[&str], prefix: &[String], fold: bool) -> bool {
    node.len() >= prefix.len()
        && node
            .iter()
            .zip(prefix)
            .all(|(node, prefix)| same(node, prefix, fold))
}

fn starts_with_node(prefix: &[String], node: &[&str], fold: bool) -> bool {
    prefix.len() >= node.len()
        && node
            .iter()
            .zip(prefix)
            .all(|(node, prefix)| same(node, prefix, fold))
}

/// Why the restore must not write a path as it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A link or a junction that leads out of the target or nowhere.
    OutwardLink(PathBuf),
    /// A file with another name, which can lie outside the target.
    HardLink(PathBuf),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutwardLink(path) => write!(
                formatter,
                "the restore target holds {}, a link that leads out of the target or nowhere, on a path that the restore writes; remove the link or restore into an empty directory",
                path.display()
            ),
            Self::HardLink(path) => write!(
                formatter,
                "the restore target holds {}, a file with more than one name, on a path that the restore writes; remove the other names or restore into an empty directory",
                path.display()
            ),
        }
    }
}

/// The existing target of one restore and the directories that the check already passed.
pub struct TargetCheck {
    base: PathBuf,
    inside: String,
    flavor: PathFlavor,
    passed: HashSet<PathBuf>,
}

enum Entry {
    Missing,
    Passed,
    Refused(Refusal),
}

impl TargetCheck {
    /// Opens the target, or gives `None` when it is missing or empty: no existing entry can
    /// redirect the restore then.
    ///
    /// # Errors
    ///
    /// Returns an error that names the target when it cannot be read.
    pub fn open(target: &Path) -> Result<Option<Self>> {
        let base = match crate::restore_roots::resolve(target) {
            Ok(base) => base,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("cannot resolve {}", target.display()));
            }
        };
        let mut entries = match std::fs::read_dir(&base) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            // restic reports a target that is not a directory itself.
            Err(_) if !base.is_dir() => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("cannot read {}", base.display()));
            }
        };
        if entries.next().is_none() {
            return Ok(None);
        }
        let inside = base
            .to_str()
            .with_context(|| format!("{} is not valid UTF-8", base.display()))?
            .to_owned();
        Ok(Some(Self {
            base,
            inside,
            flavor: PathFlavor::host(),
            passed: HashSet::new(),
        }))
    }

    /// Checks `target/<node>` and its ancestors inside the target. An entry that does not exist,
    /// or that vanishes during the check, passes.
    ///
    /// # Errors
    ///
    /// Returns an error that names the entry that cannot be read.
    pub fn node(&mut self, node: &[&str]) -> Result<Option<Refusal>> {
        let mut path = self.base.clone();
        for (index, name) in node.iter().enumerate() {
            path.push(name);
            let leaf = index + 1 == node.len();
            if !leaf && self.passed.contains(&path) {
                continue;
            }
            match self.entry(&path, leaf)? {
                Entry::Missing => return Ok(None),
                Entry::Refused(refusal) => return Ok(Some(refusal)),
                Entry::Passed => {
                    if !leaf {
                        self.passed.insert(path.clone());
                    }
                }
            }
        }
        Ok(None)
    }

    fn entry(&self, path: &Path, leaf: bool) -> Result<Entry> {
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Entry::Missing),
            Err(error) => {
                return Err(error).with_context(|| format!("cannot read {}", path.display()));
            }
        };
        // `is_symlink` covers a junction on Windows.
        if metadata.file_type().is_symlink() {
            return Ok(match crate::restore_roots::resolve(path) {
                Ok(destination)
                    if destination.to_str().is_some_and(|destination| {
                        is_within(self.flavor, destination, &self.inside)
                    }) =>
                {
                    Entry::Passed
                }
                Err(_) if vanished(path) => Entry::Missing,
                _ => Entry::Refused(Refusal::OutwardLink(path.to_path_buf())),
            });
        }
        if leaf && metadata.is_file() && link_count(path, &metadata)? > 1 {
            return Ok(Entry::Refused(Refusal::HardLink(path.to_path_buf())));
        }
        Ok(Entry::Passed)
    }
}

fn vanished(path: &Path) -> bool {
    matches!(std::fs::symlink_metadata(path), Err(error) if error.kind() == io::ErrorKind::NotFound)
}

#[cfg(unix)]
#[allow(clippy::unnecessary_wraps)]
fn link_count(_path: &Path, metadata: &std::fs::Metadata) -> Result<u64> {
    use std::os::unix::fs::MetadataExt as _;
    Ok(metadata.nlink())
}

#[cfg(windows)]
fn link_count(path: &Path, _metadata: &std::fs::Metadata) -> Result<u64> {
    Ok(crate::winsec::Entry::inspect(path)?.map_or(1, |entry| u64::from(entry.links())))
}

#[cfg(not(any(unix, windows)))]
#[allow(clippy::unnecessary_wraps)]
fn link_count(_path: &Path, _metadata: &std::fs::Metadata) -> Result<u64> {
    Ok(1)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    fn parts(path: &str) -> Vec<&str> {
        path.split('/').filter(|part| !part.is_empty()).collect()
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "restichub-restore-links-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("cannot create the directory");
        dir.canonicalize().expect("cannot resolve the directory")
    }

    #[cfg(unix)]
    fn link(original: &Path, link: &Path) {
        std::os::unix::fs::symlink(original, link).expect("cannot create the link");
    }

    #[cfg(windows)]
    fn link(original: &Path, link: &Path) {
        if original.is_dir() {
            std::os::windows::fs::symlink_dir(original, link).expect("cannot create the link");
        } else {
            std::os::windows::fs::symlink_file(original, link).expect("cannot create the link");
        }
    }

    #[test]
    fn fr_rs_03_an_include_selects_its_subtree_and_its_ancestors() {
        let selection = Selection::new(&strings(&["/srv/app/config.yml"]), &[], PathFlavor::Posix);
        assert!(selection.writes(&parts("/srv")));
        assert!(selection.writes(&parts("/srv/app")));
        assert!(selection.writes(&parts("/srv/app/config.yml")));
        assert!(selection.writes(&parts("/srv/app/config.yml/x")));
        assert!(!selection.writes(&parts("/srv/app/venv/bin/python")));
        assert!(!selection.writes(&parts("/etc")));
    }

    #[test]
    fn fr_rs_03_a_wildcard_include_narrows_by_its_literal_part_only() {
        let selection = Selection::new(&strings(&["/srv/*/config.yml"]), &[], PathFlavor::Posix);
        assert!(selection.writes(&parts("/srv/other/venv")));
        assert!(!selection.writes(&parts("/etc/passwd")));
        let anywhere = Selection::new(&strings(&["config.yml"]), &[], PathFlavor::Posix);
        assert!(anywhere.writes(&parts("/etc/passwd")));
        let none = Selection::new(&[], &[], PathFlavor::Posix);
        assert!(none.writes(&parts("/etc/passwd")));
    }

    #[test]
    fn fr_rs_03_only_a_literal_exclude_narrows_the_selection() {
        let selection = Selection::new(
            &[],
            &strings(&["/srv/app/venv", "/srv/*/cache", "*.log"]),
            PathFlavor::Posix,
        );
        assert!(!selection.writes(&parts("/srv/app/venv")));
        assert!(!selection.writes(&parts("/srv/app/venv/bin/python")));
        assert!(selection.writes(&parts("/srv/app/cache/x")));
        assert!(selection.writes(&parts("/srv/app/a.log")));
        let negated = Selection::new(
            &[],
            &strings(&["/srv/app/venv", "!/srv/app/venv/keep"]),
            PathFlavor::Posix,
        );
        assert!(negated.writes(&parts("/srv/app/venv/keep")));
    }

    #[test]
    fn fr_rs_03_a_windows_include_ignores_the_case_and_takes_both_separators() {
        let selection = Selection::new(&strings(&[r"\C\Data\a.txt"]), &[], PathFlavor::Windows);
        assert!(selection.writes(&parts("/c/data/a.txt")));
        assert!(!selection.writes(&parts("/C/Other")));
        let exclude = Selection::new(&[], &strings(&["/C/Data"]), PathFlavor::Windows);
        assert!(
            exclude.writes(&parts("/C/data/a.txt")),
            "restic compares the case"
        );
    }

    #[test]
    fn fr_rs_03_a_pattern_that_restic_rewrites_selects_the_rewritten_path() {
        let dotdot = Selection::new(&strings(&["/C/../D/x"]), &[], PathFlavor::Posix);
        assert!(dotdot.writes(&parts("/D/x")));
        let after_wildcard = Selection::new(&strings(&["/C/*/../../D/x"]), &[], PathFlavor::Posix);
        assert!(after_wildcard.writes(&parts("/D/x")));
        let windows = Selection::new(&strings(&[r"/C/*/../../D/x"]), &[], PathFlavor::Windows);
        assert!(windows.writes(&parts("/D/x")));
        let unc = Selection::new(&strings(&[r"\\srv\share"]), &[], PathFlavor::Windows);
        assert!(unc.writes(&parts("/C/a/srv/share")));
        let dots = Selection::new(&strings(&["/./srv//app/./x"]), &[], PathFlavor::Posix);
        assert!(dots.writes(&parts("/srv/app/x/y")));
        assert!(!dots.writes(&parts("/srv/other")));
        let exclude = Selection::new(&[], &strings(&["/srv/../etc"]), PathFlavor::Posix);
        assert!(exclude.writes(&parts("/etc/passwd")));
    }

    #[test]
    fn fr_rs_03_a_missing_or_empty_target_needs_no_check() {
        let base = temp_dir("empty");
        assert!(
            TargetCheck::open(&base.join("missing"))
                .expect("open")
                .is_none()
        );
        assert!(TargetCheck::open(&base).expect("open").is_none());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_rs_03_an_outward_link_on_a_written_path_or_its_ancestor_is_refused() {
        let base = temp_dir("outward");
        let target = base.join("target");
        let outside = base.join("outside");
        std::fs::create_dir_all(target.join("data")).expect("cannot create the target");
        std::fs::create_dir_all(&outside).expect("cannot create the outside");
        std::fs::write(outside.join("victim.txt"), b"OUTSIDE").expect("cannot write");
        link(
            &outside.join("victim.txt"),
            &target.join("data").join("a.txt"),
        );
        link(&outside, &target.join("sub"));
        link(&outside, &target.join("venv"));
        let mut check = TargetCheck::open(&target)
            .expect("open")
            .expect("not empty");
        assert_eq!(
            check.node(&["data", "a.txt"]).expect("check"),
            Some(Refusal::OutwardLink(target.join("data").join("a.txt")))
        );
        assert_eq!(
            check.node(&["sub", "f.txt"]).expect("check"),
            Some(Refusal::OutwardLink(target.join("sub"))),
            "an ancestor link carries the node out"
        );
        assert_eq!(check.node(&["data", "other.txt"]).expect("check"), None);
        assert_eq!(check.node(&["new", "f.txt"]).expect("check"), None);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_rs_03_a_dangling_link_on_a_written_path_is_refused() {
        let base = temp_dir("dangling");
        let target = base.join("target");
        std::fs::create_dir_all(&target).expect("cannot create the target");
        std::fs::write(target.join("keep"), b"x").expect("cannot write");
        std::fs::write(base.join("gone"), b"x").expect("cannot write");
        link(&base.join("gone"), &target.join("a.txt"));
        std::fs::remove_file(base.join("gone")).expect("cannot remove");
        let mut check = TargetCheck::open(&target)
            .expect("open")
            .expect("not empty");
        assert_eq!(
            check.node(&["a.txt"]).expect("check"),
            Some(Refusal::OutwardLink(target.join("a.txt")))
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_rs_03_a_link_inside_the_target_or_to_the_target_itself_passes() {
        let base = temp_dir("inside");
        let target = base.join("target");
        std::fs::create_dir_all(target.join("elsewhere")).expect("cannot create the target");
        link(&target.join("elsewhere"), &target.join("sub"));
        link(&target, &target.join("self"));
        let mut check = TargetCheck::open(&target)
            .expect("open")
            .expect("not empty");
        assert_eq!(check.node(&["sub", "f.txt"]).expect("check"), None);
        assert_eq!(check.node(&["self", "f.txt"]).expect("check"), None);
        assert_eq!(check.node(&["self"]).expect("check"), None);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_rs_03_a_file_with_another_name_on_a_written_path_is_refused() {
        let base = temp_dir("hardlink");
        let target = base.join("target");
        std::fs::create_dir_all(&target).expect("cannot create the target");
        std::fs::write(base.join("victim.txt"), b"OUTSIDE").expect("cannot write");
        std::fs::hard_link(base.join("victim.txt"), target.join("a.txt"))
            .expect("cannot create the hard link");
        std::fs::write(target.join("b.txt"), b"x").expect("cannot write");
        let mut check = TargetCheck::open(&target)
            .expect("open")
            .expect("not empty");
        assert_eq!(
            check.node(&["a.txt"]).expect("check"),
            Some(Refusal::HardLink(target.join("a.txt")))
        );
        assert_eq!(check.node(&["b.txt"]).expect("check"), None);
        let _ = std::fs::remove_dir_all(&base);
    }
}
