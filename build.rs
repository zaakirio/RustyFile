use std::path::Path;

fn main() {
    // When the embed-frontend feature is active and the dist directory doesn't
    // yet exist (clean checkout, before `pnpm build` has been run), create a
    // minimal placeholder so that `rust-embed` can compile successfully.
    // Production builds should run `pnpm build` first to replace this stub.
    if std::env::var("CARGO_FEATURE_EMBED_FRONTEND").is_ok() {
        let dist = Path::new("frontend/dist");
        if !dist.exists() {
            std::fs::create_dir_all(dist).expect("failed to create frontend/dist");
            std::fs::write(
                dist.join("index.html"),
                "<!DOCTYPE html><html><body>Frontend not built. \
                 Run: cd frontend &amp;&amp; pnpm build</body></html>",
            )
            .expect("failed to write frontend/dist/index.html stub");
        }
        // Re-run this script if the dist directory is removed.
        println!("cargo:rerun-if-changed=frontend/dist");
    }
}
