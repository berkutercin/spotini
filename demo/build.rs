fn main() {
    // The app's own screens.
    slint_build::compile_with_config(
        "../app/ui/app.slint",
        slint_build::CompilerConfiguration::new().with_style("fluent-dark".into()),
    )
    .expect("the UI should compile");
}
