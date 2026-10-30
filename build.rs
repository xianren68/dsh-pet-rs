fn main() {
    #[cfg(windows)]
    {
        // app.rc 里只有一行 `1 24 "app.manifest"`，把清单嵌成 RT_MANIFEST；
        // 清单里声明了 comctl32 v6，设置面板的按钮才会有现代外观。
        // 2.x 的签名是 (file, &[&str])：传空参数表，等价于 3.x 的 NONE。
        // 3.x 依赖的 toml 1.x 用 edition 2024，cargo 1.77 编不动，所以这条分支锁 2.x。
        let _ = embed_resource::compile("app.rc", &[] as &[&str]);
        println!("cargo:rerun-if-changed=app.manifest");
        println!("cargo:rerun-if-changed=app.rc");
    }
}
