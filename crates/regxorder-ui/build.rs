fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("native".into());
    slint_build::compile_with_config("ui/app-window.slint", config)
        .expect("failed to compile the Slint desktop shell");
}
