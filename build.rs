fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("resources/icon/writer.ico");
        if let Err(e) = res.compile() {
            eprintln!("cargo:warning=Failed to compile Windows resource: {e}");
        }
    }
}
