use anyhow::Result;

fn main() -> Result<()> {
    if cfg!(target_os = "windows") {
        let mut res = winres::WindowsResource::new();
        res.set_icon("images/favicon.ico");
        res.compile()?;
    }
    Ok(())
}
