fn main() {
    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/app.ico");
        resource.set("ProductName", "Binance Tools");
        resource.set("FileDescription", "Binance Tools Desktop");
        resource.set("CompanyName", "Binance Tools");
        resource.compile().expect("failed to embed Windows icon");
    }
}
