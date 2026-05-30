//! `agentfs explain` — Show whether a path is hidden and why.

use agentfs::fs::AgentFS;
use std::path::PathBuf;

/// Handle `agentfs explain <path>`.
///
/// Prints whether the given path is VISIBLE or HIDDEN, and if hidden,
/// which rule matched.
pub fn explain(path: PathBuf) {
    let root = std::env::current_dir().unwrap();
    let abs = if path.is_absolute() {
        path.clone()
    } else {
        root.join(&path)
    };
    let canonical = abs.canonicalize().unwrap_or(abs);
    let agentfs = AgentFS::new(root.clone());
    if agentfs.is_hidden(&canonical) {
        let mut builder = ignore::gitignore::GitignoreBuilder::new(&root);
        let _ = builder.add(root.join(".agentignore"));
        if let Ok(matcher) = builder.build() {
            let m = matcher.matched_path_or_any_parents(&canonical, canonical.is_dir());
            println!("HIDDEN");
            println!("canonical path: {:?}", canonical);
            if let ignore::Match::Ignore(glob) = m {
                println!("matched rule:   {}", glob.original());
            }
        }
    } else {
        println!("VISIBLE");
        println!("canonical path: {:?}", canonical);
    }
}
