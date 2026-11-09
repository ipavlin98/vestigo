use std::env::var;

const RESOURCES: &str = "resources/app.rc";
const ICON: &str = "resources/icon.ico";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed={RESOURCES}");
    println!("cargo:rerun-if-changed={ICON}");
    let numbers = format!(
        "VERSION_NUMBERS={},{},{},0",
        var("CARGO_PKG_VERSION_MAJOR")?,
        var("CARGO_PKG_VERSION_MINOR")?,
        var("CARGO_PKG_VERSION_PATCH")?
    );
    let text = format!("VERSION_TEXT=\"{}\"", var("CARGO_PKG_VERSION")?);
    embed_resource::compile(RESOURCES, [numbers, text]).manifest_required()?;
    Ok(())
}
