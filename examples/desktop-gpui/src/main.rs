#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod theme;
mod ui;

fn main() {
    set_working_directory_to_exe_dir();
    app::run();
}

fn set_working_directory_to_exe_dir() {
    let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
    else {
        return;
    };

    let working_dir = development_workspace_dir(&exe_dir).unwrap_or(exe_dir);

    if let Err(err) = std::env::set_current_dir(&working_dir) {
        eprintln!(
            "failed to set working directory to executable directory {}: {err}",
            working_dir.display()
        );
    }
}

fn development_workspace_dir(exe_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let exe_dir_text = exe_dir.to_string_lossy().replace('\\', "/");
    if !exe_dir_text.contains("/target/debug") && !exe_dir_text.contains("/target/release") {
        return None;
    }

    let workspace_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    workspace_dir
        .canonicalize()
        .ok()
        .filter(|path| path.join("Cargo.toml").exists())
}
