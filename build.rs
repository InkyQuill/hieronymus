use std::path::Path;

fn main() {
    for input in [
        "../../frontend/bun.lock",
        "../../frontend/index.html",
        "../../frontend/package.json",
        "../../frontend/src",
        "../../frontend/tsconfig.json",
        "../../frontend/tsconfig.test.json",
        "../../frontend/vite.config.ts",
        "../../frontend/vitest.config.ts",
    ] {
        println!("cargo::rerun-if-changed={input}");
    }
    if !Path::new("../../frontend/dist").is_dir() {
        println!(
            "cargo::warning=frontend/dist is absent; build the console before producing a release binary"
        );
    }
}
