fn main() {
    if let Err(error) = slint_build::compile("ui/app-window.slint") {
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
