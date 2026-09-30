fn main() {
    if let Err(error) = slint_build::compile("ui/app-window.slint") {
        panic!("failed to compile the Slint UI: {error}");
    }
}
