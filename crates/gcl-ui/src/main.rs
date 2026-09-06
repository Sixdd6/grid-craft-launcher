//! Desktop UI entry point. Core logic stays in `gcl-core`; this crate only renders and forwards events.

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    let window = AppWindow::new()?;
    window.set_version(gcl_core::VERSION.into());
    window.run()
}
