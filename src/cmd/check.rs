//! `agentfs check` — Validate the `.agentignore` in the current directory.

/// Handle `agentfs check`.
///
/// Validates that `.agentignore` in the current directory has valid gitignore
/// syntax. Exits with code 1 on errors.
pub fn check() {
    let root = std::env::current_dir().unwrap();
    let agentignore = root.join(".agentignore");
    if !agentignore.exists() {
        println!("No .agentignore found in {:?}", root);
        return;
    }
    let mut builder = ignore::gitignore::GitignoreBuilder::new(&root);
    if let Some(err) = builder.add(&agentignore) {
        eprintln!("Error: {}", err);
        std::process::exit(1);
    }
    match builder.build() {
        Ok(_) => println!("✓ .agentignore is valid"),
        Err(e) => {
            eprintln!("✗ .agentignore has errors: {}", e);
            std::process::exit(1);
        }
    }
}
