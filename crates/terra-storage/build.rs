// `sqlx::migrate!` embeds the migrations at compile time, but the compiler
// does not watch the directory, so a new migration file would not trigger a
// rebuild without this.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
