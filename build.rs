fn main() {
    #[cfg(windows)]
    {
        // app.rc 里只有一行 `1 24 "app.manifest"`，把清单嵌成 RT_MANIFEST；
        // 清单里声明了 comctl32 v6，设置面板的按钮才会有现代外观。
        let _ = embed_resource::compile("app.rc", embed_resource::NONE);
        println!("cargo:rerun-if-changed=app.manifest");
        println!("cargo:rerun-if-changed=app.rc");
    }
}
