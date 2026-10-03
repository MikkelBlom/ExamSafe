fn main() {
    // Element debug info lets the headless UI tests find elements; release builds skip it.
    let debug_build = std::env::var("PROFILE").is_ok_and(|profile| profile == "debug");
    let config = slint_build::CompilerConfiguration::new().with_debug_info(debug_build);
    if let Err(error) = slint_build::compile_with_config("ui/app-window.slint", config) {
        panic!("failed to compile the Slint UI: {error}");
    }
    #[cfg(windows)]
    embed_windows_resources();
}

/// Icon and version info shown by Explorer and Task Manager. The description is deliberately
/// plain: monitoring tools list processes by name and description, and ExamSafe has nothing to
/// hide.
#[cfg(windows)]
fn embed_windows_resources() {
    println!("cargo:rerun-if-changed=../../assets/icon.ico");
    let mut resources = winresource::WindowsResource::new();
    resources
        .set_icon("../../assets/icon.ico")
        .set("ProductName", "ExamSafe")
        .set("FileDescription", "ExamSafe - prepares this PC for an exam")
        .set(
            "LegalCopyright",
            "(c) 2026 Mikkel Blom. All rights reserved.",
        );
    if let Err(error) = resources.compile() {
        panic!("failed to embed Windows resources: {error}");
    }
}
