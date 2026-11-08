// `sqlx::migrate!` embeds the migrations, but on stable Rust cargo does not know it read them: a new
// one would be missing from incremental builds (as `sqlx migrate build-script` sets up).
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
