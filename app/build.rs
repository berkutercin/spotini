fn main() {
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new().with_style("fluent-dark".into()),
    ).expect("the UI should compile");
    // The icon (for the window, taskbar and tray) and a manifest declaring
    // Windows 10, which the mini player's see-through taskbar window needs.
    let mut resources = tauri_winres::WindowsResource::new();
    resources.set_icon_with_id("icon.ico", "1");
    resources.set_manifest(include_str!("windows-app-manifest.xml"));
    resources.compile().expect("the Windows resources should compile");
}
