//! Rebuild the crate when a migration is added or changed, because `sqlx::migrate!` embeds them.

fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
