//! Brand-specific data paths, with a one-time copy of previous user data.
use std::path::{Path, PathBuf};

/// Existing TraceReel files take priority. Legacy files remain untouched.
pub fn migrated_dir(base: &Path, name: &str, legacy_name: &str) -> PathBuf {
    let destination = base.join(name);
    let source = base.join(legacy_name);
    let marker = destination.join(".legacy-brand-migrated");
    if source.is_dir() && !marker.exists() {
        match copy_missing(&source, &destination) {
            Ok(()) => {
                if let Err(error) = std::fs::write(&marker, b"1\n") {
                    eprintln!("TraceReel data migration marker: {error}");
                }
            }
            Err(error) => eprintln!("TraceReel data migration: {error}"),
        }
    }
    destination
}

pub fn data_dir(environment: &str) -> PathBuf {
    let base = std::env::var_os(environment)
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    migrated_dir(&base, "TraceReel", "OrientView")
}

fn copy_missing(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if kind.is_dir() {
            copy_missing(&entry.path(), &target)?;
        } else if kind.is_file() {
            let mut input = std::fs::File::open(entry.path())?;
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
            {
                Ok(mut output) => {
                    if let Err(error) = std::io::copy(&mut input, &mut output) {
                        drop(output);
                        let _ = std::fs::remove_file(&target);
                        return Err(error);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_preserves_existing_files_and_does_not_restore_deleted_templates() {
        let root =
            std::env::temp_dir().join(format!("tracereel-migration-test-{}", std::process::id()));
        let legacy = root.join("Legacy");
        let current = root.join("Current");
        std::fs::create_dir_all(legacy.join("hud-templates")).unwrap();
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(legacy.join("export-queue.json"), "old queue").unwrap();
        std::fs::write(current.join("export-queue.json"), "new queue").unwrap();
        std::fs::write(legacy.join("hud-templates/example.json"), "template").unwrap();
        assert_eq!(migrated_dir(&root, "Current", "Legacy"), current);
        assert_eq!(
            std::fs::read_to_string(current.join("export-queue.json")).unwrap(),
            "new queue"
        );
        assert_eq!(
            std::fs::read_to_string(current.join("hud-templates/example.json")).unwrap(),
            "template"
        );
        assert!(legacy.join("hud-templates/example.json").is_file());
        std::fs::remove_file(current.join("hud-templates/example.json")).unwrap();
        migrated_dir(&root, "Current", "Legacy");
        assert!(!current.join("hud-templates/example.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
