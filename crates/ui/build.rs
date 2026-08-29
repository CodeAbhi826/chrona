fn main() {
    // Material is the default widget style; the whole custom UI follows the
    // Material You palette in ui/theme.slint either way.
    std::env::set_var("SLINT_STYLE", "material");
    slint_build::compile("ui/app.slint").unwrap();
}
